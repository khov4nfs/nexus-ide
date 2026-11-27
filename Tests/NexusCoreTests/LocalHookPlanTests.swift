import Foundation
import XCTest
@testable import NexusCore

final class LocalHookPlanTests: XCTestCase {
    func testExplicitHooks() throws {
        let plan = try LocalHookPlan(command: "echo main", before: "echo pre", after: "")
        let obj = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(plan.wire().utf8)) as? [String: Any])
        XCTAssertEqual(obj["before"] as? String, "echo pre")
        XCTAssertTrue(obj["after"] is NSNull)
        XCTAssertThrowsError(try LocalHookPlan(command: "echo main", before: "", after: ""))
        XCTAssertThrowsError(try LocalHookPlan(command: "echo main", before: " ", after: ""))
        XCTAssertThrowsError(try LocalHookPlan(command: "echo main", before: "echo\u{0}x", after: ""))
    }
}
