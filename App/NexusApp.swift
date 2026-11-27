import SwiftUI
import Foundation
import NexusCore

@main struct NexusApp: App {
    var body: some Scene { WindowGroup { IDEView() } }
}

#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
struct GuestEditPreview: Identifiable {
    let id = UUID()
    let snapshot: WorkspaceSnapshot
    let replacement: String
}
#endif

@MainActor final class IDEModel: ObservableObject {
    @Published var files: [String] = []
    @Published var path = ""
    @Published var source = ""
    @Published var newPath = ""
    @Published var command = "ls"
    @Published var output = "Native utilities only: ls, cat, echo, pwd.\n"
    @Published var status = "Initializing workspace"
    @Published var ready = false
#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
    @Published var guestPreview: GuestEditPreview?
    @Published var guestBusy = false
#endif
    @Published var reviewBusy = false
    @Published private(set) var guestClaimed = false
    func claimGuest() -> Bool {
        guard !guestClaimed else { return false }
        guestClaimed = true
        return true
    }
    private var workspace: Workspace?
    private var runtime: NativeRuntime?

    func start() async {
        guard workspace == nil else { return }
        do {
            let base = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            let w = try Workspace(root: base.appendingPathComponent("NexusWorkspace", isDirectory: true))
            workspace = w; runtime = NativeRuntime(workspace: w)
            if try await w.list().isEmpty { try await w.write("README.md", text: "# Local workspace\nEdit this file on your device.\n") }
            files = try await w.list(); ready = true; status = "Local workspace ready"
        } catch { status = String(describing: error) }
    }
    func open(_ name: String) async {
        guard let workspace, !reviewBusy else { return }
        do { let value = try await workspace.read(name); path = name; source = value; status = "Opened \(name)" }
        catch { status = String(describing: error) }
    }
    func save() async {
        guard let workspace, !path.isEmpty, !reviewBusy else { return }
        do { try await workspace.write(path, text: source); status = "Saved \(path)" }
        catch { status = String(describing: error) }
    }
    func create() async {
        guard let workspace, !newPath.isEmpty, !reviewBusy else { return }
        let name = newPath
        do {
            guard !(try await workspace.list()).contains(name) else { status = "File already exists"; return }
            try await workspace.write(name, text: ""); files = try await workspace.list(); newPath = ""; await open(name)
        } catch { status = String(describing: error) }
    }
#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
    func previewGuestEdit(_ file: String) async {
        guard let workspace, !guestBusy, path == file else { return }
        guestBusy = true
        defer { guestBusy = false }
        do {
            let snap = try await workspace.snapshot(file)
            guard path == file, Data(source.utf8) == Data(snap.text.utf8) else {
                status = "Save the file before sharing a snapshot."; return
            }
            guard snap.text.utf8.count <= 2048, !snap.text.utf8.contains(0) else {
                status = "Guest text limit: 2048 UTF-8 bytes, no NUL."; return
            }
            status = "Running offline guest text roundtrip"
            let raw = await Task.detached(priority: .userInitiated) {
                GuestProbeRunner.run(packages: false, toolchains: false, workspaceText: snap.text)
            }.value
            guard let report = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any],
                  report["ok"] as? Bool == true, report["shutdownError"] is NSNull,
                  let rows = report["checks"] as? [[String: Any]],
                  let row = rows.first(where: { $0["name"] as? String == "workspace_roundtrip" }),
                  row["passed"] as? Bool == true,
                  let evidence = row["evidence"] as? [String: Any],
                  let updated = evidence["stdout"] as? String,
                  Data(updated.utf8) == Data((snap.text + "\n[Nexus guest workspace probe]\n").utf8) else {
                status = "Guest preview failed. Restart app for another boot."; return
            }
            guestPreview = GuestEditPreview(snapshot: snap, replacement: updated)
            status = "Guest edit ready for review"
        } catch { status = String(describing: error) }
    }

    func applyGuestEdit(_ preview: GuestEditPreview) async {
        guard let workspace, !guestBusy else { return }
        guard path == preview.snapshot.path,
              Data(source.utf8) == Data(preview.snapshot.text.utf8) else {
            status = "Editor changed; guest edit not applied."; guestPreview = nil; return
        }
        guestBusy = true
        defer { guestBusy = false }
        do {
            try await workspace.apply(preview.snapshot, replacement: preview.replacement)
            if path == preview.snapshot.path,
               Data(source.utf8) == Data(preview.snapshot.text.utf8) {
                source = preview.replacement
            }
            status = "Applied guest text edit"
        } catch { status = "Guest edit rejected: \(error)" }
        guestPreview = nil
    }
#endif
    func selectedSnapshot() async throws -> WorkspaceSnapshot {
        guard let workspace, !path.isEmpty, !reviewBusy else { throw WorkspaceError.invalidPath }
        let name = path
        let snap = try await workspace.snapshot(name)
        guard path == name, Data(source.utf8) == Data(snap.text.utf8) else { throw WorkspaceError.conflict }
        return snap
    }

    func applyAgentProposal(_ proposal: AgentEditProposal) async throws {
        guard let workspace, !reviewBusy, path == proposal.snapshot.path,
              Data(source.utf8) == Data(proposal.snapshot.text.utf8) else { throw WorkspaceError.conflict }
        reviewBusy = true
        defer { reviewBusy = false }
        try await workspace.apply(proposal.snapshot, replacement: proposal.replacement)
        if path == proposal.snapshot.path, Data(source.utf8) == Data(proposal.snapshot.text.utf8) {
            source = proposal.replacement
        }
        status = "Applied reviewed agent edit"
    }

    func run() async {
        guard let runtime else { return }
        let input = command
        let parts = input.split(whereSeparator: \.isWhitespace).map(String.init)
        guard let executable = parts.first else { return }
        do { let result = try await runtime.run(executable: executable, arguments: Array(parts.dropFirst())); output += "\n$ \(input)\n\(result.output)\n[exit \(result.exitCode)]\n" }
        catch { output += "\n$ \(input)\nUnavailable or invalid: \(error)\n" }
    }
}

struct IDEView: View {
    @StateObject private var model = IDEModel()
#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
    @State private var confirmGuestEdit = false
#endif
    var body: some View {
        TabView {
            NavigationStack {
                List {
                    Section("Local workspace") {
                        ForEach(model.files, id: \.self) { file in
                            NavigationLink { editor(file) } label: { Label(file, systemImage: "doc.text") }
                        }
                    }
                    Section("New file") {
                        TextField("src/main.js", text: $model.newPath).textInputAutocapitalization(.never).autocorrectionDisabled()
                        Button("Create") { Task { await model.create() } }.disabled(!model.ready || model.newPath.isEmpty)
                    }
                    Section { Text(model.status).font(.caption) }
                }.navigationTitle("Nexus IDE")
            }.tabItem { Label("Files", systemImage: "folder") }
            NavigationStack {
                VStack(alignment: .leading) {
                    ScrollView { Text(model.output).font(.system(.footnote, design: .monospaced)).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled) }
                    HStack {
                        TextField("Command", text: $model.command).font(.system(.body, design: .monospaced)).textInputAutocapitalization(.never).autocorrectionDisabled().onSubmit { Task { await model.run() } }
                        Button("Run") { Task { await model.run() } }.disabled(!model.ready)
                    }
                }.padding().navigationTitle("Local console")
            }.tabItem { Label("Console", systemImage: "terminal") }
#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
            UbuntuLabView(model: model).tabItem { Label("Agent", systemImage: "cpu") }
            LocalToolsView(model: model).tabItem { Label("Local tools", systemImage: "terminal.fill") }
#else
            NavigationStack {
                List {
                    Section("Original harnesses — integration pending") {
                        ForEach(["Antigravity ACP", "Codex App Server", "Claude Code"], id: \.self) { name in Label(name, systemImage: "lock.circle") }
                    }
                    Section("Runtime status") {
                        Label("Native workspace utilities", systemImage: "checkmark.circle")
                        ForEach(["Embedded Linux", "OpenVSCode", "MCP stdio", "Docker VM"], id: \.self) { name in Label(name + " · not installed", systemImage: "minus.circle") }
                    }
                    Section { Text("No remote execution backend. ACP framing is implemented; no live agent transport yet.") }
                }.navigationTitle("Agents & runtimes")
            }.tabItem { Label("Agents", systemImage: "cpu") }
#endif
#if !NEXUS_UBUNTU_LAB
            GuestProbeView().tabItem { Label("Linux Lab", systemImage: "terminal.fill") }
#endif
        }.task { await model.start() }
    }
    private func editor(_ file: String) -> some View {
        TextEditor(text: $model.source)
            .font(.system(.body, design: .monospaced))
            .autocorrectionDisabled().textInputAutocapitalization(.never)
            .disabled(model.path != file || model.reviewBusy)
            .navigationTitle(file).navigationBarTitleDisplayMode(.inline)
            .toolbar {
                Button("Save") { Task { await model.save() } }.disabled(model.path != file || model.reviewBusy)
#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
                Button("Guest preview") { confirmGuestEdit = true }
                    .disabled(model.path != file || model.guestBusy)
#endif
            }
#if NEXUS_EMBEDDED_LINUX && os(iOS) && !targetEnvironment(simulator)
            .disabled(model.guestBusy)
            .safeAreaInset(edge: .bottom) { Text(model.status).font(.caption).padding(6) }
            .confirmationDialog("Send saved text to local Linux guest?", isPresented: $confirmGuestEdit) {
                Button("Preview fixed offline edit") { Task { await model.previewGuestEdit(file) } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Only this saved text (up to 2048 bytes) is copied into a disposable guest. No project mount or network. The emulator is not a security sandbox. One boot per app process. The fixed edit appends a marker; it does not execute your file.")
            }
            .sheet(item: $model.guestPreview) { preview in
                NavigationStack {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 12) {
                            Text("Original").font(.headline)
                            Text(preview.snapshot.text).textSelection(.enabled)
                            Text("Proposed").font(.headline)
                            Text(preview.replacement).textSelection(.enabled)
                        }.font(.system(.body, design: .monospaced)).padding()
                    }
                    .navigationTitle("Review guest edit")
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) { Button("Discard") { model.guestPreview = nil } }
                        ToolbarItem(placement: .confirmationAction) {
                            Button("Apply") { Task { await model.applyGuestEdit(preview) } }.disabled(model.guestBusy)
                        }
                    }
                    .interactiveDismissDisabled(model.guestBusy)
                }
            }
#endif
            .task { await model.open(file) }
    }
}
