# Antigravity compatibility gate — v0.3.0 (R&D)

PR #6 merged at dead7d4; run 45953554823 verified all nine Alpine guest checks and clean shutdown on macOS, plus unsigned ARM64 iOS build. Ubuntu is a separate compatibility lane, not a replacement for Alpine.

Official references checked 2026-11-27:
- https://antigravity.google/download?platform=linux (Linux glibc >= 2.28, glibcxx >= 3.4.25)
- https://antigravity.google/docs/ide/extensions/acp (official agy acp, ACP protocol 1, stdio, --disable-telemetry)
- https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release/SHA256SUMS (Ubuntu Base 24.04.5 ARM64 digest a91d5a93010193712d346d761372b7c9db6dfcf093893161c64ca107f05914f2). Pinned HTTPS-published digest, not independently verified GPG signature.

Build the existing Alpine device target first, then run bash scripts/build-glibc-rootfs.sh. The script converts the official pinned Ubuntu archive with the same fakefs converter; it refuses to overwrite an existing Ubuntu rootfs. CI starts a separate Rust test process, boots Ubuntu, runs uname, getconf, apt-get --version and checks the libstdc++ version marker. The marker check is not a comprehensive dynamic dependency audit. It requires a fresh JSON report and clean shutdown. The Ubuntu lane is host testing only; iOS still bundles Alpine.

## Optional official AGY initialize

The developer must obtain the original Linux ARM64 AGY distribution from Google under applicable terms, verify its provenance and prepare a disposable Ubuntu fakefs with the executable at /opt/nexus/agy and any required dependencies. This PR does not automate binary import or run an installer. Do not place credentials or project data in this image. Do not copy files directly into fakefs/data without registering fakefs metadata; prepare a trusted archive with the official files and use the upstream fakefsify converter before boot. No AGY binary is redistributed or downloaded by CI.

Use the same macOS build environment as guest-probe.yml, in a fresh process:
```sh
export NEXUS_GLIBC_ROOTFS=/absolute/path/to/prepared/fs/data
export NEXUS_GLIBC_EVIDENCE=/absolute/path/to/new/report.json
export NEXUS_TEST_AGY=1
rm -f "$NEXUS_GLIBC_EVIDENCE"
cargo test --manifest-path RuntimeBridge/Cargo.toml --lib real_glibc_compatibility -- --ignored --nocapture --test-threads=1
python3 scripts/verify-glibc-evidence.py "$NEXUS_GLIBC_EVIDENCE" --agy
```

The optional test launches only /opt/nexus/agy acp --disable-telemetry via the real guest process API, writes ACP initialize, checks protocol version and agent identity, terminates and drains the process, then shuts down. It does not send session/new, prompt, login, secrets or permission grants. No claim of hard network isolation: the binary can perform its own startup operations. The identity check is protocol evidence, not a cryptographic provenance check. Authentication errors fail rather than masquerade as success. CI does not exercise this optional path, and no native ACP transport or device agent UI is implemented here. Boot may block upstream; CI job timeout is the outer bound. No remote execution backend or permission bypass.

Next acceptance: real AGY initialize evidence from the original binary, device glibc boot, then native Swift ACP transport and session lifecycle. If Ubuntu fails, diagnose the guest kernel/ABI rather than silently replacing glibc with musl or a mock.

## v0.3.1 local import and separate device lab (pending CI)
PR #7 merged at 9c71bbf: run 45954632541 verified all nine Alpine checks, glibc 2.39, GLIBCXX marker and clean Ubuntu shutdown. It explicitly did not execute AGY. Google documents the official installer and CLI location ~/.local/bin/agy (https://antigravity.google/docs/cli/getting-started) and disabling auto-update (https://antigravity.google/docs/cli/troubleshooting). The installer content could not be inspected in this session; no automatic installer execution or guessed download URL. Obtain the original Linux ARM64 binary from the official distribution in accordance with its terms, review its provenance independently, then pin its SHA-256. Do not import macOS binaries, credentials, user config or arbitrary helper executables. This only imports the main ELF; if the official distribution needs additional runtime files, stop and review separately.

After building the pinned runtime and Ubuntu rootfs (before boot), run:
```sh
python3 scripts/prepare-agy-rootfs.py --binary /path/to/reviewed/linux-arm64/agy --sha256 <reviewed-lowercase-sha256> --confirm-local-use
export NEXUS_GLIBC_ROOTFS="$PWD/Vendor/litter-ish/build/agy-fs/data"
export NEXUS_AGY_SHA256="$(cat Vendor/litter-ish/build/agy-fs/agy.sha256)"
export NEXUS_GLIBC_EVIDENCE="$PWD/Vendor/litter-ish/build/agy-evidence.json"
export NEXUS_TEST_AGY=1
# Use the same macOS SDK and compiler env as CI.
cargo test --manifest-path RuntimeBridge/Cargo.toml --lib real_glibc_compatibility -- --ignored --nocapture --test-threads=1
python3 scripts/verify-glibc-evidence.py "$NEXUS_GLIBC_EVIDENCE" --agy
```
The importer verifies the pinned Ubuntu archive, exact AGY digest and ELF64 AArch64 header, then overlays the executable in a new archive without host extraction and converts through fakefsify. SHA pin is integrity, not publisher authenticity. Unit tests use synthetic headers only; CI never executes or imports a proprietary binary. The optional runtime path verifies the actual guest executable hash before launch, disables documented auto-update, requires a real ACP response, and never supplies credentials or prompts. No hard network isolation. Authentication or missing dependencies may block initialize. A failed handshake is not success.

Separate unsigned iOS target: run bash scripts/stage-ubuntu-device.sh for pristine Ubuntu, or bash scripts/stage-ubuntu-device.sh --agy for a reviewed locally prepared, unbooted image. Stage only one image; move any prior RuntimeAssets/ubuntu-fs aside manually. For device builds stage before running host probe, or create a fresh image. Open NexusIDEUbuntuLab.xcodeproj and select your signing team. The separate SwiftUI lab supports offline libc checks and explicit AGY initialize only if a digest manifest exists. CI builds the Ubuntu target without AGY; no binary artifacts published. Do not distribute AGY-containing builds. Device execution, licensing for distribution, persistent agent sessions, authentication/Keyring, native ACP editor transport remain unverified. The original Alpine target is unchanged.

## v0.3.2 native initialize-only ACP client (pending CI)
PR #8 merged at b400435, run 45955750691 verified both unsigned device builds, importer tests, nine Alpine checks and Ubuntu compatibility; no AGY execution. Official ACP documentation https://antigravity.google/docs/ide/extensions/acp describes stdio and authentication-required code -32000. Ubuntu Lab now uses NexusCore to generate the initialize request, passes it through a dedicated FFI entry point, and Rust validates the exact JSON shape before boot/launch (id 1, protocol 1, no advertised tool capabilities). Existing CLI probe uses the same fixed request. Rust projects only bounded identity/protocol fields or a sanitized error code and message; agent requests are rejected, raw notifications, stderr and error data are not exported. Swift validates the projected response, correlates the request id, distinguishes explicit authentication-required from other errors and reports success only after clean probe termination/shutdown. This is a one-shot initialize exchange, not live session streaming, login, prompt, filesystem/tool callbacks or a full agent panel. No new credentials or permission grants. Swift and Rust unit tests use protocol fixtures only; CI still does not execute AGY. Actual binary and physical iPhone remain required.
