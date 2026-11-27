import Foundation
import XCTest
@testable import NexusCore

final class AgentEditProposalTests: XCTestCase {
    func testReviewAndConflict() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let w = try Workspace(root: root)
        try await w.write("a.txt", text: "first\nold\nlast\n")
        let snap = try await w.snapshot("a.txt")
        let p = try AgentEditProposal(snapshot: snap, response: #"{"replacement":"first\nnew\nlast\n"}"#, credential: "test_secret")
        XCTAssertEqual(p.diff, ["  first", "- old", "+ new", "  last", "  "])
        XCTAssertEqual(p.snapshot.path, "a.txt")
        let before = try await w.read("a.txt")
        XCTAssertEqual(before, snap.text)
        try await w.apply(p.snapshot, replacement: p.replacement)
        let after = try await w.read("a.txt")
        XCTAssertEqual(after, "first\nnew\nlast\n")
        do { try await w.apply(p.snapshot, replacement: "stale"); XCTFail("Stale edit accepted") }
        catch { XCTAssertEqual(error as? WorkspaceError, .conflict) }
    }

    func testProposalRejectsUntrustedFieldsAndDecodedCredential() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let w = try Workspace(root: root)
        try await w.write("a.txt", text: "old")
        let snap = try await w.snapshot("a.txt")
        for response in [#"{"path":"other","replacement":"new"}"#, #"{"replacement":42}"#,
                         #"{"replacement":"old"}"#, #"{"replacement":"test\u005fsecret"}"#,
                         #"{"replacement":"\u0000"}"#, "not JSON"] {
            XCTAssertThrowsError(try AgentEditProposal(snapshot: snap, response: response, credential: "test_secret"))
        }
        XCTAssertTrue(try AgentEditProposal.prompt(snapshot: snap, question: "Improve").contains("complete revised file"))
    }
}
