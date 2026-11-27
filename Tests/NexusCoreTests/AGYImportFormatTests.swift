import Foundation
import XCTest
@testable import NexusCore

final class AGYImportFormatTests: XCTestCase {
    func testArm64ELFAndDigest() {
        var bytes = [UInt8](repeating: 0, count: 64)
        bytes.replaceSubrange(0..<7, with: [0x7f,0x45,0x4c,0x46,2,1,1])
        bytes[16] = 3; bytes[18] = 183; bytes[20] = 1
        XCTAssertTrue(AGYImportFormat.validHeader(Data(bytes)))
        bytes[18] = 62
        XCTAssertFalse(AGYImportFormat.validHeader(Data(bytes)))
        XCTAssertFalse(AGYImportFormat.validHeader(AGYImportFormat.slot))
        XCTAssertTrue(AGYImportFormat.validDigest(String(repeating: "a",count:64)))
        XCTAssertFalse(AGYImportFormat.validDigest(String(repeating: "A",count:64)))
    }
}
