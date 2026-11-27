import XCTest
@testable import NexusCore

final class CoreTests: XCTestCase {
    func testWorkspaceRoundTrip() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let w = try Workspace(root: root)
        try await w.write("src/index.js", text: "console.log('hello')")
        let value = try await w.read("src/index.js")
        let files = try await w.list()
        XCTAssertEqual(value, "console.log('hello')")
        XCTAssertEqual(files, ["src/index.js"])
    }
    func testTraversalAndSymlink() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let w = try Workspace(root: root)
        for p in ["../escape", "/absolute", "foo/../bar", "a//b"] {
            do { try await w.write(p, text: "bad"); XCTFail(p) } catch { XCTAssertEqual(error as? WorkspaceError, .invalidPath) }
        }
        try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("link"), withDestinationURL: root.deletingLastPathComponent())
        do { _ = try await w.read("link/file"); XCTFail() } catch { XCTAssertEqual(error as? WorkspaceError, .symbolicLink) }
    }
    func testRuntimeFailsClosed() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let w = try Workspace(root: root)
        try await w.write("a.txt", text: "hi")
        let runtime = NativeRuntime(workspace: w)
        let result = try await runtime.run(executable: "cat", arguments: ["a.txt"])
        XCTAssertEqual(result, .init(output: "hi", exitCode: 0))
        XCTAssertFalse(runtime.capabilities.contains(.docker))
        do { _ = try await runtime.run(executable: "agy", arguments: ["acp"]); XCTFail() } catch { XCTAssertEqual(error as? RuntimeError, .unavailable(.linuxProcesses)) }
    }
    func testACPChunkedFrames() throws {
        let request = try ACPRequest.initialize(id: 1)
        var f = ACPFramer()
        XCTAssertEqual(try f.feed(Data(request.prefix(7))), [])
        let frames = try f.feed(Data(request.dropFirst(7)) + request)
        XCTAssertEqual(frames.count, 2)
        let obj = try XCTUnwrap(JSONSerialization.jsonObject(with: frames[0]) as? [String: Any])
        XCTAssertEqual(obj["method"] as? String, "initialize")
    }
    func testACPRejectsInvalidAndOversized() throws {
        var f = ACPFramer(maxFrameBytes: 8)
        XCTAssertThrowsError(try f.feed(Data("x\n".utf8))) { XCTAssertEqual($0 as? ACPError, .invalidFrame) }
        XCTAssertThrowsError(try f.feed(Data("123456789".utf8))) { XCTAssertEqual($0 as? ACPError, .oversizedFrame) }
    }
    func testACPSessionPrompt() throws {
        let request = try ACPRequest.prompt(id: 3, sessionID: "test", text: "hello")
        let obj = try XCTUnwrap(JSONSerialization.jsonObject(with: request) as? [String: Any])
        XCTAssertEqual(obj["method"] as? String, "session/prompt")
        let params = try XCTUnwrap(obj["params"] as? [String: Any])
        XCTAssertEqual(params["sessionId"] as? String, "test")
    }
}

extension CoreTests {
    func testWorkspaceListThroughRootAlias() async throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let actual = base.appendingPathComponent("actual")
        let alias = base.appendingPathComponent("alias")
        try FileManager.default.createDirectory(at: actual, withIntermediateDirectories: true)
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: actual)
        let w = try Workspace(root: alias)
        try await w.write("src/main.js", text: "hello")
        try await w.write(".hidden/private.txt", text: "hidden")
        try FileManager.default.createSymbolicLink(at: actual.appendingPathComponent("linked"), withDestinationURL: actual.appendingPathComponent("src"))
        let listed = try await w.list()
        XCTAssertEqual(listed, ["src/main.js"])
    }
}

extension CoreTests {
    func testWorkspaceSnapshotApplyAndConflict() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let w = try Workspace(root: root)
        try await w.write("note.txt", text: "Tiếng Việt\n")
        let snap = try await w.snapshot("note.txt")
        try await w.apply(snap, replacement: "guest edit\n")
        let applied = try await w.read("note.txt")
        XCTAssertEqual(applied, "guest edit\n")
        do {
            try await w.apply(snap, replacement: "stale")
            XCTFail("stale snapshot accepted")
        } catch { XCTAssertEqual(error as? WorkspaceError, .conflict) }
        let retained = try await w.read("note.txt")
        XCTAssertEqual(retained, "guest edit\n")
    }

    func testWorkspaceSnapshotRejectsDifferentRootAndByteConflict() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let a = try Workspace(root: root.appendingPathComponent("a"))
        let b = try Workspace(root: root.appendingPathComponent("b"))
        try await a.write("note.txt", text: "\u{00e9}")
        try await b.write("note.txt", text: "\u{00e9}")
        let snap = try await a.snapshot("note.txt")
        do {
            try await b.apply(snap, replacement: "wrong root")
            XCTFail("cross-root snapshot accepted")
        } catch { XCTAssertEqual(error as? WorkspaceError, .outsideWorkspace) }
        try await a.write("note.txt", text: "e\u{0301}")
        do {
            try await a.apply(snap, replacement: "wrong bytes")
            XCTFail("canonically equivalent but byte-different text accepted")
        } catch { XCTAssertEqual(error as? WorkspaceError, .conflict) }
    }
}
