@testable import ChatkeepKit
import Foundation
import Testing

/// A stand-in for the chatkeep tool: a shell script in a scratch folder.
private struct FakeTool {
    let folder: URL
    let url: URL

    init(_ script: String) throws {
        folder = FileManager.default.temporaryDirectory.appending(path: "ChatkeepRunnerTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        url = folder.appending(path: "chatkeep")
        try Data("#!/bin/sh\n\(script)\n".utf8).write(to: url)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
    }

    /// A tool that prints a fixture and exits with `status`.
    init(printing fixture: String, status: Int32 = 0) throws {
        let document = try #require(Bundle.module.url(forResource: fixture, withExtension: "json", subdirectory: "Fixtures"))
        try self.init("cat '\(document.path)'\nexit \(status)")
    }

    var runner: ChatkeepRunner { ChatkeepRunner(executable: url) }

    func remove() {
        try? FileManager.default.removeItem(at: folder)
    }
}

struct RunnerTests {
    @Test func passesTheArgumentsAndDecodesTheAnswer() async throws {
        let tool = try FakeTool(printing: "version")
        defer { tool.remove() }

        let response = try await tool.runner.run(CLI.version())

        #expect(response.data?.version == "1.2.0")
    }

    @Test func handsTheArgvToTheTool() async throws {
        // Echo the arguments back as the version, so the test sees exactly what the tool got.
        let tool = try FakeTool(#"printf '{"ok":true,"command":"version","dry_run":false,"results":[{"tool":null,"data":{"version":"%s"}}]}\n' "$*""#)
        defer { tool.remove() }
        let details = CLI.project("-Users-me-app", scope: Scope(tool: .claude))
        let echo = Command<VersionInfo>(
            Arguments(words: ["ls"], options: ["--tool", "claude"], positionals: ["-Users-me-app"]),
            changesData: false
        )

        let response = try await tool.runner.run(echo)

        #expect(echo.argv == details.argv)
        #expect(response.data?.version == details.argv.joined(separator: " "))
    }

    @Test func errorDocumentsBecomeTypedErrors() async throws {
        let tool = try FakeTool(printing: "envelope-error", status: 75)
        defer { tool.remove() }

        do {
            _ = try await tool.runner.run(CLI.move(.project(from: "/a", to: "/b")).command(.apply))
            Issue.record("expected an error")
        } catch {
            #expect(error.kind == .toolRunning)
            #expect(error.exitCode == 75)
            #expect(error.canQueue)
        }
    }

    @Test func aToolThatPrintsNothingExplainsWhy() async throws {
        let tool = try FakeTool("echo 'thread main panicked' >&2\nexit 101")
        defer { tool.remove() }

        do {
            _ = try await tool.runner.run(CLI.history())
            Issue.record("expected an error")
        } catch {
            #expect(error.kind == .unreadableOutput)
            #expect(error.exitCode == 101)
            #expect(error.hint == "thread main panicked")
        }
    }

    @Test func largeAnswersDoNotBlockThePipe() async throws {
        // More than a pipe buffer holds on stdout and stderr at once.
        let tool = try FakeTool("""
        i=0; while [ $i -lt 2000 ]; do echo 'warning line warning line warning line warning line' >&2; i=$((i+1)); done
        printf '{"ok":true,"command":"history","dry_run":false,"results":[{"tool":null,"data":{"path":"/h","entries":['
        i=0; while [ $i -lt 3000 ]; do printf '{"at":null,"command":"ls","args":["a"],"outcome":"ok","duration_ms":1},'; i=$((i+1)); done
        printf '{"at":null,"command":null,"args":[],"outcome":null,"duration_ms":null}],"unreadable":0}}]}\\n'
        """)
        defer { tool.remove() }

        let response = try await tool.runner.run(CLI.history())

        #expect(response.data?.entries.count == 3001)
    }

    @Test func cancellingAReadStopsTheTool() async throws {
        let tool = try FakeTool("sleep 30")
        defer { tool.remove() }
        let runner = tool.runner

        let started = ContinuousClock.now
        let task = Task { try await runner.run(CLI.stats()) }
        try await Task.sleep(for: .milliseconds(300))
        task.cancel()
        let result = await task.result

        #expect(ContinuousClock.now - started < .seconds(10))
        #expect(throws: ChatkeepError.cancelled) { try result.get() }
    }

    @Test func cancellingAWriteLetsItFinish() async throws {
        let tool = try FakeTool("""
        sleep 1
        printf '{"ok":true,"command":"queue clear","dry_run":false,"results":[{"tool":null,"data":{"title":"Queue clear","warnings":[],"skipped":[],"applied":["remove de1cd8d8 ls"],"rewritten_keys":[]}}]}\\n'
        """)
        defer { tool.remove() }
        let runner = tool.runner

        let task = Task { try await runner.run(CLI.queueClear().command(.apply)) }
        try await Task.sleep(for: .milliseconds(200))
        task.cancel()
        let response = try await task.value

        #expect(response.data?.applied == ["remove de1cd8d8 ls"])
    }

    @Test func aMissingToolIsReported() {
        #expect(throws: ChatkeepError.self) {
            try ChatkeepRunner(cli: ChatkeepCLI(resourceDirectory: nil, searchPath: ""))
        }
    }
}
