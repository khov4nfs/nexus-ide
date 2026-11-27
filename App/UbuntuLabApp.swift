#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import Foundation

@main struct UbuntuLabApp: App {
    var body: some Scene { WindowGroup { UbuntuLabView() } }
}

private struct UbuntuLabView: View {
    @State private var attempted = false
    @State private var running = false
    @State private var confirm = false
    @State private var report = "No device evidence."
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
            report = await Task.detached(priority: .userInitiated) { UbuntuRunner.run(sha: sha) }.value
            running = false
        }
    }
}

private enum UbuntuRunner {
    static func run(sha: String?) -> String {
        do {
            let fm = FileManager.default
            guard let bundled = Bundle.main.url(forResource: "ubuntu-fs", withExtension: nil) else {
                return "Missing Ubuntu fakefs."
            }
            let trial = fm.temporaryDirectory.appendingPathComponent("NexusUbuntu-\(UUID().uuidString)", isDirectory: true)
            try fm.copyItem(at: bundled, to: trial)
            let data = trial.appendingPathComponent("data", isDirectory: true)
            let pointer = data.path.withCString { path in
                if let sha { return sha.withCString { nexus_guest_probe_glibc(path, $0) } }
                return nexus_guest_probe_glibc(path, nil)
            }
            guard let pointer else { return "No report returned." }
            defer { nexus_guest_free(pointer) }
            let raw = String(cString: pointer)
            if let object = try? JSONSerialization.jsonObject(with: Data(raw.utf8)),
               let formatted = try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys]),
               let result = String(data: formatted, encoding: .utf8) { return result }
            return raw
        } catch { return "Preparation failed: \(error)" }
    }
}
#endif
