import Foundation

/// Exact user-reviewed argv; never an agent-supplied shell string.
public struct ACPTerminalApproval: Sendable {
    public enum Failure: Error { case invalid }
    public let arguments: [String]

    public static func parse(_ data: Data) throws -> Self {
        guard data.count <= 2048,
              let argv = try JSONSerialization.jsonObject(with: data) as? [String],
              let executable = argv.first, (1...17).contains(argv.count),
              executable.hasPrefix("/"), executable.utf8.count <= 256,
              argv.allSatisfy({ $0.utf8.count <= 512 &&
                  !$0.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) }) else { throw Failure.invalid }
        let parts = executable.split(separator: "/", omittingEmptySubsequences: false).dropFirst()
        guard parts.allSatisfy({ part in
            !part.isEmpty && part != "." && part != ".." && part.utf8.allSatisfy {
                (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [46,95,43,45].contains($0)
            }
        }) else { throw Failure.invalid }
        return Self(arguments: argv)
    }

    public func wire() throws -> String {
        let data = try JSONSerialization.data(withJSONObject: arguments, options: [.withoutEscapingSlashes])
        guard data.count <= 2048 else { throw Failure.invalid }
        return String(decoding: data, as: UTF8.self)
    }
}
