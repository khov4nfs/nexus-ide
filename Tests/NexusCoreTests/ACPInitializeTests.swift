import Foundation
import XCTest
@testable import NexusCore

final class ACPInitializeTests: XCTestCase {
    func testNativeRequestAndChunkedResponse() throws {
        var client = ACPInitializeClient()
        let data = try client.request()
        let obj = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(obj["method"] as? String, "initialize")
        XCTAssertEqual(obj["id"] as? Int, 1)
        XCTAssertEqual(data.last, 10)
        XCTAssertThrowsError(try client.request())
        let reply = Data(#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"Antigravity","version":"fixture"}}}"#.utf8) + Data([10])
        try client.receive(Data(reply.prefix(7)))
        XCTAssertEqual(client.state, .waiting)
        try client.receive(Data(reply.dropFirst(7)))
        try client.finish()
        XCTAssertEqual(client.state, .initialized(name: "Antigravity", version: "fixture"))
    }

    func testAuthenticationIsNotSuccess() throws {
        var client = ACPInitializeClient()
        _ = try client.request()
        try client.receive(Data(#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}"#.utf8) + Data([10]))
        try client.finish()
        XCTAssertEqual(client.state, .authenticationRequired)
    }

    func testInvalidResponsesAndIncompleteFrame() throws {
        for response in [
            #"{"jsonrpc":"2.0","id":2,"result":{"protocolVersion":1,"agentInfo":{"name":"Antigravity"}}}"#,
            #"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":2,"agentInfo":{"name":"Antigravity"}}}"#,
            #"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"fixture"}}}"#,
            #"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Something else"}}"#,
            #"{"jsonrpc":"2.0","id":1,"method":"session/request_permission"}"#
        ] {
            var client = ACPInitializeClient()
            _ = try client.request()
            XCTAssertThrowsError(try client.receive(Data(response.utf8) + Data([10])))
            XCTAssertEqual(client.state, .failed)
        }
        var truncated = ACPInitializeClient()
        _ = try truncated.request()
        try truncated.receive(Data("{".utf8))
        XCTAssertThrowsError(try truncated.finish())
        XCTAssertEqual(truncated.state, .failed)
    }
}
