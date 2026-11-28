# Reviewed optional and project-wide language servers

This extends the existing on-device Workbench and native snapshot clients. It is not a certification of all toolchains on iPhone.

## Workbench profiles

Default pinned servers remain pylsp 1.10.0 and clangd 18.1.3. Optional profiles are TypeScript/JavaScript (including TSX and JSX) via typescript-language-server, Rust via rust-analyzer, Go via gopls, and Bash via bash-language-server. No additional toolchain binary is downloaded, selected or bundled by default. Optional profiles require an independently reviewed installed entry and dependencies. At most four Workbench server sessions can exist at once. Use Stop/Restart Language Servers to manage resources. Server failure never triggers an automatic crash loop. LSP response capability determines whether a feature is available.

For a private reviewed build, prepare an ARM64 offline tar.gz with these relative paths for the selected servers:

    manifest.json
    notices/
    typescript/node_modules/typescript-language-server/lib/cli.mjs
    typescript/node_modules/typescript/lib/tsserver.js
    bin/rust-analyzer
    bin/gopls
    bash/node_modules/bash-language-server/out/cli.js

Only selected entries are required. Include their full dependencies and license/source notices. Rust needs a compatible reviewed cargo/rustc and standard library sources; Go needs a compatible reviewed Go toolchain and locally available modules. JS servers must be compatible with the pinned Workbench Node runtime; use exact package/lockfile versions, not latest or arbitrary install scripts. The archive must contain explicit regular-file/directory ancestors, no symlinks or hardlinks; omit npm bin links and use the fixed JS entrypoints. An example manifest shape (replace all placeholders with verified values; not approval):

    {"schemaVersion":1,"architecture":"arm64","servers":{"typescript":{"version":"REVIEWED_SERVER_VERSION","entrySHA256":"REVIEWED_64_HEX_DIGEST"}}}

Use prepare-workbench-rootfs.py with its existing required options plus --lsp-bundle /path/to/reviewed.tar.gz --lsp-bundle-sha256 <reviewed hash>. It checks archive integrity, entry hashes, native ELF ARM64 headers, notices, limits and path confinement, then overlays under /opt/nexus/lsp without extracting on the host. The whole bundle's license/source obligations and executable dependencies still require owner review. This is an optional local staging path, not enabled in the final release workflow. No approval or binary is fabricated by CI.

In the trusted Workbench workspace, invoke Nexus: Enable Reviewed Language Server. Select the profile, enter the exact reviewed serverInfo.version and SHA-256 of its entry, and explicitly approve. The entry digest is checked again before launch. If the server omits identity, entry hash is the only identity check; do not count it as protocol version verification. The optional TS client can duplicate the built-in OpenVSCode TS extension; disable the overlapping provider explicitly when testing the optional client. All servers run locally in the guest. Go module/toolchain network downloads and Rust cargo network/build-script/proc-macro/check-on-save are disabled by the configured settings/environment, not a security sandbox. Server versions can differ in supported settings, diagnostics version support and protocol capabilities.

## Cross-file operations

In a trusted guest project, enable project indexing using Nexus: Enable Project Indexing (clangd background indexing, off by default; restarts existing sessions, retains explicit optional approvals for this Workbench boot). clangd config and clang-tidy remain disabled, and compile database/toolchain availability determine index coverage. The real-URI client syncs up to 12 open snapshots before definitions, references, workspace symbols, rename and code actions. Project operations are disabled for snapshot-URI Python LSP, which cannot safely identify real module paths. Other servers may support different capabilities; unversioned asynchronous diagnostics for real-URI profiles are not projected.

Nexus: Review Symbol Rename and Nexus: Review Refactoring show the full before/after text in a read-only virtual editor, require explicit apply, and do not automatically save. Only text-only server-returned WorkspaceEdit is allowed (32 affected files, 512 edits, 128 KiB per file). Baselines cover up to 128 allowlisted source files and 2 MiB, and are checked again after server response and after review, including file-set membership. Reject symlinks, hidden/dependency/generated paths, stale versions, overlapping edits, invalid UTF-16, file create/delete/rename operations, additional server commands and edit annotations. Code actions that require resolve or command execution are not supported. This is bounded project-wide analysis, not unrestricted indexing or full parity with desktop IDEs. A project requiring more limits or arbitrary refactors fails closed.

The new Chromium gate invokes the synthetic cross-file acceptance command after the existing real Python/C/C++ tests. It creates only a new synthetic fixture directory, checks actual clangd definition/references across two open files and validates its two-file rename edit, applies it only to the synthetic files, saves them and verifies exact bytes. It does not qualify arbitrary refactoring, whole-project indexing or the optional server binaries.

## Native Swift editor

Local tools > Live editor LSP uses one reviewed argv and supports up to eight logical native files in the same language family and 64 reviewed saved snapshots. Start with a saved file and approved server. Return to Files, open another saved compatible file, tap Refresh LSP, review the snapshot, and send. Native diagnostics are cached by file and exact text. Rust mailbox checks generation, file identity, language family, size and count, and keeps guest documents open using unique virtual URIs to reject stale/unversioned diagnostics. No native project mount, cross-file semantic index, multi-server native session or native refactoring transaction is implied. Native completion remains the existing separate reviewed snapshot mode. Session state does not survive iOS app termination.

The added real-pylsp-multi-document CI gate exercises A, B, refreshed A in one process, rejects stale generation and wrong family, checks distinct diagnostics and clean server/guest exit. Physical iPhone checks still require the signed exact IPA, actual provider tools, operator observations and source review.

## Local verification

In the exact nexus-ide checkout at the PR head:

    node --check scripts/workbench-lsp-session.js
    node --check scripts/workbench-real-lsp-extension.js
    node --test scripts/test-workbench-lsp-session.js scripts/test-workbench-lsp-edits.js
    python3 scripts/test-workbench-import.py
    python3 scripts/test-release-acceptance.py
    python3 scripts/test-device-acceptance.py
    swift test

Run RuntimeBridge build/guest tests on the documented macOS runner with the pinned upstream runtime/fakefs; Xcode/iPhoneOS build and signed physical-device acceptance are separate. Do not treat pure Node tests or a Linux Swift core build as iPhone verification.

SourceNerve workspace listing did not contain khov4nfs/nexus-ide when this change was implemented; no local SourceNerve run was possible. The separate container executed the exact Git-blob-matched workspace-edit planner and its three tests, all passed. No full repository checkout was available in that container; the GitHub hostname was not resolvable. The other tests are wired to CI, not claimed passed here.

## Pinned TypeScript/JavaScript implementation

The Workbench CI candidate now stages TypeScript 5.7.3 and typescript-language-server 4.3.4 from scripts/lsp-typescript/package-lock.json via npm ci --ignore-scripts. The builder checks the reviewed server entry hash and copies license notices, runs the real host regression, then archives the JS packages. Workbench CI runs the same real two-file TS/JS protocol test inside the guest via the reviewed VSIX. It requires definition, references, symbol rename from the exported declaration, completion, one server initialization for both languages, all documents closed and clean process exit. The IPA verifier checks the staged manifest, entry hash and package versions. The test exercises protocol behavior and edit planning, not the native Swift UI or all possible TS refactors. Optional activation still requires explicit reviewed-server consent. Review the new packages and actual source/notice obligations before release.

SourceNerve now has the nexus-ide workspace. Exact-commit archived tests at 4c7d7f7 passed 6 Node cases and 9 Python cases. The local npm-locked real TS/JS test and bundle/VSIX build passed at 73a1178. Local installed rust-analyzer 1.88.0 passed the two-file real protocol test at cda4696 (host x86_64, not ARM64 guest). Swift and Xcode are not installed on this Linux test host. No device verification is claimed.

Go host verification at cda4696 passed with gopls v0.18.0 built using Go 1.23.6, checksum database enabled. Building the same gopls version with the workstation's Go 1.26 failed because of the pinned x/tools tokeninternal layout assertion; the test used a temporary official toolchain, without replacing system Go. Rust Analyzer and Go real host tests both verified two-file definition, references, rename planning and clean persistent process exit. The integrated workflow now requires the separate ARM64 Linux host regression for these pinned toolchains. They remain optional guest profiles: full ARM64 guest toolchain packaging, runtime qualification, and Bash real-server qualification are outstanding. Latest portable tests at b5eb2b2: six Node tests and ten Python tests passed, along with Rust/shell/Python syntax preflight and VSIX creation. No full Rust bridge link, Swift build, or iPhone acceptance on the Linux host.
