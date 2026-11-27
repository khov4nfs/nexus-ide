import Foundation

/// One reviewed server descriptor for ACP v1 session/new. No native launch,
/// network URL, installer or custom environment supplied by Nexus.
public struct ACPStdioServer: Sendable {
    public enum Failure: Error { case invalid }
    public let name: String
    public let argv: ACPTerminalApproval
    public init(name: String, argv: ACPTerminalApproval) throws {
        guard !name.isEmpty, name.utf8.count <= 64, name.utf8.allSatisfy({
            (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [46,95,45].contains($0)
        }) else { throw Failure.invalid }
        self.name = name
        self.argv = argv
    }
    var descriptor: [String: Any] {
        ["name": name, "command": argv.arguments[0], "args": Array(argv.arguments.dropFirst()), "env": [] as [String]]
    }
    public var display: String { name + "\n" + ((try? argv.wire()) ?? "") }
}
