#!/usr/bin/env python3
"""Package the macOS release for Apple silicon: Yapfast.app, with the
dependencies' licenses and notices inside it, on a disk image, beside the
source snapshot.

The app is signed ad hoc, which Apple silicon requires of any program;
without a Developer ID signature and Apple's notarization, Gatekeeper asks
before the first start. scripts/build-macos builds the executable and runs
this."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import urllib.parse

from release_package import ROOT, dependency_notices, digest, names_home, source_snapshot

TARGET = "aarch64-apple-darwin"
# Cargo features, and the suffix of file names and release tag, for each
# flavour scripts/build-macos builds.
FLAVOURS = {
    "discord": {"features": ["--no-default-features", "--features", "desktop,discord"], "suffix": ""},
    "full": {"features": [], "suffix": "-full-preview"},
}
# Mach-O, 64-bit, for arm64.
MACH_O_ARM64 = (0xFEEDFACF, 0x0100000C)


def skia_source():
    """Skia's source with the third-party code its archives are built from,
    as nix/flake.nix assembles it for the Linux and Windows packages, whose
    notices the prebuilt archive lacks. Kept in .local/macos."""
    flake = (ROOT / "nix/flake.nix").read_text()
    url, sha256 = re.search(
        r'windowsSkiaSourceArchive = pkgs\.fetchurl \{\s*url = "([^"]+)";\s*sha256 = "([0-9a-f]{64})";',
        flake).groups()
    # Nix pins each dependency by its archive's contents; Git pins the
    # same commit by its hash.
    dependencies = re.findall(r'skiaDependency "([^"]+)" "([^"]+)/\+archive/([0-9a-f]{40})\.tar\.gz"', flake)
    pins = hashlib.sha256(json.dumps([url, sha256, dependencies]).encode()).hexdigest()
    cache = ROOT / ".local/macos"
    source = cache / "skia-source"
    stamp = cache / "skia-source.json"
    if source.is_dir() and stamp.is_file() and json.loads(stamp.read_text()).get("pins") == pins:
        return source
    # The stamp goes first, so a fetch that fails partway isn't taken later.
    stamp.unlink(missing_ok=True)
    shutil.rmtree(source, ignore_errors=True)
    archive = cache / "skia-source.tar.gz"
    if not archive.is_file() or digest(archive) != sha256:
        subprocess.run(["curl", "--fail", "--location", "--silent", "--show-error",
                        "--output", str(archive), url], check=True, timeout=600)
        if digest(archive) != sha256:
            archive.unlink()
            raise SystemExit(f"{url} doesn't match its pinned SHA-256")
    source.mkdir(parents=True)
    subprocess.run(["tar", "-xzf", str(archive), "--strip-components=1", "-C", str(source)],
                   check=True, timeout=600)
    for name, repository, commit in dependencies:
        destination = source / "third_party/externals" / name
        destination.mkdir(parents=True, exist_ok=True)
        git = ["git", "-C", str(destination)]
        subprocess.run([*git, "init", "--quiet"], check=True, timeout=60)
        subprocess.run([*git, "fetch", "--quiet", "--depth", "1", repository, commit], check=True, timeout=600)
        subprocess.run([*git, "checkout", "--quiet", "FETCH_HEAD"], check=True, timeout=600)
        shutil.rmtree(destination / ".git")
    stamp.write_text(json.dumps({"pins": pins}) + "\n")
    return source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, default=ROOT / "target" / TARGET / "release/yapfast")
    parser.add_argument("--flavour", choices=FLAVOURS, default="discord",
                        help="the features the executable was built with (default: discord)")
    parser.add_argument("--output", type=Path, help="default: target/dist/macos")
    args = parser.parse_args()
    flavour = FLAVOURS[args.flavour]
    output = args.output or ROOT / "target/dist/macos"
    executable = args.exe.resolve()
    newest_input = max(path.stat().st_mtime for path in
                       [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "build.rs",
                        *list((ROOT / "src").rglob("*.rs")), *list((ROOT / "ui").rglob("*"))]
                       if path.is_file())
    if executable.stat().st_mtime < newest_input:
        raise SystemExit("macOS executable predates the current source; run scripts/build-macos first")
    binary = executable.read_bytes()
    if (int.from_bytes(binary[:4], "little"), int.from_bytes(binary[4:8], "little")) != MACH_O_ARM64:
        raise SystemExit("Expected an arm64 Mach-O executable")
    if names_home(binary):
        raise SystemExit(f"{executable} names {Path.home()}; build it with scripts/build-macos")
    if (b"/matrix-rust-sdk-" in binary) != (args.flavour == "full"):
        raise SystemExit(f"{executable} isn't the {args.flavour} flavour; run scripts/build-macos {args.flavour}")
    del binary
    # Only the packages this flavour's features build.
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", TARGET,
         *flavour["features"]],
        cwd=ROOT, timeout=60,
    ))
    version = next(package["version"] for package in metadata["packages"] if package["name"] == "yapfast")
    skia_url = urllib.parse.urlparse(os.environ["SKIA_BINARIES_URL"])
    if skia_url.scheme != "file":
        raise SystemExit("Packaging requires the pinned local Skia archive")
    skia_archive = Path(urllib.parse.unquote(skia_url.path))
    webrtc = Path(os.environ["LK_CUSTOM_WEBRTC"])
    webrtc_archive = Path(os.environ["YAPFAST_WEBRTC_ARCHIVE"])
    output.mkdir(parents=True, exist_ok=True)
    base = f"Yapfast-{version}{flavour['suffix']}"
    image_path = output / f"{base}-macos-arm64.dmg"
    source_path = output / f"{base}-source.zip"
    with tempfile.TemporaryDirectory(prefix="yapfast-macos-package-") as temporary:
        temporary = Path(temporary)
        rtc = source_snapshot(metadata, base, temporary, source_path)

        image = temporary / "image"
        app = image / "Yapfast.app"
        contents = app / "Contents"
        resources = contents / "Resources"
        (contents / "MacOS").mkdir(parents=True)
        resources.mkdir()
        shutil.copy2(executable, contents / "MacOS/yapfast")
        (contents / "Info.plist").write_text(
            (ROOT / "packaging/macos/Info.plist").read_text().replace("@VERSION@", version))
        shutil.copy2(ROOT / "packaging/macos/Yapfast.icns", resources / "Yapfast.icns")
        shutil.copy2(ROOT / "LICENSE", resources / "LICENSE.txt")
        # The source snapshot goes beside the image, on the release page,
        # rather than into every install; SOURCE.txt says where.
        source_url = f"https://github.com/yapfast/yapfast/releases/tag/v{version}{flavour['suffix']}"
        (resources / "SOURCE.txt").write_text(
            f"The source code of this build of Yapfast {version}"
            + (", with the exact source of\nthe libraries it includes under the AGPL," if rtc else "")
            + " is published with it:\n\n"
            f"  {source_url}\n  {source_path.name}, SHA-256 in BUILD.json\n\n"
            "and in the repository, https://github.com/yapfast/yapfast. The licenses folder\n"
            "holds every dependency's own license and notices.\n",
            encoding="utf-8",
        )
        dependencies = dependency_notices(metadata, resources, resources / "licenses", skia_archive,
                                          skia_source(), webrtc)
        (resources / "THIRD-PARTY.json").write_text(json.dumps(dependencies, indent=2) + "\n", encoding="utf-8")
        build = {
            "version": version, "flavour": args.flavour, "target": TARGET,
            "cargo_features": " ".join(flavour["features"]) or "default",
            "rustc": subprocess.check_output(["rustc", "--version"], text=True, timeout=10).strip(),
            "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True,
                                                     timeout=10).strip(),
            "macos_minimum": "11.0",
            "signature": "ad hoc",
            "ui_framework": "Slint 1.18.1",
            "renderer_default": "skia-software",
            "renderer_gpu": "skia-metal",
            "skia_version": "0.153.3",
            "skia_archive_sha256": digest(skia_archive),
            "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
            "executable_sha256": digest(executable),
            "source_archive_sha256": digest(source_path),
            "webrtc_tag": "webrtc-51ef663",
            "webrtc_archive_sha256": digest(webrtc_archive),
        }
        if rtc:
            build["homeserver_default"] = "https://matrix.org"
        (resources / "BUILD.json").write_text(json.dumps(build, indent=2) + "\n", encoding="utf-8")
        # Apple silicon runs only signed code. Signing seals the bundle, so
        # nothing in it may change after. An ad hoc signature's own designated
        # requirement is the build's hash, so macOS would take each update for
        # another app and ask again for the microphone and the Keychain item;
        # one on the bundle identifier keeps them. See docs/macos.md.
        subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none",
                        '-r=designated => identifier "io.github.yapfast.Yapfast"', str(app)],
                       check=True, timeout=300)
        subprocess.run(["codesign", "--verify", "--strict", str(app)], check=True, timeout=300)
        # Dragging the app onto Applications installs it.
        (image / "Applications").symlink_to("/Applications")
        image_path.unlink(missing_ok=True)
        create = ["hdiutil", "create", "-quiet", "-volname", "Yapfast", "-srcfolder", str(image),
                  "-fs", "HFS+", "-format", "ULFO", str(image_path)]
        # hdiutil sometimes fails with "Resource busy" on GitHub's runners; it
        # gets one more try.
        if subprocess.run(create, timeout=1800).returncode:
            image_path.unlink(missing_ok=True)
            time.sleep(10)
            subprocess.run(create, check=True, timeout=1800)
    hashes = output / "SHA256SUMS.txt"
    artifacts = [image_path, source_path]
    hashes.write_text("".join(f"{digest(path)}  {path.name}\n" for path in artifacts), encoding="utf-8")
    print(f"Packaged {image_path} ({image_path.stat().st_size / 1024 / 1024:.1f} MiB)")
    print(f"Source snapshot: {source_path}")
    print(f"Checksums: {hashes}")


if __name__ == "__main__":
    main()
