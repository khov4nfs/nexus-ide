import Foundation

/// Persistent fakefs lifecycle. Callers must use separate stores for local tools, workbench and authenticated agents.
/// A dirty marker prevents silently reopening after a crash or failed shutdown.
/// Caller must own the sole guest boot and serialize access.
public struct LocalGuestStore: Sendable {
    public enum Failure: Error { case invalid, dirty, incompatible }
    public let directory: URL
    private let fm = FileManager.default
    private var state: URL { directory.appendingPathComponent("nexus-state") }
    private var provenance: URL { directory.appendingPathComponent("provenance.txt") }
    public init(directory: URL) { self.directory = directory }

    private func regular(_ url: URL) -> Bool {
        guard let v = try? url.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey]) else { return false }
        return v.isRegularFile == true && v.isSymbolicLink != true
    }
    private func folder(_ url: URL) -> Bool {
        guard let v = try? url.resourceValues(forKeys: [.isDirectoryKey,.isSymbolicLinkKey]) else { return false }
        return v.isDirectory == true && v.isSymbolicLink != true
    }
    private func valid(_ root: URL) -> Bool {
        folder(root) && folder(root.appendingPathComponent("data")) &&
        regular(root.appendingPathComponent("meta.db")) && regular(root.appendingPathComponent("provenance.txt"))
    }
    private func smallRead(_ url: URL) throws -> Data {
        guard regular(url), let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize,
              size <= 8192 else { throw Failure.invalid }
        return try Data(contentsOf: url)
    }
    public func prepare(from bundled: URL) throws -> URL {
        guard valid(bundled) else { throw Failure.invalid }
        let identity = try smallRead(bundled.appendingPathComponent("provenance.txt"))
        if fm.fileExists(atPath: directory.path) {
            guard valid(directory) else { throw Failure.invalid }
            guard try smallRead(provenance) == identity else { throw Failure.incompatible }
            guard try smallRead(state) == Data("clean-v1\n".utf8) else { throw Failure.dirty }
        } else {
            let parent = directory.deletingLastPathComponent()
            try fm.createDirectory(at: parent, withIntermediateDirectories: true)
            let stage = parent.appendingPathComponent("stage-\(UUID().uuidString)")
            defer { try? fm.removeItem(at: stage) }
            try fm.copyItem(at: bundled, to: stage)
            try Data("clean-v1\n".utf8).write(to: stage.appendingPathComponent("nexus-state"), options: .atomic)
            guard valid(stage) else { throw Failure.invalid }
            try fm.moveItem(at: stage, to: directory)
        }
        try Data("dirty-v1\n".utf8).write(to: state, options: .atomic)
        return directory.appendingPathComponent("data", isDirectory: true)
    }

    /// Only call after the runtime reports successful guest shutdown.
    public func markClean() throws {
        guard valid(directory), try smallRead(state) == Data("dirty-v1\n".utf8) else { throw Failure.invalid }
        try Data("clean-v1\n".utf8).write(to: state, options: .atomic)
    }
    /// Explicit destructive reset only while no guest is running.
    public func reset() throws {
        guard fm.fileExists(atPath: directory.path) else { return }
        guard folder(directory) else { throw Failure.invalid }
        try fm.removeItem(at: directory)
    }
}
