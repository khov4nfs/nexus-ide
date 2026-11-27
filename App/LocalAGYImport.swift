#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import Foundation
import CryptoKit
import NexusCore

/// Local-only reviewed binary. No auto-download, execution on import, or
/// persistence of the source URL. User must supply the expected digest.
struct LocalAGYImport: Sendable {
    enum Failure: Error { case invalid }
    let file: URL
    let digest: String

    static func available(in root: URL) -> Bool {
        let slot = root.appendingPathComponent("data/opt/nexus/agy")
        let values = try? slot.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey])
        return values?.isRegularFile == true && values?.isSymbolicLink != true &&
            values?.fileSize == AGYImportFormat.slot.count &&
            (try? Data(contentsOf: slot)) == AGYImportFormat.slot
    }

    private static func storage() throws -> URL {
        let fm = FileManager.default
        let support = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        var root = support.appendingPathComponent("NexusReviewedAGY", isDirectory: true)
        try fm.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try root.setResourceValues(values)
        return root
    }
    private static func savedDirectory() throws -> URL { try storage().appendingPathComponent("current", isDirectory: true) }

    static func loadSaved() throws -> Self? {
        let root = try savedDirectory()
        guard FileManager.default.fileExists(atPath: root.path) else { return nil }
        let attributes = try root.resourceValues(forKeys: [.isDirectoryKey,.isSymbolicLinkKey])
        guard attributes.isDirectory == true, attributes.isSymbolicLink != true else { throw Failure.invalid }
        let manifest = root.appendingPathComponent("sha256")
        let mv = try manifest.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey])
        guard mv.isRegularFile == true, mv.isSymbolicLink != true, mv.fileSize == 65 else { throw Failure.invalid }
        let digest = try String(contentsOf: manifest, encoding: .utf8).trimmingCharacters(in: .newlines)
        guard AGYImportFormat.validDigest(digest) else { throw Failure.invalid }
        let file = root.appendingPathComponent("reviewed.elf")
        let fv = try file.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey])
        guard fv.isRegularFile == true, fv.isSymbolicLink != true, let size = fv.fileSize,
              size >= 64, UInt64(size) <= AGYImportFormat.maxBytes else { throw Failure.invalid }
        // Full digest is verified again when installed in a fresh guest.
        return Self(file: file, digest: digest)
    }

    static func removeSaved() throws {
        let root = try savedDirectory()
        if FileManager.default.fileExists(atPath: root.path) {
            let v = try root.resourceValues(forKeys: [.isDirectoryKey,.isSymbolicLinkKey])
            guard v.isDirectory == true, v.isSymbolicLink != true else { throw Failure.invalid }
            try FileManager.default.removeItem(at: root)
        }
    }

    static func select(_ url: URL, expected: String) throws -> Self {
        guard AGYImportFormat.validDigest(expected), url.isFileURL else { throw Failure.invalid }
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        let destination = try savedDirectory()
        guard !FileManager.default.fileExists(atPath: destination.path) else { throw Failure.invalid }
        let root = try storage().appendingPathComponent("stage-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        let file = root.appendingPathComponent("reviewed.elf")
        do {
            guard FileManager.default.createFile(atPath: file.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw Failure.invalid }
            try transfer(from: url, to: file, expected: expected)
            try Data((expected + "\n").utf8).write(to: root.appendingPathComponent("sha256"), options: .atomic)
            try FileManager.default.setAttributes([.protectionKey: FileProtectionType.complete], ofItemAtPath: file.path)
            try FileManager.default.moveItem(at: root, to: destination)
            return Self(file: destination.appendingPathComponent("reviewed.elf"), digest: expected)
        } catch {
            try? FileManager.default.removeItem(at: root)
            throw error
        }
    }



    func install(in trial: URL) throws {
        guard Self.available(in: trial) else { throw Failure.invalid }
        let target = trial.appendingPathComponent("data/opt/nexus/agy")
        // The stopped fakefs already has guest executable metadata. Preserve
        // the backing file, replace only its contents before boot.
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: target.path)
        try Self.transfer(from: file, to: target, expected: digest)
    }

    private static func transfer(from source: URL, to target: URL, expected: String) throws {
        let values = try source.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey])
        guard values.isRegularFile == true, values.isSymbolicLink != true,
              let length = values.fileSize, length >= 64, UInt64(length) <= AGYImportFormat.maxBytes else { throw Failure.invalid }
        let reader = try FileHandle(forReadingFrom: source)
        defer { try? reader.close() }
        guard let header = try reader.read(upToCount: 64), AGYImportFormat.validHeader(header) else { throw Failure.invalid }
        try reader.seek(toOffset: 0)
        let writer = try FileHandle(forWritingTo: target)
        defer { try? writer.close() }
        try writer.truncate(atOffset: 0)
        var sha = SHA256()
        var count: UInt64 = 0
        while let chunk = try reader.read(upToCount: 1024 * 1024), !chunk.isEmpty {
            count += UInt64(chunk.count)
            guard count <= AGYImportFormat.maxBytes else { throw Failure.invalid }
            sha.update(data: chunk)
            try writer.write(contentsOf: chunk)
        }
        let digest = sha.finalize().map { String(format: "%02x", $0) }.joined()
        guard count == UInt64(length), digest == expected else { throw Failure.invalid }
        try writer.synchronize()
    }
}
#endif
