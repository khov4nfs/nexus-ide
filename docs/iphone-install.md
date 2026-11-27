# iPhone delivery without a personal Mac

## Download and install

GitHub-hosted macOS runners do the compilation. The iPhone runs the actual IDE locally; no remote runtime is added.

1. Open https://github.com/khov4nfs/nexus-ide/actions in Safari, log into GitHub, select a successful run of **Core and unsigned iOS build**, and download artifact **NexusIDE-native-unsigned**. Extract the downloaded ZIP in Files; inside is the real ARM64 .ipa, SHA256SUMS and build.json. Native-only: Files/editor/limited console, no embedded Linux.
2. For the integrated Ubuntu version, first review upstream/runtime/rootfs redistribution terms, corresponding source and license notices. On the merged main branch, Actions → **Experimental iOS guest build** → Run workflow; choose ubuntu or workbench, acknowledge license_reviewed only after completing the review, optionally select publish_prerelease. Download **NexusIDE-integrated-unsigned** from the successful run. The workbench option also imports the pinned public ARM64 archive; a failure blocks the artifact. Neither includes proprietary AGY.
3. If you already have a legitimately configured on-device sideloader, e.g. SideStore (https://docs.sidestore.io/), import the IPA from Files and let it sign with your own account. Initial setup/pairing depends on your iOS version and may need Windows/Linux/macOS; no personal Mac is required for this project. Do not give your Apple credentials to this repository or untrusted signing services.
4. If you want direct installation without a sideloader, use a valid Apple development/ad hoc profile for your registered device or TestFlight. The workflow does not obtain Apple certificates, profile, account access or device UDID and does not claim to sign. Never publish a profile containing private device IDs or signing secrets in a public artifact. See https://developer.apple.com/documentation/xcode/distributing-your-app-to-registered-devices and https://developer.apple.com/testflight/.

For direct GitHub Release asset downloads, on main manually run **Core and unsigned iOS build** with publish_prerelease=true, or the integrated workflow with both license_reviewed and publish_prerelease. Releases are explicitly labeled prerelease; unique run/attempt tags target the exact commit. Download the IPA asset, not source ZIP. A GitHub release asset is not an Apple OTA installation manifest or a signed app.

## What the packaging gate proves

The ZIP contains exactly one Payload/*.app. It checks the expected bundle ID, supported platform iPhoneOS, minimum iOS 17.0, executable permission, thin ARM64 Mach-O and LC_BUILD_VERSION iOS (simulator rejected), CRC, fakefs assets for integrated builds and absence of proprietary AGY and provisioning profile. It records exact source commit, variant, build/version, SHA-256 and size in build.json. Signing is deliberately not performed; an unsigned package is NOT an installable signed application. No fake simulator IPA, automatic signing, stolen certificate or bypass of platform controls.

The integrated workflow keeps all existing guest/ACP/PTY/MCP fixture gates before Release packaging. PR runs verify packaging but do not distribute the linked guest binary. Public release/upload is gated by the owner's explicit license review on main; this checkbox does not itself provide legal compliance. Supply corresponding source, license texts and third-party notices (including guest packages and OpenVSCode if selected) as required before distribution. No proprietary AGY download/upload; see docs/agy-compatibility.md for local reviewed import. A standalone public IPA with actual AGY remains blocked on a permitted import/distribution path.

## Device acceptance checklist

- Install and launch the signed test IPA on iPhone with iOS 17+; record model/iOS/build.json commit and installation method (no device identifiers or credentials).
- Create, save, reopen native file; confirm workspace persists across app restart.
- Integrated variant: run offline libc gate, restart app, PTY command/input/Ctrl-C/stop; inspect actual evidence and clean shutdown. Separate boot for MCP and workbench.
- If workbench staged: verify loopback page, editor save, LSP and extension behavior; do not infer from a successful build.
- Original AGY: only after reviewed permitted binary import and restricted credential; actual authentication, streaming, cancellation and reviewed edit still need device proof.
- Verify app relaunch after suspension and memory/thermal conditions. Do not claim full VM/Docker, MCP tool execution, unrestricted native tools, other original harnesses or automatic hooks; these remain incomplete.

Report failures on GitHub with redacted logs. Never share API keys, pairing files, signing identities, or raw project data. No production readiness claim until these gates pass.
