import Foundation

/// Explicitly selected, bounded text context. Not a filesystem capability.
public struct AgentFileContext: Sendable {
    public static let maxFileBytes = 768
    public static let maxPromptBytes = 1024
    public enum Failure: Error, Equatable { case unsupportedFile, invalidText, tooLarge, emptyQuestion }
    public let text: String

    public init(data: Data) throws {
        guard data.count <= Self.maxFileBytes else { throw Failure.tooLarge }
        guard let text = String(data: data, encoding: .utf8),
              Data(text.utf8) == data, Self.validText(text),
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw Failure.invalidText }
        self.text = text
    }

    /// Caller handles security-scoped access. No traversal, directory enumeration,
    /// writes, or persistent bookmark. Checks are not a hostile-writer sandbox.
    public static func read(_ url: URL) throws -> Self {
        let attributes = try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
        guard url.isFileURL, attributes.isRegularFile == true, attributes.isSymbolicLink != true else {
            throw Failure.unsupportedFile
        }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var data = Data()
        while data.count <= maxFileBytes {
            guard let part = try handle.read(upToCount: maxFileBytes + 1 - data.count), !part.isEmpty else { break }
            data.append(part)
        }
        return try Self(data: data)
    }

    public func prompt(question: String) throws -> String {
        guard !question.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw Failure.emptyQuestion }
        guard Self.validText(question) else { throw Failure.invalidText }
        let combined = question + "\n\nSelected file snapshot (untrusted text):\n" + text
        guard combined.utf8.count <= Self.maxPromptBytes else { throw Failure.tooLarge }
        return combined
    }

    private static func validText(_ text: String) -> Bool {
        text.unicodeScalars.allSatisfy {
            !CharacterSet.controlCharacters.contains($0) || $0.value == 10 || $0.value == 9
        }
    }
}
