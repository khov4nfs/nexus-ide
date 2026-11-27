import Foundation

/// A proposal, never a write capability. The destination is selected by the
/// native user, not read from agent output.
public struct AgentEditProposal: Identifiable, Sendable {
    public enum Failure: Error, Equatable { case invalidResponse, invalidText, tooLarge, unchanged, credential }
    public let id = UUID()
    public let snapshot: WorkspaceSnapshot
    public let replacement: String

    public init(snapshot: WorkspaceSnapshot, response: String, credential: String) throws {
        guard snapshot.text.utf8.count <= AgentFileContext.maxFileBytes,
              response.utf8.count <= 8192 else { throw Failure.tooLarge }
        guard let object = try? JSONSerialization.jsonObject(with: Data(response.utf8)) as? [String: Any],
              Set(object.keys) == ["replacement"], let text = object["replacement"] as? String else {
            throw Failure.invalidResponse
        }
        try self.init(snapshot: snapshot, stagedReplacement: text, credential: credential)
    }

    /// Already correlated virtual-file result, not JSON embedded in model text.
    public init(snapshot: WorkspaceSnapshot, stagedReplacement: String?, credential: String) throws {
        guard snapshot.text.utf8.count <= AgentFileContext.maxFileBytes else { throw Failure.tooLarge }
        guard let text = stagedReplacement else { throw Failure.invalidResponse }
        guard text.utf8.count <= 4096 else { throw Failure.tooLarge }
        guard text.unicodeScalars.allSatisfy({
            !CharacterSet.controlCharacters.contains($0) || $0.value == 10 || $0.value == 9
        }) else { throw Failure.invalidText }
        guard !credential.isEmpty, !text.contains(credential) else { throw Failure.credential }
        guard Data(text.utf8) != Data(snapshot.text.utf8) else { throw Failure.unchanged }
        self.snapshot = snapshot
        self.replacement = text
    }

    /// Context around a contiguous change; not a minimal diff. Exact full
    /// before/after text remains available for review, including final newline.
    public var diff: [String] {
        let old = snapshot.text.components(separatedBy: "\n")
        let new = replacement.components(separatedBy: "\n")
        var prefix = 0
        while prefix < min(old.count, new.count), Data(old[prefix].utf8) == Data(new[prefix].utf8) { prefix += 1 }
        var suffix = 0
        while suffix < min(old.count, new.count) - prefix,
              Data(old[old.count - 1 - suffix].utf8) == Data(new[new.count - 1 - suffix].utf8) { suffix += 1 }
        return old[..<prefix].map { "  " + $0 } +
            old[prefix..<(old.count - suffix)].map { "- " + $0 } +
            new[prefix..<(new.count - suffix)].map { "+ " + $0 } +
            old[(old.count - suffix)...].map { "  " + $0 }
    }

    public static func prompt(snapshot: WorkspaceSnapshot, question: String) throws -> String {
        guard !question.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw AgentFileContext.Failure.emptyQuestion }
        let context = try AgentFileContext(data: Data(snapshot.text.utf8))
        return try context.prompt(question: question + "\nReturn only JSON with one string field replacement containing the complete revised file. No markdown or tool calls.")
    }
}
