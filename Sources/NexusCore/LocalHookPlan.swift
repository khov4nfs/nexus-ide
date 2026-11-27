import Foundation

/// Explicitly confirmed local PTY lifecycle, not a workspace-provided autorun.
public struct LocalHookPlan: Sendable {
    public enum Failure: Error { case invalid }
    public let command: String
    public let before: String?
    public let after: String?

    public init(command: String, before: String, after: String) throws {
        func valid(_ value: String) -> Bool {
            !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
            value.utf8.count <= 2048 && !value.utf8.contains(0)
        }
        guard valid(command), before.isEmpty || valid(before), after.isEmpty || valid(after),
              !before.isEmpty || !after.isEmpty else { throw Failure.invalid }
        self.command = command
        self.before = before.isEmpty ? nil : before
        self.after = after.isEmpty ? nil : after
    }

    public func wire() throws -> String {
        let object: [String: Any] = ["command": command, "before": before.map { $0 as Any } ?? NSNull(), "after": after.map { $0 as Any } ?? NSNull()]
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes])
        guard data.count <= 8192 else { throw Failure.invalid }
        return String(decoding: data, as: UTF8.self)
    }
}
