import Foundation
import XCTest
@testable import NexusCore

final class AgentFileContextTests: XCTestCase {
    func testExactSnapshotAndPrompt() throws {
        let context = try AgentFileContext(data: Data("let value = 42\n".utf8))
        XCTAssertEqual(try context.prompt(question: "Cảm ơn"),
                       "Cảm ơn\n\nSelected file snapshot (untrusted text):\nlet value = 42\n")
        XCTAssertThrowsError(try context.prompt(question: " \n"))
        XCTAssertThrowsError(try context.prompt(question: String(repeating: "é", count: 512)))
        XCTAssertThrowsError(try context.prompt(question: "x\0y"))
    }

    func testReadBoundsAndTypes() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("selected.txt")
        try Data(repeating: 65, count: 768).write(to: file)
        let snapshot = try AgentFileContext.read(file)
        XCTAssertEqual(snapshot.text.utf8.count, 768)
        try Data(repeating: 66, count: 769).write(to: file)
        XCTAssertEqual(snapshot.text, String(repeating: "A", count: 768))
        XCTAssertThrowsError(try AgentFileContext.read(file)) { XCTAssertEqual($0 as? AgentFileContext.Failure, .tooLarge) }
        XCTAssertThrowsError(try AgentFileContext.read(root))
        let link = root.appendingPathComponent("link.txt")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: file)
        XCTAssertThrowsError(try AgentFileContext.read(link))
        for invalid in [Data(), Data([0xff]), Data([0]), Data(" \n".utf8)] {
            XCTAssertThrowsError(try AgentFileContext(data: invalid))
        }
    }
}
