#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import Foundation
import NexusCore

@main struct UbuntuLabApp: App {
    var body: some Scene { WindowGroup { UbuntuLabView() } }
}

private struct UbuntuLabView: View {
    @State private var attempted = false
    @State private var running = false
    @State private var confirm = false
    @State private var report = "No device evidence."
    @State private var agentStatus = "Not initialized"
    @State private var active = false
    @State private var created = false
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
                    Button("Initialize imported AGY") { confirm = true }.disabled(attempted || digest == nil)
                    Text(digest == nil ? "No reviewed AGY binary imported." : "Local AGY digest manifest present. Execution not verified.")
                    if running { ProgressView("Keep this app open") }
                }
                Section("Native ACP client") {
                    Text(agentStatus)
                    if active {
                        Text("Connection closes after 60 seconds without a request. Session uses /tmp, not your workspace.").font(.caption)
                        Button("Create empty guest session") { act(close: false) }.disabled(running || created)
                        Button("Close local connection", role: .destructive) { act(close: true) }.disabled(running)
                    }
                }
                Section("Evidence") {
                    Text(report).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
                }
            }
            .navigationTitle("Ubuntu Lab")
            .confirmationDialog("Run original AGY in the local guest?", isPresented: $confirm) {
                Button("Verify digest and connect") { if let sha = digest { run(sha: sha) } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("AGY may make startup network requests. No credentials, user project, prompts or permission grants are supplied. This emulator is not a security sandbox. Run only a binary you obtained and reviewed under applicable terms.")
            }
        }
    }

    private func run(sha: String?) {
        guard !attempted else { return }
        attempted = true
        running = true
        Task {
            let result = await Task.detached(priority: .userInitiated) { UbuntuRunner.run(sha: sha) }.value
            report = result.report
            agentStatus = result.status
            active = result.active
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
    static func run(sha: String?) -> (report: String, status: String, active: Bool) {
        var client = ACPInitializeClient()
        func failed(_ reason: String) -> (String, String, Bool) { (reason, "Not initialized", false) }
        do {
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
                        request.withCString { nexus_acp_start(path, digest, $0) }
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
                return (pretty, "Authentication required; sign-in not implemented", false)
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
                    return (pretty, "\(name) \(version): ACP v1 connected", true)
                }
            } catch {}
            nexus_guest_free(nexus_acp_close())
            return (pretty, "Invalid ACP response; connection closed", false)
        } catch { return failed("Preparation failed: \(error)") }
    }

    static func action(close: Bool) -> (report: String, status: String, active: Bool, ok: Bool) {
        do {
            let pointer: UnsafeMutablePointer<CChar>?
            if close { pointer = nexus_acp_close() }
            else {
                let request = String(decoding: try ACPRequest.newSession(id: 2, cwd: "/tmp"), as: UTF8.self)
                pointer = request.withCString { nexus_acp_new_session($0) }
            }
            guard let pointer else {
                nexus_guest_free(nexus_acp_close())
                return ("No report", "Connection unavailable", false, false)
            }
            defer { nexus_guest_free(pointer) }
            let raw = String(cString: pointer)
            guard let object = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any] else {
                nexus_guest_free(nexus_acp_close())
                return ("Invalid report", "Connection unavailable", false, false)
            }
            let active = object["active"] as? Bool == true
            let ok = object["ok"] as? Bool == true
            let status: String
            if close && ok && object["transportClosed"] as? Bool == true && object["shutdownClean"] as? Bool == true {
                status = "Local connection closed; guest shutdown verified"
            } else if !close && ok && active && object["sessionCreated"] as? Bool == true && object["responseId"] as? Int == 2 {
                status = "Empty guest session created; prompt disabled"
            } else if object["authenticationRequired"] as? Bool == true {
                status = "Authentication required; connection stopped"
            } else {
                status = "Request failed or connection expired; restart app"
            }
            return (raw, status, active, ok)
        } catch {
            nexus_guest_free(nexus_acp_close())
            return ("Request encoding failed", "Connection stopped", false, false)
        }
    }
}
#endif
