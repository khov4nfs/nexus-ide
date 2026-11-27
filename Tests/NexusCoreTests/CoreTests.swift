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
