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
