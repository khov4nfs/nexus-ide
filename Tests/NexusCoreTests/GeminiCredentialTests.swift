import XCTest
@testable import NexusCore

final class GeminiCredentialTests: XCTestCase {
    func testShape() {
        XCTAssertTrue(GeminiCredential.isValid("NEXUS_TEST_ONLY_NOT_A_REAL_KEY"))
        for key in ["", "short", "not a valid key with spaces", String(repeating: "x", count: 257),
                    String(repeating: "x", count: 20) + "\0", String(repeating: "é", count: 20)] {
            XCTAssertFalse(GeminiCredential.isValid(key))
        }
    }
}
