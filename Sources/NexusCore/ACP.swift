import Foundation

public enum ACPError: Error, Equatable { case oversizedFrame, invalidFrame, unexpectedResponse, serverError(Int), unsupportedRequest }

/// Newline-delimited JSON-RPC framing used by ACP stdio. No process launch here.
public struct ACPFramer: Sendable {
    private var pending = Data()
    private let limit: Int
    public var hasPendingBytes: Bool { !pending.isEmpty }
    public init(maxFrameBytes: Int = 1_048_576) { limit = maxFrameBytes }
    public mutating func feed(_ chunk: Data) throws -> [Data] {
        var frames: [Data] = []
        for byte in chunk {
            if byte == 10 {
                if !pending.isEmpty {
                    guard let object = try? JSONSerialization.jsonObject(with: pending), object is [String: Any] else { pending.removeAll(); throw ACPError.invalidFrame }
                    frames.append(pending)
                    pending = Data()
                }
            } else {
                guard pending.count < limit else { pending.removeAll(); throw ACPError.oversizedFrame }
                pending.append(byte)
            }
        }
        return frames
    }
}

/// Typed JSON-RPC request encoder; transport and response dispatch are separate work.
public enum ACPRequest {
    public static func initialize(id: Int, readSnapshot: Bool = false, writeSnapshot: Bool = false, terminal: Bool = false) throws -> Data {
        var capabilities: [String: Any] = (readSnapshot || writeSnapshot) ? ["fs": ["readTextFile": true, "writeTextFile": writeSnapshot]] : [:]
        if terminal { capabilities["terminal"] = true }
        return try encode(id: id, method: "initialize", params: ["protocolVersion": 1, "clientCapabilities": capabilities, "clientInfo": ["name": "nexus-ios", "version": "0.3.2"]])
    }
    public static func newSession(id: Int, cwd: String, mcp: ACPStdioServer? = nil) throws -> Data {
        try encode(id: id, method: "session/new", params: ["cwd": cwd, "mcpServers": mcp.map { [$0.descriptor] } ?? []])
    }
    public static func prompt(id: Int, sessionID: String, text: String) throws -> Data {
        try encode(id: id, method: "session/prompt", params: ["sessionId": sessionID, "prompt": [["type": "text", "text": text]]])
    }
    private static func encode(id: Int, method: String, params: [String: Any]) throws -> Data {
        var data = try JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": id, "method": method, "params": params], options: [.sortedKeys])
        data.append(10)
        return data
    }
}
