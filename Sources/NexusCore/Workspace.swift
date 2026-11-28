import Foundation

public enum WorkspaceError: Error, Equatable {
    case invalidPath, symbolicLink, outsideWorkspace, tooLarge, notRegularFile, conflict
}

public struct WorkspaceSnapshot: Sendable {
    public let path: String
    public let text: String
    fileprivate let root: URL
}

/// For trusted native UI operations only. Not a security sandbox for guest code.
public actor Workspace {
    public let root: URL
    private let fm = FileManager.default
    private let maxBytes = 1_048_576

    public init(root: URL) throws {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        self.root = root.resolvingSymlinksInPath().standardizedFileURL
    }

    private func resolve(_ path: String) throws -> URL {
        let parts = path.split(separator: "/", omittingEmptySubsequences: false)
        guard !path.isEmpty, !path.contains("\\"), !path.contains("\0"),
              parts.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." }) else {
            throw WorkspaceError.invalidPath
        }
        var current = root
        for part in parts {
            current.appendPathComponent(String(part))
            if (try? fm.destinationOfSymbolicLink(atPath: current.path)) != nil {
                throw WorkspaceError.symbolicLink
            }
        }
        guard current.standardizedFileURL.path.hasPrefix(root.path + "/") else {
            throw WorkspaceError.outsideWorkspace
        }
        return current
    }

    public func list() throws -> [String] {
        // Enumerate relative names rather than slicing absolute URL paths.
        // Darwin may spell the same temporary root as /var or /private/var.
        guard let e = fm.enumerator(atPath: root.path) else { return [] }
        var result: [String] = []
        for case let relative as String in e {
            let url = root.appendingPathComponent(relative)
            let v = try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .isDirectoryKey])
            if v.isSymbolicLink == true || relative.split(separator: "/").contains(where: { $0.hasPrefix(".") }) {
                e.skipDescendants(); continue
            }
            if v.isRegularFile == true { result.append(relative) }
        }
        return result.sorted()
    }

    public func read(_ path: String) throws -> String {
        let url = try resolve(path)
        let v = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
        guard v.isRegularFile == true else { throw WorkspaceError.notRegularFile }
        guard let size = v.fileSize, size <= maxBytes else { throw WorkspaceError.tooLarge }
        let data = try Data(contentsOf: url)
        guard data.count <= maxBytes else { throw WorkspaceError.tooLarge }
        guard let value = String(data: data, encoding: .utf8) else { throw CocoaError(.fileReadInapplicableStringEncoding) }
        return value
    }

    public func snapshot(_ path: String) throws -> WorkspaceSnapshot {
        WorkspaceSnapshot(path: path, text: try read(path), root: root)
    }

    public func snapshots(_ paths: [String]) throws -> [WorkspaceSnapshot] {
        guard (1...8).contains(paths.count), Set(paths).count == paths.count else { throw WorkspaceError.invalidPath }
        let result = try paths.map { try snapshot($0) }
        guard result.allSatisfy({ $0.text.utf8.count <= 8192 && !$0.text.utf8.contains(0) }), result.reduce(0, { $0 + $1.text.utf8.count }) <= 32768 else { throw WorkspaceError.tooLarge }
        return result
    }

    /// Compare exact UTF-8 bytes and write within one actor turn.
    /// This protects cooperative native edits, not concurrent external writers.
    public func apply(_ snapshot: WorkspaceSnapshot, replacement: String) throws {
        guard snapshot.root == root else { throw WorkspaceError.outsideWorkspace }
        guard Data(try read(snapshot.path).utf8) == Data(snapshot.text.utf8) else {
            throw WorkspaceError.conflict
        }
        try write(snapshot.path, text: replacement)
    }

    public func write(_ path: String, text: String) throws {
        let url = try resolve(path)
        let data = Data(text.utf8)
        guard data.count <= maxBytes else { throw WorkspaceError.tooLarge }
        if fm.fileExists(atPath: url.path) {
            guard try url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true else { throw WorkspaceError.notRegularFile }
        }
        try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url, options: .atomic)
    }
}
