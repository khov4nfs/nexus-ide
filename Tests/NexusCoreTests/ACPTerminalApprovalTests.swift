import Foundation
import XCTest
@testable import NexusCore

final class ACPTerminalApprovalTests: XCTestCase {
    func testExactArgv() throws {
        let approval = try ACPTerminalApproval.parse(Data(#"["/usr/bin/printf","%s","hello world"]"#.utf8))
        XCTAssertEqual(approval.arguments, ["/usr/bin/printf","%s","hello world"])
        XCTAssertEqual(try ACPTerminalApproval.parse(Data(approval.wire().utf8)).arguments, approval.arguments)
        for invalid in [#"[]"#, #"["printf","x"]"#, #"["/usr/bin/../bin/sh"]"#, #"["/bin//sh"]"#, #"["/bin/sh","\u0000"]"#, #"{"command":"pwd"}"#] {
            XCTAssertThrowsError(try ACPTerminalApproval.parse(Data(invalid.utf8)))
        }
    }
}
