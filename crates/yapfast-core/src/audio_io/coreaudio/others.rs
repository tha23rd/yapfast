//! Other applications' sound, for Go Live, through a Core Audio process tap,
//! from macOS 14.2 on: a stereo mix of the processes it names, as one
//! stream, as Windows's process loopback gives it. It names every process
//! using sound but Yapfast's own and those whose program or bundle id `skip`
//! picks. Leaving processes out instead would let Yapfast's own through: a
//! process has a process object only once it starts using sound, too late to
//! leave out its first moments. Core Audio says when the list of process
//! objects changes, and the tap gets the new names; a process that just
//! started may lose its first moments instead.
//!
//! The tap is the only input of a private aggregate device run at [`RATE`],
//! so Core Audio resamples the mix there. The device's IOProc, on Core
//! Audio's real-time thread, copies each buffer into a ring and wakes the
//! thread `others` started, which hands it to `heard`; neither takes a lock.
//! A device whose IOProc stops being called, as after the Mac sleeps or Core
//! Audio restarts, or whose format changes, is made again.
//!
//! macOS asks the person once whether Yapfast may record what other
//! applications play, naming why from `NSAudioCaptureUsageDescription`, and
//! holds the first change to the tap until they answer. Refused, the tap
//! would hear silence, and refuses any change to it, so Yapfast gives it its
//! description again before starting, to tell.
//!
//! Yapfast runs from macOS 11, so the two functions taps came with are looked
//! up as it runs: linked, Yapfast wouldn't start on older systems.

use std::{
    ffi::{CStr, c_void},
    panic::AssertUnwindSafe,
    ptr::{NonNull, null, null_mut},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{JoinHandle, Thread},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use objc2::{
    AnyThread as _,
    rc::{Retained, autoreleasepool},
    runtime::{AnyClass, AnyObject},
};
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioHardwareCreateAggregateDevice, AudioHardwareDestroyAggregateDevice,
    AudioObjectAddPropertyListener, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectPropertySelector, AudioObjectSetPropertyData, CATapDescription,
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey,
    kAudioAggregateDeviceUIDKey, kAudioDevicePermissionsError, kAudioDevicePropertyBufferFrameSize,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyStreams,
    kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeInput, kAudioObjectSystemObject, kAudioObjectUnknown,
    kAudioProcessPropertyBundleID, kAudioProcessPropertyPID, kAudioStreamPropertyVirtualFormat,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyDescription,
    kAudioTapPropertyUID,
};
use objc2_core_audio_types::{
    AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp, kAudioFormatFlagIsFloat,
    kAudioFormatFlagIsNonInterleaved, kAudioFormatLinearPCM,
};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

use super::{address, check, data, number, objects, string};
use crate::audio_io::{Hear, Heard, RATE};

/// How long the thread waits for sound before it looks at `stop`.
const STOP_CHECK: Duration = Duration::from_millis(100);
/// How often it looks at the list of processes when Core Audio won't say
/// that it changed.
const LOOK: Duration = Duration::from_millis(500);
/// How often it looks at the device, and how long the IOProc may go uncalled
/// before the device counts as stopped.
const CHECK: Duration = Duration::from_secs(1);
const STALLED: Duration = Duration::from_secs(2);
/// How long the device may take to run at [`RATE`].
const RATE_WAIT: Duration = Duration::from_secs(1);
/// The frames of each IOProc call: 20 ms, one of the mixer's frames.
const BUFFER_FRAMES: u32 = RATE / 50;
/// The samples the ring holds: 100 ms, five IOProc calls.
const RING: usize = RATE as usize / 10 * 2;
/// What a change to the tap fails with when nobody answered macOS's
/// question in time: `MACH_RCV_TIMED_OUT`.
const TIMED_OUT: i32 = 0x1000_4003;

type CreateTap = unsafe extern "C-unwind" fn(*const CATapDescription, *mut AudioObjectID) -> i32;
type DestroyTap = unsafe extern "C-unwind" fn(AudioObjectID) -> i32;

pub fn others(
    skip: fn(&str) -> bool,
    heard: Hear,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>> {
    let taps =
        Taps::find().context("Yapfast shares other applications' sound on macOS 14.2 or later")?;
    let (opened, opening) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("yapfast-stream-hear".into())
        .spawn(move || {
            let hearing = Hearing::new(taps, skip);
            match hearing.open() {
                Ok(tap) => {
                    let _ = opened.send(Ok(()));
                    hearing.run(tap, heard, &stop);
                }
                Err(error) => {
                    let _ = opened.send(Err(error));
                }
            }
        })
        .context("could not start hearing other applications")?;
    match opening.recv() {
        Ok(Ok(())) => Ok(thread),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error.context("macOS won't share other applications' sound"))
        }
        Err(_) => {
            let _ = thread.join();
            Err(anyhow!("could not hear other applications"))
        }
    }
}

/// The process tap functions, from macOS 14.2 on.
#[derive(Clone, Copy)]
struct Taps {
    create: CreateTap,
    destroy: DestroyTap,
}

impl Taps {
    fn find() -> Option<Self> {
        // SAFETY: looking up a symbol in what's loaded, Core Audio among it.
        let symbol = |name: &CStr| unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        let create = symbol(c"AudioHardwareCreateProcessTap");
        let destroy = symbol(c"AudioHardwareDestroyProcessTap");
        if create.is_null() || destroy.is_null() || AnyClass::get(c"CATapDescription").is_none() {
            return None;
        }
        // SAFETY: Core Audio's functions, of these types.
        unsafe {
            Some(Self {
                create: std::mem::transmute::<*mut c_void, CreateTap>(create),
                destroy: std::mem::transmute::<*mut c_void, DestroyTap>(destroy),
            })
        }
    }
}

/// Bumped whenever Core Audio's list of process objects changes.
static PROCESSES_CHANGED: AtomicU64 = AtomicU64::new(0);
/// The threads hearing, woken when it does.
static HEARING: Mutex<Vec<Thread>> = Mutex::new(Vec::new());

/// Has Core Audio say when its list of process objects changes, once for as
/// long as this process runs; false when it won't.
fn listen() -> bool {
    static LISTENING: OnceLock<bool> = OnceLock::new();
    *LISTENING.get_or_init(|| {
        // SAFETY: a listener that touches only statics.
        let status = unsafe {
            AudioObjectAddPropertyListener(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&global(kAudioHardwarePropertyProcessObjectList)),
                Some(processes_changed),
                null_mut(),
            )
        };
        status == 0
    })
}

unsafe extern "C-unwind" fn processes_changed(
    _: AudioObjectID,
    _: u32,
    _: NonNull<AudioObjectPropertyAddress>,
    _: *mut c_void,
) -> i32 {
    PROCESSES_CHANGED.fetch_add(1, Ordering::Relaxed);
    for thread in HEARING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
    {
        thread.unpark();
    }
    0
}

/// What the thread `others` started keeps while it hears.
struct Hearing {
    taps: Taps,
    skip: fn(&str) -> bool,
    own: Own,
    ring: Arc<Ring>,
    /// Core Audio says when the list of processes changes.
    listening: bool,
}

impl Hearing {
    fn new(taps: Taps, skip: fn(&str) -> bool) -> Self {
        HEARING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(std::thread::current());
        Self {
            taps,
            skip,
            own: Own::find(),
            ring: Arc::new(Ring::new()),
            listening: listen(),
        }
    }

    fn open(&self) -> Result<Tap> {
        autoreleasepool(|_| Tap::open(self.taps, &self.own, self.skip, &self.ring))
    }

    /// Hands what the tap hears to `heard` until `stop`, following the
    /// processes that come and go, and making the tap again when its device
    /// stops or changes.
    fn run(&self, tap: Tap, mut heard: Hear, stop: &AtomicBool) {
        heard(Heard::Started {
            source: 0,
            process: None,
        });
        let mut tap = Some(tap);
        let mut samples = Vec::with_capacity(RING);
        // None at first, for a look at what changed while the tap was made.
        let mut changes = None;
        let (mut looked, mut checked) = (Instant::now(), Instant::now());
        let mut calls = (self.ring.calls.load(Ordering::Relaxed), Instant::now());
        let mut failing = false;
        while !stop.load(Ordering::Relaxed) {
            std::thread::park_timeout(STOP_CHECK);
            if self.ring.take(&mut samples) {
                heard(Heard::Samples {
                    source: 0,
                    samples: &samples,
                });
            }
            let changed = Some(PROCESSES_CHANGED.load(Ordering::Relaxed));
            if let Some(tap) = tap.as_mut()
                && (changed != changes || !self.listening && looked.elapsed() >= LOOK)
            {
                (changes, looked) = (changed, Instant::now());
                autoreleasepool(|_| tap.look(&self.own, self.skip));
            }
            if checked.elapsed() < CHECK {
                continue;
            }
            checked = Instant::now();
            let called = self.ring.calls.load(Ordering::Relaxed);
            if called != calls.0 {
                calls = (called, Instant::now());
            }
            let why = match &tap {
                None => "it couldn't be made",
                Some(_) if calls.1.elapsed() >= STALLED => "its device stopped",
                Some(tap) if !tap.device.suits() => "its device's format changed",
                Some(_) => continue,
            };
            tap = None;
            match self.open() {
                Ok(made) => {
                    tracing::info!(why, "hearing other applications again");
                    tap = Some(made);
                    calls.1 = Instant::now();
                    failing = false;
                }
                Err(error) if !failing => {
                    failing = true;
                    tracing::warn!(
                        error = format!("{error:#}"),
                        why,
                        "could not hear other applications again"
                    );
                }
                Err(_) => {}
            }
        }
        drop(tap);
        heard(Heard::Stopped { source: 0 });
    }
}

impl Drop for Hearing {
    fn drop(&mut self) {
        let this = std::thread::current().id();
        HEARING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|thread| thread.id() != this);
    }
}

/// What the IOProc heard and the hearing thread hasn't handed on yet: one
/// writes, the other reads, and neither waits for the other.
struct Ring {
    samples: Box<[AtomicU32]>,
    /// The samples ever written, and ever taken.
    written: AtomicUsize,
    taken: AtomicUsize,
    /// The IOProc's calls, silent or not, to tell a device that stopped.
    calls: AtomicU64,
    reader: Thread,
}

impl Ring {
    fn new() -> Self {
        Self {
            samples: (0..RING).map(|_| AtomicU32::new(0)).collect(),
            written: AtomicUsize::new(0),
            taken: AtomicUsize::new(0),
            calls: AtomicU64::new(0),
            reader: std::thread::current(),
        }
    }

    /// From the IOProc: keeps what fits, whole frames, and wakes the reader.
    fn put(&self, samples: &[f32]) {
        let written = self.written.load(Ordering::Relaxed);
        let free = RING - (written - self.taken.load(Ordering::Acquire));
        let kept = &samples[..samples.len().min(free) & !1];
        for (at, sample) in kept.iter().enumerate() {
            self.samples[(written + at) % RING].store(sample.to_bits(), Ordering::Relaxed);
        }
        self.written.store(written + kept.len(), Ordering::Release);
        self.reader.unpark();
    }

    /// From the reader: everything waiting, into `into`; false for nothing.
    fn take(&self, into: &mut Vec<f32>) -> bool {
        let written = self.written.load(Ordering::Acquire);
        let taken = self.taken.load(Ordering::Relaxed);
        into.clear();
        into.extend(
            (taken..written)
                .map(|at| f32::from_bits(self.samples[at % RING].load(Ordering::Relaxed))),
        );
        self.taken.store(written, Ordering::Release);
        !into.is_empty()
    }
}

/// Tells Yapfast's own processes: this one, the one that started it, and,
/// when that one runs this program too, so is Yapfast, the others it started,
/// such as a worker playing a watched stream.
struct Own {
    this: u32,
    parent: u32,
    family: bool,
}

impl Own {
    fn find() -> Self {
        let (this, parent) = (std::process::id(), std::os::unix::process::parent_id());
        let program = path(this);
        Self {
            this,
            parent,
            family: program.is_some() && program == path(parent),
        }
    }

    fn has(&self, pid: u32) -> bool {
        pid == self.this || pid == self.parent || self.family && parent_of(pid) == Some(self.parent)
    }
}

/// The tap and what's made of it, taken apart in this order.
struct Tap {
    _proc: Proc,
    device: Aggregate,
    tap: ProcessTap,
    description: Retained<CATapDescription>,
    /// The process objects when last looked at, and those the tap names.
    processes: Vec<AudioObjectID>,
    included: Vec<AudioObjectID>,
    /// A change to the names failed, and was logged.
    warned: bool,
}

impl Tap {
    fn open(taps: Taps, own: &Own, skip: fn(&str) -> bool, ring: &Arc<Ring>) -> Result<Self> {
        let mut processes = self::processes()?;
        let mut included = self::included(&processes, own, skip);
        // SAFETY: the class is there, as the functions are.
        let description = unsafe {
            CATapDescription::initStereoMixdownOfProcesses(
                CATapDescription::alloc(),
                &numbers(&included),
            )
        };
        // SAFETY: a description of a tap this process alone sees.
        unsafe { description.setPrivate(true) };
        let mut id = kAudioObjectUnknown;
        // SAFETY: a description, and room for the tap's id.
        check(
            unsafe { (taps.create)(&raw const *description, &mut id) },
            "macOS wouldn't make a tap",
        )?;
        ensure!(id != kAudioObjectUnknown, "macOS wouldn't make a tap");
        let tap = ProcessTap {
            id,
            destroy: taps.destroy,
        };
        let device = Aggregate::of(&tap)?;
        device.run()?;
        let mut proc = Proc::new(device.0, ring.clone())?;
        // The first time, this waits for the person's answer; refused,
        // macOS lets nothing change the tap.
        let mut described = describe(tap.id, &description);
        if described.is_err_and(|status| !refused(status)) {
            // A process may have gone meanwhile.
            processes = self::processes()?;
            included = self::included(&processes, own, skip);
            // SAFETY: the description the tap was made from.
            unsafe { description.setProcesses(&numbers(&included)) };
            described = describe(tap.id, &description);
        }
        match described {
            Ok(()) => {}
            Err(status) if refused(status) => bail!(
                "Yapfast may not record other applications' sound: allow it in System \
                 Settings, Privacy & Security, Screen & System Audio Recording (Core Audio \
                 status {status})"
            ),
            Err(status) => {
                bail!("the tap refused the processes to hear (Core Audio status {status})")
            }
        }
        proc.start()?;
        tracing::debug!(processes = included.len(), "hearing other applications");
        Ok(Self {
            _proc: proc,
            device,
            tap,
            description,
            processes,
            included,
            warned: false,
        })
    }

    /// Names to the tap the processes that started using sound since the
    /// last look, and leaves out those that ended.
    fn look(&mut self, own: &Own, skip: fn(&str) -> bool) {
        let Ok(processes) = processes() else { return };
        if processes == self.processes {
            return;
        }
        let included = included(&processes, own, skip);
        self.processes = processes;
        if included == self.included {
            return;
        }
        // SAFETY: the description the tap was made from.
        unsafe { self.description.setProcesses(&numbers(&included)) };
        match describe(self.tap.id, &self.description) {
            Ok(()) => {
                tracing::debug!(processes = included.len(), "hearing other processes");
                self.included = included;
                self.warned = false;
            }
            Err(status) if !self.warned => {
                self.warned = true;
                tracing::warn!(
                    status,
                    "could not change which processes the shared sound hears"
                );
            }
            Err(_) => {}
        }
    }
}

/// Every process object: each process that has used sound and is still there.
fn processes() -> Result<Vec<AudioObjectID>> {
    objects(
        kAudioObjectSystemObject as AudioObjectID,
        global(kAudioHardwarePropertyProcessObjectList),
    )
    .context("could not list the processes using sound")
}

/// The processes to hear: all but Yapfast's own, those whose program or bundle
/// id `skip` picks, and those that are gone.
fn included(processes: &[AudioObjectID], own: &Own, skip: fn(&str) -> bool) -> Vec<AudioObjectID> {
    let heard = |process: AudioObjectID| {
        let Some(pid) = number::<i32>(process, global(kAudioProcessPropertyPID)) else {
            return false;
        };
        !own.has(pid.cast_unsigned())
            && !program(pid).is_some_and(|program| skip(&program))
            && !string(process, kAudioProcessPropertyBundleID)
                .is_ok_and(|bundle| skip(&bundle.to_string()))
    };
    processes
        .iter()
        .copied()
        .filter(|&process| heard(process))
        .collect()
}

/// The name of the program a process runs: its file's, such as `yapfast`.
fn program(pid: i32) -> Option<String> {
    let mut name = [0u8; 64];
    // SAFETY: a buffer of the size given.
    let length = unsafe { libc::proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
    let name = name.get(..usize::try_from(length).ok().filter(|&length| length > 0)?)?;
    Some(String::from_utf8_lossy(name).into_owned())
}

/// The path of the program a process runs.
fn path(pid: u32) -> Option<Vec<u8>> {
    let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: a buffer of the size given.
    let length = unsafe {
        libc::proc_pidpath(
            pid.cast_signed(),
            path.as_mut_ptr().cast(),
            path.len() as u32,
        )
    };
    path.truncate(usize::try_from(length).ok().filter(|&length| length > 0)?);
    Some(path)
}

/// The process that started a process.
fn parent_of(pid: u32) -> Option<u32> {
    // SAFETY: a plain C struct, which zeroes are valid for.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: room for the information asked for.
    let got = unsafe {
        libc::proc_pidinfo(
            pid.cast_signed(),
            libc::PROC_PIDTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    (got == size).then_some(info.pbi_ppid)
}

fn numbers(processes: &[AudioObjectID]) -> Retained<NSArray<NSNumber>> {
    let numbers: Vec<_> = processes.iter().map(|&id| NSNumber::new_u32(id)).collect();
    NSArray::from_retained_slice(&numbers)
}

fn global(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    address(selector, kAudioObjectPropertyScopeGlobal)
}

/// Sets a property of a fixed size; the status when it fails.
fn set<T>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    value: &T,
) -> Result<(), i32> {
    // SAFETY: a property of the value's size, which Core Audio copies.
    let status = unsafe {
        AudioObjectSetPropertyData(
            object,
            NonNull::from(&global(selector)),
            0,
            null(),
            size_of::<T>() as u32,
            NonNull::from(value).cast(),
        )
    };
    if status == 0 { Ok(()) } else { Err(status) }
}

/// Gives the tap `description`, which it copies.
fn describe(tap: AudioObjectID, description: &CATapDescription) -> Result<(), i32> {
    set(
        tap,
        kAudioTapPropertyDescription,
        &(description as *const CATapDescription),
    )
}

/// Whether a change to the tap failed because Yapfast may not record, or
/// because nobody said whether it may.
fn refused(status: i32) -> bool {
    status == kAudioDevicePermissionsError || status == TIMED_OUT
}

/// A process tap, destroyed when dropped.
struct ProcessTap {
    id: AudioObjectID,
    destroy: DestroyTap,
}

impl Drop for ProcessTap {
    fn drop(&mut self) {
        // SAFETY: a tap this made, which nothing uses any more.
        unsafe { (self.destroy)(self.id) };
    }
}

/// A private aggregate device, destroyed when dropped.
struct Aggregate(AudioObjectID);

impl Aggregate {
    /// A device whose only input is `tap`, which it resamples to its own rate.
    fn of(tap: &ProcessTap) -> Result<Self> {
        let tap_uid = string(tap.id, kAudioTapPropertyUID).context("the tap has no id")?;
        // SAFETY: a CFString is an NSString, toll-free bridged.
        let tap_uid = unsafe { &*(&raw const *tap_uid).cast::<NSString>() };
        let key = |key: &CStr| NSString::from_str(&key.to_string_lossy());
        let yes = NSNumber::new_bool(true);
        let no = NSNumber::new_bool(false);
        let sub_tap = NSDictionary::<NSString, AnyObject>::from_slices(
            &[
                &*key(kAudioSubTapUIDKey),
                &*key(kAudioSubTapDriftCompensationKey),
            ],
            &[tap_uid, &yes],
        );
        let taps = NSArray::from_retained_slice(&[sub_tap]);
        let name = NSString::from_str("Yapfast's Go Live sound");
        let uid = NSString::from_str(&format!(
            "io.github.yapfast.Yapfast.go-live.{}.{}",
            std::process::id(),
            tap.id
        ));
        let description = NSDictionary::<NSString, AnyObject>::from_slices(
            &[
                &*key(kAudioAggregateDeviceNameKey),
                &*key(kAudioAggregateDeviceUIDKey),
                &*key(kAudioAggregateDeviceTapListKey),
                &*key(kAudioAggregateDeviceTapAutoStartKey),
                &*key(kAudioAggregateDeviceIsPrivateKey),
            ],
            &[&name, &uid, &taps, &no, &yes],
        );
        let mut device = kAudioObjectUnknown;
        // SAFETY: an NSDictionary is a CFDictionary, toll-free bridged.
        check(
            unsafe {
                AudioHardwareCreateAggregateDevice(
                    &*Retained::as_ptr(&description).cast::<CFDictionary>(),
                    NonNull::from(&mut device),
                )
            },
            "macOS wouldn't make a device of the tap",
        )?;
        Ok(Self(device))
    }

    /// Runs the device at [`RATE`], in the mixer's 20 ms, and waits for it to
    /// take the rate, which Core Audio may change a moment later.
    fn run(&self) -> Result<()> {
        set(
            self.0,
            kAudioDevicePropertyNominalSampleRate,
            &f64::from(RATE),
        )
        .map_err(|status| anyhow!("the tap won't run at 48 kHz (Core Audio status {status})"))?;
        if let Err(status) = set(self.0, kAudioDevicePropertyBufferFrameSize, &BUFFER_FRAMES) {
            tracing::debug!(status, "the tap keeps its own buffer size");
        }
        let deadline = Instant::now() + RATE_WAIT;
        loop {
            let format = input_format(self.0);
            if format.is_some_and(suits) {
                tracing::debug!(?format, "the tap's device");
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("the tap gives sound in a format Yapfast doesn't take: {format:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Whether the device still gives 32-bit floats, stereo, interleaved,
    /// at [`RATE`].
    fn suits(&self) -> bool {
        input_format(self.0).is_some_and(suits)
    }
}

impl Drop for Aggregate {
    fn drop(&mut self) {
        // SAFETY: a device this made, whose IOProc is gone.
        unsafe { AudioHardwareDestroyAggregateDevice(self.0) };
    }
}

fn suits(format: AudioStreamBasicDescription) -> bool {
    format.mFormatID == kAudioFormatLinearPCM
        && format.mFormatFlags & kAudioFormatFlagIsFloat != 0
        && format.mFormatFlags & kAudioFormatFlagIsNonInterleaved == 0
        && format.mBitsPerChannel == 32
        && format.mChannelsPerFrame == 2
        && format.mSampleRate == f64::from(RATE)
}

/// What a device gives its IOProc: its input stream's format.
fn input_format(device: AudioObjectID) -> Option<AudioStreamBasicDescription> {
    let streams = objects(
        device,
        address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput),
    )
    .ok()?;
    let (storage, size) =
        data(*streams.first()?, global(kAudioStreamPropertyVirtualFormat)).ok()?;
    // SAFETY: storage aligned for any of its parts, holding the description.
    (size == size_of::<AudioStreamBasicDescription>()).then(|| unsafe {
        storage
            .as_ptr()
            .cast::<AudioStreamBasicDescription>()
            .read()
    })
}

/// The aggregate device's IOProc, which puts what the device hears in the
/// ring.
struct Proc {
    device: AudioObjectID,
    id: AudioDeviceIOProcID,
    ring: Arc<Ring>,
    started: bool,
}

impl Proc {
    fn new(device: AudioObjectID, ring: Arc<Ring>) -> Result<Self> {
        let mut proc = Self {
            device,
            id: None,
            ring,
            started: false,
        };
        // SAFETY: the ring outlives the IOProc: dropping destroys the IOProc
        // first, and keeps the ring if Core Audio won't let go of it.
        check(
            unsafe {
                AudioDeviceCreateIOProcID(
                    device,
                    Some(hear),
                    Arc::as_ptr(&proc.ring).cast_mut().cast(),
                    NonNull::from(&mut proc.id),
                )
            },
            "the tap won't be heard",
        )?;
        Ok(proc)
    }

    fn start(&mut self) -> Result<()> {
        // SAFETY: the device's IOProc.
        check(
            unsafe { AudioDeviceStart(self.device, self.id) },
            "the tap won't start",
        )?;
        self.started = true;
        Ok(())
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if self.id.is_none() {
            return;
        }
        // SAFETY: the device's IOProc; once destroyed, Core Audio calls it
        // no more.
        let destroyed = unsafe {
            if self.started {
                AudioDeviceStop(self.device, self.id);
            }
            AudioDeviceDestroyIOProcID(self.device, self.id) == 0
        };
        if !destroyed {
            tracing::warn!("Core Audio didn't let go of the tap's IOProc");
            std::mem::forget(self.ring.clone());
        }
    }
}

/// Puts what the tap heard in the ring: interleaved stereo floats at
/// [`RATE`], as the device's format is. Silence through and through means
/// nothing plays, which the mixer hears as nothing, as elsewhere.
unsafe extern "C-unwind" fn hear(
    _: AudioObjectID,
    _: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    _: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    ring: *mut c_void,
) -> i32 {
    // Nothing may unwind into Core Audio.
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the Proc's, which outlives the IOProc.
        let ring = unsafe { &*ring.cast::<Ring>() };
        ring.calls.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the device's input, one buffer for its one stream.
        let buffer = unsafe {
            let list = input.as_ref();
            if list.mNumberBuffers == 0 {
                return;
            }
            list.mBuffers[0]
        };
        if buffer.mData.is_null() || buffer.mNumberChannels != 2 {
            return;
        }
        // SAFETY: the buffer holds that many bytes of floats, for this call.
        let samples = unsafe {
            std::slice::from_raw_parts(
                buffer.mData.cast::<f32>(),
                buffer.mDataByteSize as usize / size_of::<f32>(),
            )
        };
        if samples.iter().any(|&sample| sample != 0.0) {
            ring.put(samples);
        }
    }));
    0
}
