#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import Foundation
import WebKit
import Security
import NexusCore
import UniformTypeIdentifiers

struct LocalToolsView: View {
    @ObservedObject var model: IDEModel
    @State private var mode: UInt8 = 1
    @State private var command = "exec /bin/sh -i"
    @State private var line = ""
    @State private var running = false
    @State private var ready = false
    @State private var output = ""
    @State private var report = ""
    @State private var confirm = false
    @State private var frozenMode: UInt8 = 1
    @State private var frozenInput = ""
    @State private var token = ""
    @State private var workbenchShown = false
    @State private var manifestPicker = false
    @State private var manifestName = ""
    private var stagedWorkbench: Bool {
        guard let root = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil) else { return false }
        return FileManager.default.fileExists(atPath: root.appendingPathComponent("workbench.sha256").path)
    }
    var body: some View {
        NavigationStack {
            List {
                Section("Disposable local Ubuntu · one boot") {
                    Picker("Mode", selection: $mode) {
                        Text("PTY terminal").tag(UInt8(1))
                        Text("MCP discovery").tag(UInt8(2))
                        Text("OpenVSCode").tag(UInt8(3))
                    }.disabled(model.guestClaimed)
                    if mode != 3 {
                        TextField("Reviewed guest shell command", text: $command, axis: .vertical)
                            .font(.system(.body, design: .monospaced)).autocorrectionDisabled()
                            .textInputAutocapitalization(.never).disabled(model.guestClaimed)
                        Button("Import manual tool profile") { manifestPicker = true }.disabled(model.guestClaimed)
                        if !manifestName.isEmpty { Text(manifestName).font(.caption) }
                    } else {
                        Text(stagedWorkbench ? "Pinned workbench archive staged; runtime unverified." : "No workbench staged. See build instructions.")
                        Text("Separate guest workspace, not native Files. Token-protected loopback only.").font(.caption)
                    }
                    Button("Review and start local job") {
                        frozenMode = mode
                        frozenInput = command
                        confirm = true
                    }.disabled(model.guestClaimed || (mode == 3 ? !stagedWorkbench : command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || command.utf8.count > 2048 || command.utf8.contains(0)))
                    Text("No provider key or native workspace injected. Commands and extensions may use the network and are not sandboxed. Restart app for another boot.").font(.caption)
                }
                if running && frozenMode != 2 {
                    Section("Process controls") {
                        if frozenMode == 1 {
                            TextField("Input line", text: $line).autocorrectionDisabled().textInputAutocapitalization(.never)
                            Button("Send line") {
                                if LocalRunner.input(line + "\n") { line = "" }
                                else { report = "Input not accepted" }
                            }.disabled(!ready || line.utf8.count > 1023 || line.utf8.contains(0))
                            Button("Send Ctrl-C") { _ = LocalRunner.input("\u{3}") }.disabled(!ready)
                            Button("Resize to 111 × 33") { _ = LocalRunner.resize() }.disabled(!ready)
                        } else {
                            Button("Open local workbench") { workbenchShown = true }.disabled(!ready || token.isEmpty)
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
                    Text("ACP native tool grants, automatic hooks, extension compatibility, additional agent harnesses and Docker VM remain disabled.")
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
            .confirmationDialog("Execute in disposable local Linux?", isPresented: $confirm) {
                Button("Start") { start() }
                Button("Cancel", role: .cancel) { frozenInput = "" }
            } message: {
                Text(frozenMode == 3 ? "Launch reviewed OpenVSCode on loopback with a random token. No native workspace mount. No guarantee of compatibility." : "Run this reviewed command locally:\n" + frozenInput + "\nNo provider key is injected. It may use the network, modify guest files or consume resources. No automatic execution.")
            }
            .task(id: running) {
                while running && !Task.isCancelled {
                    let p = LocalRunner.progress()
                    ready = p.ready
                    output = p.output
                    try? await Task.sleep(nanoseconds: 100_000_000)
                }
            }
            .sheet(isPresented: $workbenchShown) {
                NavigationStack {
                    LoopbackWorkbench(token: token)
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
        let selectedMode = frozenMode
        running = true
        report = "Starting local guest"
        Task {
            report = await Task.detached(priority: .userInitiated) {
                LocalRunner.run(mode: selectedMode, input: input)
            }.value
            output = LocalRunner.progress().output
            ready = false
            running = false
            token = ""
            workbenchShown = false
        }
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
    static func run(mode: UInt8, input: String) -> String {
        do {
            guard let root = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil) else { return "Missing Ubuntu assets" }
            let trial = FileManager.default.temporaryDirectory.appendingPathComponent("NexusLocal-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.copyItem(at: root, to: trial)
            let result = trial.appendingPathComponent("data").path.withCString { path in
                input.withCString { decode(nexus_local_run(path, mode, $0)) }
            }
            guard let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted,.sortedKeys]) else { return "Invalid local report" }
            return String(decoding: data, as: UTF8.self)
        } catch { return "Local guest preparation failed" }
    }
}

/// A navigation-restricted preview; not a security boundary for guest extensions.
/// Content rules block ordinary non-loopback HTTP(S)/WebSocket resource requests.
private struct LoopbackWorkbench: UIViewRepresentable {
    let token: String
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        let rules = #"[{"trigger":{"url-filter":"^(https?|wss?)://"},"action":{"type":"block"}},{"trigger":{"url-filter":"^(http|ws)://127[.]0[.]0[.]1:18765/"},"action":{"type":"ignore-previous-rules"}}]"#
        WKContentRuleListStore.default().compileContentRuleList(forIdentifier: "NexusLoopback-v1", encodedContentRuleList: rules) { list, _ in
            DispatchQueue.main.async {
                guard let list, token.utf8.count == 64,
                      let url = URL(string: "http://127.0.0.1:18765/?tkn=" + token) else { return }
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
