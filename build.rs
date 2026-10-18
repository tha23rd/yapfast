//! Gives the Windows executable its icon: compiles packaging/windows/yapfast.rc
//! with LLVM's resource compiler and links the result. On macOS, links all
//! of libwebrtc's Objective-C.

use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        objective_c();
    }
    if target_os != "windows" {
        return;
    }
    let resources = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("yapfast.res");
    // The script names only the icon beside it, so it needs no preprocessor.
    let status = Command::new("llvm-rc")
        .args(["-no-preprocess", "-FO"])
        .arg(&resources)
        .arg("packaging/windows/yapfast.rc")
        .status()
        .unwrap_or_else(|error| panic!("could not run llvm-rc: {error}"));
    assert!(status.success(), "llvm-rc could not compile yapfast.rc");
    // The MSVC linker takes compiled resources as an input.
    println!("cargo:rustc-link-arg-bins={}", resources.display());
}

/// libwebrtc's Objective-C categories, such as the one on NSString its video
/// codecs call as LiveKit starts, hold no symbol the linker looks for, so it
/// leaves them out of the archive unless told to load every Objective-C
/// member, as LiveKit's documentation says. Without them LiveKit's first
/// call ends Yapfast with an unrecognized selector.
fn objective_c() {
    println!("cargo:rustc-link-arg=-ObjC");
}
