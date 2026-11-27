import Foundation
import XCTest
@testable import NexusCore

final class LocalToolProfileTests: XCTestCase {
    func testManualProfileOnly() throws {
        let p = try LocalToolProfile.parse(Data(#"{"version":1,"name":"Local server","mode":"mcpDiscovery","command":"exec /opt/nexus/mcp"}"#.utf8))
        XCTAssertEqual(p.command,"exec /opt/nexus/mcp")
        XCTAssertEqual(p.mode.rawValue,"mcpDiscovery")
        for value in [
            #"{"version":1,"name":"x","mode":"pty","command":"echo x","hook":"onSave"}"#,
            #"{"version":2,"name":"x","mode":"pty","command":"echo x"}"#,
            #"{"version":1,"name":"x","mode":"pty","command":"\u0000"}"#,
            #"{"version":1,"name":"x","mode":"remote","command":"echo x"}"#
        ] { XCTAssertThrowsError(try LocalToolProfile.parse(Data(value.utf8))) }
    }
}
