# macOS

The disk image targets Macs with Apple silicon on macOS 11 or later. Like the
other platforms' releases it's the Discord flavour; `scripts/build-macos full`
builds the full preview with Matrix. It uses Slint with Skia, as everywhere,
LiveKit's macOS libwebrtc, and macOS's own services for what the other
platforms do in their own: Audio Queue Services for sound, the menu bar for
the tray, Notification Center for notifications and the Keychain for the
Discord token.

## Run the package

1. Open `Yapfast-0.1.0-macos-arm64.dmg` and drag Yapfast onto Applications.
2. Open Yapfast from Applications. It's signed ad hoc, not by a Developer ID,
   and not notarized, so Gatekeeper stops the first start of a downloaded
   copy: open System Settings → Privacy & Security and choose Open Anyway
   beside Yapfast, then Open. Later starts go straight through.
3. Link Discord by its QR code. Its token goes to the login Keychain, under
   `io.github.yapfast.Yapfast`.
4. Join a voice channel. macOS asks once for the microphone, naming why from
   `NSMicrophoneUsageDescription` in `packaging/macos/Info.plist`; without
   it, the microphone records silence. It asks once for notifications too.

Opened from the Finder or the Dock, Yapfast's output goes nowhere, so it
writes its diagnostics to `yapfast.log` in its data folder,
`~/Library/Application Support/io.github.yapfast.Yapfast`, as it does on
Windows; started from a terminal, as
`/Applications/Yapfast.app/Contents/MacOS/yapfast`, it writes there instead.
Yapfast renders with Skia Software; `--renderer gpu` draws with Skia on Metal.

Not there yet on the Mac: cameras (`camera.rs` captures through V4L2 and
DirectShow only), sharing other applications' sound with Go Live, which on
Linux and Windows comes from the sound server, and starting at sign-in. Each
is hidden or says so where it would be.

## Build

`scripts/build-macos` runs on a Mac with Xcode's command line tools, Rustup,
whose `rust-toolchain.toml` picks Rust 1.97.1, and `protoc` (Homebrew's
`protobuf`). It downloads the same releases of Skia's and LiveKit's prebuilt
archives `nix/native.nix` uses on Linux, for `aarch64-apple-darwin`, pinned by
SHA-256, into `.local/macos`, points `SKIA_BINARIES_URL` and
`LK_CUSTOM_WEBRTC` at them, sources `scripts/release-env`, builds the release
and runs `scripts/package-macos.py`. That puts `Yapfast.app` together:
`Contents/MacOS/yapfast`, `Info.plist` with the version filled in,
`Yapfast.icns`, and in `Resources` the licenses and notices,
`THIRD-PARTY.json`, `SOURCE.txt` and `BUILD.json`, as the Windows ZIP has
them. Skia's prebuilt archive carries only its own license, so the notices of
its third-party code come from the pinned Skia source and the dependencies
`nix/flake.nix` names, fetched once into `.local/macos/skia-source`, Git
checking out the same commits Nix pins. It signs the app ad hoc, which Apple
silicon requires of all code, and packs it with a link to Applications into
`target/dist/macos/Yapfast-VERSION-macos-arm64.dmg`, beside the source
snapshot and `SHA256SUMS.txt`. Packaging needs Python 3.11 or newer; the
script runs it with `uv` where there is one, as macOS's own Python is 3.9.

A development build needs only `PROTOC`: Skia's and LiveKit's build scripts
download their archives themselves.

```sh
PROTOC=$(command -v protoc) cargo build --profile fast --no-default-features --features desktop,discord
```

Three things differ from the other platforms' builds:

- libwebrtc's Objective-C categories, such as one on `NSString` its video
  codecs call as LiveKit starts, hold no symbol the linker looks for, so the
  linker leaves them out of the archive, and LiveKit's first call ends Yapfast
  with an unrecognized selector. `build.rs` and `crates/yapfast-core/build.rs`
  pass `-ObjC`, which loads every Objective-C member, as LiveKit's
  documentation says.
- With them loaded, Apple's linker warns that `_jpeg_natural_order` is in
  both libwebrtc's and Skia's libjpeg-turbo. On Linux that's an error, and
  `nix/prepare-skia-native.py` renames Skia's copy; Apple's linker keeps the
  first and goes on. Both are `private external` 80-entry tables in
  `jutils.o`, and their `__TEXT,__const` sections are byte for byte the same,
  so either serves both.
- The icon is `packaging/macos/Yapfast.icns`, drawn by
  `scripts/make-macos-icon` from `packaging/macos/icon.svg`: the logo on the
  rounded square macOS draws app icons on.

Signing with a Developer ID and notarizing, which would let the first start
through Gatekeeper, need an Apple Developer account; nothing in the package
stands in their way.

## Pictures and video

LiveKit's macOS archive builds FFmpeg without decoders for H264 and AAC, as
WebRTC on the Mac decodes H264 with VideoToolbox itself; its codec list has
only FLAC, MP3, Opus, PCM and Vorbis. Discord's GIFs are H264 MP4s, as are
most videos and many Go Live streams, and most MP4s' sound is AAC, so the Mac
decodes those through the system instead, where FFmpeg has no decoder:

- `native/videotoolbox.c` decodes H264 with a VideoToolbox session, on the
  hardware decoder, synchronously on the caller's thread. An MP4 file's avcC
  box gives it the parameter sets; a Go Live stream's come in its Annex B
  access units, which it rewrites with lengths, starting a new session when
  they change. VideoToolbox gives pictures in decode order, so it holds back
  as many as the demuxer says B-frames reorder by (`video_delay`) and gives
  them by presentation time. libyuv makes its NV12 the I420 `video.c` and
  `decode.c` take, the range from the pixel format and the matrix from the
  buffer's attachment.
- `native/audiotoolbox.c` decodes AAC with an `AudioConverter`, taking the
  AudioSpecificConfig wrapped in an ES descriptor, as FFmpeg's own
  AudioToolbox decoder does, and the richest format it describes, so HE-AAC
  comes out at its full rate. `video.c` trims the encoder's priming and
  padding by the demuxer's skip-samples marks, as FFmpeg's decoders do.

VP8, VP9 and AV1 go through libvpx and dav1d from the archive, as elsewhere,
and the favourite GIFs' VP9 clips with them.

Custom emoji draw from a colour bitmap font Yapfast makes (`src/emoji_font.rs`).
Elsewhere its PNGs are in CBDT tables, which FreeType and DirectWrite read;
Core Text, which Skia reads fonts through on the Mac, takes Apple's sbix
instead, and a font only with outlines, and draws a bitmap in its glyph's
box. So on macOS the font has an sbix strike, and each emoji an outline of two
one-point contours at its bitmap's corners, as Apple Color Emoji's glyphs
have.

## Sound

`crates/yapfast-core/src/audio_io/coreaudio.rs` is the third backend of
`audio_io.rs`. Each stream is an audio queue, which converts between the
stream's rate, channels and samples and the device's, so the meter's 16 kHz
mono and voice's 48 kHz both open as asked. The queue hands each buffer back
on a thread of its own.

Speakers queue in the queue's own 10 ms buffers, as many as the queue asked
for: six for Discord voice's 60 ms. Writing fills a free buffer and gives it to
the queue, waiting while the queue has them all. The queue starts once it
has them all, or when draining, and pauses when it hands the last back, so
all 60 ms absorb a late write, as with PulseAudio's prebuffering, and an
idle stream, as when deafened, lets the device rest. `latency()` counts what's
queued and the device's own latency, safety offset and buffer, as Core Audio
reports them, which for Bluetooth is long; draining waits that out too, so a
sound's tail isn't cut. Flushing, for a paused or moved video, drops the
queue for a new one on the same device. Microphones' buffers, about 100 ms of
them, go back to the queue as soon as they're recorded, their samples into a
ring that reading takes from.

LiveKit's macOS device module names the default device `default` and the rest
by their Core Audio object id, such as `77`; the backend finds the device with
that id, or that name for an id made from one, and names its UID to the
queue. A queue keeps the device it opened on, so a stream on the default
follows it by hand, as PulseAudio moves its streams: every 250 ms, writing and
reading look at the default, and when it's another device, as when AirPods
connect, the stream opens a new queue there. A queue that hands nothing back
for two seconds, as when its device goes away, is made again, on its device if
it's still there and on the default if not; it fails only if the new one
stops too. The first buffer after a start may take 30 seconds, as macOS asks
for the microphone the first time, and Bluetooth devices wake slowly.

Not handled yet: Core Audio's object ids change across restarts, and a saved
device is saved by LiveKit's id, so a device chosen in Settings may be another
after a restart.

Checked on a MacBook Air (M4, macOS 15.3) over SSH, with its own speakers
and microphone only:

- `audio_io::tests::opens_every_listed_device` opens both microphones and both
  speakers LiveKit lists, the defaults and the built-in ones, reads and
  writes on each, and refuses a device that isn't there.
- `audio_io::tests::streams_keep_time`, added with this backend, writes a
  second of silence in 20 ms frames into a 60 ms queue in 1.05 s, the device
  taking about 100 ms to start, drains by 1.16 s, and reads a second of
  16 kHz mono in 992 ms.
- Following the default and recovering from a device going away need a
  second device, such as Bluetooth headphones, which weren't there.

An adversarial review of the first version found no memory-safety problem
(the queue's disposal against its callback, Core Foundation ownership, the
formats) and the issues this version answers: following the default device,
jitter only half the queue absorbed, the device's latency missing, a stall
ending the call's sound for good, thin microphone buffering, refused buffers
going unnoticed, and an unaligned read of the stream configuration.

## Checks

On the MacBook Air, with the `fast` profile, the whole workspace's tests pass
for the Discord flavour (100 in the app, 234 in `yapfast-core`, 164 in
`yapfast-discord`), as does Clippy with warnings denied for it and for the
full flavour. `scripts/test-fake-discord`, run with `IN_NIX_SHELL=1` and Bun
from mise, passes its Discord service and `discord_text` tests; its example
environment needs `ffprobe` and `ffmpeg`, which the Mac didn't have.
