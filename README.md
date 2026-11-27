# Nexus IDE — on-device iOS R&D

Independent prototype, not affiliated with Google. iOS 17+; all project execution is local. Model inference APIs and explicit downloads may use the network. No remote shell, remote IDE, remote agent runtime or Docker service fallback.

## Build variants

| Target | Purpose |
|---|---|
| NexusIDE | Native Files, UTF-8 editor, bounded native console; simulator build |
| NexusIDEGuest | Pinned ARM64 Alpine in-process runtime and offline/network opt-in probes |
| NexusIDEUbuntuLab | Integrated native Files + Console + Agent + Local tools; disposable Ubuntu guest, optional reviewed AGY and pinned OpenVSCode |

Previous PR #17 CI passed native tests, both unsigned ARM64 device builds, real macOS-hosted guest process/toolchain tests and ACP fixtures. No physical iPhone or original AGY execution has been verified. This integration PR adds edit proposals, actual local PTY and MCP discovery paths, optional workbench launcher; new CI is required. No signed IPA or production release.

## Build on Apple Silicon macOS

Use Xcode and XcodeGen. For the native target:

```sh
swift test
brew install xcodegen
xcodegen generate
open NexusIDE.xcodeproj
```

For the device target, follow [integrated build instructions](docs/final-integration.md). Select your signing team in Xcode. The build host is only for preparation/signing/testing; no execution backend is required by the app.

## Architecture and evidence

See [architecture](docs/architecture.md), [integration gates](docs/runtime-gates.md), [guest evidence](docs/guest-probe.md), and [AGY compatibility](docs/agy-compatibility.md).

The embedded runtime is not a security sandbox. Agent uses an explicitly consented credential and selected context only; no unrestricted native file/tool grants. One guest boot per app process. Do not use production secrets or private code in the lab. Licensing, App Store eligibility, original AGY authentication, OpenVSCode compatibility, full VM/Docker and physical-device validation remain release gates.
