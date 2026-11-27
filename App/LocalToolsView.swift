#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import UIKit
import Foundation
import WebKit
import Security
import CryptoKit
import NexusCore
import UniformTypeIdentifiers

struct LocalToolsView: View {
    @ObservedObject var model: IDEModel
    @State private var mode: UInt8 = 1
    @State private var keepGuest = false
    @State private var keepWorkbench = false
    @State private var resetWorkbench = false
    @State private var frozenKeep = false
    @State private var enableHooks = false
    @State private var beforeHook = ""
    @State private var afterHook = ""
    @State private var frozenHooks = false
    @State private var confirmReset = false
    @State private var command = "exec /bin/sh -i"
    @State private var toolName = ""
    @State private var toolArguments = "{}"
    @State private var line = ""
    @State private var running = false
    @State private var ready = false
    @State private var output = ""
    @State private var report = "Select a mode and start a local job."
    @State private var confirm = false
    @State private var frozenMode: UInt8 = 1
    @State private var frozenInput = ""
    @State private var token = ""
    @State private var workbenchShown = false
    @State private var terminalShown = false
    @State private var webStatus = ""
    @State private var manifestPicker = false
    @State private var manifestName = ""
    @State private var vsixPicker = false
    @State private var vsixDigest = ""
    @State private var selectedVSIX: LocalVSIX?
    @State private var frozenVSIX: LocalVSIX?
    @State private var importingVSIX = false
    @State private var cliKind = "codex"
    @State private var cliDigest = ""
    @State private var cliPicker = false
    @State private var cliImport: LocalCLIImport?
    @State private var frozenCLI: LocalCLIImport?
    @State private var importingCLI = false
    @State private var adapterArgv = #"["/usr/local/bin/claude-agent-acp"]"#
    @State private var adapterDigest = ""
    @State private var adapterPrompt = ""
    @State private var adapterResume = ""
    @State private var transferName = ""
    @State private var transferAction = "push"
    @State private var transferSnapshot: WorkspaceSnapshot?
    @State private var transferLoading = false
    @State private var transferReplacement: String?
    @State private var transferReview = false
    @State private var lspArgv = ""
    @State private var lspLanguage = "python"
    @State private var lspCompletion = false
    @State private var lspLine = "0"
    @State private var lspCharacter = "0"
    @State private var lspItems: [LSPTextEdit] = []
    @State private var reviewTitle = "Review Linux import"
    @State private var lspSnapshot: WorkspaceSnapshot?
    @State private var lspLoading = false
    @State private var cliAction = "shell"
    @State private var cliPrompt = ""
    @State private var cliResume = ""
    @State private var enableAgentHooks = false
    @State private var agentBefore = ""
    @State private var agentAfter = ""
    @State private var reviewedTurn = UInt32(0)
    @State private var nativeChat = false
    @State private var agentWaiting = false
    @State private var agentTurns: [NativeAgentTurn] = []
    @State private var agentHandle = ""
    @State private var followup = ""
    @State private var reviewedFollowup = ""
    @State private var confirmFollowup = false
    @State private var keepCLIGuest = false
    @State private var confirmResetCLI = false
    private var stagedWorkbench: Bool {
        guard let root = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil) else { return false }
        return (try? String(contentsOf: root.appendingPathComponent("workbench.sha256"), encoding: .utf8))?.trimmingCharacters(in: .whitespacesAndNewlines) == "36d9c14036489b63de84ebace837fcacf7e60e669a0dc715802c5443684ea4dc"
    }
    var body: some View {
        NavigationStack {
            List {
                Section("Local Ubuntu · one boot per app launch") {
                    Picker("Mode", selection: $mode) {
                        Text("PTY terminal").tag(UInt8(1))
                        Text("MCP discovery").tag(UInt8(2))
                        Text("MCP manual call").tag(UInt8(4))
                        Text("OpenVSCode").tag(UInt8(3))
                        Text("Original agent CLI").tag(UInt8(6))
                        Text("Native LSP").tag(UInt8(8))
                        Text("Native/Linux file transfer").tag(UInt8(9))
                        Text("Local ACP adapter (Claude etc.)").tag(UInt8(10))
                    }.disabled(model.guestClaimed)
                    if mode == 10 {
                        TextField("Reviewed absolute adapter argv (JSON)", text: $adapterArgv, axis: .vertical).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                        TextField("Adapter executable SHA-256", text: $adapterDigest).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                        TextField("First message", text: $adapterPrompt, axis: .vertical).disabled(model.guestClaimed)
                        TextField("Saved session handle (optional)", text: $adapterResume).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                        Text("Use an upstream ACP v1 adapter installed and authenticated in the saved local-tools guest. Review its executable and dependencies. Resume only when the adapter advertises loadSession; old replay is not displayed. No native grants, no automatic install/login. Permission requests denied. Agent may access guest data and network.").font(.caption)
                    } else if mode == 9 {
                        Picker("Direction", selection: $transferAction) { Text("Export native file").tag("push"); Text("Import Linux file").tag("pull") }.disabled(model.guestClaimed)
                        TextField("Guest filename", text: $transferName).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                        Button("Select saved native editor file") {
                            transferLoading = true
                            Task {
                                defer { transferLoading = false }
                                do { let snap = try await model.selectedSnapshot(); guard snap.text.utf8.count <= 8192 else { throw WorkspaceError.tooLarge }; transferSnapshot = snap; transferName = (snap.path as NSString).lastPathComponent; report = "Selected saved file" }
                                catch { transferSnapshot = nil; report = "Select and save a UTF-8 editor file (8 KiB max)" }
                            }
                        }.disabled(model.guestClaimed || transferLoading || model.path.isEmpty)
                        if let snap = transferSnapshot { Text("Selected: " + snap.path).font(.caption) }
                        Text("Uses saved local-tools guest, /root/nexus-transfer. Export creates only; never overwrites existing guest files. Import stages a replacement for explicit native review. Restart app between transfer and PTY work.").font(.caption)
                    } else if mode == 8 {
                        TextField("Reviewed absolute guest argv (JSON array)", text: $lspArgv, axis: .vertical).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                        TextField("LSP language ID", text: $lspLanguage).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                        Button("Attach saved editor snapshot") {
                            lspLoading = true
                            Task {
                                defer { lspLoading = false }
                                do { let snapshot = try await model.selectedSnapshot(); guard snapshot.text.utf8.count <= 8192 else { throw WorkspaceError.tooLarge }; lspSnapshot = snapshot; report = "Snapshot selected; not yet sent" }
                                catch { lspSnapshot = nil; report = "Save and select an editor file, max 8192 bytes" }
                            }
                        }.disabled(model.guestClaimed || lspLoading || model.path.isEmpty)
                        if let snapshot = lspSnapshot { Text(snapshot.path + " · " + String(snapshot.text.utf8.count) + " bytes").font(.caption); Button("Remove snapshot") { lspSnapshot = nil }.disabled(model.guestClaimed) }
                        Toggle("Request completion instead of diagnostics", isOn: $lspCompletion).disabled(model.guestClaimed)
                        if lspCompletion {
                            TextField("Zero-based line", text: $lspLine).keyboardType(.numberPad).disabled(model.guestClaimed)
                            TextField("Zero-based UTF-16 character", text: $lspCharacter).keyboardType(.numberPad).disabled(model.guestClaimed)
                        }
                        Text("Snapshot diagnostics or one completion request. Only plain single-range edits may be reviewed; no snippets, commands, automatic edits or native path. Server may use network or retain data.").font(.caption)
                        Toggle("Use saved local-tools guest", isOn: $keepGuest).disabled(model.guestClaimed)
                    } else if mode == 6 {
                        Picker("Official executable", selection: $cliKind) {
                            Text("Codex CLI").tag("codex")
                            Text("Claude Code").tag("claude")
                        }.disabled(model.guestClaimed || importingCLI || cliImport != nil)
                        .onChange(of: cliKind) { _, _ in cliAction = "shell" }
                        TextField("Reviewed Linux ARM64 ELF SHA-256", text: $cliDigest).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed || importingCLI || cliImport != nil)
                        Button("Choose official executable from Files") { cliPicker = true }.disabled(model.guestClaimed || importingCLI || !AGYImportFormat.validDigest(cliDigest) || cliImport != nil)
                        if importingCLI { ProgressView("Verifying local executable") }
                        if let cliImport {
                            Text(cliImport.name + " · " + cliImport.digest).font(.caption).textSelection(.enabled)
                            Button("Remove selected executable") { cliImport.remove(); self.cliImport = nil }.disabled(model.guestClaimed)
                        }
                        Picker("Launch mode", selection: $cliAction) {
                            Text("Interactive local shell").tag("shell")
                            Text("Version probe").tag("version")
                            if cliKind == "codex" {
                                Text("Native app-server account probe").tag("appServer")
                                Text("Native Codex text turn").tag("appServerChat")
                                Text("Native Codex conversation").tag("codexSession")
                            } else {
                                Text("Native Claude account probe").tag("claudeAuth")
                                Text("Native Claude text turn").tag("claudeChat")
                                Text("Native Claude conversation").tag("claudeSession")
                            }
                        }.disabled(model.guestClaimed)
                        if ["appServerChat","claudeChat","codexSession","claudeSession"].contains(cliAction) {
                            TextField("Message to agent (max 2048 bytes)", text: $cliPrompt, axis: .vertical).disabled(model.guestClaimed)
                            Text("Requires CLI-owned sign-in. Model request may incur charges. Codex requests read-only policy; Claude disables customizations and tools. No native grants. Output is untrusted and may contain guest secrets.").font(.caption)
                        }
                        Toggle("Keep separate agent guest and CLI login", isOn: $keepCLIGuest).disabled(model.guestClaimed || importingCLI)
                        if ["codexSession","claudeSession"].contains(cliAction) {
                            Text("Up to 10 reviewed turns. Claude stores history inside the guest during conversation. Saved guests retain history after shutdown.").font(.caption)
                            if keepCLIGuest { TextField("Saved session ID (optional)", text: $cliResume).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed) }
                            Toggle("Review session lifecycle hooks", isOn: $enableAgentHooks).disabled(model.guestClaimed)
                            if enableAgentHooks {
                                TextField("Before session (optional shell command)", text: $agentBefore, axis: .vertical).disabled(model.guestClaimed)
                                TextField("After successful finish (optional)", text: $agentAfter, axis: .vertical).disabled(model.guestClaimed)
                                Text("Runs once per session, max 30 seconds per hook, same guest. No auto-loaded hooks. Commands may access guest credentials and network. After hook is skipped on failure or stop.").font(.caption)
                            }
                        }
                        Text("Optional: persists CLI-owned login state and project files for this exact binary digest. May retain secrets. Exit the shell cleanly before restarting. No backup, no sharing with other guests, no native token collection.").font(.caption)
                        Button("Reset saved CLI guest", role: .destructive) { confirmResetCLI = true }.disabled(model.guestClaimed || importingCLI || !AGYImportFormat.validDigest(cliDigest))
                        Text("Account probes read status only. Use the original CLI login inside the terminal. Linux sandbox/syscall compatibility remains unverified; do not disable agent safeguards.").font(.caption)
                    } else if mode != 3 {
                        TextField("Reviewed guest shell command", text: $command, axis: .vertical)
                            .font(.system(.body, design: .monospaced)).autocorrectionDisabled()
                            .textInputAutocapitalization(.never).disabled(model.guestClaimed)
                        Button("Import manual tool profile") { manifestPicker = true }.disabled(model.guestClaimed)
                        if !manifestName.isEmpty { Text(manifestName).font(.caption) }
                        if mode == 4 {
                            TextField("Exact advertised tool name", text: $toolName).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                            Text("JSON arguments (object only)").font(.caption)
                            TerminalInput(text: $toolArguments).frame(minHeight: 84, maxHeight: 140).disabled(model.guestClaimed)
                            Text("One explicitly reviewed call. Server may perform side effects; do not enter secrets. No automatic agent grants.").font(.caption)
                        }
                    } else {
                        Text(stagedWorkbench ? "Pinned workbench archive staged; runtime unverified." : "No workbench staged. See build instructions.")
                        Text("Separate guest workspace, not native Files. Token-protected loopback only.")
                        TextField("Reviewed VSIX SHA-256", text: $vsixDigest).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed || importingVSIX || selectedVSIX != nil)
                        Button("Choose reviewed VSIX from Files") { vsixPicker = true }.disabled(model.guestClaimed || importingVSIX || !stagedWorkbench || !AGYImportFormat.validDigest(vsixDigest))
                        if importingVSIX { ProgressView("Verifying VSIX") }
                        if let selectedVSIX {
                            Text("Verified SHA-256: " + selectedVSIX.digest).font(.caption).textSelection(.enabled)
                            Button("Remove selected VSIX") { self.selectedVSIX = nil }.disabled(model.guestClaimed)
                        }
                        Text("Optional offline archive, maximum 32 MiB. Extension code can run locally and use network or guest data; review provenance and digest first.").font(.caption)
                        Toggle("Keep workbench files and extensions", isOn: $keepWorkbench).disabled(model.guestClaimed)
                        Text("Uses a separate saved workbench guest. A new token is generated on every launch. Unclean shutdown blocks reuse.").font(.caption)
                        Button("Reset saved workbench", role: .destructive) { resetWorkbench = true; confirmReset = true }.disabled(model.guestClaimed).font(.caption)
                    }
                    if mode == 1 {
                        Toggle("Run reviewed lifecycle hooks", isOn: $enableHooks).disabled(model.guestClaimed)
                        if enableHooks {
                            TextField("Before command (optional)", text: $beforeHook, axis: .vertical).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                            TextField("After successful command (optional)", text: $afterHook, axis: .vertical).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                            Text("Hooks execute automatically only within this confirmed PTY job. Each runs in its own guest process. No provider key; no hook on app launch or file save.").font(.caption)
                        }
                    }
                    if mode != 3 && mode != 6 {
                        Toggle("Keep Linux files across launches", isOn: $keepGuest).disabled(model.guestClaimed)
                        Text("Local tools only; never shared with Agent. Unclean shutdown blocks reuse. Reset deletes saved Linux files, not native Files.").font(.caption)
                        Button("Reset saved Linux environment", role: .destructive) { resetWorkbench = false; confirmReset = true }.disabled(model.guestClaimed)
                    }
                    Button("Review and start local job") {
                        frozenKeep = mode == 9 || mode == 10 ? true : mode == 6 ? keepCLIGuest : mode == 3 ? keepWorkbench : keepGuest
                        frozenMode = mode
                        frozenVSIX = mode == 3 ? selectedVSIX : nil
                        frozenCLI = mode == 6 ? cliImport : nil
                        frozenHooks = mode == 1 && enableHooks
                        if mode == 10 {
                            guard let argv = try? ACPTerminalApproval.parse(Data(adapterArgv.utf8)), AGYImportFormat.validDigest(adapterDigest), !adapterPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, adapterPrompt.utf8.count <= 2048, !adapterPrompt.utf8.contains(0), adapterResume.utf8.count <= 128, adapterResume.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [45,95].contains($0) }) else { report = "Invalid reviewed adapter request"; return }
                            var request: [String: Any] = ["argv":argv.arguments,"sha256":adapterDigest,"prompt":adapterPrompt]
                            if !adapterResume.isEmpty { request["resume"] = adapterResume }
                            guard let wire = try? JSONSerialization.data(withJSONObject: request, options: [.sortedKeys]), wire.count <= 8192 else { report = "Invalid reviewed adapter request"; return }
                            frozenInput = String(decoding: wire, as: UTF8.self)
                        } else if mode == 9 {
                            guard let snap = transferSnapshot, !transferName.isEmpty, transferName.utf8.count <= 128, !transferName.hasPrefix("."), transferName.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [45,46,95].contains($0) }) else { report = "Invalid guest filename or snapshot"; return }
                            var request = ["action":transferAction,"name":transferName]
                            if transferAction == "push" { request["text"] = snap.text }
                            guard let wire = try? JSONSerialization.data(withJSONObject: request, options: [.sortedKeys]), wire.count <= 65536 else { report = "Invalid transfer"; return }
                            frozenInput = String(decoding: wire, as: UTF8.self)
                        } else if mode == 8 {
                            guard let snapshot = lspSnapshot, let approved = try? ACPTerminalApproval.parse(Data(lspArgv.utf8)) else { report = "Select a saved snapshot and valid argv"; return }
                            let ext = (snapshot.path as NSString).pathExtension.lowercased()
                            var request: [String: Any] = ["argv":approved.arguments,"language":lspLanguage,"extension":ext,"text":snapshot.text]
                            if lspCompletion {
                                guard let line = Int(lspLine), let character = Int(lspCharacter), LSPTextEdit.offset(.init(line: line, character: character), in: snapshot.text) != nil else { report = "Invalid UTF-16 cursor"; return }
                                request["completion"] = ["line":line,"character":character]
                            }
                            guard let wire = try? JSONSerialization.data(withJSONObject: request, options: [.sortedKeys]), wire.count <= 65536 else { report = "Invalid LSP snapshot"; return }
                            frozenInput = String(decoding: wire, as: UTF8.self)
                        } else if mode == 6 {
                            guard let frozenCLI else { report = "Select reviewed executable"; return }
                            var request: [String: Any] = ["kind":frozenCLI.kind,"sha256":frozenCLI.digest,"action":cliAction]
                            if ["appServerChat","claudeChat","codexSession","claudeSession"].contains(cliAction) {
                                guard !cliPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, cliPrompt.utf8.count <= 2048, !cliPrompt.utf8.contains(0) else { report = "Invalid message"; return }
                                request["prompt"] = cliPrompt
                            }
                            if ["codexSession","claudeSession"].contains(cliAction) {
                                let handle = frozenKeep ? cliResume : ""
                                guard handle.utf8.count <= 128, handle.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [45,95].contains($0) }), cliKind != "claude" || handle.isEmpty || UUID(uuidString: handle) != nil else { report = "Invalid session ID"; return }
                                request["retain"] = frozenKeep; request["resume"] = handle
                                if enableAgentHooks {
                                    guard (try? LocalHookPlan(command: "session", before: agentBefore, after: agentAfter)) != nil else { report = "Invalid lifecycle hooks"; return }
                                    request["before"] = agentBefore; request["after"] = agentAfter
                                }
                            }
                            guard let wire = try? JSONSerialization.data(withJSONObject: request, options: [.sortedKeys]), wire.count <= 16384 else { report = "Invalid request"; return }
                            frozenInput = String(decoding: wire, as: UTF8.self)
                        } else if frozenHooks {
                            do {
                                frozenInput = try LocalHookPlan(command: command, before: beforeHook, after: afterHook).wire()
                            } catch { report = "Invalid or empty lifecycle hook plan"; return }
                        } else if mode == 4 {
                            guard let args = try? JSONSerialization.jsonObject(with: Data(toolArguments.utf8)) as? [String: Any],
                                  let wire = try? JSONSerialization.data(withJSONObject: ["command":command,"name":toolName,"arguments":args],options:[.sortedKeys]),
                                  wire.count <= 8192, toolArguments.utf8.count <= 4096,
                                  !toolName.isEmpty, toolName.utf8.count <= 128,
                                  toolName.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [45,46,95].contains($0) }) else {
                                report = "Invalid tool name or JSON object"; return
                            }
                            frozenInput = String(decoding: wire,as: UTF8.self)
                        } else { frozenInput = command }
                        confirm = true
                    }.disabled(model.guestClaimed || importingVSIX || importingCLI || (mode == 10 ? adapterPrompt.isEmpty || !AGYImportFormat.validDigest(adapterDigest) : mode == 9 ? transferLoading || transferSnapshot == nil : mode == 8 ? lspLoading || lspSnapshot == nil || lspArgv.isEmpty : mode == 6 ? cliImport == nil : mode == 3 ? !stagedWorkbench : command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || command.utf8.count > 2048 || command.utf8.contains(0)))
                    Text("No provider key or native workspace injected. Commands and extensions may use the network and are not sandboxed. Restart app for another boot.").font(.caption)
                }
                if running && frozenMode != 2 && frozenMode != 4 {
                    Section("Process controls") {
                        if nativeChat {
                            Text(agentWaiting ? "Ready for a reviewed follow-up" : "Agent processing or closing").font(.caption)
                            TextField("Next message", text: $followup, axis: .vertical).disabled(!agentWaiting)
                            Button("Review next message") { reviewedFollowup = followup; reviewedTurn = UInt32(agentTurns.count); confirmFollowup = true }.disabled(!agentWaiting || followup.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || followup.utf8.count > 2048 || followup.utf8.contains(0))
                            Button("Finish conversation") { if LocalRunner.agentFinish(UInt32(agentTurns.count)) { agentWaiting = false } }.disabled(!agentWaiting)
                        } else if frozenMode == 1 || frozenMode == 6 {
                            Text("Paste commands on separate lines. Smart quotes and dashes are disabled.").font(.caption)
                            TerminalInput(text: $line)
                                .frame(minHeight: 84, maxHeight: 140)
                            Button("Send command(s)") {
                                let normalized = line.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
                                let wire = normalized.hasSuffix("\n") ? normalized : normalized + "\n"
                                if LocalRunner.input(wire) { line = "" }
                                else { report = "Input not accepted" }
                            }.disabled(!ready || line.isEmpty || line.utf8.count > 1023 || line.utf8.contains(0))
                            Button("Open interactive ANSI terminal") { terminalShown = true }.disabled(!ready)
                            Button("Send Ctrl-C") { _ = LocalRunner.input("\u{3}") }.disabled(!ready)
                            Button("Resize to 111 × 33") { if !LocalRunner.resize() { report = "Resize not accepted" } }.disabled(!ready)
                        } else if frozenMode == 3 {
                            Button("Open local workbench") { webStatus = "Opening verified bind endpoint…"; workbenchShown = true }.disabled(!ready || token.isEmpty)
                            Text("Server launch requested; web view may fail until ready.").font(.caption)
                        }
                        Button("Stop process", role: .destructive) {
                            if LocalRunner.stop() { ready = false; report = "Stop requested" }
                            else { report = "Stop not accepted yet" }
                        }
                    }
                }
                if nativeChat {
                    Section("Native conversation · untrusted text") {
                        ForEach(agentTurns.indices, id: \.self) { i in
                            VStack(alignment: .leading, spacing: 5) {
                                Text("You").font(.caption); Text(verbatim: agentTurns[i].prompt)
                                Text("Agent").font(.caption); Text(verbatim: agentTurns[i].response).textSelection(.enabled)
                            }
                        }
                        if !agentHandle.isEmpty { Text("Save this session ID for a clean restart").font(.caption); Text(verbatim: agentHandle).textSelection(.enabled) }
                    }
                } else if frozenMode == 1 || frozenMode == 6 {
                    Section("PTY output · plain text") {
                        Text(verbatim: output).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
                    }
                }
                if !lspItems.isEmpty {
                    Section("Completion · review before apply") {
                        ForEach(lspItems.indices, id: \.self) { i in
                            Button {
                                guard let snap = lspSnapshot, let replacement = try? lspItems[i].replacement(in: snap.text) else { report = "Completion cannot be applied"; return }
                                transferSnapshot = snap; transferReplacement = replacement; reviewTitle = "Review completion"; transferReview = true
                            } label: { VStack(alignment: .leading) { Text(verbatim: lspItems[i].label); Text(verbatim: lspItems[i].text).font(.caption) } }
                            .disabled(running || model.reviewBusy)
                        }
                    }
                }
                Section("Status") {
                    if running { ProgressView("Keep app open") }
                    Text(verbatim: report).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
                }
                Section("Unavailable until verified") {
                    Text("Manual MCP calls and PTY lifecycle hooks are separate from the agent. Extension compatibility, additional agent harnesses and Docker VM remain unverified or disabled.")
                }
            }
            .navigationTitle("Local tools")
            .fileImporter(isPresented: $manifestPicker, allowedContentTypes: [.json]) { result in
                guard !model.guestClaimed, case .success(let url) = result else { return }
                let access = url.startAccessingSecurityScopedResource()
                defer { if access { url.stopAccessingSecurityScopedResource() } }
                do {
                    let profile = try LocalToolProfile.read(url)
                    command = profile.command
                    mode = profile.mode == .pty ? 1 : 2
                    manifestName = profile.name + " · manual only"
                    report = "Profile loaded; no execution until confirmed"
                } catch { report = "Invalid local tool profile" }
            }
            .fileImporter(isPresented: $cliPicker, allowedContentTypes: [.data]) { result in
                guard !model.guestClaimed, !importingCLI, case .success(let url) = result else { return }
                let kind = cliKind, digest = cliDigest
                importingCLI = true
                Task {
                    let reviewed = await Task.detached(priority: .userInitiated) { try? LocalCLIImport.select(url, kind: kind, expected: digest) }.value
                    importingCLI = false
                    cliImport = reviewed
                    report = reviewed == nil ? "Invalid ARM64 ELF or digest mismatch" : "Executable verified; no execution until confirmed"
                }
            }
            .fileImporter(isPresented: $vsixPicker, allowedContentTypes: [.data]) { result in
                guard !model.guestClaimed, !importingVSIX, case .success(let url) = result else { return }
                let digest = vsixDigest
                importingVSIX = true
                Task {
                    let archive = await Task.detached(priority: .userInitiated) { try? LocalVSIX.read(url, expected: digest) }.value
                    importingVSIX = false
                    selectedVSIX = archive
                    report = archive == nil ? "Invalid VSIX or digest mismatch" : "VSIX verified; install only after confirmation"
                }
            }
            .alert(resetWorkbench ? "Delete saved workbench?" : "Delete saved Linux environment?", isPresented: $confirmReset) {
                Button("Delete saved Linux files", role: .destructive) {
                    guard !model.guestClaimed else { return }
                    report = LocalRunner.resetSaved(workbench: resetWorkbench)
                }
                Button("Cancel", role: .cancel) {}
            } message: { Text(resetWorkbench ? "This permanently deletes saved workbench files, settings and extensions. Native Files and local-tools guest remain separate." : "This permanently deletes the saved local-tools guest, including installed packages and guest files. Native editor files and Agent are separate.") }
            .alert("Delete saved CLI guest?", isPresented: $confirmResetCLI) {
                Button("Delete guest files", role: .destructive) {
                    guard !model.guestClaimed else { return }
                    report = LocalRunner.resetCLI(kind: cliKind, digest: cliDigest)
                }
                Button("Cancel", role: .cancel) {}
            } message: { Text("Deletes this harness and digest guest, including local CLI credentials and project files. Other guests and native files are unaffected.") }
            .confirmationDialog("Execute in local Linux?", isPresented: $confirm) {
                Button("Start") { start() }
                Button("Cancel", role: .cancel) { frozenInput = ""; frozenHooks = false; frozenVSIX = nil; frozenCLI = nil }
            } message: {
                Text(frozenMode == 10 ? "Start this reviewed ACP adapter and send the first model message?\n" + frozenInput + "\nModel requests may incur charges; saved guest can retain credentials and history. No native grants." : frozenMode == 9 ? "Transfer this exact reviewed file to/from the saved Linux guest?\n" + frozenInput + "\nImport requires separate native review; no automatic overwrite." : frozenMode == 8 ? "Send this reviewed snapshot to the specified local LSP server?\n" + frozenInput + "\nNo native write grant. Server may use network or retain data." : frozenMode == 6 ? "Start this reviewed original CLI in Ubuntu?\n" + frozenInput + "\nNo provider key is supplied by Nexus. Use the CLI own sign-in. A text turn sends your prompt to the provider and may incur charges. Commands may access guest files and network; no security sandbox." + (frozenKeep ? "\nRetain this guest, including CLI-owned credentials, after clean shutdown." : "\nUse disposable guest.") : frozenHooks ? "Run this exact lifecycle plan locally? Before hook must succeed. After hook runs only after a successful main command; no hook after cancellation.\n" + frozenInput + "\nCommands can modify guest files or use its network. No security sandbox." + (frozenKeep ? "\nUse saved Linux environment." : "\nUse disposable Linux environment.") : frozenMode == 4 ? "Execute one reviewed MCP call (may have side effects):\n" + frozenInput : frozenMode == 3 ? "Launch reviewed OpenVSCode on loopback with a random token. No native workspace mount. No guarantee of compatibility." + (frozenKeep ? "\nUse saved workbench files, settings and extensions." : "\nUse disposable workbench.") + (frozenVSIX.map { "\nInstall this reviewed VSIX before startup: " + $0.digest } ?? "") : "Run this reviewed command locally:\n" + frozenInput + "\nNo provider key is injected. It may use the network, modify guest files or consume resources. No automatic execution." + (frozenKeep ? "\nUse saved Linux environment. Changes persist after clean shutdown." : "\nUse disposable Linux environment."))
            }
            .sheet(isPresented: $transferReview) {
                NavigationStack {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 12) {
                            if let snap = transferSnapshot, let replacement = transferReplacement {
                                Text(snap.path).font(.headline)
                                Text("Original").font(.caption)
                                Text(verbatim: snap.text).textSelection(.enabled)
                                Text("Proposed replacement (untrusted)").font(.caption)
                                Text(verbatim: replacement).textSelection(.enabled)
                                Text("Apply only if the saved file and editor still match the original.").font(.caption)
                            }
                        }.padding().frame(maxWidth: .infinity, alignment: .leading)
                    }.navigationTitle(reviewTitle)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) { Button("Discard") { transferReplacement = nil; transferReview = false }.disabled(model.reviewBusy) }
                        ToolbarItem(placement: .confirmationAction) {
                            Button("Apply") {
                                guard let snap = transferSnapshot, let replacement = transferReplacement else { return }
                                Task {
                                    do { try await model.applyReviewedTransfer(snap, replacement: replacement); report = "Reviewed transfer applied"; transferReplacement = nil; transferReview = false }
                                    catch { report = "Import conflict: native file or editor changed"; transferReplacement = nil; transferReview = false }
                                }
                            }.disabled(model.reviewBusy)
                        }
                    }
                }.interactiveDismissDisabled(model.reviewBusy)
            }
            .confirmationDialog("Send reviewed message to model?", isPresented: $confirmFollowup) {
                Button("Send") { if LocalRunner.agentSend(reviewedFollowup, ticket: reviewedTurn) { followup = ""; agentWaiting = false } else { report = "Message not accepted for this turn" }; reviewedFollowup = "" }
                Button("Cancel", role: .cancel) { reviewedFollowup = "" }
            } message: { Text(reviewedFollowup + "\nMay incur provider charges. No native grants.") }
            .task(id: running) {
                while running && !Task.isCancelled {
                    let p = LocalRunner.progress()
                    ready = p.ready
                    output = p.output
                    if nativeChat { agentWaiting = p.agentWaiting; agentTurns = p.turns; agentHandle = p.handle }
                    try? await Task.sleep(nanoseconds: 100_000_000)
                }
            }
            .fullScreenCover(isPresented: $terminalShown) {
                LiveTerminalScreen()
            }
            .sheet(isPresented: $workbenchShown) {
                NavigationStack {
                    VStack {
                        Text(webStatus).font(.caption)
                        LoopbackWorkbench(token: token, status: $webStatus)
                    }
                        .navigationTitle("Guest workbench")
                        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close view") { workbenchShown = false } } }
                }
            }
        }
    }
    private func start() {
        guard !running, !model.guestClaimed else { return }
        let input: String
        if frozenMode == 3 {
            guard stagedWorkbench, let generated = LocalRunner.token() else { report = "Token or archive unavailable"; return }
            token = generated
            if let frozenVSIX {
                guard let wire = try? JSONSerialization.data(withJSONObject: ["token": generated, "vsixSha256": frozenVSIX.digest]) else { return }
                input = String(decoding: wire, as: UTF8.self)
            } else { input = generated }
        } else { input = frozenInput }
        guard model.claimGuest() else { return }
        let selectedMode: UInt8 = frozenHooks ? 5 : frozenMode
        let persistent = frozenKeep
        let vsix = frozenVSIX
        let cli = frozenCLI
        frozenVSIX = nil
        selectedVSIX = nil
        frozenCLI = nil
        if cli != nil { cliImport = nil }
        let reviewedAction = (try? JSONSerialization.jsonObject(with: Data(input.utf8)) as? [String: Any])?["action"] as? String ?? ""
        nativeChat = selectedMode == 10 || selectedMode == 6 && ["codexSession","claudeSession"].contains(reviewedAction)
        agentWaiting = false; agentTurns = []; agentHandle = ""; followup = ""
        running = true
        report = "Starting local guest"
        Task {
            report = await Task.detached(priority: .userInitiated) {
                LocalRunner.run(mode: selectedMode, input: input, persistent: persistent, vsix: vsix, cli: cli)
            }.value
            if selectedMode == 9, let obj = try? JSONSerialization.jsonObject(with: Data(report.utf8)) as? [String: Any], obj["ok"] as? Bool == true, obj["shutdownClean"] as? Bool == true, obj["action"] as? String == "pull", let text = obj["text"] as? String, text.utf8.count <= 8192, !text.utf8.contains(0) {
                transferReplacement = text; reviewTitle = "Review Linux import"; transferReview = true; report = "Linux file retrieved; native review required"
            }
            if selectedMode == 8, let obj = try? JSONSerialization.jsonObject(with: Data(report.utf8)) as? [String: Any], obj["ok"] as? Bool == true, obj["shutdownClean"] as? Bool == true, let items = obj["completions"], let data = try? JSONSerialization.data(withJSONObject: items), let decoded = try? JSONDecoder().decode([LSPTextEdit].self, from: data), decoded.count <= 32 {
                lspItems = decoded; report = "Received \(decoded.count) plain completion suggestions"
            }
            let progress = LocalRunner.progress()
            output = progress.output
            if nativeChat { agentTurns = progress.turns; agentHandle = progress.handle; agentWaiting = false }
            ready = false
            running = false
            token = ""
            workbenchShown = false
        }
    }
}

/// UITextView preserves pasted newlines and explicitly disables iOS smart punctuation.
private struct TerminalInput: UIViewRepresentable {
    @Binding var text: String
    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }
    func makeUIView(context: Context) -> UITextView {
        let v = UITextView()
        v.font = .monospacedSystemFont(ofSize: 15, weight: .regular)
        v.backgroundColor = .secondarySystemBackground
        v.textColor = .label
        v.smartQuotesType = .no
        v.smartDashesType = .no
        v.smartInsertDeleteType = .no
        v.autocorrectionType = .no
        v.autocapitalizationType = .none
        v.spellCheckingType = .no
        v.accessibilityLabel = "Terminal command input"
        v.delegate = context.coordinator
        v.text = text
        return v
    }
    func updateUIView(_ view: UITextView, context: Context) {
        if view.text != text { view.text = text }
    }
    final class Coordinator: NSObject, UITextViewDelegate {
        @Binding var text: String
        init(text: Binding<String>) { _text = text }
        func textViewDidChange(_ textView: UITextView) { text = textView.text ?? "" }
    }
}

private struct NativeAgentTurn: Decodable { let prompt: String; let response: String }

private enum LocalRunner {
    static func token() -> String? {
        var data = [UInt8](repeating: 0, count: 32)
        let status = data.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }
        return status == errSecSuccess ? data.map { String(format: "%02x", $0) }.joined() : nil
    }
    static func decode(_ pointer: UnsafeMutablePointer<CChar>?) -> [String: Any] {
        guard let pointer else { return [:] }
        defer { nexus_guest_free(pointer) }
        return (try? JSONSerialization.jsonObject(with: Data(String(cString: pointer).utf8)) as? [String: Any]) ?? [:]
    }
    static func progress() -> (ready: Bool, output: String, agentWaiting: Bool, turns: [NativeAgentTurn], handle: String) {
        let object = decode(nexus_local_progress())
        let rows = object["agentTurns"] as? [[String: Any]] ?? []
        let turns = rows.compactMap { row -> NativeAgentTurn? in
            guard let prompt = row["prompt"] as? String, let response = row["response"] as? String else { return nil }
            return NativeAgentTurn(prompt: prompt, response: response)
        }
        return (object["ready"] as? Bool == true, object["output"] as? String ?? "", object["agentWaiting"] as? Bool == true, turns, object["agentHandle"] as? String ?? "")
    }
    static func agentSend(_ text: String, ticket: UInt32) -> Bool { text.withCString { decode(nexus_local_agent_send(ticket,$0))["ok"] as? Bool == true } }
    static func agentFinish(_ ticket: UInt32) -> Bool { decode(nexus_local_agent_finish(ticket))["ok"] as? Bool == true }
    static func input(_ text: String) -> Bool {
        text.withCString { decode(nexus_local_input($0))["ok"] as? Bool == true }
    }
    static func resize() -> Bool { decode(nexus_local_resize(111, 33))["ok"] as? Bool == true }
    static func stop() -> Bool { decode(nexus_local_stop())["ok"] as? Bool == true }
    static func savedStore(workbench: Bool = false) throws -> LocalGuestStore {
        let parent = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        var container = parent.appendingPathComponent(workbench ? "NexusWorkbench" : "NexusLocalTools", isDirectory: true)
        try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try container.setResourceValues(values)
        return LocalGuestStore(directory: container.appendingPathComponent("guest-v1", isDirectory: true))
    }
    static func resetSaved(workbench: Bool = false) -> String {
        do { try savedStore(workbench: workbench).reset(); return workbench ? "Saved workbench deleted" : "Saved Linux environment deleted" }
        catch { return "Saved environment reset failed" }
    }
    private static func cliStore(kind: String, digest: String) throws -> LocalGuestStore {
        guard ["codex","claude"].contains(kind), AGYImportFormat.validDigest(digest) else { throw LocalGuestStore.Failure.invalid }
        let parent = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        var root = parent.appendingPathComponent("NexusOriginalCLIGuests", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700, .protectionKey: FileProtectionType.complete])
        var values = URLResourceValues(); values.isExcludedFromBackup = true; try root.setResourceValues(values)
        return LocalGuestStore(directory: root.appendingPathComponent(kind + "-" + digest, isDirectory: true))
    }
    static func resetCLI(kind: String, digest: String) -> String {
        do { try cliStore(kind: kind, digest: digest).reset(); return "Saved CLI guest deleted" }
        catch { return "Saved CLI guest reset failed" }
    }
    private static func verifyWorkbench(dataPath: URL, bundled: URL) -> Bool {
        // Read expected digests only from the installed app bundle, not from
        // mutable guest metadata. This checks launcher and Node, not all files.
        for (name, relative) in [("launcher","bin/openvscode-server"),("node","node")] {
            do {
                let expected = try String(contentsOf: bundled.appendingPathComponent("workbench-\(name).sha256"), encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
                guard AGYImportFormat.validDigest(expected) else { return false }
                let file = dataPath.appendingPathComponent("opt/nexus/workbench/" + relative)
                let v = try file.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey])
                guard v.isRegularFile == true, v.isSymbolicLink != true, let size = v.fileSize,
                      size > 0, size <= 256 * 1024 * 1024 else { return false }
                let handle = try FileHandle(forReadingFrom: file)
                defer { try? handle.close() }
                var sha = SHA256()
                var bytes = 0
                while let chunk = try handle.read(upToCount: 1024 * 1024), !chunk.isEmpty {
                    bytes += chunk.count
                    guard bytes <= size else { return false }
                    sha.update(data: chunk)
                }
                guard bytes == size, sha.finalize().map({ String(format: "%02x", $0) }).joined() == expected else { return false }
            } catch { return false }
        }
        return true
    }

    static func run(mode: UInt8, input: String, persistent: Bool, vsix: LocalVSIX? = nil, cli: LocalCLIImport? = nil) -> String {
        defer { cli?.remove() }
        guard mode != 6 || cli != nil else { return "Original CLI requires a reviewed import" }
        do {
            guard let root = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil) else { return "Missing Ubuntu assets" }
            let fm = FileManager.default
            let store: LocalGuestStore?
            if persistent, mode == 6, let cli { store = try cliStore(kind: cli.kind, digest: cli.digest) }
            else { store = persistent ? try savedStore(workbench: mode == 3) : nil }
            let trial = fm.temporaryDirectory.appendingPathComponent("NexusLocal-\(UUID().uuidString)", isDirectory: true)
            let dataPath: URL
            if let store {
                dataPath = try store.prepare(from: root)
            } else {
                try fm.copyItem(at: root, to: trial)
                dataPath = trial.appendingPathComponent("data")
            }
            if mode == 3, !verifyWorkbench(dataPath: dataPath, bundled: root) {
                if store == nil { try? fm.removeItem(at: trial) }
                return "Workbench executable digest mismatch; saved environment not booted. Reset explicitly or use disposable mode."
            }
            if let cli {
                guard mode == 6 else { return "CLI executable requires original agent mode" }
                try cli.install(in: dataPath, reuse: persistent)
            }
            if let vsix {
                guard mode == 3 else { return "VSIX requires workbench" }
                try vsix.install(in: dataPath)
            }
            var result = dataPath.path.withCString { path in
                input.withCString { decode(nexus_local_run(path, mode, $0)) }
            }
            if result["shutdownClean"] as? Bool == true {
                if let store {
                    do {
                        if mode == 6, result["binaryDigestVerified"] as? Bool != true { throw LocalGuestStore.Failure.invalid }
                        if mode == 3, vsix != nil { try LocalVSIX.clearSlot(in: dataPath) }
                        try store.markClean(); result["savedGuestReusable"] = true
                    }
                    catch { result["savedGuestReusable"] = false }
                } else { try? fm.removeItem(at: trial) }
            } else if persistent { result["savedGuestReusable"] = false }
            guard let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted,.sortedKeys]) else { return "Invalid local report" }
            return String(decoding: data, as: UTF8.self)
        } catch LocalGuestStore.Failure.dirty { return "Saved guest was not cleanly shut down. Reset explicitly or use disposable mode." }
        catch LocalGuestStore.Failure.incompatible { return "Saved guest belongs to a different rootfs. Reset explicitly or use disposable mode." }
        catch { return "Local guest preparation failed" }
    }
}

/// User-selected archive, not an installer or automatically downloaded binary.
private struct LocalVSIX: Sendable {
    enum Failure: Error { case invalid }
    static let marker = Data("NEXUS_VSIX_IMPORT_SLOT_V1\n".utf8)
    static let limit = 32 * 1024 * 1024
    let bytes: Data
    let digest: String

    static func read(_ url: URL, expected: String) throws -> Self {
        guard url.isFileURL, url.pathExtension.lowercased() == "vsix", AGYImportFormat.validDigest(expected) else { throw Failure.invalid }
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        let v = try url.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey])
        guard v.isRegularFile == true, v.isSymbolicLink != true, let size = v.fileSize, size > 4, size <= limit else { throw Failure.invalid }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var data = Data()
        while let chunk = try handle.read(upToCount: 1024 * 1024), !chunk.isEmpty {
            guard data.count + chunk.count <= size else { throw Failure.invalid }
            data.append(chunk)
        }
        guard data.count == size, data.starts(with: [0x50,0x4b,0x03,0x04]),
              SHA256.hash(data: data).map({ String(format: "%02x", $0) }).joined() == expected else { throw Failure.invalid }
        return Self(bytes: data, digest: expected)
    }
    private static func slot(in dataPath: URL) throws -> URL {
        let slot = dataPath.appendingPathComponent("opt/nexus/extension.vsix")
        let v = try slot.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey])
        guard v.isRegularFile == true, v.isSymbolicLink != true, let size = v.fileSize,
              size <= limit, slot.resolvingSymlinksInPath().path.hasPrefix(dataPath.resolvingSymlinksInPath().path + "/") else { throw Failure.invalid }
        return slot
    }
    func install(in dataPath: URL) throws {
        let target = try Self.slot(in: dataPath)
        guard try Data(contentsOf: target) == Self.marker else { throw Failure.invalid }
        try Self.overwrite(target, bytes: bytes)
    }
    static func clearSlot(in dataPath: URL) throws {
        try overwrite(slot(in: dataPath), bytes: marker)
    }
    private static func overwrite(_ target: URL, bytes: Data) throws {
        // Preserve the existing backing inode and fakefs metadata.
        let handle = try FileHandle(forWritingTo: target)
        defer { try? handle.close() }
        try handle.truncate(atOffset: 0)
        try handle.write(contentsOf: bytes)
        try handle.synchronize()
    }
}

/// A navigation-restricted preview; not a security boundary for guest extensions.
/// Content rules block ordinary non-loopback HTTP(S)/WebSocket resource requests.
private struct LoopbackWorkbench: UIViewRepresentable {
    let token: String
    @Binding var status: String
    func makeCoordinator() -> Coordinator { Coordinator(status: $status) }
    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        let rules = #"[{"trigger":{"url-filter":"^(https?|wss?)://"},"action":{"type":"block"}},{"trigger":{"url-filter":"^(http|ws)://127[.]0[.]0[.]1:18765/"},"action":{"type":"ignore-previous-rules"}}]"#
        WKContentRuleListStore.default().compileContentRuleList(forIdentifier: "NexusLoopback-v1", encodedContentRuleList: rules) { list, _ in
            DispatchQueue.main.async {
                guard let list, token.utf8.count == 64,
                      let url = URL(string: "http://127.0.0.1:18765/?tkn=" + token) else {
                    status = "Local resource filter unavailable; preview blocked"; return
                }
                guard web.navigationDelegate != nil else { return }
                web.configuration.userContentController.add(list)
                web.load(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData))
            }
        }
        return web
    }
    func updateUIView(_ view: WKWebView, context: Context) {}
    static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
        view.stopLoading()
        view.navigationDelegate = nil
    }
    final class Coordinator: NSObject, WKNavigationDelegate {
        @Binding var status: String
        init(status: Binding<String>) { _status = status }
        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { status = "Loopback page loaded; editor compatibility not verified" }
        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { status = "Local workbench navigation failed" }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { status = "Local workbench unavailable" }
        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                     decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            let url = navigationAction.request.url
            decisionHandler(url?.scheme == "http" && url?.host == "127.0.0.1" && url?.port == 18765 && url?.user == nil && url?.password == nil ? .allow : .cancel)
        }
        func webView(_ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse,
                     decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void) {
            let url = navigationResponse.response.url
            decisionHandler(url?.scheme == "http" && url?.host == "127.0.0.1" && url?.port == 18765 ? .allow : .cancel)
        }
    }
}
#endif
