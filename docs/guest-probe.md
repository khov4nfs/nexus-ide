# Embedded ARM64 Linux device experiment

Status: implementation / unverified device build. This is not a general-purpose
shell or agent host. Native workspace utilities remain the default. No remote
runtime or mock output is used in the app.

## Upstream evidence
- https://github.com/dnakov/litter-ish/tree/c8e9dcb954963b0d9f359b3ad9da871f19b28652/embed
- The pinned Rust host exposes IshInstance::boot, run_oneshot, spawn and
  IshSession read/write/resize/terminate. Its build.rs contains iOS cross-target
  wiring. Kernel state is process-global; upstream tests avoid reboot.
- GPL-3.0-or-later upstream host and iSH licensing / LICENSE.IOS require
  review before distributing linked binaries. This branch contains only our
  integration source, not upstream binaries or a new license decision.
- Upstream SECURITY.md explicitly says iSH is not a security boundary.
  Keep this experiment to trusted bundled commands and no secrets. Do not
  expose arbitrary guest process launch until a credible isolation model exists.

## Build (Apple Silicon macOS)
Install Xcode, Rust and Homebrew dependencies:
```sh
brew install meson ninja llvm lld libarchive sqlite xcodegen
bash scripts/build-guest-device.sh
open NexusIDEGuest.xcodeproj
```
The script checks the pinned upstream revision, builds the upstream fakefs
using Alpine 3.21.0, verifies the archive digest against Alpine's TLS-served
checksum (or caller-supplied ROOTFS_SHA256), and builds the Rust static library.
It records provenance locally. The fetched checksum is integrity evidence,
not independently authenticated package provenance or a committed digest.
Cargo dependency lock and reproducibility still need review after first build.
The separate Xcode project links the library only for physical ARM64 iOS.
Set signing team and unique bundle identifier if required. No IPA is produced.
CI checks cross-build only; the normal app and Swift tests remain independent.
The guest build script and Xcode linker settings are unverified until CI.

## Acceptance on an actual iPhone
1. Install the signed NexusIDEGuest target. Save existing work.
2. Open Linux Lab, confirm the fixed probe. Keep app foregrounded.
3. Record device model, iOS version, app commit, upstream commit, rootfs digest,
   and the JSON output. Check architecture is aarch64, echo marker matches,
   shell marker matches with exit 7, and shutdown has no error.
4. A crash, freeze, timeout, missing rootfs metadata, incorrect output, or
   inability to copy fakefs correctly is a failed gate, not a pass.
5. Restart the app before another attempt. The native and guest filesystems
   are separate for this smoke experiment. No existing project is mounted.

Next gates: validate package installation on trusted guest, then streaming
PTY/stdin/stdout/stderr and cancellation, explicit workspace mediation,
permission handling and real ACP session. Only after these gates attempt
official AGY login or OpenVSCode. No Docker or extension compatibility claims.
Boot itself may block inside upstream; fixed command timeout does not bound
boot. A stuck boot requires app termination. Do not run on production data.

## CI troubleshooting
Run 45946162960 failed during host Meson setup after successful rootfs checksum verification. Upstream suppressed stdout, so the exact configuration error was not visible. The build now explicitly selects Apple clang and the macOS SDK for the host converter, sets host-only pkg-config paths, puts Homebrew lld on PATH, and prints/uploads Meson diagnostics on failure. This is a targeted remediation, not a verified root-cause diagnosis. Device cross-build must still pass independently.
