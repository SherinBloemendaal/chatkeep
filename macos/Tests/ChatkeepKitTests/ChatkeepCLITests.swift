import ChatkeepKit
import Foundation
import Testing

/// A scratch folder for one test; call `remove()` in a `defer`.
private struct Scratch {
    let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory.appending(path: "ChatkeepCLITests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    func remove() {
        try? FileManager.default.removeItem(at: root)
    }

    /// Creates `<folder>/chatkeep`, executable or not, and returns the folder.
    func folder(_ name: String, withTool executable: Bool) throws -> URL {
        let folder = root.appending(path: name, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let tool = folder.appending(path: ChatkeepCLI.executableName)
        try Data("#!/bin/sh\n".utf8).write(to: tool)
        try FileManager.default.setAttributes([.posixPermissions: executable ? 0o755 : 0o644], ofItemAtPath: tool.path)
        return folder
    }
}

struct ChatkeepCLITests {
    @Test func prefersTheToolInsideTheApp() throws {
        let scratch = try Scratch()
        defer { scratch.remove() }
        let resources = try scratch.folder("Resources", withTool: true)
        let bin = try scratch.folder("bin", withTool: true)

        let found = try ChatkeepCLI(resourceDirectory: resources, searchPath: bin.path).locate()

        #expect(found == resources.appending(path: "chatkeep"))
    }

    @Test func fallsBackToTheFirstExecutableOnPath() throws {
        let scratch = try Scratch()
        defer { scratch.remove() }
        let notExecutable = try scratch.folder("plain", withTool: false)
        let bin = try scratch.folder("bin", withTool: true)
        let empty = scratch.root.appending(path: "empty", directoryHint: .isDirectory)

        let found = try ChatkeepCLI(
            resourceDirectory: empty,
            searchPath: "\(notExecutable.path):\(bin.path)"
        ).locate()

        #expect(found.standardizedFileURL == bin.appending(path: "chatkeep").standardizedFileURL)
    }

    @Test func reportsAMissingTool() throws {
        let scratch = try Scratch()
        defer { scratch.remove() }

        #expect(throws: ChatkeepCLI.LocateError.notFound) {
            try ChatkeepCLI(resourceDirectory: scratch.root, searchPath: "").locate()
        }
    }
}
