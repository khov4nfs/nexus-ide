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

    static func select(_ url: URL, expected: String) throws -> Self {
        guard AGYImportFormat.validDigest(expected), url.isFileURL else { throw Failure.invalid }
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("NexusAGYImport-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        let file = root.appendingPathComponent("reviewed.elf")
        do {
            guard FileManager.default.createFile(atPath: file.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw Failure.invalid }
            try transfer(from: url, to: file, expected: expected)
            return Self(file: file, digest: expected)
        } catch {
            try? FileManager.default.removeItem(at: root)
            throw error
        }
    }

    func discard() { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }

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
