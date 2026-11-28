import Foundation
import XCTest
@testable import NexusCore

final class LocalGuestStoreTests: XCTestCase {
    func testCleanReopenDirtyRefusalAndReset() throws {
        let root=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let bundle=root.appendingPathComponent("bundle")
        try FileManager.default.createDirectory(at: bundle.appendingPathComponent("data"), withIntermediateDirectories: true)
        try Data("db".utf8).write(to: bundle.appendingPathComponent("meta.db"))
        try Data("pinned-rootfs".utf8).write(to: bundle.appendingPathComponent("provenance.txt"))
        let store=LocalGuestStore(directory: root.appendingPathComponent("saved"))
        let data=try store.prepare(from: bundle)
        try Data("42".utf8).write(to: data.appendingPathComponent("test"))
        XCTAssertThrowsError(try store.prepare(from: bundle))
        try store.markClean()
        XCTAssertEqual(try store.prepare(from: bundle),data)
        XCTAssertEqual(try Data(contentsOf: data.appendingPathComponent("test")),Data("42".utf8))
        try store.markClean()
        try Data("changed-rootfs".utf8).write(to: bundle.appendingPathComponent("provenance.txt"))
        XCTAssertThrowsError(try store.prepare(from: bundle))
        XCTAssertTrue(FileManager.default.fileExists(atPath: data.appendingPathComponent("test").path))
        try store.reset()
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.directory.path))
    }
}

extension LocalGuestStoreTests {
    func testAgentAuthStoresSeparateAndResetScoped() throws {
        let root=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let digest=String(repeating:"a",count:64)
        let gemini=try LocalGuestStore.agentDirectoryName(digest:digest,google:false)
        let google=try LocalGuestStore.agentDirectoryName(digest:digest,google:true)
        XCTAssertEqual(gemini,"guest-"+digest)
        XCTAssertEqual(google,"google-guest-"+digest)
        XCTAssertThrowsError(try LocalGuestStore.agentDirectoryName(digest:"../bad",google:true))
        let first=LocalGuestStore(directory:root.appendingPathComponent(gemini))
        let second=LocalGuestStore(directory:root.appendingPathComponent(google))
        try FileManager.default.createDirectory(at:first.directory,withIntermediateDirectories:true)
        try FileManager.default.createDirectory(at:second.directory,withIntermediateDirectories:true)
        try second.reset()
        XCTAssertTrue(FileManager.default.fileExists(atPath:first.directory.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath:second.directory.path))
    }
}
