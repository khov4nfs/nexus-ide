import SwiftUI
import Foundation

struct GuestProbeView: View {
    @State private var attempted = false
    @State private var running = false
    @State private var report = "No device probe has run."
    @State private var confirm = false

    var body: some View {
        NavigationStack {
            List {
                Section("Local ARM64 Linux · experimental") {
#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
                    Text("Runtime linked into this app. Device execution is not verified until the checks below pass.")
                    Button("Run trusted guest smoke test") { confirm = true }
                        .disabled(attempted || running)
                    if running { ProgressView("Booting guest… keep the app open") }
                    Text("One attempt per app process. No custom commands, secrets, or untrusted packages.")
#else
                    Text("Guest runtime is not linked in this build.")
                    Text("Build the separate NexusIDEGuest device target using the pinned upstream build script. No remote fallback.")
#endif
                }
                Section("Evidence") {
                    Text(report).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
                }
            }
            .navigationTitle("Linux Lab")
            .confirmationDialog("Run experimental in-process Linux code?", isPresented: $confirm, titleVisibility: .visible) {
                Button("Run fixed smoke test") { runProbe() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The emulator is not a security sandbox. Only the bundled test rootfs is used. Save open work first; a runtime crash may terminate the app.")
            }
        }
    }

    private func runProbe() {
#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
        guard !attempted else { return }
        attempted = true
        running = true
        Task {
            report = await Task.detached(priority: .userInitiated) { GuestProbeRunner.run() }.value
            running = false
        }
#endif
    }
}

#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
private enum GuestProbeRunner {
    static func run() -> String {
        do {
            let fm = FileManager.default
            guard let bundled = Bundle.main.url(forResource: "fs", withExtension: nil) else {
                return "Missing bundled fakefs; rebuild the guest target."
            }
            let trial = fm.temporaryDirectory.appendingPathComponent("NexusGuest-\(UUID().uuidString)", isDirectory: true)
            try fm.createDirectory(at: trial, withIntermediateDirectories: false)
            let fs = trial.appendingPathComponent("fs", isDirectory: true)
            try fm.copyItem(at: bundled, to: fs)
            let data = fs.appendingPathComponent("data", isDirectory: true)
            guard let result = data.path.withCString({ nexus_guest_probe($0) }) else {
                return "Guest bridge returned no report."
            }
            defer { nexus_guest_free(result) }
            let raw = String(cString: result)
            if let object = try? JSONSerialization.jsonObject(with: Data(raw.utf8)),
               let formatted = try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys]),
               let text = String(data: formatted, encoding: .utf8) {
                return text
            }
            return raw
        } catch { return "Guest preparation failed: \(error)" }
    }
}
#endif
