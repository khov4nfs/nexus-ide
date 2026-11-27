#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import UIKit
import Foundation
import WebKit
import Security
import NexusCore
import UniformTypeIdentifiers

struct LocalToolsView: View {
    @ObservedObject var model: IDEModel
    @State private var mode: UInt8 = 1
    @State private var keepGuest = false
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
                    }.disabled(model.guestClaimed)
                    if mode != 3 {
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
                        Text("Separate guest workspace, not native Files. Token-protected loopback only.").font(.caption)
                    }
                    if mode == 1 {
                        Toggle("Run reviewed lifecycle hooks", isOn: $enableHooks).disabled(model.guestClaimed)
                        if enableHooks {
                            TextField("Before command (optional)", text: $beforeHook, axis: .vertical).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                            TextField("After successful command (optional)", text: $afterHook, axis: .vertical).autocorrectionDisabled().textInputAutocapitalization(.never).disabled(model.guestClaimed)
                            Text("Hooks execute automatically only within this confirmed PTY job. Each runs in its own guest process. No provider key; no hook on app launch or file save.").font(.caption)
                        }
                    }
                    if mode != 3 {
                        Toggle("Keep Linux files across launches", isOn: $keepGuest).disabled(model.guestClaimed)
                        Text("Local tools only; never shared with Agent. Unclean shutdown blocks reuse. Reset deletes saved Linux files, not native Files.").font(.caption)
                        Button("Reset saved Linux environment", role: .destructive) { confirmReset = true }.disabled(model.guestClaimed)
                    }
                    Button("Review and start local job") {
                        frozenKeep = keepGuest && mode != 3
                        frozenMode = mode
                        frozenHooks = mode == 1 && enableHooks
                        if frozenHooks {
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
                    }.disabled(model.guestClaimed || (mode == 3 ? !stagedWorkbench : command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || command.utf8.count > 2048 || command.utf8.contains(0)))
                    Text("No provider key or native workspace injected. Commands and extensions may use the network and are not sandboxed. Restart app for another boot.").font(.caption)
                }
                if running && frozenMode != 2 && frozenMode != 4 {
                    Section("Process controls") {
                        if frozenMode == 1 {
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
                        } else {
                            Button("Open local workbench") { webStatus = "Opening verified bind endpoint…"; workbenchShown = true }.disabled(!ready || token.isEmpty)
                            Text("Server launch requested; web view may fail until ready.").font(.caption)
                        }
                        Button("Stop process", role: .destructive) {
                            if LocalRunner.stop() { ready = false; report = "Stop requested" }
                            else { report = "Stop not accepted yet" }
                        }
                    }
                }
                if frozenMode == 1 {
                    Section("PTY output · plain text") {
                        Text(verbatim: output).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
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
            .alert("Delete saved Linux environment?", isPresented: $confirmReset) {
                Button("Delete saved Linux files", role: .destructive) {
                    guard !model.guestClaimed else { return }
                    report = LocalRunner.resetSaved()
                }
                Button("Cancel", role: .cancel) {}
            } message: { Text("This permanently deletes the saved local-tools guest, including installed packages and guest files. Native editor files and Agent are separate.") }
            .confirmationDialog("Execute in local Linux?", isPresented: $confirm) {
                Button("Start") { start() }
                Button("Cancel", role: .cancel) { frozenInput = ""; frozenHooks = false }
            } message: {
                Text(frozenHooks ? "Run this exact lifecycle plan locally? Before hook must succeed. After hook runs only after a successful main command; no hook after cancellation.\n" + frozenInput + "\nCommands can modify guest files or use its network. No security sandbox." + (frozenKeep ? "\nUse saved Linux environment." : "\nUse disposable Linux environment.") : frozenMode == 4 ? "Execute one reviewed MCP call (may have side effects):\n" + frozenInput : frozenMode == 3 ? "Launch reviewed OpenVSCode on loopback with a random token. No native workspace mount. No guarantee of compatibility." : "Run this reviewed command locally:\n" + frozenInput + "\nNo provider key is injected. It may use the network, modify guest files or consume resources. No automatic execution." + (frozenKeep ? "\nUse saved Linux environment. Changes persist after clean shutdown." : "\nUse disposable Linux environment."))
            }
            .task(id: running) {
                while running && !Task.isCancelled {
                    let p = LocalRunner.progress()
                    ready = p.ready
                    output = p.output
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
            input = generated
        } else { input = frozenInput }
        guard model.claimGuest() else { return }
        let selectedMode: UInt8 = frozenHooks ? 5 : frozenMode
        let persistent = frozenKeep
        running = true
        report = "Starting local guest"
        Task {
            report = await Task.detached(priority: .userInitiated) {
                LocalRunner.run(mode: selectedMode, input: input, persistent: persistent)
            }.value
            output = LocalRunner.progress().output
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
    static func progress() -> (ready: Bool, output: String) {
        let object = decode(nexus_local_progress())
        return (object["ready"] as? Bool == true, object["output"] as? String ?? "")
    }
    static func input(_ text: String) -> Bool {
        text.withCString { decode(nexus_local_input($0))["ok"] as? Bool == true }
    }
    static func resize() -> Bool { decode(nexus_local_resize(111, 33))["ok"] as? Bool == true }
    static func stop() -> Bool { decode(nexus_local_stop())["ok"] as? Bool == true }
    static func savedStore() throws -> LocalGuestStore {
        let parent = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        var container = parent.appendingPathComponent("NexusLocalTools", isDirectory: true)
        try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try container.setResourceValues(values)
        return LocalGuestStore(directory: container.appendingPathComponent("guest-v1", isDirectory: true))
    }
    static func resetSaved() -> String {
        do { try savedStore().reset(); return "Saved Linux environment deleted" }
        catch { return "Saved environment reset failed" }
    }
    static func run(mode: UInt8, input: String, persistent: Bool) -> String {
        do {
            guard let root = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil) else { return "Missing Ubuntu assets" }
            let fm = FileManager.default
            let store = persistent ? try savedStore() : nil
            let trial = fm.temporaryDirectory.appendingPathComponent("NexusLocal-\(UUID().uuidString)", isDirectory: true)
            let dataPath: URL
            if let store {
                dataPath = try store.prepare(from: root)
            } else {
                try fm.copyItem(at: root, to: trial)
                dataPath = trial.appendingPathComponent("data")
            }
            var result = dataPath.path.withCString { path in
                input.withCString { decode(nexus_local_run(path, mode, $0)) }
            }
            if result["shutdownClean"] as? Bool == true {
                if let store {
                    do { try store.markClean(); result["savedGuestReusable"] = true }
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
