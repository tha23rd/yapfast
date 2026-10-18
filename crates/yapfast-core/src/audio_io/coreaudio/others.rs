//! Other applications' sound, for Go Live, through a Core Audio process tap,
//! from macOS 14.2 on: one stream of everything the system plays but the
//! processes the tap leaves out, as Windows's process loopback mixes it. The
//! tap is the only input of a private aggregate device, which runs at
//! [`RATE`], so Core Audio resamples the mix there; the device's IOProc hands
//! each buffer as it is to `heard`, on Core Audio's own thread.
//!
//! The tap leaves out Yapfast's own processes, and those whose program or
//! bundle id `skip` picks. It names them by their process objects, and a
//! process gets one only once it uses sound, so the list of them is looked at
//! again every half second, and the tap is told when what it leaves out
//! changes.
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
    ptr::{NonNull, null},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, ensure};
use objc2::{
    AnyThread as _,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioHardwareCreateAggregateDevice, AudioHardwareDestroyAggregateDevice,
    AudioObjectID, AudioObjectSetPropertyData, CATapDescription, kAudioAggregateDeviceIsPrivateKey,
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey,
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

/// How often the thread looks for `stop`.
const STOP_CHECK: Duration = Duration::from_millis(100);
/// How often it looks for processes that started or stopped using sound.
const LOOK: Duration = Duration::from_millis(500);

type CreateTap = unsafe extern "C-unwind" fn(*const CATapDescription, *mut AudioObjectID) -> i32;
type DestroyTap = unsafe extern "C-unwind" fn(AudioObjectID) -> i32;

/// Where the IOProc hands what the tap hears, until it's taken away.
type Hearing = Mutex<Option<Hear>>;

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
            let mut tap = match Tap::open(taps, skip, heard) {
                Ok(tap) => {
                    let _ = opened.send(Ok(()));
                    tap
                }
                Err(error) => {
                    let _ = opened.send(Err(error));
                    return;
                }
            };
            let mut looked = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(STOP_CHECK);
                if looked.elapsed() >= LOOK {
                    looked = Instant::now();
                    tap.look();
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

/// The tap and what's made of it, taken apart in this order.
struct Tap {
    _proc: Proc,
    _device: Aggregate,
    tap: ProcessTap,
    description: Retained<CATapDescription>,
    skip: fn(&str) -> bool,
    /// The process objects when last looked at, and those the tap leaves out.
    processes: Vec<AudioObjectID>,
    left_out: Vec<AudioObjectID>,
}

impl Tap {
    fn open(taps: Taps, skip: fn(&str) -> bool, heard: Hear) -> Result<Self> {
        let processes = processes()?;
        let left_out = left_out(&processes, skip);
        // SAFETY: the class is there, as the functions are.
        let description = unsafe {
            CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &numbers(&left_out),
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
        let rate = f64::from(RATE);
        // SAFETY: the rate is a Float64.
        check(
            unsafe {
                AudioObjectSetPropertyData(
                    device.0,
                    NonNull::from(&address(
                        kAudioDevicePropertyNominalSampleRate,
                        kAudioObjectPropertyScopeGlobal,
                    )),
                    0,
                    null(),
                    size_of::<f64>() as u32,
                    NonNull::from(&rate).cast(),
                )
            },
            "the tap won't run at 48 kHz",
        )?;
        ensure!(
            input_format(device.0).is_some_and(|format| format.mFormatID == kAudioFormatLinearPCM
                && format.mFormatFlags & kAudioFormatFlagIsFloat != 0
                && format.mFormatFlags & kAudioFormatFlagIsNonInterleaved == 0
                && format.mBitsPerChannel == 32
                && format.mChannelsPerFrame == 2
                && format.mSampleRate == rate),
            "the tap gives sound in a format Yapfast doesn't take"
        );
        let mut proc = Proc::new(device.0, heard)?;
        // The first time, this waits for the person's answer; refused,
        // macOS lets nothing change the tap.
        describe(tap.id, &description).context(
            "Yapfast may not record other applications' sound: allow it in System Settings, \
             Privacy & Security, Screen & System Audio Recording",
        )?;
        proc.start()?;
        tracing::debug!(left_out = left_out.len(), "hearing other applications");
        Ok(Self {
            _proc: proc,
            _device: device,
            tap,
            description,
            skip,
            processes,
            left_out,
        })
    }

    /// Leaves out processes that started using sound since the last look,
    /// such as Discord or a Yapfast worker playing a watched stream.
    fn look(&mut self) {
        let Ok(processes) = processes() else { return };
        if processes == self.processes {
            return;
        }
        let left_out = left_out(&processes, self.skip);
        if left_out != self.left_out {
            // SAFETY: a description this tap was made from.
            unsafe { self.description.setProcesses(&numbers(&left_out)) };
            if let Err(error) = describe(self.tap.id, &self.description) {
                tracing::warn!(
                    error = format!("{error:#}"),
                    "could not leave a process out of the shared sound"
                );
                return;
            }
            tracing::debug!(left_out = left_out.len(), "processes left out changed");
            self.left_out = left_out;
        }
        self.processes = processes;
    }
}

/// Every process object: each process that has used sound and is still there.
fn processes() -> Result<Vec<AudioObjectID>> {
    objects(
        kAudioObjectSystemObject as AudioObjectID,
        address(
            kAudioHardwarePropertyProcessObjectList,
            kAudioObjectPropertyScopeGlobal,
        ),
    )
    .context("could not list the processes using sound")
}

/// The processes to leave out: this one and the Yapfast that started it, as
/// on Linux, and those whose program or bundle id `skip` picks.
fn left_out(processes: &[AudioObjectID], skip: fn(&str) -> bool) -> Vec<AudioObjectID> {
    let own = [std::process::id(), std::os::unix::process::parent_id()];
    let leaves_out = |process: AudioObjectID| {
        let Some(pid) = number::<i32>(
            process,
            address(kAudioProcessPropertyPID, kAudioObjectPropertyScopeGlobal),
        ) else {
            return false;
        };
        own.contains(&pid.cast_unsigned())
            || program(pid).is_some_and(|program| skip(&program))
            || string(process, kAudioProcessPropertyBundleID)
                .is_ok_and(|bundle| skip(&bundle.to_string()))
    };
    processes
        .iter()
        .copied()
        .filter(|&process| leaves_out(process))
        .collect()
}

/// The name of the program a process runs.
fn program(pid: i32) -> Option<String> {
    let mut name = [0u8; 64];
    // SAFETY: a buffer of the size given.
    let length = unsafe { libc::proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
    let name = name.get(..usize::try_from(length).ok().filter(|&length| length > 0)?)?;
    Some(String::from_utf8_lossy(name).into_owned())
}

fn numbers(processes: &[AudioObjectID]) -> Retained<NSArray<NSNumber>> {
    let numbers: Vec<_> = processes.iter().map(|&id| NSNumber::new_u32(id)).collect();
    NSArray::from_retained_slice(&numbers)
}

/// Gives the tap `description`, which macOS refuses while Yapfast may not
/// record.
fn describe(tap: AudioObjectID, description: &CATapDescription) -> Result<()> {
    let description: *const CATapDescription = description;
    // SAFETY: the property is a CATapDescription, which the tap copies.
    check(
        unsafe {
            AudioObjectSetPropertyData(
                tap,
                NonNull::from(&address(
                    kAudioTapPropertyDescription,
                    kAudioObjectPropertyScopeGlobal,
                )),
                0,
                null(),
                size_of_val(&description) as u32,
                NonNull::from(&description).cast(),
            )
        },
        "the tap refused a change",
    )
}

/// What a device gives its IOProc: its input stream's format.
fn input_format(device: AudioObjectID) -> Option<AudioStreamBasicDescription> {
    let streams = objects(
        device,
        address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput),
    )
    .ok()?;
    let (storage, size) = data(
        *streams.first()?,
        address(
            kAudioStreamPropertyVirtualFormat,
            kAudioObjectPropertyScopeGlobal,
        ),
    )
    .ok()?;
    // SAFETY: storage aligned for any of its parts, holding the description.
    (size == size_of::<AudioStreamBasicDescription>()).then(|| unsafe {
        storage
            .as_ptr()
            .cast::<AudioStreamBasicDescription>()
            .read()
    })
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
}

impl Drop for Aggregate {
    fn drop(&mut self) {
        // SAFETY: a device this made, whose IOProc is gone.
        unsafe { AudioHardwareDestroyAggregateDevice(self.0) };
    }
}

/// The aggregate device's IOProc, and what it hands the sound to: one of the
/// device's own threads, so under a lock.
struct Proc {
    device: AudioObjectID,
    id: AudioDeviceIOProcID,
    hearing: NonNull<Hearing>,
    started: bool,
}

impl Proc {
    fn new(device: AudioObjectID, heard: Hear) -> Result<Self> {
        let mut proc = Self {
            device,
            id: None,
            hearing: NonNull::from(Box::leak(Box::new(Mutex::new(Some(heard))))),
            started: false,
        };
        // SAFETY: `hearing` outlives the IOProc, which dropping destroys first.
        check(
            unsafe {
                AudioDeviceCreateIOProcID(
                    device,
                    Some(hear),
                    proc.hearing.as_ptr().cast(),
                    NonNull::from(&mut proc.id),
                )
            },
            "the tap won't be heard",
        )?;
        Ok(proc)
    }

    fn lock(&self) -> MutexGuard<'_, Option<Hear>> {
        // SAFETY: freed only when dropped.
        let hearing = unsafe { self.hearing.as_ref() };
        hearing.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn start(&mut self) -> Result<()> {
        if let Some(heard) = self.lock().as_mut() {
            heard(Heard::Started {
                source: 0,
                process: None,
            });
        }
        self.started = true;
        // SAFETY: the device's IOProc.
        check(
            unsafe { AudioDeviceStart(self.device, self.id) },
            "the tap won't start",
        )
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        // SAFETY: the device's IOProc; once stopped and destroyed, Core Audio
        // calls it no more.
        let released = self.id.is_none()
            || unsafe {
                let stopped = AudioDeviceStop(self.device, self.id) == 0;
                let destroyed = AudioDeviceDestroyIOProcID(self.device, self.id) == 0;
                stopped && destroyed
            };
        // An IOProc still running hears nothing from here on.
        let heard = self.lock().take();
        if let Some(mut heard) = heard
            && self.started
        {
            heard(Heard::Stopped { source: 0 });
        }
        if released {
            // SAFETY: leaked in `new`, and no longer anyone's.
            drop(unsafe { Box::from_raw(self.hearing.as_ptr()) });
        } else {
            tracing::warn!("Core Audio didn't let go of the tap's IOProc");
        }
    }
}

/// Hands what the tap heard to `heard`: interleaved stereo floats at
/// [`RATE`], as the device's format is. Silence through and through means
/// nothing plays, which the mixer hears as nothing, as elsewhere.
unsafe extern "C-unwind" fn hear(
    _: AudioObjectID,
    _: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    _: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    hearing: *mut c_void,
) -> i32 {
    // SAFETY: the Proc's, which outlives the IOProc.
    let hearing = unsafe { &*hearing.cast::<Hearing>() };
    let mut hearing = hearing.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(heard) = hearing.as_mut() else {
        return 0;
    };
    // SAFETY: the device's input, one buffer for its one stream.
    let buffer = unsafe {
        let list = input.as_ref();
        if list.mNumberBuffers == 0 {
            return 0;
        }
        list.mBuffers[0]
    };
    if buffer.mData.is_null() || buffer.mNumberChannels != 2 {
        return 0;
    }
    // SAFETY: the buffer holds that many bytes of floats, for this call.
    let samples = unsafe {
        std::slice::from_raw_parts(
            buffer.mData.cast::<f32>(),
            buffer.mDataByteSize as usize / size_of::<f32>(),
        )
    };
    if samples.iter().all(|&sample| sample == 0.0) {
        return 0;
    }
    heard(Heard::Samples { source: 0, samples });
    0
}
