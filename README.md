# Nexus IDE — on-device iOS R&D

Independent prototype, not affiliated with Google. All project execution must stay on iPhone/iPad; model APIs may be used for inference only. No SSH or cloud compute fallback. Target iOS 17+ (provisional).

## Implemented in 0.1.0
- Swift package with local workspace, UTF-8 editing, path validation and basic file tools.
- Native SwiftUI app source with file explorer/editor, console (ls/cat/echo/pwd only), capability status.
- ACP JSON-RPC request encoding (initialize, session/new, session/prompt) and bounded newline framing. **No live ACP client transport or agent loop yet.**
- Six core tests passed with Swift 6.2.1 on Linux. iOS build and physical device tests not performed.

## Build
On macOS with Xcode and XcodeGen:

```sh
swift test
brew install xcodegen
xcodegen generate
open NexusIDE.xcodeproj
```
Select your signing team and unique bundle identifier, then build/run on an iPhone. CI is configured for an unsigned iOS Simulator build; it is not a signed IPA release. The development machine is needed for building/signing, not app execution.

## Roadmap
See [architecture](docs/architecture.md) and [upstream integration gates](docs/runtime-gates.md). AGY, OpenVSCode, embedded Linux, MCP, Docker and extensions are not bundled or operational. No upstream code or proprietary binaries are redistributed. Licensing and App Store eligibility must be reviewed before distribution.

## Experimental on-device Linux probe (0.2 development)

The Linux Lab tab is disabled in the ordinary build. An opt-in device build links
the pinned litter-ish Rust host library in-process, copies a bundled fakefs into
a disposable writable directory, and runs a fixed smoke probe (ARM64, shell,
stdout and nonzero exit). See [device build and acceptance gate](docs/guest-probe.md).
This is not an available general Linux terminal or an AGY integration yet.
