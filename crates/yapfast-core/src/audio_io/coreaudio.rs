//! Sound through Audio Queue Services. A queue converts between a stream's
//! format and the device's, and hands its buffers back on a thread of its
//! own as it plays or records them.
//!
//! Speakers queue in the queue's own buffers. Writing fills a free buffer and
//! gives it to the queue, and waits while the queue has them all. After the
//! queue runs dry, and before it first plays, filled buffers wait until all
//! are, then go together, so a late write doesn't break up into gaps, as with
//! PulseAudio's prebuffering; the queue keeps running meanwhile, so playing
//! again costs no restart. Draining pauses it, so the device may rest.
//! Microphones' buffers go back to the queue as soon as they're recorded,
//! their samples into a ring that reading takes from.
//!
//! Each stream names its device to its queue, so one on the default device
//! follows the default by hand, as PulseAudio moves its streams: writing and
//! reading look at the default now and then, and when it's another device,
//! the stream makes a queue there, and swaps it in once it's made. A stream
//! whose device is gone, or whose queue stops handing buffers back, makes a
//! new one too, on its device if it's still there and on the default if not.
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
    kAudioDevicePropertyBufferFrameSize, kAudioDevicePropertyDeviceIsAlive,
    kAudioDevicePropertyDeviceUID, kAudioDevicePropertyLatency,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertySafetyOffset,
    kAudioDevicePropertyStreamConfiguration, kAudioDevicePropertyStreams,
    kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMain, kAudioObjectPropertyName,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeInput,
    kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, kAudioObjectUnknown,
    kAudioStreamPropertyLatency,
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
/// The first buffer after speakers start may take longer, as a Bluetooth
/// device wakes or changes profile.
const SPEAKERS_STARTING: Duration = Duration::from_secs(5);
/// A microphone's first buffer may take longer still: the first time, macOS
/// asks the person whether Yapfast may use it.
const MICROPHONE_STARTING: Duration = Duration::from_secs(30);
/// How often a stream looks whether its device is gone or the default
/// changed, and how long it waits between looks.
const LOOK: Duration = Duration::from_millis(250);
/// After a new queue couldn't be made, how long the stream keeps the one it
/// has before trying again.
const RETRY: Duration = Duration::from_secs(1);

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
    /// The queue in use. A queue being disposed of may still call back, and
    /// is ignored.
    queue: usize,
    /// Buffers the queue handed back, to tell a stalled device.
    returned: u64,
    /// A buffer couldn't go back to the queue: it needs a new one.
    failed: bool,
    /// Speakers' buffers that are empty; writing fills the last.
    free: VecDeque<Buffer>,
    /// The bytes written into the last free buffer.
    filled: usize,
    /// Speakers' buffers filled while waiting to fill them all.
    ready: Vec<Buffer>,
    waiting: bool,
    /// The bytes in the buffers the queue has.
    queued: usize,
    /// The queue was started and isn't paused.
    running: bool,
    draining: bool,
    /// What the microphone recorded and reading hasn't taken.
    ring: VecDeque<u8>,
    /// The most the ring holds, in bytes.
    capacity: usize,
}

impl State {
    fn current(&self, queue: AudioQueueRef) -> bool {
        self.queue == queue as usize
    }
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
    /// When the device was last looked at, and when a queue that couldn't be
    /// made may be tried again.
    looked: Instant,
    retry: Option<Instant>,
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
            retry: None,
            progress: Instant::now(),
            seen: 0,
            fresh: true,
            retried: false,
        };
        stream.make(device, follows)?;
        Ok(stream)
    }

    fn input(&self) -> bool {
        self.scope == kAudioObjectPropertyScopeInput
    }

    fn queue(&self) -> AudioQueueRef {
        self.queue.as_ref().map_or(null_mut(), |queue| queue.0)
    }

    /// A queue on `device`, with its buffers, made before the one in use
    /// goes, so a device that won't take one leaves the stream as it was.
    /// Swapped in, it takes the place of the one before, whose buffers and
    /// what they held go with it. A microphone's starts recording; speakers'
    /// buffers wait to be written.
    fn make(&mut self, device: AudioObjectID, follows: bool) -> Result<()> {
        let uid = string(device, kAudioDevicePropertyDeviceUID).context("device not found")?;
        let mut made: AudioQueueRef = null_mut();
        let shared = Arc::as_ptr(&self.shared).cast_mut().cast();
        let description = description(self.format);
        // SAFETY: `shared` outlives the queue, which `Stream` disposes of
        // first.
        let status = unsafe {
            if self.input() {
                AudioQueueNewInput(
                    NonNull::from(&description),
                    Some(record),
                    shared,
                    None,
                    None,
                    0,
                    NonNull::from(&mut made),
                )
            } else {
                AudioQueueNewOutput(
                    NonNull::from(&description),
                    Some(play),
                    shared,
                    None,
                    None,
                    0,
                    NonNull::from(&mut made),
                )
            }
        };
        check(status, "the device refused the stream's format")?;
        let queue = Queue(made);
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
        if self.input() {
            for Buffer(buffer) in buffers.drain(..) {
                // SAFETY: a buffer of this queue's, which records nothing
                // and calls back for nothing until started.
                check(
                    unsafe { AudioQueueEnqueueBuffer(made, buffer, 0, null()) },
                    "the microphone refused a buffer",
                )?;
            }
        }
        {
            let mut state = self.shared.lock();
            state.queue = made as usize;
            state.free.clear();
            state.ready.clear();
            state.filled = 0;
            state.queued = 0;
            state.running = false;
            state.draining = false;
            state.failed = false;
            if self.input() {
                state.ring.clear();
            } else {
                state.free.extend(buffers.drain(..));
                state.waiting = true;
            }
        }
        // Disposing of the old queue waits for its callback, which takes the
        // lock, and is ignored from here on.
        self.queue = Some(queue);
        self.device = device;
        self.follows = follows;
        self.latency = device_latency(device, self.scope);
        self.retry = None;
        if self.input() {
            self.shared.lock().running = true;
            if let Err(error) = self.start() {
                // Swapped in already: it's tried again in a moment.
                let mut state = self.shared.lock();
                state.running = false;
                state.failed = true;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Starts the queue, which runs until paused. `running` is already set.
    fn start(&mut self) -> Result<()> {
        // SAFETY: a queue of this stream's.
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
        self.make(device, follows)
    }

    /// Follows the default device, and replaces a queue whose device is gone
    /// or that refused a buffer. A new queue that can't be made yet leaves
    /// the old one in use, to try again in a moment.
    fn look(&mut self) {
        if self.retry.is_some_and(|retry| Instant::now() < retry)
            || self.looked.elapsed() < LOOK && !self.shared.lock().failed
        {
            return;
        }
        self.looked = Instant::now();
        let why = if self.shared.lock().failed {
            "a buffer was refused"
        } else if !alive(self.device) {
            "the device went away"
        } else if self.follows
            && find(None, self.scope).is_ok_and(|(device, _)| device != self.device)
        {
            "the default device changed"
        } else {
            return;
        };
        if let Err(error) = self.reopen(why) {
            tracing::warn!(error = format!("{error:#}"), why, "sound couldn't move yet");
            self.retry = Some(Instant::now() + RETRY);
        }
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
        if self.progress.elapsed() < self.patience() {
            return Ok(());
        }
        ensure!(!self.retried, stopped);
        self.retried = true;
        self.reopen("the device stopped")
    }

    /// How long the queue may go without handing a buffer back.
    fn patience(&self) -> Duration {
        match (self.fresh, self.input()) {
            (false, _) => SILENCE,
            (true, false) => SPEAKERS_STARTING,
            (true, true) => MICROPHONE_STARTING,
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
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
        let shared = self.stream.shared.clone();
        let mut rest = bytes;
        while rest.len() >= frame {
            self.stream.look();
            let mut state = shared.lock();
            // A queue that failed waits for look() to replace it, rather
            // than taking what's written and losing it.
            let free = state.free.back().filter(|_| !state.failed);
            let Some(&Buffer(buffer)) = free else {
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
            if data.len() - state.filled >= frame {
                continue;
            }
            let full = take_filled(&mut state);
            let again = state.waiting;
            let given = if state.waiting {
                // Until they're all full, or what's written drains.
                state.ready.extend(full);
                if !state.free.is_empty() {
                    continue;
                }
                state.waiting = false;
                std::mem::take(&mut state.ready)
            } else {
                full.into_iter().collect()
            };
            drop(state);
            self.give(given, again);
        }
        Ok(())
    }

    /// Gives the queue filled buffers, starting it if it isn't running, or
    /// `again`, after it ran dry, in case the system stopped it meanwhile.
    fn give(&mut self, buffers: Vec<Buffer>, again: bool) {
        let queue = self.stream.queue();
        let start = {
            let mut state = self.stream.shared.lock();
            for Buffer(buffer) in &buffers {
                // SAFETY: a buffer of this queue's, which it doesn't have yet.
                state.queued += unsafe { (**buffer).mAudioDataByteSize } as usize;
            }
            !std::mem::replace(&mut state.running, true) || again
        };
        // Outside the lock, which the queue's callback takes: an enqueue
        // that waited for the callback would wait for this.
        for Buffer(buffer) in buffers {
            // SAFETY: a filled buffer of this queue's.
            if unsafe { AudioQueueEnqueueBuffer(queue, buffer, 0, null()) } != 0 {
                let mut state = self.stream.shared.lock();
                if state.current(queue) {
                    // SAFETY: as above; the queue refused it.
                    let size = unsafe { (*buffer).mAudioDataByteSize } as usize;
                    state.queued -= size;
                    state.free.push_front(Buffer(buffer));
                    state.failed = true;
                }
            }
        }
        if start && let Err(error) = self.stream.start() {
            // The next look makes a new queue.
            tracing::warn!(error = format!("{error:#}"), "the speakers wouldn't start");
            let mut state = self.stream.shared.lock();
            if state.current(queue) {
                state.running = false;
                state.failed = true;
            }
        }
    }

    pub fn drain(&mut self) -> Result<()> {
        let shared = self.stream.shared.clone();
        let (given, started, again) = {
            let mut state = shared.lock();
            let mut given = std::mem::take(&mut state.ready);
            given.extend(take_filled(&mut state));
            if given.is_empty() && state.queued == 0 {
                if state.running && state.current(self.stream.queue()) {
                    state.running = false;
                    // SAFETY: the queue has no buffers, so its callback
                    // won't run.
                    unsafe { AudioQueuePause(self.stream.queue()) };
                }
                return Ok(());
            }
            let again = std::mem::replace(&mut state.waiting, false);
            state.draining = true;
            (given, !state.running, again)
        };
        self.give(given, again);
        let drained = self.played(started);
        // The device plays what it took last before the queue pauses.
        if drained.is_ok() {
            std::thread::sleep(self.stream.latency.min(Duration::from_millis(500)));
        }
        let mut state = shared.lock();
        state.draining = false;
        state.waiting = true;
        if state.running && state.queued == 0 && state.current(self.stream.queue()) {
            state.running = false;
            // SAFETY: the queue has no buffers, so its callback won't run.
            unsafe { AudioQueuePause(self.stream.queue()) };
        }
        drained
    }

    /// Waits until the queue has played everything it has.
    fn played(&mut self, started: bool) -> Result<()> {
        let shared = self.stream.shared.clone();
        // A queue just started may take a moment to start playing.
        let starting = if started || self.stream.fresh {
            SPEAKERS_STARTING
        } else {
            Duration::ZERO
        };
        let deadline = Instant::now()
            + self.duration(shared.lock().queued)
            + self.stream.latency
            + starting
            + Duration::from_secs(1);
        loop {
            self.stream.look();
            let state = shared.lock();
            if state.queued == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("the speakers stopped playing");
            }
            self.stream.wait(state, "the speakers stopped playing")?;
        }
    }

    /// Drops what's queued with the queue, for a new one on the same device.
    pub fn flush(&mut self) {
        let unplayed = {
            let state = self.stream.shared.lock();
            state.queued + state.filled > 0 || !state.ready.is_empty()
        };
        if unplayed && let Err(error) = self.stream.make(self.stream.device, self.stream.follows) {
            tracing::warn!(error = format!("{error:#}"), "could not drop queued sound");
            let mut state = self.stream.shared.lock();
            state.filled = 0;
            let ready = std::mem::take(&mut state.ready);
            state.free.extend(ready);
        }
    }

    pub fn latency(&self) -> Duration {
        let state = self.stream.shared.lock();
        let ready = state.ready.len() * self.stream.size;
        self.duration(state.queued + ready + state.filled) + self.stream.latency
    }

    fn duration(&self, bytes: usize) -> Duration {
        let format = self.stream.format;
        Duration::from_secs_f64(
            bytes as f64 / (f64::from(format.rate) * format.frame_bytes() as f64),
        )
    }
}

/// Takes the last free buffer, with what was written into it, if anything was.
fn take_filled(state: &mut State) -> Option<Buffer> {
    let size = std::mem::take(&mut state.filled);
    if size == 0 {
        return None;
    }
    let buffer = state.free.pop_back()?;
    // SAFETY: a buffer the queue doesn't have, holding `size` bytes.
    unsafe { (*buffer.0).mAudioDataByteSize = size as u32 };
    Some(buffer)
}

/// Takes back a buffer the speakers' queue played. When it was the last,
/// filled buffers wait until they all are; the queue runs on, starved.
unsafe extern "C-unwind" fn play(
    shared: *mut c_void,
    queue: AudioQueueRef,
    buffer: AudioQueueBufferRef,
) {
    // SAFETY: the stream's `Shared`, which outlives its queues, and a buffer
    // of this queue's, which it gives back.
    let (shared, played) = unsafe { (&*shared.cast::<Shared>(), (*buffer).mAudioDataByteSize) };
    let mut state = shared.lock();
    if !state.current(queue) {
        return;
    }
    state.returned += 1;
    state.queued = state.queued.saturating_sub(played as usize);
    // The buffer being written stays last.
    state.free.push_front(Buffer(buffer));
    if state.queued == 0 && !state.draining {
        state.waiting = true;
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
        let shared = self.stream.shared.clone();
        ensure!(
            bytes.len() <= shared.lock().capacity,
            "reading more than the microphone keeps"
        );
        loop {
            self.stream.look();
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
    // SAFETY: the stream's `Shared`, which outlives its queues, and a buffer
    // of this queue's holding what it recorded, which goes back after.
    let shared = unsafe { &*shared.cast::<Shared>() };
    let mut state = shared.lock();
    if !state.current(queue) {
        return;
    }
    // SAFETY: as above.
    let recorded = unsafe {
        std::slice::from_raw_parts(
            (*buffer).mAudioData.as_ptr().cast::<u8>(),
            (*buffer).mAudioDataByteSize as usize,
        )
    };
    state.returned += 1;
    let over = (state.ring.len() + recorded.len())
        .saturating_sub(state.capacity)
        .min(state.ring.len());
    state.ring.drain(..over);
    state.ring.extend(recorded);
    // SAFETY: a buffer of this queue's. Reading never calls the queue while
    // holding the lock, so enqueueing under it waits on nothing of ours.
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
        if !has_streams(device, scope) || !alive(device) {
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

/// Whether a device is still there to play or record.
fn alive(device: AudioObjectID) -> bool {
    number::<u32>(
        device,
        address(
            kAudioDevicePropertyDeviceIsAlive,
            kAudioObjectPropertyScopeGlobal,
        ),
    )
    .is_some_and(|alive| alive != 0)
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
mod others;
#[cfg(feature = "discord")]
pub use others::others;
