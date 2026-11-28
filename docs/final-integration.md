# Final integration PR — v0.5.0 development candidate

## Baseline and scope

PR #17 merged at 034c977; CI run 45965660101 passed 11 Rust unit tests, nine real Alpine guest checks, both unsigned device builds, Ubuntu compatibility and ACP completion, cancellation, multi-turn, permission-denial and context fixtures. This is macOS-hosted guest evidence, not physical iPhone or AGY evidence. All further integration work is consolidated in this one PR; failed or missing validation remains a blocker, not a completed milestone.

## Implemented source paths

- Integrated Ubuntu target uses the native IDE app entry point: Files, editor, console, ACP Agent and Local tools share one native model and one boot claim. Ordinary simulator and Alpine targets remain separate.
- Explicitly attach the saved native editor file (768 UTF-8 bytes); agent returns JSON with one replacement string, max 4096 bytes. Review contiguous line diff and full before/after. Apply only on explicit user action and exact saved snapshot/current editor match. No agent-specified destination or automatic execution. External Files picker remains read-only context.
- Local PTY starts a user-reviewed shell command in the disposable Ubuntu guest, not a host shell. Line input, Ctrl-C, resize and stop via bounded mailbox. 64 KiB cumulative capture and 300s process deadline; plain text, not ANSI/full-screen terminal. No native project mount or provider key injection. Guest commands can access network and guest files; not a sandbox. Process termination alone does not guarantee descendant cleanup; report requires clean guest shutdown.
- MCP stdio discovery uses legacy protocol version 2025-11-25, initialize/notifications/initialized/tools/list; maximum 32 names, no pagination or tool execution, no raw server metadata. User explicitly supplies reviewed local command; no automatic install. Test peer runs in Ubuntu, not a production MCP server. Manual tool profiles are JSON, version 1, name, mode (pty or mcpDiscovery), command. Unknown keys rejected, no hooks/autorun/remote backend. A profile does not prove provenance; review command before running.
- Optional OpenVSCode Server release v1.109.5 Linux ARM64, exact published asset size 74488835 and SHA-256 36d9c14036489b63de84ebace837fcacf7e60e669a0dc715802c5443684ea4dc. Import overlay without extracting into host, reject unsafe paths, preserve release license, pin Ubuntu/runtime. Node/launcher digest checked at stage. Optional combined AGY import. Runtime launcher binds 127.0.0.1:18765 with 32-byte CSPRNG token, no raw server log export, NODE_OPTIONS=--jitless, separate guest working/data/extensions dirs. Only after exact bind marker, native WKWebView can load token URL; nonpersistent cookies, navigation and ordinary HTTP resource filter. This is a compatibility experiment, not proof of authenticated server or sandbox. WebSocket/filter behavior, token handling, port races, node native modules/WASM and extension host require device validation. Guest extensions may use their own network. Workspace is separate from native Files; no two-way sync.

## Prepare device build

A personal Mac is optional: see [GitHub-hosted IPA delivery](iphone-install.md). The following commands describe the local developer alternative.

On Apple Silicon macOS, install Xcode, XcodeGen, Rust and build dependencies per guest-probe.md. Use a clean checkout and fresh output directories:

```sh
bash scripts/build-guest-device.sh
bash scripts/build-glibc-rootfs.sh
# Plain Ubuntu:
bash scripts/stage-ubuntu-device.sh
```

For AGY, obtain the official Linux ARM64 binary separately under its terms, review and pin its digest using the existing prepare-agy-rootfs.py, then stage --agy instead of the plain stage. Do not execute unknown installers or redistribute AGY.

For workbench, download the exact public asset explicitly (no installer):

```sh
curl --proto '=https' --tlsv1.2 --fail --location \
  https://github.com/gitpod-io/openvscode-server/releases/download/openvscode-server-v1.109.5/openvscode-server-v1.109.5-linux-arm64.tar.gz \
  -o openvscode-server-v1.109.5-linux-arm64.tar.gz
printf '%s  %s\n' 36d9c14036489b63de84ebace837fcacf7e60e669a0dc715802c5443684ea4dc openvscode-server-v1.109.5-linux-arm64.tar.gz | shasum -a 256 -c -
python3 scripts/prepare-workbench-rootfs.py \
  --archive openvscode-server-v1.109.5-linux-arm64.tar.gz --confirm-local-use
bash scripts/stage-ubuntu-device.sh --workbench
open NexusIDEUbuntuLab.xcodeproj
```

To include both AGY and workbench, add --agy-binary /absolute/path/to/reviewed/agy --agy-sha256 REVIEWED_SHA to prepare-workbench-rootfs.py. Stage exactly once: move previous output aside explicitly; do not copy a live guest. Release import currently rejects hardlinks and missing explicit ancestor directories; if the real archive violates these constraints, importer must be reviewed rather than relaxed blindly. Actual archive conversion and OpenVSCode startup have not been tested. Digest is published GitHub integrity metadata, not independent signature verification.

One process/boot: restart app to switch Agent, PTY, MCP discovery or workbench modes. No daemon/background execution guarantee. Temporary fakefs may remain until OS cleanup. Sign locally with developer team and device; do not distribute without license/App Store review.

## Acceptance matrix — do not mark complete without evidence

| Area | Source status | Required next evidence |
|---|---|---|
| Native edit proposal | Implemented; tests added | New Swift CI, physical editor/diff/apply/conflict |
| PTY | Implemented; real guest fixture added | CI PTY input and resize, iPhone shell/stop/cleanup |
| MCP | Discovery implemented; fixture added | CI handshake/list/close, reviewed actual server on device; tool call/HTTP/agent grants not implemented |
| OpenVSCode | Pinned importer/launcher/WebView experimental | Real archive conversion, device startup, authenticated loopback, editor save, LSP, extension installation and network/isolation tests |
| Original AGY | Existing reviewed import, ACP/Keychain paths | Official binary, login/model prompt, real edit, device cancellation |
| Native ACP filesystem/terminal grants | Disabled | Separate scoped permissions and tests; reviewed native edit is not an ACP fs callback |
| Plugins/hooks | Manual command profiles only | Provenance, package installation, isolation and consented automatic lifecycle hooks not implemented |
| Other original agents / OAuth | Disabled | Official harness imports and documented protocol/auth verification |
| Docker | Disabled | Full-system kernel boot, no-JIT QEMU/UTM integration, cgroups/namespaces, dockerd, Compose, RAM/thermal testing |
| Production distribution | Blocked | Signed device evidence, licensing (including iSH exception and upstream notices), sandbox/App Store review |

No automatic approval, broad host filesystem mount, remote execution or fake success is added. ACP v1 and MCP legacy versions are pinned; newer protocol revisions need separate compatibility evaluation.

Upstream references:
- https://github.com/dnakov/litter-ish/tree/c8e9dcb954963b0d9f359b3ad9da871f19b28652
- https://github.com/gitpod-io/openvscode-server/releases/tag/openvscode-server-v1.109.5
- https://agentclientprotocol.com/protocol/v1/file-system
- https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle
- https://github.com/utmapp/UTM

For the pinned OpenVSCode v1.109.5 archive, if the release lacks a root license, obtain both `LICENSE.txt` and `ThirdPartyNotices.txt` from https://github.com/gitpod-io/openvscode-server/tree/4ffe2270acdf711bbefecc3e8c79f4b3631640e5 and pass `--source-license /path/to/LICENSE.txt --source-notices /path/to/ThirdPartyNotices.txt` to the importer. It checks the exact source Git blob hashes and preserves the notices. This does not replace distribution license review.
