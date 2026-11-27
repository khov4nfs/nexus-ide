#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import Foundation
import NexusCore
import UniformTypeIdentifiers

private struct ChatTurn: Identifiable {
    let id: Int
    let user: String
    let response: String
    let status: String
}

struct UbuntuLabView: View {
    @ObservedObject var model: IDEModel
    @State private var editSnapshot: WorkspaceSnapshot?
    @State private var pendingEdit: WorkspaceSnapshot?
    @State private var proposal: AgentEditProposal?
    @State private var editStatus = ""
    @State private var showingProposal = false
    @State private var attempted = false
    @State private var running = false
    @State private var confirm = false
    @State private var report = "No device evidence."
    @State private var agentStatus = "Not initialized"
    @State private var active = false
    @State private var created = false
    @State private var turnCount = 0
    @State private var smokeUsed = false
    @State private var transcript: [ChatTurn] = []
    private let maxTurns = 20
    @State private var confirmPrompt = false
    @State private var confirmChat = false
    @State private var chatInput = ""
    @State private var pickingContext = false
    @State private var importingContext = false
    @State private var fileContext: AgentFileContext?
    @State private var contextStatus = ""
    @State private var pendingChat = ""
    @State private var pendingDisplay = ""
    @State private var chatResponse = ""
    @State private var sentChat = ""
    @State private var promptRunning = false
    @State private var receivedChunks = 0
    @State private var permissionDenials = 0
    @State private var toolUpdates = 0
    @State private var cancelRequested = false
    @State private var keyInput = ""
    @State private var keyStored = false
    @State private var keyStatus = ""
    @State private var useGemini = false
    @State private var useGoogle = false
    @State private var enableSnapshotRead = false
    @State private var enableSnapshotWrite = false
    @State private var enableTerminal = false
    @State private var terminalChoice = ""
    @State private var customTerminal = "[\"/usr/bin/printf\",\"%s\\n\",\"hello\"]"
    @State private var pendingTerminal: String?
    @State private var snapshotReadThisTurn = false
    @State private var pendingReadContent: String?
    @State private var loginURL = ""
    @State private var loginCode = ""
    @State private var loginSubmitted = false
    @State private var loginRunning = false
    @Environment(\.openURL) private var openURL
    @State private var importedAGY: LocalAGYImport?
    @State private var expectedAGY = ""
    @State private var frozenAGYDigest = ""
    @State private var pickingAGY = false
    @State private var importingAGY = false
    @State private var importStatus = ""
    @State private var savedImportInvalid = false
    @State private var confirmRemoveAGY = false
    private var importSlot: Bool {
        guard let root = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil) else { return false }
        return LocalAGYImport.available(in: root)
    }
    private var digest: String? {
        if let importedAGY { return importedAGY.digest }
        guard let root = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil),
              let text = try? String(contentsOf: root.appendingPathComponent("agy.sha256"), encoding: .utf8) else { return nil }
        let sha = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return sha.utf8.count == 64 && sha.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) } ? sha : nil
    }

    var body: some View {
        NavigationStack {
            List {
                Section("Ubuntu ARM64 · glibc lab") {
                    Text("One boot per app process. No remote backend.")
                    if importSlot {
                        Text("Import a reviewed official Linux ARM64 AGY executable from Files. Supply its independently reviewed SHA-256; no archive or installer.")
                            .font(.caption)
                        TextField("Expected lowercase SHA-256", text: $expectedAGY)
                            .textInputAutocapitalization(.never).autocorrectionDisabled()
                            .disabled(attempted || model.guestClaimed || importingAGY)
                        Button("Choose AGY executable") {
                            frozenAGYDigest = expectedAGY
                            pickingAGY = true
                        }.disabled(attempted || model.guestClaimed || importingAGY || importedAGY != nil || savedImportInvalid || !AGYImportFormat.validDigest(expectedAGY))
                            .fileImporter(isPresented: $pickingAGY, allowedContentTypes: [.data]) { result in
                                importAGY(result)
                            }
                        if importingAGY { ProgressView("Verifying local ELF and SHA-256") }
                        if importedAGY != nil || savedImportInvalid {
                            Text("Reviewed executable is saved privately; SHA-256 checked again before every boot.").font(.caption)
                            Button("Remove saved AGY executable", role: .destructive) {
                                confirmRemoveAGY = true
                            }.disabled(attempted || model.guestClaimed || importingAGY)
                        }
                        Text(importStatus).font(.caption)
                    }
                    Toggle("Enable read-only ACP attachment tool", isOn: $enableSnapshotRead)
                        .disabled(attempted || model.guestClaimed)
                    Toggle("Enable staged ACP editor edits", isOn: $enableSnapshotWrite)
                        .disabled(attempted || model.guestClaimed)
                    Toggle("Enable approved ACP terminal commands (Gemini key)", isOn: $enableTerminal)
                        .disabled(attempted || model.guestClaimed)
                    Text("Optional: selected text only, for one confirmed turn. Staged edits require separate editor selection and native diff review; no direct iOS file write.").font(.caption)
                    Button("Run offline libc checks") { run(sha: nil) }.disabled(attempted || model.guestClaimed || importingAGY)
                    Button("Initialize imported AGY (no key)") { useGemini = false; useGoogle = false; confirm = true }.disabled(attempted || model.guestClaimed || importingAGY || digest == nil)
                    Button("Connect with saved Gemini API key") { useGemini = true; useGoogle = false; confirm = true }
                        .disabled(attempted || model.guestClaimed || importingAGY || digest == nil || !keyStored)
                    Button("Sign in with Google") { useGoogle = true; useGemini = false; confirm = true }
                        .disabled(attempted || model.guestClaimed || importingAGY || digest == nil)
                    if running && useGoogle {
                        Text("Google sign-in uses the official AGY CLI. Open the browser, return promptly and enter the code. Do not share the code or URL. The app may suspend in the background.").font(.caption)
                        if let url = UbuntuRunner.googleURL(loginURL), !loginSubmitted {
                            Button("Open Google sign-in in Safari") { openURL(url) }
                            SecureField("Authorization code from Google", text: $loginCode)
                                .textInputAutocapitalization(.never).autocorrectionDisabled()
                            Button("Submit authorization code") {
                                if UbuntuRunner.submitGoogle(loginCode.trimmingCharacters(in: .whitespacesAndNewlines)) {
                                    loginCode = ""; loginSubmitted = true; loginURL = ""
                                } else { agentStatus = "Code rejected; check the format" }
                            }.disabled(loginCode.isEmpty)
                        }
                        if loginSubmitted { Text("Code submitted; verifying login and starting ACP").font(.caption) }
                        Button("Cancel Google sign-in", role: .destructive) {
                            _ = UbuntuRunner.cancelGoogle()
                            loginCode = ""; loginURL = ""
                        }.disabled(!loginRunning)
                    }
                    Text(digest == nil ? "No reviewed AGY binary imported." : "Reviewed digest present. Execution and publisher identity not verified.")
                    if running { ProgressView("Keep this app open") }
                }
                Section("Gemini credential · device only") {
                    SecureField("Gemini API key", text: $keyInput)
                        .textContentType(.password)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .disabled(attempted)
                    Button("Save to Keychain") {
                        defer { keyInput = "" }
                        do {
                            try GeminiKeychain.save(keyInput)
                            refreshKey()
                            keyStatus = "Saved on this device"
                        } catch { keyStatus = "Invalid key or Keychain unavailable" }
                    }.disabled(attempted || !GeminiCredential.isValid(keyInput))
                    Button("Delete saved key", role: .destructive) {
                        do { try GeminiKeychain.remove(); refreshKey(); keyStatus = "Deleted" }
                        catch { keyStatus = "Keychain deletion failed" }
                    }.disabled(attempted || !keyStored)
                    Text(keyStatus).font(.caption)
                    Text("Keychain is not synced. Deleting a key does not revoke it at Google.").font(.caption)
                }
                Section("Native ACP client") {
                    Text(agentStatus)
                    Text("ACP permissions deny by default. Selected virtual file edits require native review. Agent internal actions are not sandboxed.").font(.caption)
                    if active {
                        Text("Connection closes after 180 seconds without a request. Session uses /tmp, not your workspace.").font(.caption)
                        Button("Create empty guest session") { act(close: false) }.disabled(running || created)
                        if created && (useGemini || useGoogle) {
                            Button("Test model response") { confirmPrompt = true }.disabled(running || importingContext || turnCount != 0)
                            TextField("Ask the agent", text: $chatInput, axis: .vertical)
                                .lineLimit(2...6).disabled(running || smokeUsed || turnCount >= maxTurns)
                            Button("Attach saved editor file for edit") { attachEditor() }
                                .disabled(running || importingContext || smokeUsed || turnCount >= maxTurns || model.path.isEmpty || model.reviewBusy || proposal != nil || useGoogle)
                            Button("Choose text file") { pickingContext = true }
                                .disabled(running || importingContext || smokeUsed || turnCount >= maxTurns)
                            if importingContext { ProgressView("Reading selected file") }
                            if let context = fileContext {
                                Text("Selected snapshot · \(context.text.utf8.count) bytes").font(.caption)
                                ScrollView {
                                    Text(context.text).font(.system(.footnote, design: .monospaced))
                                        .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                                }.frame(maxHeight: 160)
                                if let editSnapshot { Text("Edit target: " + editSnapshot.path).font(.caption) }
                                if enableSnapshotRead && editSnapshot == nil {
                                    Toggle("Share via read-only ACP tool", isOn: $snapshotReadThisTurn).disabled(running)
                                }
                                Button("Remove attachment") { fileContext = nil; editSnapshot = nil; contextStatus = "" }
                                    .disabled(running || importingContext)
                            }
                            if !contextStatus.isEmpty { Text(contextStatus).font(.caption) }
                            if let context = fileContext, let combined = try? context.prompt(question: chatInput) {
                                Text("Combined prompt: \(combined.utf8.count)/1024 bytes").font(.caption)
                            }
                            if enableTerminal && useGemini && fileContext == nil && editSnapshot == nil {
                                Picker("Allow one terminal command this turn", selection: $terminalChoice) {
                                    Text("None").tag("")
                                    Text("uname -m").tag("uname")
                                    Text("pwd").tag("pwd")
                                    Text("Custom exact argv").tag("custom")
                                }.disabled(running)
                                if terminalChoice == "custom" {
                                    TextField("JSON argv (absolute guest executable)", text: $customTerminal, axis: .vertical)
                                        .font(.system(.body, design: .monospaced))
                                        .autocorrectionDisabled().textInputAutocapitalization(.never).disabled(running)
                                    Text("A reviewed command can modify guest files or access its network and credentials. Do not grant commands you do not understand. No shell interpolation by Nexus.").font(.caption)
                                }
                            }
                            Button(editSnapshot == nil ? "Send chat" : "Request edit proposal") { prepareChat() }
                                .disabled(running || importingContext || smokeUsed || turnCount >= maxTurns || !UbuntuRunner.validChat(chatInput))
                            Text("Chat turns: \(turnCount)/\(maxTurns)").font(.caption)
                        }
                        if turnCount > 0 {
                            Text("Permission denials selected: \(permissionDenials) · Tool updates: \(toolUpdates)").font(.caption)
                        }
                        if promptRunning {
                            Text("Received text chunks: \(receivedChunks)").font(.caption)
                            Button(cancelRequested ? "Cancellation requested" : "Cancel prompt", role: .destructive) {
                                let accepted = UbuntuRunner.cancelPrompt()
                                if accepted { cancelRequested = true }
                            }.disabled(cancelRequested)
                        }
                        Button("Close local connection", role: .destructive) { act(close: true) }.disabled(running || importingContext)
                    }
                }
                if !transcript.isEmpty || !sentChat.isEmpty {
                    Section("Chat · temporary") {
                        ForEach(transcript) { turn in
                            VStack(alignment: .leading, spacing: 6) {
                                Text("You").font(.caption)
                                Text(turn.user)
                                Text("Agent").font(.caption)
                                Text(turn.response.isEmpty ? "No response available" : turn.response)
                                Text(turn.status).font(.caption)
                            }
                        }
                        if !sentChat.isEmpty {
                            Text("You").font(.caption)
                            Text(sentChat)
                            Text("Agent").font(.caption)
                            Text(chatResponse.isEmpty ? "Waiting for text…" : chatResponse)
                        }
                        Text("Untrusted response · literal key filter only").font(.caption)
                    }
                }
                if !editStatus.isEmpty || proposal != nil {
                    Section("Agent edit review") {
                        Text(editStatus)
                        if proposal != nil {
                            Button("Review proposal") { showingProposal = true }
                                .disabled(running || model.reviewBusy)
                            Button("Discard proposal", role: .destructive) { proposal = nil; editStatus = "Discarded" }
                                .disabled(model.reviewBusy)
                        }
                    }
                }
                Section("Evidence") {
                    Text(report).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
                }
            }
            .navigationTitle("Agent · Ubuntu")
            .sheet(isPresented: $showingProposal) {
                if let proposal {
                    NavigationStack {
                        ScrollView {
                            VStack(alignment: .leading, spacing: 12) {
                                Text(proposal.snapshot.path).font(.headline)
                                Text("Change preview (not a minimal diff)").font(.caption)
                                ForEach(Array(proposal.diff.enumerated()), id: \.offset) { _, line in
                                    Text(line).font(.system(.footnote, design: .monospaced))
                                        .foregroundStyle(line.hasPrefix("+ ") ? Color.green : line.hasPrefix("- ") ? Color.red : Color.primary)
                                }
                                Text("Original · \(proposal.snapshot.text.utf8.count) bytes").font(.headline)
                                Text(proposal.snapshot.text).textSelection(.enabled)
                                Text("Replacement · \(proposal.replacement.utf8.count) bytes").font(.headline)
                                Text(proposal.replacement).textSelection(.enabled)
                                Text("Review the full replacement. It will not run automatically.").font(.caption)
                                Text(editStatus).font(.caption)
                            }.padding().frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .navigationTitle("Review edit")
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("Discard") { self.proposal = nil; showingProposal = false; editStatus = "Discarded" }
                                    .disabled(model.reviewBusy)
                            }
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Apply") { applyProposal(proposal) }.disabled(model.reviewBusy || running)
                            }
                        }
                        .interactiveDismissDisabled(model.reviewBusy)
                    }
                }
            }
            .onAppear {
                refreshKey()
                if !attempted && !importingAGY && !model.guestClaimed {
                    do {
                        importedAGY = try LocalAGYImport.loadSaved()
                        savedImportInvalid = false
                        if importedAGY != nil { importStatus = "Saved import found; digest rechecked before execution" }
                    } catch { importedAGY = nil; savedImportInvalid = true; importStatus = "Saved import invalid; remove it explicitly" }
                }
            }
            .alert("Remove saved AGY executable?", isPresented: $confirmRemoveAGY) {
                Button("Remove executable", role: .destructive) {
                    guard !model.guestClaimed, !importingAGY else { return }
                    do {
                        try LocalAGYImport.removeSaved()
                        importedAGY = nil; savedImportInvalid = false; importStatus = "Removed"
                    } catch { importStatus = "Removal failed" }
                }
                Button("Cancel", role: .cancel) {}
            } message: { Text("Only the separately imported binary is removed. No native project files are deleted.") }
            .fileImporter(isPresented: $pickingContext, allowedContentTypes: [.plainText]) { result in
                importContext(result)
            }
            .task(id: active && !running) {
                while active && !running && !Task.isCancelled {
                    if !UbuntuRunner.connectionActive() {
                        active = false
                        created = false
                        fileContext = nil; editSnapshot = nil; pendingEdit = nil
                        pendingChat = ""; pendingDisplay = ""; pendingReadContent = nil
                        agentStatus = "Agent connection closed or idle timeout; restart app"
                        break
                    }
                    try? await Task.sleep(nanoseconds: 1_000_000_000)
                }
            }
            .task(id: promptRunning) {
                guard promptRunning else { return }
                while !Task.isCancelled && promptRunning {
                    let progress = UbuntuRunner.progress()
                    if progress.responseID == turnCount + 2 {
                        receivedChunks = progress.chunks
                        permissionDenials = progress.permissionDenials
                        toolUpdates = progress.toolUpdates
                        if !sentChat.isEmpty { chatResponse = progress.text }
                        if progress.cancelRequested { cancelRequested = true }
                    }
                    try? await Task.sleep(nanoseconds: 100_000_000)
                }
            }
            .task(id: running && useGoogle && !active) {
                while running && useGoogle && !active && !Task.isCancelled {
                    let state = UbuntuRunner.googleProgress()
                    loginRunning = state.running
                    loginURL = state.url
                    if state.submitted { loginSubmitted = true }
                    try? await Task.sleep(nanoseconds: 200_000_000)
                }
            }
            .confirmationDialog("Send your message to the model?", isPresented: $confirmChat) {
                Button("Send message") {
                    let wire = pendingChat
                    let display = pendingDisplay
                    let edit = pendingEdit
                    let content = pendingReadContent
                    let terminal = pendingTerminal
                    pendingTerminal = nil
                    pendingChat = ""; pendingDisplay = ""; pendingEdit = nil; pendingReadContent = nil
                    runPrompt(chat: wire, display: display, edit: edit, readContent: content, terminal: terminal)
                }
                Button("Cancel", role: .cancel) { pendingChat = ""; pendingDisplay = ""; pendingEdit = nil; pendingReadContent = nil; pendingTerminal = nil; terminalChoice = "" }
            } message: {
                Text(pendingTerminal != nil ? "Allow this exact guest command once?\n" + Self.terminalDescription(pendingTerminal ?? "") + "\nIts output may be sent to the model. Commands may modify guest files, access guest credentials or use the network. No security sandbox." : pendingEdit != nil && pendingReadContent != nil ? "Allow this turn to read the selected saved editor file and stage one complete replacement? Nothing is written until you review and apply. The model may receive the selected text and incur charges. Do not share secrets." : "Your message and any selected snapshot may be sent to Google and incur charges. Review the file text before sending. Do not include passwords, keys or private data. No file path or project mount is sent. Agent actions are not sandboxed.")
            }
            .alert("Send fixed test prompt to Google?", isPresented: $confirmPrompt) {
                Button("Send test prompt") { runPrompt() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The agent may use model inference and incur API charges. Only a fixed test message is sent by Nexus; no user project is mounted. The agent may still perform internal actions; this is not a sandbox.")
            }
            .confirmationDialog("Run original AGY in the local guest?", isPresented: $confirm) {
                Button("Verify digest and connect") { if let sha = digest { run(sha: sha, gemini: useGemini, google: useGoogle) } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(useGoogle ? "AGY will start its official Google terminal sign-in in a disposable local guest. Safari opens only a Google OAuth URL; paste the authorization code here. Credentials may be cached by AGY in guest storage. No key or project is supplied. Not a security sandbox." : useGemini ? "Your saved key will enter the local agent process environment. AGY may connect to Google and write its own logs. The emulator is not a security sandbox. Only use a reviewed official binary and a restricted key. No project or prompt is supplied." : "AGY may make startup network requests. No credentials, user project, prompts or permission grants are supplied. This emulator is not a security sandbox. Run only a reviewed official binary.")
            }
        }
    }

    private func refreshKey() {
        do {
            keyStored = try GeminiKeychain.load() != nil
            keyStatus = keyStored ? "Saved key available" : "No key saved"
        } catch { keyStored = false; keyStatus = "Keychain unavailable" }
    }

    private func importAGY(_ result: Result<URL, Error>) {
        guard !attempted, !model.guestClaimed, !importingAGY, importSlot,
              case .success(let url) = result else { return }
        let expected = frozenAGYDigest
        importingAGY = true
        guard importedAGY == nil, !savedImportInvalid else { importingAGY = false; return }
        Task {
            let selected = await Task.detached(priority: .userInitiated) {
                try? LocalAGYImport.select(url, expected: expected)
            }.value
            importingAGY = false
            guard !model.guestClaimed else { return }
            importedAGY = selected
            importStatus = selected == nil ? "Import rejected: file, ARM64 ELF or digest mismatch" : "Local import verified. Nothing executed; consent to connect separately."
        }
    }

    private func run(sha: String?, gemini: Bool = false, google: Bool = false) {
        guard !attempted, !importingAGY, model.claimGuest() else { return }
        let selected = sha == nil ? nil : importedAGY
        let writeSnapshot = enableSnapshotWrite && gemini
        let readSnapshot = enableSnapshotRead || writeSnapshot
        let terminal = enableTerminal && gemini
        attempted = true
        running = true
        loginURL = ""; loginCode = ""; loginSubmitted = false
        Task {
            let result = await Task.detached(priority: .userInitiated) { UbuntuRunner.run(sha: sha, gemini: gemini, google: google, imported: selected, readSnapshot: readSnapshot, writeSnapshot: writeSnapshot, terminal: terminal) }.value
            loginURL = ""; loginCode = ""; loginRunning = false
            // Keep the separately reviewed binary; never persist OAuth tokens with it.
            report = result.report
            agentStatus = result.status
            active = result.active
            running = false
        }
    }

    private func attachEditor() {
        guard !running, !importingContext, active, created, proposal == nil else { return }
        importingContext = true
        fileContext = nil; editSnapshot = nil
        Task {
            defer { importingContext = false }
            do {
                let snapshot = try await model.selectedSnapshot()
                let context = try AgentFileContext(data: Data(snapshot.text.utf8))
                guard let key = try GeminiKeychain.load(), !snapshot.text.contains(key) else {
                    contextStatus = "Credential detected or unavailable"; return
                }
                fileContext = context
                editSnapshot = snapshot
                contextStatus = "Review the saved editor snapshot before requesting an edit"
            } catch { contextStatus = "Save the selected editor file first; text limit 768 bytes" }
        }
    }

    private func applyProposal(_ proposal: AgentEditProposal) {
        Task {
            do {
                try await model.applyAgentProposal(proposal)
                self.proposal = nil
                showingProposal = false
                editStatus = "Applied reviewed edit to native workspace"
            } catch { editStatus = "Edit rejected: file or editor changed; discard and request a new proposal" }
        }
    }

    private func importContext(_ result: Result<URL, Error>) {
        guard !running, !importingContext, active, created, !smokeUsed, turnCount < maxTurns else { return }
        fileContext = nil
        editSnapshot = nil
        contextStatus = ""
        guard case .success(let url) = result else { contextStatus = "No file selected"; return }
        importingContext = true
        Task {
            let snapshot = await Task.detached(priority: .userInitiated) {
                let accessed = url.startAccessingSecurityScopedResource()
                defer { if accessed { url.stopAccessingSecurityScopedResource() } }
                return try? AgentFileContext.read(url)
            }.value
            importingContext = false
            guard active, !running else { return }
            fileContext = snapshot
            contextStatus = snapshot == nil ? "Select a regular UTF-8 text file, at most 768 bytes" : "Review the snapshot before sending; filename and path are not included"
        }
    }

    private func credentialAllows(_ text: String) throws -> Bool {
        if let key = try GeminiKeychain.load() { return !text.contains(key) }
        return useGoogle
    }

    private static func terminalDescription(_ choice: String) -> String {
        switch choice {
        case "uname": return "[\"/usr/bin/uname\",\"-m\"]"
        case "pwd": return "[\"/usr/bin/pwd\"]"
        default: return choice
        }
    }

    private func prepareChat() {
        guard !running, !importingContext, active, created, !smokeUsed, turnCount < maxTurns else { return }
        do {
            var wire: String
            pendingReadContent = nil
            pendingTerminal = nil
            if let editSnapshot {
                guard proposal == nil else { contextStatus = "Review or discard the previous proposal"; return }
                if enableSnapshotWrite {
                    guard useGemini, !chatInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, try credentialAllows(editSnapshot.text) else { contextStatus = "Credential detected"; return }
                    wire = chatInput + "\nRead /nexus-selected.txt and stage one complete replacement via fs/write_text_file. Native user will review before applying."
                    pendingReadContent = editSnapshot.text
                } else { wire = try AgentEditProposal.prompt(snapshot: editSnapshot, question: chatInput) }
            } else if enableSnapshotRead, snapshotReadThisTurn, let context = fileContext {
                guard try credentialAllows(context.text) else { contextStatus = "Credential detected"; return }
                wire = chatInput + "\nRead the selected text via fs/read_text_file at /nexus-selected.txt if needed."
                pendingReadContent = context.text
            } else { wire = try fileContext?.prompt(question: chatInput) ?? chatInput }
            if enableTerminal && useGemini && fileContext == nil && editSnapshot == nil && !terminalChoice.isEmpty {
                let choice: String
                if terminalChoice == "custom" {
                    let argv = try ACPTerminalApproval.parse(Data(customTerminal.utf8))
                    guard try argv.arguments.allSatisfy({ try credentialAllows($0) }) else { contextStatus = "Credential detected"; return }
                    choice = try argv.wire()
                } else { choice = terminalChoice }
                pendingTerminal = choice
                wire += "\nYou may use the one granted terminal argv: " + Self.terminalDescription(choice) + ". cwd /tmp, no custom environment."
            }
            guard UbuntuRunner.validChat(wire) else {
                contextStatus = "Question and snapshot must fit 1024 UTF-8 bytes"; return
            }
            guard try credentialAllows(wire) else {
                fileContext = nil; editSnapshot = nil; chatInput = ""; contextStatus = "Credential unavailable or detected"; return
            }
            pendingChat = wire
            pendingEdit = editSnapshot
            pendingDisplay = fileContext == nil ? chatInput : chatInput + "\n[Selected text snapshot included]"
            if let pendingTerminal { pendingDisplay += "\n[Allowed terminal: " + Self.terminalDescription(pendingTerminal) + "]" }
            confirmChat = true
        } catch {
            contextStatus = "Invalid question, oversized context, or Keychain unavailable"
        }
    }

    private func runPrompt(chat: String? = nil, display: String? = nil, edit: WorkspaceSnapshot? = nil, readContent: String? = nil, terminal: String? = nil) {
        guard active, created, (useGemini || useGoogle), !running, !importingContext, !smokeUsed, turnCount < maxTurns,
              chat != nil || turnCount == 0 else { return }
        if let chat {
            guard UbuntuRunner.validChat(chat) else { return }
            // Do not display or send a prompt containing the stored literal key.
            do {
                guard try credentialAllows(chat), try readContent.map({ try credentialAllows($0) }) ?? true else {
                    chatInput = ""; agentStatus = "Credential unavailable or detected in message"; return
                }
            } catch { chatInput = ""; agentStatus = "Keychain unavailable"; return }
            sentChat = display ?? chat
            terminalChoice = ""
            chatInput = ""
            fileContext = nil
            editSnapshot = nil
            contextStatus = ""
        }
        chatResponse = ""
        running = true
        turnCount += 1
        smokeUsed = chat == nil
        let expectedID = turnCount + 2
        promptRunning = true
        receivedChunks = 0
        permissionDenials = 0
        toolUpdates = 0
        cancelRequested = false
        Task {
            let result = await Task.detached(priority: .userInitiated) { UbuntuRunner.action(close: false, prompt: true, chat: chat, readContent: readContent, stageEdit: edit != nil && readContent != nil, terminal: terminal) }.value
            let snapshot = UbuntuRunner.progress()
            if snapshot.responseID == expectedID {
                permissionDenials = snapshot.permissionDenials
                toolUpdates = snapshot.toolUpdates
            }
            let matches = result.responseID == expectedID && snapshot.responseID == expectedID && !snapshot.running
            if let chat {
                let text = result.ok && matches ? snapshot.text : ""
                transcript.append(ChatTurn(id: expectedID, user: display ?? chat, response: text,
                    status: matches ? result.status : "Turn failed or response mismatch"))
                if let edit {
                    if result.ok && matches && result.status == "Chat turn completed", let key = try? GeminiKeychain.load(),
                       let candidate = try? (readContent != nil
                           ? AgentEditProposal(snapshot: edit, stagedReplacement: result.stagedResponse, credential: key)
                           : AgentEditProposal(snapshot: edit, response: text, credential: key)) {
                        proposal = candidate
                        editStatus = "Untrusted proposal ready for explicit review"
                    } else { editStatus = "No valid complete replacement returned; nothing written" }
                }
                sentChat = ""
                chatResponse = ""
            }
            promptRunning = false
            report = result.report
            agentStatus = result.status
            active = result.active
            // Never continue with mismatched local/worker turn counters.
            if result.active && (!result.ok || !matches) {
                let closed = await Task.detached(priority: .userInitiated) { UbuntuRunner.action(close: true) }.value
                active = false
                agentStatus = "Turn failed; restart app. " + closed.status
            }
            running = false
        }
    }

    private func act(close: Bool) {
        guard active, !running else { return }
        running = true
        Task {
            let result = await Task.detached(priority: .userInitiated) { UbuntuRunner.action(close: close) }.value
            report = result.report
            agentStatus = result.status
            active = result.active
            if !close && result.ok { created = true }
            if !active { fileContext = nil; editSnapshot = nil; pendingEdit = nil; pendingChat = ""; pendingDisplay = ""; pendingReadContent = nil; pendingTerminal = nil; terminalChoice = ""; contextStatus = "" }
            running = false
        }
    }
}

private enum UbuntuRunner {
    private static let trialLock = NSLock()
    private static var activeTrial: URL?
    private static func rememberTrial(_ url: URL) {
        trialLock.lock(); defer { trialLock.unlock() }; activeTrial = url
    }
    private static func cleanTrialIfShutdown(_ object: [String: Any]) {
        guard object["shutdownClean"] as? Bool == true else { return }
        trialLock.lock(); defer { trialLock.unlock() }
        if let trial = activeTrial {
            do { try FileManager.default.removeItem(at: trial); activeTrial = nil } catch { /* leave for next launch */ }
        }
    }
    static func run(sha: String?, gemini: Bool, google: Bool, imported: LocalAGYImport?, readSnapshot: Bool, writeSnapshot: Bool, terminal: Bool) -> (report: String, status: String, active: Bool) {
        var client = ACPInitializeClient()
        func failed(_ reason: String) -> (String, String, Bool) { (reason, "Not initialized", false) }
        do {
            let key: String?
            if gemini {
                guard sha != nil, let saved = try GeminiKeychain.load() else { return failed("Saved key unavailable") }
                key = saved
            } else { key = nil }
            let fm = FileManager.default
            guard let bundled = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil) else {
                return failed("Missing Ubuntu fakefs.")
            }
            let trial = fm.temporaryDirectory.appendingPathComponent("NexusUbuntu-\(UUID().uuidString)", isDirectory: true)
            // Only one boot per app process. Clear stale trials from prior launches.
            if let old = try? fm.contentsOfDirectory(at: fm.temporaryDirectory, includingPropertiesForKeys: [.isDirectoryKey,.isSymbolicLinkKey]) {
                for url in old where url.lastPathComponent.hasPrefix("NexusUbuntu-") {
                    let suffix = String(url.lastPathComponent.dropFirst("NexusUbuntu-".count))
                    if UUID(uuidString: suffix) != nil, let v = try? url.resourceValues(forKeys: [.isDirectoryKey,.isSymbolicLinkKey]), v.isDirectory == true, v.isSymbolicLink != true { try? fm.removeItem(at: url) }
                }
            }
            try fm.copyItem(at: bundled, to: trial)
            rememberTrial(trial)
            if let imported {
                do {
                    guard sha == imported.digest else { throw LocalAGYImport.Failure.invalid }
                    try imported.install(in: trial)
                } catch {
                    try? fm.removeItem(at: trial)
                    return failed("Reviewed binary could not be staged")
                }
            }
            let data = trial.appendingPathComponent("data", isDirectory: true)
            let request = sha == nil ? nil : String(decoding: try client.request(readSnapshot: readSnapshot, writeSnapshot: writeSnapshot, terminal: terminal), as: UTF8.self)
            let pointer = data.path.withCString { path in
                if let sha, let request {
                    return sha.withCString { digest in
                        request.withCString { wire in
                            if let key { return key.withCString { nexus_acp_start_gemini(path, digest, wire, $0) } }
                            if google { return nexus_acp_start_google(path, digest, wire) }
                            return nexus_acp_start(path, digest, wire)
                        }
                    }
                }
                return nexus_guest_probe_glibc(path, nil)
            }
            guard let pointer else { return failed("No report returned.") }
            defer { nexus_guest_free(pointer) }
            let raw = String(cString: pointer)
            guard let object = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any] else {
                return failed("Invalid bridge report.")
            }
            cleanTrialIfShutdown(object)
            let pretty = (try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys]))
                .flatMap { String(data: $0, encoding: .utf8) } ?? raw
            guard sha != nil else {
                return (pretty, "Libc only; AGY not executed", false)
            }
            if object["authenticationRequired"] as? Bool == true {
                return (pretty, "Authentication required; restart and select Google sign-in", false)
            }
            guard object["ok"] as? Bool == true, object["active"] as? Bool == true,
                  let response = object["response"] as? [String: Any] else {
                if object["active"] as? Bool == true { nexus_guest_free(nexus_acp_close()) }
                return (pretty, "Initialize failed; restart app", false)
            }
            do {
                var frame = try JSONSerialization.data(withJSONObject: response)
                frame.append(10)
                try client.receive(frame)
                try client.finish()
                if case let .initialized(name, version) = client.state {
                    return (pretty, google ? "Google login process completed; ACP connected; model access not verified" : gemini ? "ACP connected with key supplied; model access not verified" : "\(name) \(version): ACP v1 connected", true)
                }
            } catch {}
            nexus_guest_free(nexus_acp_close())
            return (pretty, "Invalid ACP response; connection closed", false)
        } catch { return failed("Keychain or guest preparation failed") }
    }

    // Small, bounded native mailbox reads; never wait on the command worker.
    private static func controlReport(_ pointer: UnsafeMutablePointer<CChar>?) -> [String: Any] {
        guard let pointer else { return [:] }
        defer { nexus_guest_free(pointer) }
        return (try? JSONSerialization.jsonObject(with: Data(String(cString: pointer).utf8)) as? [String: Any]) ?? [:]
    }

    static func connectionActive() -> Bool { controlReport(nexus_acp_connection_status())["active"] as? Bool == true }
    static func googleURL(_ text: String) -> URL? {
        guard let c = URLComponents(string: text), c.scheme == "https", c.host == "accounts.google.com",
              c.port == nil, c.user == nil, c.password == nil, c.fragment == nil,
              ["/o/oauth2/v2/auth", "/o/oauth2/auth"].contains(c.path) else { return nil }
        return c.url
    }
    static func googleProgress() -> (running: Bool, url: String, submitted: Bool) {
        let obj = controlReport(nexus_google_login_progress())
        return (obj["running"] as? Bool == true,obj["url"] as? String ?? "",obj["submitted"] as? Bool == true)
    }
    static func submitGoogle(_ code: String) -> Bool {
        guard !code.utf8.contains(0) else { return false }
        return code.withCString { controlReport(nexus_google_login_code($0))["ok"] as? Bool == true }
    }
    static func cancelGoogle() -> Bool { controlReport(nexus_google_login_cancel())["ok"] as? Bool == true }

    static func validChat(_ text: String) -> Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && text.utf8.count <= 1024 &&
            text.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) || $0.value == 10 || $0.value == 9 }
    }

    static func progress() -> (chunks: Int, cancelRequested: Bool, text: String, responseID: Int, running: Bool, permissionDenials: Int, toolUpdates: Int) {
        let obj = controlReport(nexus_acp_prompt_progress())
        return (obj["chunkCount"] as? Int ?? 0, obj["cancelRequested"] as? Bool == true, obj["text"] as? String ?? "", obj["responseId"] as? Int ?? 0, obj["running"] as? Bool == true, obj["permissionDenials"] as? Int ?? 0, obj["toolUpdates"] as? Int ?? 0)
    }

    static func cancelPrompt() -> Bool {
        controlReport(nexus_acp_cancel_prompt())["ok"] as? Bool == true
    }

    static func action(close: Bool, prompt: Bool = false, chat: String? = nil, readContent: String? = nil, stageEdit: Bool = false, terminal: String? = nil) -> (report: String, status: String, active: Bool, ok: Bool, responseID: Int, stagedResponse: String?) {
        do {
            let pointer: UnsafeMutablePointer<CChar>?
            if close { pointer = nexus_acp_close() }
            else if let chat, let readContent { pointer = chat.withCString { t in readContent.withCString { stageEdit ? nexus_acp_chat_edit(t,$0) : nexus_acp_chat_snapshot(t,$0) } } }
            else if let chat, let terminal { pointer = chat.withCString { t in terminal.withCString { nexus_acp_chat_terminal(t,$0) } } }
            else if let chat { pointer = chat.withCString { nexus_acp_chat($0) } }
            else if prompt { pointer = nexus_acp_prompt_probe() }
            else {
                let request = String(decoding: try ACPRequest.newSession(id: 2, cwd: "/tmp"), as: UTF8.self)
                pointer = request.withCString { nexus_acp_new_session($0) }
            }
            guard let pointer else {
                nexus_guest_free(nexus_acp_close())
                return ("No report", "Connection unavailable", false, false, 0, nil)
            }
            defer { nexus_guest_free(pointer) }
            let raw = String(cString: pointer)
            guard let object = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any] else {
                nexus_guest_free(nexus_acp_close())
                return ("Invalid report", "Connection unavailable", false, false, 0, nil)
            }
            cleanTrialIfShutdown(object)
            let active = object["active"] as? Bool == true
            let ok = object["ok"] as? Bool == true
            let status: String
            if close && ok && object["transportClosed"] as? Bool == true && object["shutdownClean"] as? Bool == true {
                status = "Local connection closed; guest shutdown verified"
            } else if prompt && ok && active && object["cancelled"] as? Bool == true && object["stopReason"] as? String == "cancelled" {
                status = "Agent acknowledged cancellation"
            } else if chat != nil && ok && active && object["chatCompleted"] as? Bool == true {
                status = "Chat turn completed"
            } else if prompt && ok && active && object["promptVerified"] as? Bool == true && object["marker"] as? String == "NEXUS_ACP_OK" {
                status = "Fixed model response verified"
            } else if !close && !prompt && ok && active && object["sessionCreated"] as? Bool == true && object["responseId"] as? Int == 2 {
                status = "Empty guest session created; fixed test available in credential mode"
            } else if object["authenticationRequired"] as? Bool == true {
                status = "Authentication required; connection stopped"
            } else {
                status = "Request failed or connection expired; restart app"
            }
            var reportObject = object
            let replacement = reportObject.removeValue(forKey: "stagedReplacement") as? String
            let stagedResponse: String?
            if stageEdit, ok, status == "Chat turn completed" { stagedResponse = replacement }
            else { stagedResponse = nil }
            let report = (try? JSONSerialization.data(withJSONObject: reportObject, options: [.sortedKeys])).map { String(decoding: $0, as: UTF8.self) } ?? "Report unavailable"
            return (report, status, active, ok, object["responseId"] as? Int ?? 0, stagedResponse)
        } catch {
            nexus_guest_free(nexus_acp_close())
            return ("Request encoding failed", "Connection stopped", false, false, 0, nil)
        }
    }
}
#endif
