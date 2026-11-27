import Foundation
import XCTest
@testable import NexusCore

final class ACPStdioServerTests: XCTestCase {
    func testExplicitSessionDescriptor() throws {
        let argv = try ACPTerminalApproval.parse(Data(#"["/usr/bin/printf","%s","hello"]"#.utf8))
        let server = try ACPStdioServer(name: "local-test", argv: argv)
        let data = try ACPRequest.newSession(id: 2, cwd: "/tmp", mcp: server)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let params = try XCTUnwrap(object["params"] as? [String: Any])
        let servers = try XCTUnwrap(params["mcpServers"] as? [[String: Any]])
        XCTAssertEqual(servers.count, 1)
        XCTAssertEqual(servers[0]["command"] as? String, "/usr/bin/printf")
        XCTAssertEqual(servers[0]["args"] as? [String], ["%s","hello"])
        XCTAssertEqual(servers[0]["env"] as? [String], [])
        XCTAssertThrowsError(try ACPStdioServer(name: "https://example.com", argv: argv))
    }
}
