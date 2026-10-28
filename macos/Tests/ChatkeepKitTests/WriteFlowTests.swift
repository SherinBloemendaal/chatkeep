@testable import ChatkeepKit
import Foundation
import Testing

/// A stand-in tool that answers a preview with `tool_running` and `queue add` with `addAnswer`,
/// and writes the arguments of every run to `calls`, one run per line.
private struct BlockedTool {
    let folder: URL
    let runner: ChatkeepRunner

    /// `previewAnswer`, when given, answers runs with `-n`, so only the real run is blocked.
    init(queueArgs: String, addAnswer: String, previewAnswer: String? = nil) throws {
        folder = FileManager.default.temporaryDirectory.appending(path: "ChatkeepWriteFlowTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let tool = folder.appending(path: "chatkeep")
        let calls = folder.appending(path: "calls").path
        let script = """
        #!/bin/sh
        printf '%s\\n' "$*" >> '\(calls)'
        if [ "$2" = "queue" ] && [ "$3" = "add" ]; then
            printf '%s\\n' '\(addAnswer)'
            exit 0
        fi
        \(previewAnswer.map { "case \" $* \" in *\" -n \"*) printf '%s\\n' '\($0)'; exit 0;; esac" } ?? "")
        printf '%s\\n' '{"ok":false,"command":"mv","error":{"kind":"tool_running","message":"Cursor is running.","hint":null,"queue_args":\(queueArgs)},"results":[]}'
        exit 75
        """
        try Data(script.utf8).write(to: tool)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tool.path)
        runner = ChatkeepRunner(executable: tool)
    }

    var calls: [String] {
        let text = (try? String(contentsOf: folder.appending(path: "calls"), encoding: .utf8)) ?? ""
        return text.split(separator: "\n").map(String.init)
    }

    func remove() {
        try? FileManager.default.removeItem(at: folder)
    }
}

private let addedEntry = """
{"ok":true,"command":"queue add","dry_run":false,"results":[{"tool":null,"data":{"entry":{"id":"3f9c2a1b","position":2,"status":"pending","interrupted":false,"argv":["mv","-y","--","/a","/b"],"command":"mv -y -- /a /b","tool":null,"cwd":"/","profile":null,"created_at":null,"tool_version":"1.2.0","warnings":[],"last_error":null}}}]}
"""

@MainActor
struct WriteFlowTests {
    private let move = CLI.move(.project(from: "/a", to: "/b"))

    @Test func aBlockedPreviewQueuesTheToolsArgumentsAsARealRun() async throws {
        // The tool's queue_args repeat the preview's -n; the Rust side adds --tool before the --.
        let tool = try BlockedTool(queueArgs: #"["mv","-n","--tool","cursor","--","/a","/b"]"#, addAnswer: addedEntry)
        defer { tool.remove() }
        let flow = WriteFlow<Report>()

        await flow.preview(move, with: tool.runner).value
        let arguments = try #require(flow.queueArguments(for: move))
        #expect(arguments == ["mv", "-y", "--tool", "cursor", "--", "/a", "/b"])

        await flow.addToQueue(arguments, with: tool.runner).value

        #expect(tool.calls.last == "--json queue add -- mv -y --tool cursor -- /a /b")
        guard case .queued(let entry) = flow.stage else {
            Issue.record("expected the flow to be queued, got \(flow.stage)")
            return
        }
        #expect(entry?.position == 2)
        #expect(flow.isFinished)
    }

    @Test func withoutQueueArgumentsThePreviewedCommandIsQueued() async throws {
        let tool = try BlockedTool(queueArgs: "null", addAnswer: addedEntry)
        defer { tool.remove() }
        let flow = WriteFlow<Report>()

        await flow.preview(move, with: tool.runner).value

        #expect(flow.queueArguments(for: move) == ["mv", "-y", "--", "/a", "/b"])
    }

    @Test func changedFieldsAfterABlockedPreviewWithdrawTheQueueOffer() async throws {
        let tool = try BlockedTool(queueArgs: #"["mv","-n","--","/a","/b"]"#, addAnswer: addedEntry)
        defer { tool.remove() }
        let flow = WriteFlow<Report>()
        let edited = CLI.move(.project(from: "/a", to: "/elsewhere"))

        await flow.preview(move, with: tool.runner).value

        #expect(flow.queueArguments(for: edited) == nil)
        #expect(flow.queueOfferIsStale(for: edited))
        #expect(flow.queueArguments(for: nil) == nil)
        #expect(flow.queueOfferIsStale(for: move) == false)
        #expect(flow.queueArguments(for: move) != nil)
    }

    @Test func aQueueAnswerWithoutAnEntryStillEndsTheFlow() async throws {
        let tool = try BlockedTool(
            queueArgs: #"["mv","-n","--","/a","/b"]"#,
            addAnswer: #"{"ok":true,"command":"queue add","dry_run":false,"results":[]}"#
        )
        defer { tool.remove() }
        let flow = WriteFlow<Report>()

        await flow.preview(move, with: tool.runner).value
        await flow.addToQueue(try #require(flow.queueArguments(for: move)), with: tool.runner).value

        guard case .queued(let entry) = flow.stage else {
            Issue.record("expected the flow to be queued, got \(flow.stage)")
            return
        }
        #expect(entry == nil)
        #expect(flow.isBusy == false)
        #expect(flow.isFinished)
    }

    @Test func aRefusedQueueAddEndsInAReadableError() async throws {
        let tool = try BlockedTool(
            queueArgs: #"["mv","-n","--","/a","/b"]"#,
            addAnswer: #"{"ok":false,"command":"queue add","error":{"kind":"failed","message":"refusing to queue: can run directly","hint":null,"queue_args":null},"results":[]}"#
        )
        defer { tool.remove() }
        let flow = WriteFlow<Report>()

        await flow.preview(move, with: tool.runner).value
        await flow.addToQueue(try #require(flow.queueArguments(for: move)), with: tool.runner).value

        guard case .failed(let error, false) = flow.stage else {
            Issue.record("expected the flow to fail, got \(flow.stage)")
            return
        }
        #expect(error.message == "refusing to queue: can run directly")
        #expect(flow.isBusy == false)
        #expect(flow.queueArguments(for: move) == nil)
    }

    @Test func aBlockedRealRunQueuesTheToolsArgumentsAsGiven() async throws {
        let tool = try BlockedTool(
            queueArgs: #"["mv","-y","--tool","cursor","--","/a","/b"]"#,
            addAnswer: addedEntry,
            previewAnswer: #"{"ok":true,"command":"mv","dry_run":true,"results":[{"tool":"cursor","data":{"title":"Move","warnings":[],"skipped":[],"applied":["move /a -> /b"],"rewritten_keys":[]}}]}"#
        )
        defer { tool.remove() }
        let flow = WriteFlow<Report>()

        await flow.preview(move, with: tool.runner).value
        #expect(flow.previewResponse != nil)
        await flow.apply(with: tool.runner)?.value

        #expect(tool.calls.last == "--json mv -y -- /a /b")
        #expect(flow.queueArguments(for: move) == ["mv", "-y", "--tool", "cursor", "--", "/a", "/b"])
    }

    @Test func onlyThePreviewFlagBeforeTheDoubleDashBecomesYes() {
        #expect(WriteFlow<Report>.confirmed(["rm", "-n", "--", "-n"]) == ["rm", "-y", "--", "-n"])
        #expect(WriteFlow<Report>.confirmed(["mv", "-y", "--", "/a", "-n"]) == ["mv", "-y", "--", "/a", "-n"])
    }
}
