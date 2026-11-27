import Foundation

/// Bounded initialize-only client. No session, authentication or tool grants.
public struct ACPInitializeClient: Sendable {
    public enum State: Equatable, Sendable {
        case idle, waiting
        case initialized(name: String, version: String)
        case authenticationRequired
        case failed
    }

    public private(set) var state: State = .idle
    private var framer = ACPFramer(maxFrameBytes: 16_384)

    public init() {}

    public mutating func request(readSnapshot: Bool = false, writeSnapshot: Bool = false) throws -> Data {
        guard state == .idle else { throw ACPError.unexpectedResponse }
        let data = try ACPRequest.initialize(id: 1, readSnapshot: readSnapshot, writeSnapshot: writeSnapshot)
        state = .waiting
        return data
    }

    private struct Response: Decodable {
        struct Info: Decodable { let name: String; let version: String? }
        struct Result: Decodable { let protocolVersion: Int; let agentInfo: Info }
        struct Failure: Decodable { let code: Int; let message: String }
        let jsonrpc: String
        let id: Int
        let result: Result?
        let error: Failure?
    }

    /// Only a projected response from the trusted bridge is accepted here.
    /// The bridge excludes raw notifications and potentially sensitive error data.
    public mutating func receive(_ chunk: Data) throws {
        guard state == .waiting else { throw ACPError.unexpectedResponse }
        do {
            let frames = try framer.feed(chunk)
            guard frames.count <= 1 else { throw ACPError.unexpectedResponse }
            guard let frame = frames.first else { return }
            guard !framer.hasPendingBytes else { throw ACPError.unexpectedResponse }
            let response = try JSONDecoder().decode(Response.self, from: frame)
            guard response.jsonrpc == "2.0", response.id == 1,
                  (response.result == nil) != (response.error == nil) else {
                throw ACPError.unexpectedResponse
            }
            if let error = response.error {
                if error.code == -32000 && error.message == "Authentication required" {
                    state = .authenticationRequired
                    return
                }
                throw ACPError.serverError(error.code)
            }
            guard let result = response.result, result.protocolVersion == 1,
                  result.agentInfo.name.lowercased().contains("antigravity"),
                  !result.agentInfo.name.isEmpty,
                  result.agentInfo.name.utf8.count <= 256,
                  (result.agentInfo.version?.utf8.count ?? 0) <= 256 else {
                throw ACPError.unexpectedResponse
            }
            state = .initialized(name: result.agentInfo.name, version: result.agentInfo.version ?? "unknown")
        } catch {
            state = .failed
            throw error
        }
    }

    public mutating func finish() throws {
        switch state {
        case .initialized, .authenticationRequired: return
        default:
            state = .failed
            throw ACPError.unexpectedResponse
        }
    }
}
