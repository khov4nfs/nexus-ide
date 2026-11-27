import Foundation

/// Local, explicit manual invocation only. No auto-install, hooks, secret
/// expansion, network fetch, host mounts or agent-controlled invocation.
public struct LocalToolProfile: Sendable, Decodable {
    public enum Mode: String, Sendable, Decodable { case pty, mcpDiscovery }
    public enum Failure: Error { case invalid }
    public let name: String
    public let mode: Mode
    public let command: String
    private let version: Int

    public static func parse(_ data: Data) throws -> Self {
        guard data.count <= 4096,
              let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              Set(obj.keys) == ["version","name","mode","command"] else { throw Failure.invalid }
        let value = try JSONDecoder().decode(Self.self, from: data)
        guard value.version == 1, !value.name.isEmpty, value.name.utf8.count <= 80,
              !value.name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              !value.command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              value.command.utf8.count <= 2048, !value.command.utf8.contains(0) else { throw Failure.invalid }
        return value
    }

    public static func read(_ url: URL) throws -> Self {
        let attributes = try url.resourceValues(forKeys: [.isRegularFileKey,.isSymbolicLinkKey])
        guard url.isFileURL, attributes.isRegularFile == true, attributes.isSymbolicLink != true else { throw Failure.invalid }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var bytes = Data()
        while bytes.count <= 4096 {
            guard let part = try handle.read(upToCount: 4097 - bytes.count), !part.isEmpty else { break }
            bytes.append(part)
        }
        return try parse(bytes)
    }
}
