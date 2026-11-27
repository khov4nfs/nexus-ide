# Integration gates (not completed)

| Gate | Candidate | Required evidence |
|---|---|---|
| G1 | litter-ish / ios-linuxkit | Pin upstream commit and license; compile for iOS ARM64 without JIT; boot guest; verify process, PTY, network, files and cancellation on real device |
| G2 | AGY CLI / ACP | Verify current official Linux ARM64 distribution, libc and syscall compatibility; supported login; initialize/session/prompt; edit a real file, run tests and demonstrate approval events |
| G3 | OpenVSCode Server | Run locally in guest; authenticated loopback preview; editor save and LSP; test official Antigravity extension compatibility and license separately |
| G4 | Codex / Claude Code / other CLI | Install supported versions; official login; tool permissions; execute and test local project |
| G5 | MCP / plugins | stdio transport, HTTP, package provenance, isolation tests and user permission UI |
| G6 | QEMU / UTM-inspired full-system emulation | Linux kernel boot with no JIT; cgroups/namespaces/network; dockerd + hello-world + Compose; record RAM, thermal and runtime; no commitment to performance |

Potential upstreams (not vendored; verify before use): https://github.com/dnakov/litter-ish ; https://github.com/rcarmo/ios-linuxkit ; https://github.com/agentclientprotocol/agent-client-protocol ; https://github.com/gitpod-io/openvscode-server ; https://github.com/utmapp/UTM ; https://antigravity.google/ .

Original Antigravity desktop in a Linux VM is an optional experiment, not the production default. Proprietary IDE/extension binaries must not be bundled without distribution rights. Apple review, platform sandbox, background execution and JIT restrictions are separate release gates.
