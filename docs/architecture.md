# Architecture (proposed)

Native SwiftUI shell + optional local Code-OSS/OpenVSCode workbench in WKWebView. Both use one canonical workspace, with explicit filesystem mediation and edit-conflict handling. Original agent harnesses run inside a local guest, not in a remote service. Native UI communicates using ACP/JSON-RPC or provider-specific documented protocols. No claim of interchangeable agent sessions: test reconnect separately.

```
SwiftUI native IDE  |  WKWebView local workbench (planned)
          |                      |
       runtime / permission / session bridge
          |                      |
 native file tools       embedded Linux userland (planned)
                            |-- AGY ACP / other CLIs
                            |-- extension host / LSP / MCP stdio
 optional full Linux VM (planned) -- Docker experiment
```

The runtime protocol intentionally exposes only verified capabilities. Guest transport requires process start/stop, stdin/stdout/stderr separation, PTY, filesystem bridge, cancellation, bounded output and network controls. ACP requires request correlation, server notifications, permission replies, protocol negotiation and recovery before activation. Native file path validation is defense in depth for trusted UI only: TOCTOU/hardlink risks remain against concurrently executing malicious code. Do not give arbitrary guest code host filesystem access. Design and test a separate isolation boundary before running untrusted extensions.

Secrets belong in Keychain, scoped to individual agent providers. Do not copy credentials into guest environment or logs by default. Bind preview to loopback with authentication and verify origin; do not expose guest management APIs to web content. Agent autonomy is bounded by iOS lifecycle; checkpoint and resume rather than promising daemon-like background execution.
