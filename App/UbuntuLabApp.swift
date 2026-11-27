#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import Foundation
import NexusCore

@main struct UbuntuLabApp: App {
    var body: some Scene { WindowGroup { UbuntuLabView() } }
}

private struct ChatTurn: Identifiable {
    let id: Int
    let user: String
    let response: String
    let status: String
}

private struct UbuntuLabView: View {
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
    private let maxTurns = 3
    @State private var confirmPrompt = false
    @State private var confirmChat = false
    @State private var chatInput = ""
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
    private var digest: String? {
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
                    Button("Run offline libc checks") { run(sha: nil) }.disabled(attempted)
                    Button("Initialize imported AGY (no key)") { useGemini = false; confirm = true }.disabled(attempted || digest == nil)
                    Button("Connect with saved Gemini API key") { useGemini = true; confirm = true }
                        .disabled(attempted || digest == nil || !keyStored)
                    Text(digest == nil ? "No reviewed AGY binary imported." : "Local AGY digest manifest present. Execution not verified.")
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
                    Text("ACP tool permissions: deny only. No native tool access. Agent internal actions are not sandboxed.").font(.caption)
                    if active {
                        Text("Connection closes after 60 seconds without a request. Session uses /tmp, not your workspace.").font(.caption)
                        Button("Create empty guest session") { act(close: false) }.disabled(running || created)
                        if created && useGemini {
                            Button("Test model response") { confirmPrompt = true }.disabled(running || turnCount != 0)
                            TextField("Ask the agent", text: $chatInput, axis: .vertical)
                                .lineLimit(2...6).disabled(running || smokeUsed || turnCount >= maxTurns)
                            Button("Send chat") { confirmChat = true }
                                .disabled(running || smokeUsed || turnCount >= maxTurns || !UbuntuRunner.validChat(chatInput))
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
                        Button("Close local connection", role: .destructive) { act(close: true) }.disabled(running)
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
                Section("Evidence") {
                    Text(report).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
                }
            }
            .navigationTitle("Ubuntu Lab")
            .onAppear { refreshKey() }
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
            .confirmationDialog("Send your message to the model?", isPresented: $confirmChat) {
                Button("Send message") { runPrompt(chat: chatInput) }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Your message may be sent to Google and incur charges. Do not include passwords, keys or private data. Up to three text turns per boot, each with consent. No project mounted. Agent actions are not sandboxed.")
            }
            .alert("Send fixed test prompt to Google?", isPresented: $confirmPrompt) {
                Button("Send test prompt") { runPrompt() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The agent may use model inference and incur API charges. Only a fixed test message is sent by Nexus; no user project is mounted. The agent may still perform internal actions; this is not a sandbox.")
            }
            .confirmationDialog("Run original AGY in the local guest?", isPresented: $confirm) {
                Button("Verify digest and connect") { if let sha = digest { run(sha: sha, gemini: useGemini) } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(useGemini ? "Your saved key will enter the local agent process environment. AGY may connect to Google and write its own logs. The emulator is not a security sandbox. Only use a reviewed official binary and a restricted key. No project or prompt is supplied." : "AGY may make startup network requests. No credentials, user project, prompts or permission grants are supplied. This emulator is not a security sandbox. Run only a reviewed official binary.")
            }
        }
    }

    private func refreshKey() {
        do {
            keyStored = try GeminiKeychain.load() != nil
            keyStatus = keyStored ? "Saved key available" : "No key saved"
        } catch { keyStored = false; keyStatus = "Keychain unavailable" }
    }

    private func run(sha: String?, gemini: Bool = false) {
        guard !attempted else { return }
        attempted = true
        running = true
        Task {
            let result = await Task.detached(priority: .userInitiated) { UbuntuRunner.run(sha: sha, gemini: gemini) }.value
            report = result.report
            agentStatus = result.status
            active = result.active
            running = false
        }
    }

    private func runPrompt(chat: String? = nil) {
        guard active, created, useGemini, !running, !smokeUsed, turnCount < maxTurns,
              chat != nil || turnCount == 0 else { return }
        if let chat {
            guard UbuntuRunner.validChat(chat) else { return }
            // Do not display or send a prompt containing the stored literal key.
            do {
                guard let key = try GeminiKeychain.load(), !chat.contains(key) else {
                    chatInput = ""; agentStatus = "Credential unavailable or detected in message"; return
                }
            } catch { chatInput = ""; agentStatus = "Keychain unavailable"; return }
            sentChat = chat
            chatInput = ""
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
            let result = await Task.detached(priority: .userInitiated) { UbuntuRunner.action(close: false, prompt: true, chat: chat) }.value
            let snapshot = UbuntuRunner.progress()
            if snapshot.responseID == expectedID {
                permissionDenials = snapshot.permissionDenials
                toolUpdates = snapshot.toolUpdates
            }
            let matches = result.responseID == expectedID && snapshot.responseID == expectedID && !snapshot.running
            if let chat {
                let text = result.ok && matches ? snapshot.text : ""
                transcript.append(ChatTurn(id: expectedID, user: chat, response: text,
                    status: matches ? result.status : "Turn failed or response mismatch"))
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
            running = false
        }
    }
}

private enum UbuntuRunner {
    static func run(sha: String?, gemini: Bool) -> (report: String, status: String, active: Bool) {
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
            try fm.copyItem(at: bundled, to: trial)
            let data = trial.appendingPathComponent("data", isDirectory: true)
            let request = sha == nil ? nil : String(decoding: try client.request(), as: UTF8.self)
            let pointer = data.path.withCString { path in
                if let sha, let request {
                    return sha.withCString { digest in
                        request.withCString { wire in
                            if let key { return key.withCString { nexus_acp_start_gemini(path, digest, wire, $0) } }
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
            let pretty = (try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys]))
                .flatMap { String(data: $0, encoding: .utf8) } ?? raw
            guard sha != nil else {
                return (pretty, "Libc only; AGY not executed", false)
            }
            if object["authenticationRequired"] as? Bool == true {
                return (pretty, "Authentication required; interactive sign-in not implemented", false)
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
                    return (pretty, gemini ? "ACP connected with key supplied; model access not verified" : "\(name) \(version): ACP v1 connected", true)
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

    static func action(close: Bool, prompt: Bool = false, chat: String? = nil) -> (report: String, status: String, active: Bool, ok: Bool, responseID: Int) {
        do {
            let pointer: UnsafeMutablePointer<CChar>?
            if close { pointer = nexus_acp_close() }
            else if let chat { pointer = chat.withCString { nexus_acp_chat($0) } }
            else if prompt { pointer = nexus_acp_prompt_probe() }
            else {
                let request = String(decoding: try ACPRequest.newSession(id: 2, cwd: "/tmp"), as: UTF8.self)
                pointer = request.withCString { nexus_acp_new_session($0) }
            }
            guard let pointer else {
                nexus_guest_free(nexus_acp_close())
                return ("No report", "Connection unavailable", false, false, 0)
            }
            defer { nexus_guest_free(pointer) }
            let raw = String(cString: pointer)
            guard let object = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any] else {
                nexus_guest_free(nexus_acp_close())
                return ("Invalid report", "Connection unavailable", false, false, 0)
            }
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
                status = "Empty guest session created; fixed test available with Gemini key"
            } else if object["authenticationRequired"] as? Bool == true {
                status = "Authentication required; connection stopped"
            } else {
                status = "Request failed or connection expired; restart app"
            }
            return (raw, status, active, ok, object["responseId"] as? Int ?? 0)
        } catch {
            nexus_guest_free(nexus_acp_close())
            return ("Request encoding failed", "Connection stopped", false, false, 0)
        }
    }
}
#endif
