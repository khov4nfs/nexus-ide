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
                Section("Native ACP client") { Text(agentStatus) }
                Section("Evidence") {
                    Text(report).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
                }
            }
            .navigationTitle("Ubuntu Lab")
            .confirmationDialog("Run original AGY in the local guest?", isPresented: $confirm) {
                Button("Verify digest and initialize") { if let sha = digest { run(sha: sha) } }
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
            running = false
        }
    }
}

private enum UbuntuRunner {
    static func run(sha: String?) -> (report: String, status: String) {
        var client = ACPInitializeClient()
        func failed(_ reason: String) -> (String, String) { (reason, "Not initialized") }
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
                        request.withCString { nexus_guest_probe_glibc_acp(path, digest, $0) }
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
                return (pretty, "Libc only; AGY not executed")
            }
            guard object["shutdownError"] is NSNull,
                  let rows = object["checks"] as? [[String: Any]],
                  let row = rows.first(where: { $0["name"] as? String == "agy_acp_initialize" }),
                  row["transportClean"] as? Bool == true,
                  let response = row["response"] as? [String: Any] else {
                return (pretty, "Initialize or transport failed; restart app")
            }
            do {
                var frame = try JSONSerialization.data(withJSONObject: response)
                frame.append(10)
                try client.receive(frame)
                try client.finish()
                switch client.state {
                case let .initialized(name, version):
                    guard object["ok"] as? Bool == true, row["passed"] as? Bool == true else {
                        return (pretty, "Initialize failed")
                    }
                    return (pretty, "\(name) \(version): ACP v1 verified; probe stopped")
                case .authenticationRequired:
                    return (pretty, "Authentication required; sign-in not implemented")
                default: return (pretty, "Initialize failed")
                }
            } catch { return (pretty, "Invalid or rejected ACP response") }
        } catch { return failed("Preparation failed: \(error)") }
    }
}
#endif
