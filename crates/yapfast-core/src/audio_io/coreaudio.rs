//! Sound through Audio Queue Services. A queue converts between a stream's
//! format and the device's, and hands its buffers back on a thread of its
//! own as it plays or records them.
//!
//! Speakers queue in the queue's own buffers. Writing fills a free buffer and
//! gives it to the queue, and waits while the queue has them all. The queue
//! starts once it has them all, or once draining, and pauses when it hands
//! the last back: a late write doesn't break up into gaps, as with
//! PulseAudio's prebuffering, and an idle stream lets the device rest.
//! Microphones' buffers go back to the queue as soon as they're recorded,
//! their samples into a ring that reading takes from.
//!
//! Each stream names its device to its queue, so one on the default device
//! follows the default by hand, as PulseAudio moves its streams: writing and
//! reading look at the default now and then, and when it's another device,
//! the stream opens a new queue there. A stream whose queue stops handing
//! buffers back, as when its device goes away, opens a new one too, on its
//! device if it's still there and on the default if not.
//!
//! LiveKit's macOS device module names each device by its Core Audio object
//! id, and the default one "default".

use std::{
    collections::VecDeque,
    ffi::c_void,
    mem::offset_of,
    ptr::{NonNull, null, null_mut},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail, ensure};
use objc2_audio_toolbox::{
    AudioQueueAllocateBuffer, AudioQueueBufferRef, AudioQueueDispose, AudioQueueEnqueueBuffer,
    AudioQueueNewInput, AudioQueueNewOutput, AudioQueuePause, AudioQueueRef, AudioQueueSetProperty,
    AudioQueueStart, kAudioQueueProperty_CurrentDevice,
};
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyScope, AudioObjectPropertySelector,
    kAudioDevicePropertyBufferFrameSize, kAudioDevicePropertyDeviceUID,
    kAudioDevicePropertyLatency, kAudioDevicePropertyNominalSampleRate,
    kAudioDevicePropertySafetyOffset, kAudioDevicePropertyStreamConfiguration,
    kAudioDevicePropertyStreams, kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeInput, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
    kAudioObjectUnknown, kAudioStreamPropertyLatency,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioStreamPacketDescription,
    AudioTimeStamp, kAudioFormatFlagIsFloat, kAudioFormatFlagIsPacked,
    kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
};
use objc2_core_foundation::{CFRetained, CFString};

use super::Format;
use crate::voice::NAMED;

/// What a stream without a queue of its own keeps queued.
const DEFAULT_QUEUE: Duration = Duration::from_millis(200);
/// What each of a speakers' buffers holds; they have as many as their queue
/// takes.
const BUFFER: Duration = Duration::from_millis(10);
/// The least a microphone's ring holds, so a reader that's a little late
/// loses nothing.
const RECORDING: Duration = Duration::from_millis(200);
/// What a microphone's queue can record before its callback runs, in
/// buffers of the fragment asked for, at least four: a late callback loses
/// nothing either.
const RECORDING_AHEAD: Duration = Duration::from_millis(100);
/// A running queue hands a buffer back every few milliseconds; this long
/// without one, its device stopped.
const SILENCE: Duration = Duration::from_secs(2);
/// The first buffer after a queue starts may take longer: a Bluetooth device
/// wakes, or macOS asks the person whether Yapfast may use the microphone.
const STARTING: Duration = Duration::from_secs(30);
/// How often a stream on the default device looks whether the default
/// changed, and how long it waits between looks.
const LOOK: Duration = Duration::from_millis(250);

/// One of a queue's buffers.
struct Buffer(AudioQueueBufferRef);

// SAFETY: a buffer is only ever touched by whoever holds it: the queue, or
// the stream under its lock.
unsafe impl Send for Buffer {}

/// What a stream and its queue's callback share.
struct Shared {
    state: Mutex<State>,
    /// Signalled whenever the callback hands a buffer back.
    changed: Condvar,
}

#[derive(Default)]
struct State {
    /// Buffers the queue handed back, to tell a stalled device.
    returned: u64,
    /// A buffer couldn't go back to the queue: it needs a new one.
    failed: bool,
    /// Speakers' buffers the queue doesn't have; writing fills the last.
    free: VecDeque<Buffer>,
    /// The bytes written into the last free buffer.
    filled: usize,
    /// The bytes in the buffers the queue has.
    queued: usize,
    running: bool,
    draining: bool,
    /// What the microphone recorded and reading hasn't taken.
    ring: VecDeque<u8>,
    /// The most the ring holds, in bytes.
    capacity: usize,
}

impl Shared {
    fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                ring: VecDeque::with_capacity(capacity),
                capacity,
                ..State::default()
            }),
            changed: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// An audio queue, disposed of with its buffers when dropped.
struct Queue(AudioQueueRef);

// SAFETY: Audio Queue Services take calls on a queue from any thread.
unsafe impl Send for Queue {}

impl Drop for Queue {
    fn drop(&mut self) {
        // SAFETY: disposing immediately returns once the callback is done,
        // and never calls it again, so what it reads may go after this.
        unsafe { AudioQueueDispose(self.0, true) };
    }
}

fn check(status: i32, error: &'static str) -> Result<()> {
    ensure!(status == 0, "{error} (Core Audio status {status})");
    Ok(())
}

/// Packed, interleaved samples in the machine's own byte order.
fn description(format: Format) -> AudioStreamBasicDescription {
    let frame = format.frame_bytes() as u32;
    AudioStreamBasicDescription {
        mSampleRate: f64::from(format.rate),
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsPacked
            | if format.float {
                kAudioFormatFlagIsFloat
            } else {
                kAudioFormatFlagIsSignedInteger
            },
        mBytesPerPacket: frame,
        mFramesPerPacket: 1,
        mBytesPerFrame: frame,
        mChannelsPerFrame: u32::from(format.channels),
        mBitsPerChannel: if format.float { 32 } else { 16 },
        mReserved: 0,
    }
}

/// The bytes of whole frames in `duration`, at least one frame's.
fn frames(format: Format, duration: Duration) -> usize {
    format.bytes(duration).max(format.frame_bytes())
}

/// A queue on a device, and what's needed to make it again on another.
struct Stream {
    // Disposed of first, so the callback is done with `shared`.
    queue: Option<Queue>,
    shared: Arc<Shared>,
    format: Format,
    scope: AudioObjectPropertyScope,
    /// The device asked for, `None` for the default.
    asked: Option<String>,
    /// The device the queue is on, and whether it's there as the default.
    device: AudioObjectID,
    follows: bool,
    /// The bytes each buffer holds, and how many there are.
    size: usize,
    count: usize,
    /// How long the device takes to play or record, beyond the queue.
    latency: Duration,
    /// When the default was last looked at.
    looked: Instant,
    /// When the queue last started or handed a buffer back, with the count
    /// of buffers back then, and whether one came back since it started.
    progress: Instant,
    seen: u64,
    fresh: bool,
    /// The queue was made again for stalling, and hasn't handed one back.
    retried: bool,
}

impl Stream {
    fn open(
        id: Option<&str>,
        scope: AudioObjectPropertyScope,
        format: Format,
        size: usize,
        count: usize,
        shared: Arc<Shared>,
    ) -> Result<Self> {
        let (device, follows) = find(id, scope)?;
        let mut stream = Self {
            queue: None,
            shared,
            format,
            scope,
            asked: id.map(str::to_owned),
            device,
            follows,
            size,
            count,
            latency: Duration::ZERO,
            looked: Instant::now(),
            progress: Instant::now(),
            seen: 0,
            fresh: true,
            retried: false,
        };
        stream.make(device)?;
        Ok(stream)
    }

    fn input(&self) -> bool {
        self.scope == kAudioObjectPropertyScopeInput
    }

    fn queue(&self) -> AudioQueueRef {
        self.queue.as_ref().map_or(null_mut(), |queue| queue.0)
    }

    /// A queue on `device` in place of the one before, whose buffers and
    /// what they held go with it. A microphone's starts recording; speakers'
    /// buffers wait to be written.
    fn make(&mut self, device: AudioObjectID) -> Result<()> {
        self.shared.lock().running = false;
        // Disposing waits for the callback, which takes the lock.
        self.queue = None;
        {
            let mut state = self.shared.lock();
            state.free.clear();
            state.filled = 0;
            state.queued = 0;
            state.draining = false;
            state.failed = false;
        }
        let uid = string(device, kAudioDevicePropertyDeviceUID).context("device not found")?;
        let mut queue: AudioQueueRef = null_mut();
        let shared = Arc::as_ptr(&self.shared).cast_mut().cast();
        let description = description(self.format);
        // SAFETY: `shared` outlives the queue, which `Stream` disposes of
        // first.
        let made = unsafe {
            if self.input() {
                AudioQueueNewInput(
                    NonNull::from(&description),
                    Some(record),
                    shared,
                    None,
                    None,
                    0,
                    NonNull::from(&mut queue),
                )
            } else {
                AudioQueueNewOutput(
                    NonNull::from(&description),
                    Some(play),
                    shared,
                    None,
                    None,
                    0,
                    NonNull::from(&mut queue),
                )
            }
        };
        check(made, "the device refused the stream's format")?;
        let queue = Queue(queue);
        let uid: *const CFString = &*uid;
        // SAFETY: the property takes a CFStringRef, which the queue retains.
        check(
            unsafe {
                AudioQueueSetProperty(
                    queue.0,
                    kAudioQueueProperty_CurrentDevice,
                    NonNull::from(&uid).cast(),
                    size_of::<*const CFString>() as u32,
                )
            },
            "device not found",
        )?;
        let mut buffers = Vec::with_capacity(self.count);
        for _ in 0..self.count {
            let mut buffer: AudioQueueBufferRef = null_mut();
            // SAFETY: the queue owns the buffer it allocates, until disposed of.
            check(
                unsafe {
                    AudioQueueAllocateBuffer(queue.0, self.size as u32, NonNull::from(&mut buffer))
                },
                "no memory for sound",
            )?;
            buffers.push(Buffer(buffer));
        }
        self.queue = Some(queue);
        self.device = device;
        self.latency = device_latency(device, self.scope);
        if self.input() {
            for Buffer(buffer) in buffers {
                // SAFETY: a buffer of this queue's.
                check(
                    unsafe { AudioQueueEnqueueBuffer(self.queue(), buffer, 0, null()) },
                    "the microphone refused a buffer",
                )?;
            }
            self.start()?;
        } else {
            self.shared.lock().free.extend(buffers);
        }
        Ok(())
    }

    /// Starts the queue, which runs until paused.
    fn start(&mut self) -> Result<()> {
        // SAFETY: a queue with buffers in.
        check(
            unsafe { AudioQueueStart(self.queue(), null()) },
            "the device wouldn't start",
        )?;
        self.progress = Instant::now();
        self.seen = self.shared.lock().returned;
        self.fresh = true;
        Ok(())
    }

    /// A new queue: on the default device when following it, or when the
    /// device asked for went away, and on that device otherwise.
    fn reopen(&mut self, why: &'static str) -> Result<()> {
        let asked = if self.follows {
            None
        } else {
            self.asked.as_deref()
        };
        let (device, follows) = match find(asked, self.scope) {
            Ok(found) => found,
            Err(_) => find(None, self.scope)?,
        };
        tracing::info!(why, follows, input = self.input(), "sound opened again");
        self.follows = follows;
        self.make(device)
    }

    /// Follows the default device, and replaces a queue that failed.
    fn look(&mut self) -> Result<()> {
        if self.shared.lock().failed {
            return self.reopen("a buffer was refused");
        }
        if self.follows && self.looked.elapsed() >= LOOK {
            self.looked = Instant::now();
            if find(None, self.scope).is_ok_and(|(device, _)| device != self.device) {
                return self.reopen("the default device changed");
            }
        }
        Ok(())
    }

    /// Waits a little for the queue to hand a buffer back. A queue that
    /// stopped handing them back is made again, once; one that stops again
    /// before handing one back fails.
    fn wait(&mut self, state: MutexGuard<'_, State>, stopped: &'static str) -> Result<()> {
        let (state, _) = self
            .shared
            .changed
            .wait_timeout(state, LOOK)
            .unwrap_or_else(PoisonError::into_inner);
        let returned = state.returned;
        drop(state);
        if returned != self.seen {
            self.seen = returned;
            self.progress = Instant::now();
            self.fresh = false;
            self.retried = false;
            return Ok(());
        }
        let patience = if self.fresh { STARTING } else { SILENCE };
        if self.progress.elapsed() < patience {
            return Ok(());
        }
        ensure!(!self.retried, stopped);
        self.retried = true;
        self.reopen("the device stopped")
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.shared.lock().running = false;
        self.queue = None;
    }
}

pub struct Output {
    stream: Stream,
}

impl Output {
    pub fn open(
        speaker: Option<&str>,
        _name: &'static str,
        format: Format,
        queue: Option<Duration>,
    ) -> Result<Self> {
        let size = frames(format, BUFFER);
        let count = (format.bytes(queue.unwrap_or(DEFAULT_QUEUE)) / size).max(2);
        let stream = Stream::open(
            speaker,
            kAudioObjectPropertyScopeOutput,
            format,
            size,
            count,
            Shared::new(0),
        )
        .context("these speakers are no longer connected")?;
        Ok(Self { stream })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let frame = self.stream.format.frame_bytes();
        let mut rest = bytes;
        while rest.len() >= frame {
            self.stream.look()?;
            let shared = self.stream.shared.clone();
            let mut state = shared.lock();
            let Some(&Buffer(buffer)) = state.free.back() else {
                self.stream.wait(state, "the speakers stopped playing")?;
                continue;
            };
            // SAFETY: a buffer the queue doesn't have, which holds its
            // capacity in bytes, and which nothing else touches while the
            // lock is held.
            let data = unsafe {
                let buffer = &mut *buffer;
                std::slice::from_raw_parts_mut(
                    buffer.mAudioData.as_ptr().cast::<u8>(),
                    buffer.mAudioDataBytesCapacity as usize,
                )
            };
            let filled = state.filled;
            let count = (data.len() - filled).min(rest.len()) / frame * frame;
            data[filled..filled + count].copy_from_slice(&rest[..count]);
            rest = &rest[count..];
            state.filled += count;
            if data.len() - state.filled < frame {
                enqueue(self.stream.queue(), &mut state);
                // The queue has them all: it plays.
                if state.free.is_empty() && !state.running {
                    state.running = true;
                    drop(state);
                    self.stream.start()?;
                }
            }
        }
        Ok(())
    }

    pub fn drain(&mut self) -> Result<()> {
        let shared = self.stream.shared.clone();
        let deadline = {
            let mut state = shared.lock();
            if state.filled > 0 {
                enqueue(self.stream.queue(), &mut state);
            }
            if state.queued == 0 {
                return Ok(());
            }
            state.draining = true;
            let deadline = Instant::now()
                + self.duration(state.queued)
                + self.stream.latency
                + Duration::from_secs(1);
            if !state.running {
                state.running = true;
                drop(state);
                self.stream.start()?;
            }
            deadline
        };
        let drained = loop {
            if let Err(error) = self.stream.look() {
                break Err(error);
            }
            let state = shared.lock();
            if state.queued == 0 {
                break Ok(());
            }
            if Instant::now() >= deadline {
                break Err(anyhow::anyhow!("the speakers stopped playing"));
            }
            if let Err(error) = self.stream.wait(state, "the speakers stopped playing") {
                break Err(error);
            }
        };
        let mut state = shared.lock();
        state.draining = false;
        if state.running && state.queued == 0 {
            state.running = false;
            // SAFETY: the queue has no buffers, so its callback won't run.
            unsafe { AudioQueuePause(self.stream.queue()) };
        }
        drop(state);
        // The device plays what it took last.
        if drained.is_ok() {
            std::thread::sleep(self.stream.latency.min(Duration::from_millis(500)));
        }
        drained
    }

    /// Drops what's queued with the queue, for a new one on the same device.
    pub fn flush(&mut self) {
        let unplayed = {
            let state = self.stream.shared.lock();
            state.queued + state.filled > 0
        };
        if unplayed && let Err(error) = self.stream.make(self.stream.device) {
            tracing::warn!(error = format!("{error:#}"), "could not drop queued sound");
        }
    }

    pub fn latency(&self) -> Duration {
        let state = self.stream.shared.lock();
        self.duration(state.queued + state.filled) + self.stream.latency
    }

    fn duration(&self, bytes: usize) -> Duration {
        let format = self.stream.format;
        Duration::from_secs_f64(
            bytes as f64 / (f64::from(format.rate) * format.frame_bytes() as f64),
        )
    }
}

/// Gives the queue the last free buffer, with what was written into it.
fn enqueue(queue: AudioQueueRef, state: &mut State) {
    let Some(Buffer(buffer)) = state.free.pop_back() else {
        return;
    };
    let size = std::mem::take(&mut state.filled);
    // SAFETY: a buffer of this queue's, which it doesn't have, holding
    // `size` bytes of samples.
    let given = unsafe {
        (*buffer).mAudioDataByteSize = size as u32;
        AudioQueueEnqueueBuffer(queue, buffer, 0, null())
    };
    if given == 0 {
        state.queued += size;
    } else {
        state.free.push_back(Buffer(buffer));
        state.failed = true;
    }
}

/// Takes back a buffer the speakers' queue played. When it was the last, the
/// queue pauses until it has them all again.
unsafe extern "C-unwind" fn play(
    shared: *mut c_void,
    queue: AudioQueueRef,
    buffer: AudioQueueBufferRef,
) {
    // SAFETY: the stream's `Shared`, which outlives its queue, and a buffer
    // of that queue's, which it gives back.
    let (shared, played) = unsafe { (&*shared.cast::<Shared>(), (*buffer).mAudioDataByteSize) };
    let mut state = shared.lock();
    state.returned += 1;
    state.queued = state.queued.saturating_sub(played as usize);
    // The buffer being written stays last.
    state.free.push_front(Buffer(buffer));
    if state.queued == 0 && state.running && !state.draining {
        state.running = false;
        // SAFETY: the queue has no buffers left, so no callback is waiting
        // for the lock this holds.
        unsafe { AudioQueuePause(queue) };
    }
    drop(state);
    shared.changed.notify_all();
}

pub struct Input {
    stream: Stream,
}

impl Input {
    pub fn open(
        microphone: Option<&str>,
        _name: &'static str,
        format: Format,
        fragment: Duration,
    ) -> Result<Self> {
        let size = frames(format, fragment);
        let count = (frames(format, RECORDING_AHEAD) / size).max(4);
        let shared = Shared::new(frames(format, (fragment * 4).max(RECORDING)));
        let stream = Stream::open(
            microphone,
            kAudioObjectPropertyScopeInput,
            format,
            size,
            count,
            shared,
        )
        .context("this microphone is no longer connected")?;
        Ok(Self { stream })
    }

    pub fn read(&mut self, bytes: &mut [u8]) -> Result<()> {
        ensure!(
            bytes.len() <= self.stream.shared.lock().capacity,
            "reading more than the microphone keeps"
        );
        loop {
            self.stream.look()?;
            let shared = self.stream.shared.clone();
            let mut state = shared.lock();
            if state.ring.len() >= bytes.len() {
                let (front, back) = state.ring.as_slices();
                let from_front = front.len().min(bytes.len());
                bytes[..from_front].copy_from_slice(&front[..from_front]);
                let rest = bytes.len() - from_front;
                bytes[from_front..].copy_from_slice(&back[..rest]);
                state.ring.drain(..bytes.len());
                return Ok(());
            }
            self.stream
                .wait(state, "the microphone stopped recording")?;
        }
    }
}

/// Keeps what the microphone's queue recorded, and gives the buffer back.
/// A reader that fell behind loses the oldest of it.
unsafe extern "C-unwind" fn record(
    shared: *mut c_void,
    queue: AudioQueueRef,
    buffer: AudioQueueBufferRef,
    _: NonNull<AudioTimeStamp>,
    _: u32,
    _: *const AudioStreamPacketDescription,
) {
    // SAFETY: the stream's `Shared`, which outlives its queue, and a buffer
    // of that queue's holding what it recorded, which goes back after.
    let shared = unsafe { &*shared.cast::<Shared>() };
    let recorded = unsafe {
        std::slice::from_raw_parts(
            (*buffer).mAudioData.as_ptr().cast::<u8>(),
            (*buffer).mAudioDataByteSize as usize,
        )
    };
    let mut state = shared.lock();
    state.returned += 1;
    let over = (state.ring.len() + recorded.len())
        .saturating_sub(state.capacity)
        .min(state.ring.len());
    state.ring.drain(..over);
    state.ring.extend(recorded);
    // SAFETY: a buffer of this queue's. While the queue is being disposed
    // of, it's refused, and nothing reads `failed` after.
    if unsafe { AudioQueueEnqueueBuffer(queue, buffer, 0, null()) } != 0 {
        state.failed = true;
    }
    drop(state);
    shared.changed.notify_all();
}

/// The device behind a WebRTC device id that has streams in `scope`, and
/// whether it's there as the default; `None` is the default. An id made from
/// a name, or that is a device's UID, finds it too.
fn find(id: Option<&str>, scope: AudioObjectPropertyScope) -> Result<(AudioObjectID, bool)> {
    let id = id.filter(|id| {
        *id != "default"
            && !id
                .strip_prefix(NAMED)
                .is_some_and(|name| name.starts_with("default ("))
    });
    let Some(id) = id else {
        let selector = if scope == kAudioObjectPropertyScopeInput {
            kAudioHardwarePropertyDefaultInputDevice
        } else {
            kAudioHardwarePropertyDefaultOutputDevice
        };
        let device: AudioObjectID = number(
            kAudioObjectSystemObject as AudioObjectID,
            address(selector, kAudioObjectPropertyScopeGlobal),
        )
        .filter(|device| *device != kAudioObjectUnknown)
        .context("no sound device")?;
        return Ok((device, true));
    };
    let name = id.strip_prefix(NAMED);
    for device in devices()? {
        if !has_streams(device, scope) {
            continue;
        }
        let found = match name {
            Some(name) => string(device, kAudioObjectPropertyName)
                .is_ok_and(|found| found.to_string() == name),
            None => {
                id.parse() == Ok(device)
                    || string(device, kAudioDevicePropertyDeviceUID)
                        .is_ok_and(|uid| uid.to_string() == id)
            }
        };
        if found {
            return Ok((device, false));
        }
    }
    bail!("device not found")
}

fn address(
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// A property of a fixed size, such as a number.
fn number<T: Copy + Default>(
    object: AudioObjectID,
    address: AudioObjectPropertyAddress,
) -> Option<T> {
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    // SAFETY: a plain property query into storage of the size given.
    let got = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    (got == 0 && size as usize == size_of::<T>()).then_some(value)
}

/// A property of variable size, and its size in bytes, in storage aligned
/// for any of its parts.
fn data(object: AudioObjectID, address: AudioObjectPropertyAddress) -> Result<(Vec<u64>, usize)> {
    let mut size = 0u32;
    // SAFETY: plain property queries, into storage of the size given.
    unsafe {
        check(
            AudioObjectGetPropertyDataSize(
                object,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
            ),
            "no such property",
        )?;
        let mut storage = vec![0u64; (size as usize).div_ceil(8)];
        if storage.is_empty() {
            return Ok((storage, 0));
        }
        check(
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                NonNull::new_unchecked(storage.as_mut_ptr()).cast(),
            ),
            "no such property",
        )?;
        let size = (size as usize).min(storage.len() * 8);
        Ok((storage, size))
    }
}

/// The objects a property lists, such as devices or streams.
fn objects(
    object: AudioObjectID,
    address: AudioObjectPropertyAddress,
) -> Result<Vec<AudioObjectID>> {
    let (storage, size) = data(object, address)?;
    let bytes = storage.iter().flat_map(|word| word.to_ne_bytes());
    let bytes: Vec<u8> = bytes.take(size).collect();
    Ok(bytes
        .chunks_exact(size_of::<AudioObjectID>())
        .map(|id| AudioObjectID::from_ne_bytes(id.try_into().expect("four bytes")))
        .collect())
}

/// Every audio device on the system.
fn devices() -> Result<Vec<AudioObjectID>> {
    objects(
        kAudioObjectSystemObject as AudioObjectID,
        address(
            kAudioHardwarePropertyDevices,
            kAudioObjectPropertyScopeGlobal,
        ),
    )
    .context("could not list the sound devices")
}

/// Whether a device has channels in `scope`: input for a microphone,
/// output for speakers.
fn has_streams(device: AudioObjectID, scope: AudioObjectPropertyScope) -> bool {
    let Ok((storage, size)) = data(
        device,
        address(kAudioDevicePropertyStreamConfiguration, scope),
    ) else {
        return false;
    };
    // An AudioBufferList: how many buffers, then each.
    let header = offset_of!(AudioBufferList, mBuffers);
    if size < header {
        return false;
    }
    let list = storage.as_ptr().cast::<AudioBufferList>();
    // SAFETY: the storage holds the count, aligned for the whole list.
    let count = unsafe { (*list).mNumberBuffers } as usize;
    if count > (size - header) / size_of::<AudioBuffer>() {
        return false;
    }
    // SAFETY: the storage holds `count` buffers after the count, aligned
    // as they are.
    let buffers = unsafe {
        storage
            .as_ptr()
            .cast::<u8>()
            .add(header)
            .cast::<AudioBuffer>()
    };
    (0..count).any(|index| unsafe { (*buffers.add(index)).mNumberChannels } > 0)
}

/// How long a device takes to play or record what it's given, beyond the
/// queue: its own latency and its stream's, the offset it keeps from the
/// hardware, and its buffer, as Core Audio reports them. Bluetooth devices
/// report a long one.
fn device_latency(device: AudioObjectID, scope: AudioObjectPropertyScope) -> Duration {
    let Some(rate) = number::<f64>(
        device,
        address(
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
        ),
    )
    .filter(|rate| *rate > 0.) else {
        return Duration::ZERO;
    };
    let frames =
        |object, selector, scope| number::<u32>(object, address(selector, scope)).unwrap_or(0);
    let mut total = u64::from(frames(device, kAudioDevicePropertyLatency, scope))
        + u64::from(frames(device, kAudioDevicePropertySafetyOffset, scope))
        + u64::from(frames(
            device,
            kAudioDevicePropertyBufferFrameSize,
            kAudioObjectPropertyScopeGlobal,
        ));
    if let Ok(streams) = objects(device, address(kAudioDevicePropertyStreams, scope))
        && let Some(&stream) = streams.first()
    {
        total += u64::from(frames(
            stream,
            kAudioStreamPropertyLatency,
            kAudioObjectPropertyScopeGlobal,
        ));
    }
    Duration::from_secs_f64(total as f64 / rate)
}

/// A device's property that is a string.
fn string(
    device: AudioObjectID,
    selector: AudioObjectPropertySelector,
) -> Result<CFRetained<CFString>> {
    let address = address(selector, kAudioObjectPropertyScopeGlobal);
    let mut value: *const CFString = null();
    let mut size = size_of::<*const CFString>() as u32;
    // SAFETY: the property is a CFStringRef, which the caller releases.
    unsafe {
        check(
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                NonNull::from(&mut value).cast(),
            ),
            "no such property",
        )?;
        Ok(CFRetained::from_raw(
            NonNull::new(value.cast_mut()).context("no such property")?,
        ))
    }
}

#[cfg(feature = "discord")]
pub fn others(
    _: fn(&str) -> bool,
    _: super::Hear,
    _: Arc<std::sync::atomic::AtomicBool>,
) -> Result<std::thread::JoinHandle<()>> {
    bail!("Yapfast shares sound on Linux and Windows only for now")
}
