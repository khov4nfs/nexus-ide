# iPhone delivery without a personal Mac

## Download and install

GitHub-hosted macOS runners do the compilation. The iPhone runs the actual IDE locally; no remote runtime is added.

1. Open https://github.com/khov4nfs/nexus-ide/actions in Safari, log into GitHub, select a successful run of **Core and unsigned iOS build**, and download artifact **NexusIDE-native-unsigned**. Extract the downloaded ZIP in Files; inside is the real ARM64 .ipa, SHA256SUMS and build.json. Native-only: Files/editor/limited console, no embedded Linux.
2. For the integrated Ubuntu version, first review upstream/runtime/rootfs redistribution terms, corresponding source and license notices. On the merged main branch, Actions → **Experimental iOS guest build** → Run workflow; choose ubuntu or workbench, acknowledge license_reviewed only after completing the review, optionally select publish_prerelease. Download **NexusIDE-integrated-unsigned** from the successful run. The workbench option also imports the pinned public ARM64 archive; a failure blocks the artifact. Neither includes proprietary AGY. Both include a non-ELF import marker in the stopped fakefs, replaced only by user-selected bytes.
3. If you already have a legitimately configured on-device sideloader, e.g. SideStore (https://docs.sidestore.io/), import the IPA from Files and let it sign with your own account. Initial setup/pairing depends on your iOS version and may need Windows/Linux/macOS; no personal Mac is required for this project. Do not give your Apple credentials to this repository or untrusted signing services.
4. If you want direct installation without a sideloader, use a valid Apple development/ad hoc profile for your registered device or TestFlight. The workflow does not obtain Apple certificates, profile, account access or device UDID and does not claim to sign. Never publish a profile containing private device IDs or signing secrets in a public artifact. See https://developer.apple.com/documentation/xcode/distributing-your-app-to-registered-devices and https://developer.apple.com/testflight/.

For direct GitHub Release asset downloads, on main manually run **Core and unsigned iOS build** with publish_prerelease=true, or the integrated workflow with both license_reviewed and publish_prerelease. Releases are explicitly labeled prerelease; unique run/attempt tags target the exact commit. Download the IPA asset, not source ZIP. A GitHub release asset is not an Apple OTA installation manifest or a signed app.

## What the packaging gate proves

The ZIP contains exactly one Payload/*.app. It checks the expected bundle ID, supported platform iPhoneOS, minimum iOS 17.0, executable permission, thin ARM64 Mach-O and LC_BUILD_VERSION iOS (simulator rejected), CRC, fakefs assets for integrated builds and absence of proprietary AGY and provisioning profile. It records exact source commit, variant, build/version, SHA-256 and size in build.json. Signing is deliberately not performed; an unsigned package is NOT an installable signed application. No fake simulator IPA, automatic signing, stolen certificate or bypass of platform controls.

The integrated workflow keeps all existing guest/ACP/PTY/MCP fixture gates before Release packaging. PR runs verify packaging but do not distribute the linked guest binary. Public release/upload is gated by the owner's explicit license review on main; this checkbox does not itself provide legal compliance. Supply corresponding source, license texts and third-party notices (including guest packages and OpenVSCode if selected) as required before distribution. No proprietary AGY download/upload; see docs/agy-compatibility.md for local reviewed import. For personal AGY testing on the integrated IPA, obtain a permitted official Linux ARM64 executable and its reviewed SHA-256 separately. Open Agent tab, enter lowercase SHA-256, choose the ELF executable in Files (not a tar/zip), wait for validation and explicitly connect. The app streams bytes into a private temporary file, verifies ELF64/AArch64 header and digest, then copies and re-verifies into the pre-created fakefs slot before guest boot. Rust verifies digest again inside guest before execution. No installer, proprietary binary in the IPA, automatic download, publisher identity proof or redistribution. Re-import after app restart; temporary copies may remain until OS cleanup. New CI gate substitutes Ubuntu /usr/bin/true in the same slot and verifies actual execution and clean shutdown, not AGY authentication. Actual device import, official AGY and model access remain unverified.

## Device acceptance checklist

- Install and launch the signed test IPA on iPhone with iOS 17+; record model/iOS/build.json commit and installation method (no device identifiers or credentials).
- Create, save, reopen native file; confirm workspace persists across app restart.
- Integrated variant: run offline libc gate, restart app, PTY command/input/Ctrl-C/stop; inspect actual evidence and clean shutdown. Separate boot for MCP and workbench.
- If workbench staged: verify loopback page, editor save, LSP and extension behavior; do not infer from a successful build.
- Original AGY: only after reviewed permitted binary import and restricted credential; actual authentication, streaming, cancellation and reviewed edit still need device proof.
- Verify app relaunch after suspension and memory/thermal conditions. Do not claim full VM/Docker, MCP tool execution, unrestricted native tools, other original harnesses or automatic hooks; these remain incomplete.

Report failures on GitHub with redacted logs. Never share API keys, pairing files, signing identities, or raw project data. No production readiness claim until these gates pass.

## Integrated candidate acceptance metadata (0.7.0)
The integrated workflow attaches NexusIDE-integrated-acceptance even to successful PR runs (metadata only, no linked guest IPA). acceptance.json binds the exact build commit, IPA SHA-256, build.json hash and each required CI report hash with explicit test scope. Missing/failed evidence or a changed IPA blocks packaging acceptance. This is CI acceptance, not a production approval. The manual main/owner license review gate still governs distributing the unsigned integrated IPA; accepted release bundles include acceptance.json. For native Codex/Claude conversations, first use the original shell login in a saved guest, exit cleanly, restart the app, reimport the exact reviewed binary, select native conversation and saved guest. Copy the session ID if you need resume; finish cleanly. Run LSP diagnostics with a reviewed server already installed in saved local tools. No real provider/device/extension activation claim follows from the fixture checks.

For workbench builds the optional CI workflow now additionally requires Chromium-driven editor save and synthetic extension activation, with exact guest bytes after clean shutdown. The browser runs only on the build host and tests the pinned guest server; it is not an iPhone WKWebView or actual LSP test. No workbench IPA if the new gate fails.

## Claude upstream ACP adapter (candidate)
For the reviewed local ACP lane, obtain a permitted upstream Linux ARM64 glibc standalone release from https://github.com/agentclientprotocol/claude-agent-acp/releases . Keep the adapter and its matching Claude CLI together as documented upstream; review licenses, release version, archive and hashes. Install in the saved local-tools Ubuntu environment using the reviewed PTY, authenticate through upstream's supported login, and exit cleanly. Restart Nexus, select Local ACP adapter, enter the absolute adapter argv JSON and reviewed executable SHA-256, then confirm a first message. If capability-gated session handle appears, copy it and finish cleanly; on next boot enter it explicitly for resume. This is a candidate transport, not tested with the upstream binary; no automatic installation, credential handoff or guarantee of compatibility. The adapter may access guest files and network. Native file/terminal grants disabled. No proprietary binaries redistributed.

The new Docker prerequisites mode is diagnostic only; a positive capability or an installed CLI does not establish Docker Engine, Compose or a full-system Linux VM. No remote daemon is used. For the batch file and live LSP modes, see docs/production-acceptance.md; both are explicitly reviewed and require independent device testing.

## Owner-attested device acceptance (0.8.0 candidate)
After downloading the license-reviewed integrated candidate bundle, retain build.json, acceptance.json, SHA256SUMS and the exact unsigned IPA. In a trusted local Python 3.11+ environment, from this source revision:

```sh
python3 scripts/verify-device-acceptance.py --package ./release --record ./device-record.json --initialize
```

The generated record starts all checks false. Sign/install legitimately and run each named test on the actual iPhone, including real provider auth/inference, resume, hooks, batch conflict and LSP. Record the public device model, iOS version, installation method and actual component versions, never account details, keys, OAuth codes, UDID or private transcripts. For each passed check keep a redacted text observation in a private evidence folder named exactly <check>.txt and enter its SHA-256 into the record. Only acknowledge the manual review after observing the real outcomes. Then run:

```sh
python3 scripts/verify-device-acceptance.py --package ./release --record ./device-record.json --evidence ./private-evidence --output ./device-acceptance.json
```

Missing or changed evidence, wrong IPA identity, or pending checks prevents complete owner review. Successful validation checks consistency/completeness only; no independent attestation or production release is granted. Keep evidence local; never commit real credentials or account information. Full-system VM and Docker remain unsupported.

For optional Google AGY history retention, enable Keep separate Google AGY guest before connecting. This stores AGY-owned files and potentially credentials in a distinct private digest-scoped guest, excluded from backup by the existing storage policy. Complete official sign-in and close explicitly before restarting. A session handle appears only if ACP advertises loadSession; use the same digest and fresh official login next launch. No automatic sign-in, token extraction or proven cached-login support. Reset saved Google AGY guest deletes that digest's local guest, not Google authorization. Actual upstream login and resume must still be verified on iPhone.

The integrated 0.8.0 candidate now includes device-record-template.json with every real-device check pending. Copy it to a private device-record.json before editing; it is bound to the exact source commit, IPA hash and CI manifest. The release job independently rechecks the downloaded archive structure, manifest scope, expected variant/revision and untouched template before publishing. The existing main-only license acknowledgment and prerelease restrictions remain. No private device evidence is uploaded by CI.

Pull requests now run both integrated variants, with separate CI artifact names ending in -ubuntu or -workbench. The owner-triggered main workflow remains single variant and requires explicit license review for IPA distribution. Actual pylsp snapshot gate is CI-only; install reviewed language servers yourself in the local-tools guest for device testing. No pylsp package is added to the IPA.

The real pylsp CI gate uses a separately prepared ARM64 test rootfs, not shipped runtime assets. Docker on the Linux CI preparation runner is not part of the app and does not demonstrate Docker on iPhone.
