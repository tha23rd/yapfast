//! Builds the Go Live codec shims, the GIF thumbnail encoder, the video file
//! reader and the cameras against the libvpx, OpenH264, FFmpeg, dav1d,
//! libyuv and WebRTC headers in the same libwebrtc archive webrtc-sys links,
//! so they match its code.

fn main() {
    for file in [
        "vp8.c",
        "h264.c",
        "clip.c",
        "decode.c",
        "video.c",
        "picture.c",
        "camera.cc",
        "videotoolbox.c",
        "videotoolbox.h",
        "audiotoolbox.c",
        "audiotoolbox.h",
    ] {
        println!("cargo:rerun-if-changed=native/{file}");
    }
    let include = webrtc_sys_build::webrtc_dir().join("include");
    let third_party = include.join("third_party");
    cc::Build::new()
        .file("native/picture.c")
        .include(third_party.join("libyuv/include"))
        .warnings(true)
        .compile("yapfast_picture");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    // This crate's tests start LiveKit too; see the root build.rs.
    if target_os == "macos" {
        println!("cargo:rustc-link-arg=-ObjC");
    }
    // Linux captures through V4L2, and Windows through DirectShow, both in
    // the same module of the libwebrtc archive.
    if matches!(target_os.as_str(), "linux" | "windows") {
        let mut camera = cc::Build::new();
        camera
            .cpp(true)
            .file("native/camera.cc")
            .include(&include)
            .include(third_party.join("abseil-cpp"))
            .include(third_party.join("libyuv/include"))
            .std("c++20")
            .flag("-Wno-deprecated-declarations")
            // libwebrtc's headers mark some pointers and not others.
            .flag("-Wno-nullability-completeness")
            .warnings(true);
        // The toolchain sets its own hardening mode.
        for (key, value) in webrtc_sys_build::webrtc_defines() {
            if key != "_LIBCPP_HARDENING_MODE" {
                camera.define(&key, value.as_deref());
            }
        }
        camera.compile("yapfast_camera");
        // DirectShow's device list reads each camera's name and path as a
        // VARIANT; webrtc-sys links the rest.
        if target_os == "windows" {
            println!("cargo:rustc-link-lib=dylib=oleaut32");
        }
    }
    if std::env::var_os("CARGO_FEATURE_DISCORD").is_none() {
        return;
    }
    let os = match target_os.as_str() {
        "windows" => "win",
        "macos" => "mac",
        _ => "linux",
    };
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    let mut codecs = cc::Build::new();
    // The Mac's archive has no FFmpeg decoder for H264 or AAC; VideoToolbox
    // and AudioToolbox decode them.
    if target_os == "macos" {
        codecs
            .file("native/videotoolbox.c")
            .file("native/audiotoolbox.c");
        for framework in [
            "AudioToolbox",
            "CoreFoundation",
            "CoreMedia",
            "CoreVideo",
            "VideoToolbox",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
    codecs
        .file("native/vp8.c")
        .file("native/h264.c")
        .file("native/clip.c")
        .file("native/decode.c")
        .file("native/video.c")
        .include(third_party.join("libvpx/source/libvpx"))
        .include(third_party.join("openh264/src/codec/api"))
        .include(third_party.join("ffmpeg"))
        .include(third_party.join("dav1d/libdav1d/include"))
        .include(third_party.join("libyuv/include"))
        // FFmpeg's configuration for the Chrome build libwebrtc uses.
        .include(third_party.join(format!("ffmpeg/chromium/config/Chrome/{os}/{arch}")))
        .warnings(true)
        .compile("yapfast_codecs");
}
