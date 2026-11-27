# Embedded ARM64 Linux device experiment

Status: initial unsigned ARM64 device build passed (run 45946965351, commit b233c6d). Physical-device boot remains unverified; extended process probes are new. This is not a general-purpose
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
CI also runs fixed guest probes on the macOS build host using a disposable generated rootfs. This is a development test, not an execution backend of the app. The normal app and Swift tests remain independent. New revisions require fresh CI evidence.

## Acceptance on an actual iPhone
1. Install the signed NexusIDEGuest target. Save existing work.
2. Open Linux Lab, confirm the fixed probe. Keep app foregrounded.
3. Record device model, iOS version, app commit, upstream commit, rootfs digest,
   and the JSON output. Check architecture is aarch64, echo marker matches,
   shell marker matches with exit 7, separate stdout/stderr and stdin roundtrip pass, PTY reports 33 rows / 111 columns, cancellation reports signal 9, and shutdown has no error.
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

## Process I/O acceptance (v0.2.1)
The fixed shell fixture writes distinct stdout/stderr markers, blocks on a
line from stdin, then exits after echoing it. The probe requires the markers
before exit and uses an explicit sequence cursor rather than the upstream
next_seq field. A PTY fixture waits for input, is resized to 111x33, then
executes stty size. A cancellation fixture emits a readiness marker before
its process group is terminated; only a closed session with signal 9 passes.
Each read is bounded by deadline and total retained bytes; dropped output
or nonmonotonic sequence fails the probe. Errors attempt termination and
stop the remaining process tests. This is not an arbitrary process API,
interactive terminal UI, or ACP session. Boot may still block in upstream.

Host CI executes the same fixed Rust probe through the real embedded kernel
on the macOS build machine, with no remote runtime fallback in the iOS app.
The host test explicitly fails if its rootfs is missing; it is ignored by
ordinary unit tests and explicitly invoked by the workflow. Initial device
cross-build evidence does not establish the new probes' correctness.

## v0.2.2 lifecycle correction (pending validation)
CI run 45947405466 was green, but its log ended during the real guest test without final test summary or JSON. At the pinned revision kernel/exit.c calls host _exit(0) during halt before its exit callback. The supervisor also dup2s stdout onto stderr in pipe mode. Neither the old CI nor merge establishes process acceptance. The tracked, exact-source patches are limited to embedded shutdown (no host exit; fail on live guest tasks), status verification, and separate pipe stderr with bounded draining. Normal CLI halt remains unchanged. Build checks the upstream commit and refuses unexpected edits, including staged changes. No reboot support or strong guest isolation is implied. CI now requires a freshly generated JSON report after the probe returns; the verifier checks each named row, shutdown and stream evidence. A missing report fails even when cargo returns 0. Package installation is deferred until this gate passes.

## PTY tail regression (run 45948460995)
The patched iOS cross-build and two Rust unit tests passed. On macOS the real guest report returned architecture, stdout, exit code and independent stdio success with shutdownError=null. PTY failed: exit 0, output only readiness and echoed input, no stty result. Cancellation was not reached. At the pinned source, tty_read returns zero on a hung-up master before checking buffered output (err initially 0), and its raw-mode wait also discards short buffered tails on half-close. The new targeted tty.c patch allows buffered master output to drain before EIO and fixes the closed read error. Existing tty_mode_ioctl already forwards winsize to the slave; no resize semantics are changed. The test now requires both exact dimensions and a final marker written immediately before exit, without delaying exit. The fix remains unverified until the real guest test passes.

## v0.2.3 signed package installation gate (implementation, pending CI)
Run 45948969766 on commit 42160f1 confirmed all six guest process checks, PTY tail, cancellation signal 9 and a clean shutdown on macOS; unsigned ARM64 iOS build also passed. PR #3 merged at fdd343a. Physical-device execution remains unverified.

Linux Lab now offers two mutually exclusive choices per app process: offline smoke test or smoke test plus an explicit network package probe. The latter rejects a preinstalled jq, optionally writes public DNS 1.1.1.1 into the disposable guest if resolver configuration is absent, then invokes apk with the official Alpine v3.21 main HTTPS repository. It does not disable TLS or package signature verification, and disables package scripts/triggers. It validates installed package version, jq executable version and actual JSON processing. No user project is mounted, no credentials are passed, no arbitrary package selection. The repository branch is fixed, but package versions are resolved by apk and recorded in evidence rather than pinned for reproducible distribution. Alpine 3.21 is retained solely for this existing R&D rootfs; review support lifecycle and pin artifacts before production. The upstream acceptance test uses HTTP; this gate deliberately does not downgrade to HTTP if TLS or guest networking fails.

CI explicitly enables the package test, requires all seven checks and uploads JSON. Device users must explicitly select the network option. The 180-second read deadline and 64 KiB capture limit are not a guarantee against upstream boot or blocking host calls. No signed IPA, package UI, persistent package environment, Node/npm/Python, AGY or Docker is claimed. The next step is runtime/toolchain and workspace mediation after package gate passes.

## Package version evidence regression (run 45949654663)
All six process checks passed; the guest successfully downloaded the official HTTPS index, installed oniguruma 6.9.9-r0 and jq 1.8.2-r0, executed jq 1.8.2 with NEXUS_JQ_OK, exited 0 and shut down cleanly. CI failed because apk info -v jq emits description/URL/size instead of installed version. The correction queries the installed package list using apk --no-network info -v with no package argument, extracts the exact jq entry and labels both package and executable version. Rust and Python require matching upstream version plus numeric APK revision, install transition and final JSON marker. A fixture from the failed run and mismatch cases prevent weak acceptance. No TLS/signature changes or skipped runtime tests. The corrected end-to-end gate is pending CI; iPhone execution remains unverified.

## Source syntax regression (run 45950352367)
The previous version-evidence fix accidentally duplicated a suffix of process_probe.rs during source construction. Rust compilation failed at line 285 on an unexpected closing delimiter. The duplicate is removed; the existing non-package process probe and capture tests are unchanged. CI now parses every bridge Rust source using rustfmt (without applying formatting), checks the embedded package shell with sh -n, checks the build shell with bash -n and compiles Python scripts for syntax before dependency installation. These preflight checks do not replace real guest tests or the evidence verifier.
