import SwiftUI
import NexusCore

@main struct NexusApp: App {
    var body: some Scene { WindowGroup { IDEView() } }
}

@MainActor final class IDEModel: ObservableObject {
    @Published var files: [String] = []
    @Published var path = ""
    @Published var source = ""
    @Published var newPath = ""
    @Published var command = "ls"
    @Published var output = "Native utilities only: ls, cat, echo, pwd.\n"
    @Published var status = "Initializing workspace"
    @Published var ready = false
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
        guard let workspace else { return }
        do { let value = try await workspace.read(name); path = name; source = value; status = "Opened \(name)" }
        catch { status = String(describing: error) }
    }
    func save() async {
        guard let workspace, !path.isEmpty else { return }
        do { try await workspace.write(path, text: source); status = "Saved \(path)" }
        catch { status = String(describing: error) }
    }
    func create() async {
        guard let workspace, !newPath.isEmpty else { return }
        let name = newPath
        do {
            guard !(try await workspace.list()).contains(name) else { status = "File already exists"; return }
            try await workspace.write(name, text: ""); files = try await workspace.list(); newPath = ""; await open(name)
        } catch { status = String(describing: error) }
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
            GuestProbeView().tabItem { Label("Linux Lab", systemImage: "terminal.fill") }
        }.task { await model.start() }
    }
    private func editor(_ file: String) -> some View {
        TextEditor(text: $model.source)
            .font(.system(.body, design: .monospaced))
            .autocorrectionDisabled().textInputAutocapitalization(.never)
            .disabled(model.path != file)
            .navigationTitle(file).navigationBarTitleDisplayMode(.inline)
            .toolbar { Button("Save") { Task { await model.save() } }.disabled(model.path != file) }
            .task { await model.open(file) }
    }
}
