import Foundation

public enum Capability: String, CaseIterable, Sendable {
    case files, nativeCommands, linuxProcesses, pty, acp, mcpStdio, extensionHost, docker
}
public struct CommandResult: Equatable, Sendable {
    public let output: String
    public let exitCode: Int
    public init(output: String, exitCode: Int) { self.output = output; self.exitCode = exitCode }
}
public enum RuntimeError: Error, Equatable { case unavailable(Capability), invalidArguments }

/// No network or remote execution fallback. Guest implementation must be on-device.
public protocol LocalRuntime: Sendable {
    var capabilities: Set<Capability> { get }
    func run(executable: String, arguments: [String]) async throws -> CommandResult
}

/// Explicit allowlist of native file utilities; not a POSIX shell.
public struct NativeRuntime: LocalRuntime {
    public let capabilities: Set<Capability> = [.files, .nativeCommands]
    private let workspace: Workspace
    public init(workspace: Workspace) { self.workspace = workspace }
    public func run(executable: String, arguments: [String]) async throws -> CommandResult {
        switch executable {
        case "ls":
            guard arguments.isEmpty else { throw RuntimeError.invalidArguments }
            return .init(output: try await workspace.list().joined(separator: "\n"), exitCode: 0)
        case "cat":
            guard arguments.count == 1 else { throw RuntimeError.invalidArguments }
            return .init(output: try await workspace.read(arguments[0]), exitCode: 0)
        case "echo": return .init(output: arguments.joined(separator: " "), exitCode: 0)
        case "pwd": return .init(output: "/workspace", exitCode: 0)
        default: throw RuntimeError.unavailable(.linuxProcesses)
        }
    }
}
