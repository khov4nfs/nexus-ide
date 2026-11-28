# Physical iPhone final candidate acceptance

Do not start device testing until PR regression gates pass, the final source revision is merged, distribution/source obligations are reviewed by the owner, and an exact signed test candidate is available. CI cannot establish physical device behavior. No production status follows from this checklist alone.

## 1. Identity and setup

- Use the main-only integrated workflow, variant workbench, full_vm=true, only after the owner completes the distribution review documented in full-vm-distribution.md.
- Download the exact reviewed test bundle, check SHA256SUMS and WORKBENCH-VSIX-SHA256. Preserve build.json, acceptance.json, nexus-real-pylsp.vsix and device-record-template.json. Install the IPA using a legitimate signing method such as SideStore. Record the public device model, iOS version, app build, exact unsigned IPA hash, installation method, date. Do not record UDID, signing identity, accounts, tokens or credentials.
- Use an independent reviewed full VM import image built by prepare-full-vm-image.sh at the same revision. Verify SHA256SUMS; import kernel, initrd and disk individually. Review the guest packages/source before distribution; CI does not upload the full disk.

## 2. Workbench / Python LSP

1. On a fresh reviewed Workbench guest, import nexus-real-pylsp.vsix using the VSIX SHA-256 from acceptance.json. Start Workbench, trust only your reviewed guest workspace.
2. Open Command Palette and invoke Nexus: Open CI Fixture. In the created Python file replace all contents with:

       missing_nexus_symbol
       class NexusCompletion:
           nexus_value = 42
       item = NexusCompletion()
       item

3. Save using the real File: Save command. Wait for the actual pylsp diagnostic for missing_nexus_symbol. Verify it is visible in Monaco/Problems. Append a dot to item, invoke completion, choose nexus_value and save. Verify final line item.nexus_value. Shut down cleanly and relaunch saved guest to verify file bytes.
4. Separately create a normal Python file under /root/nexus-workbench; save a deliberate undefined identifier and request attribute completion on a different class. Confirm the real server works outside the acceptance fixture. The reviewed extension now supports bounded Python, C and C++ files, debounced live/on-save diagnostics, plain identifier completion, and persistent per-server sessions during a Workbench boot.

5. In the same Workbench boot, invoke Nexus: Open C++ CI Fixture and enter the C++ source from scripts/workbench-browser-probe.py (exercise_c_family). Save, check the missing_nexus_cpp diagnostic, append a dot to item, accept nexus_value and save. Repeat with Nexus: Open C CI Fixture and missing_nexus_c / nexus_c_value. Keep both documents open. Also verify live diagnostics change after a normal file edit.
6. Invoke Nexus: Stop Language Servers. Review nexus-persistent-lsp.json in the guest. Require one launch and initialize for Python and clangd respectively, Python snapshot refresh, at least two clangd changes, peakDocuments at least two, matching opens/closes, clean shutdown and process exit. Restart Language Servers only after clean/confirmed process exit. Verify new normal-document operations and saved file contents after app relaunch. Record both server versions and redacted observations for workbench_persistent_python_c_cpp_sessions. Process memory is not expected to survive app termination.

## 3. Full VM device runs

Use 1024 MiB only if the device can sustain it. Start with Offline and Run prepared image acceptance on boot. Keep the app foreground. Check actual Linux kernel serial output and the Docker/Compose markers, QMP and clean disk report. Save the bounded Share observed VM evidence report before relaunch.

Next, relaunch the app and boot the same saved disk in Restricted loopback mode. Confirm PERSIST_PREVIOUS matches the earlier PERSIST_SAVED value. Wait for NEXUS_VM_FORWARD_READY, tap Verify local container HTTP, then verify NEXUS_VM_FORWARD_OK and clean shutdown. The negative registry probe is evidence for the tested endpoint only, not proof of complete network isolation. Save the report.

Finally, relaunch and select Internet NAT explicitly. Use a trusted image and permitted network. The prepared smoke image checks DNS/TLS to Docker Registry, pulls a public ARM64 BusyBox image, records its resolved digest (does not execute that pulled tag), and runs existing local Docker/Compose tests. Verify NEXUS_VM_INTERNET_TLS_OK, NEXUS_VM_REGISTRY_PULL_OK, NEXUS_VM_REGISTRY_DIGEST, successful Compose, QMP shutdown and increasing persisted counter. Save the report. Registry outages, public rate limits, DNS errors, low memory and thermal interruption are failures or blockers, not PASS. IPv6, unrestricted ingress/port exposure, arbitrary VPN and general network performance are not qualified.

Test foreground loss, app relaunch and memory/thermal resource handling separately. Do not intentionally overheat or exhaust the phone. If a crash or forced stop leaves the disk dirty, retain redacted evidence and follow the explicit reset/reimport flow; never clear the marker manually to claim persistence.

## 4. Remaining existing device gates

Perform every check in device-record-template.json: native save/relaunch, embedded Ubuntu PTY and persistence, local MCP, reviewed real AGY Google/Gemini auth/inference, native Codex/Claude auth and resume, hooks, permission review, actual ACP, multi-file transfer, native LSP refresh, resource handling, and source/notice review. No credentials or provider transcripts in evidence.

## 5. Record, do not fabricate

Copy device-record-template.json to a new record. Enter device and tested component versions. Set a check's passed field only after the real observation. Store short redacted text observations in an evidence directory, one named <check>.txt per passed check; evidenceSha256 must be the exact SHA-256 of its bytes. For VM runs include the app's bounded evidence and exact imported image hashes, observations and counter progression. For Workbench include the exact tested VSIX digest and editor/diagnostic/completion/relaunch observations. Only the operator may acknowledge manual review.

On a trusted machine (Python 3.11+):

    python3 scripts/verify-device-acceptance.py --package ./candidate --record ./device-record.json --evidence ./device-evidence --output ./device-review-result.json

The verifier rejects altered IPA/VSIX identity, incomplete checks, mismatched evidence hashes or missing manual acknowledgement. It returns operatorReviewComplete only when every required check has evidence. It never claims independent physical-device verification or productionReady. Send redacted failures to the developer; do not upload secrets, actual workspace files, raw serial logs or device identifiers.

No final workflow was dispatched and no real iPhone run has been performed by this implementation.

## Extended project LSP device checks

Follow [reviewed-project-lsp.md](reviewed-project-lsp.md). In one native live LSP session, review and send saved A, B, and edited A; confirm independent diagnostic content, stale-generation rejection, no cross-file semantic claim and clean shutdown. For Workbench, run the real synthetic cross-file acceptance command and inspect both saved files; also run Review Symbol Rename and Review Refactoring on a separate trusted test project, review all files and verify no edits are applied when cancelled or stale. Test optional reviewed toolchains separately when actually staged; record exact versions and do not mark unavailable server/refactor capabilities as passed. No device evidence has been collected by this change.
