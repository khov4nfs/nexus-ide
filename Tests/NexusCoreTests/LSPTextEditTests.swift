import XCTest
@testable import NexusCore

final class LSPTextEditTests: XCTestCase {
    func testUTF16AndCRLF() throws {
        let source = "a😀b\r\nnext"
        XCTAssertNil(LSPTextEdit.offset(.init(line: 0, character: 2), in: source))
        XCTAssertEqual(LSPTextEdit.offset(.init(line: 1, character: 0), in: source), 6)
        XCTAssertNil(LSPTextEdit.offset(.init(line: 0, character: 5), in: source))
        let edit = LSPTextEdit(label: "replace", text: "X", range: .init(start: .init(line: 0, character: 1), end: .init(line: 0, character: 3)))
        XCTAssertEqual(try edit.replacement(in: source), "aXb\r\nnext")
        let bad = LSPTextEdit(label: "bad", text: "X", range: .init(start: .init(line: 0, character: 2), end: .init(line: 0, character: 3)))
        XCTAssertThrowsError(try bad.replacement(in: source))
    }
}
