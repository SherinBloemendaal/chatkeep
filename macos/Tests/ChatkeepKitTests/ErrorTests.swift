@testable import ChatkeepKit
import Foundation
import Testing

/// Decodes `fixture` as an error document and returns the error it maps to.
private func error(of fixture: String) throws -> ChatkeepError {
    let document = try Fixture.data(fixture)
    do {
        _ = try EnvelopeDecoder.decode(document, as: Report.self, exitCode: 1)
    } catch {
        return error
    }
    Issue.record("\(fixture) decoded without an error")
    throw CancellationError()
}

struct ErrorTests {
    @Test func toolRunningCarriesTheQueueArguments() throws {
        let failure = try error(of: "envelope-error")

        #expect(failure.kind == .toolRunning)
        #expect(failure.command == "mv")
        #expect(failure.message == "Cursor is running: default (Cursor, pid 4242).")
        #expect(failure.hint?.hasPrefix("Queue it with") == true)
        #expect(failure.queueArgs == ["mv", "-y", "/Users/me/old/app", "/Users/me/new/app"])
        #expect(failure.canQueue)
        #expect(failure.errorDescription == failure.message)
        #expect(failure.recoverySuggestion == failure.hint)
    }

    @Test(arguments: [
        ("error-1-missing-argument", ChatkeepError.Kind.missingArgument, "mv"),
        ("error-2-needs-confirmation", .needsConfirmation, "mv"),
        ("error-3-missing-argument", .missingArgument, "mv"),
        ("error-5-unsupported-in-json", .unsupportedInJSON, "update"),
    ])
    func everyErrorKindOfTheContract(fixture: String, kind: ChatkeepError.Kind, command: String) throws {
        let failure = try error(of: fixture)

        #expect(failure.kind == kind)
        #expect(failure.command == command)
        #expect(failure.queueArgs == nil)
        #expect(failure.canQueue == false)
        #expect(failure.hint != nil)
        #expect(failure.exitCode == 1)
    }

    @Test func usageHasNoCommandAndNoHint() throws {
        let failure = try error(of: "error-4-usage")

        #expect(failure.kind == .usage)
        #expect(failure.command == nil)
        #expect(failure.hint == nil)
        #expect(failure.message == "unrecognized subcommand 'bogus'")
    }

    @Test func wireNamesOfEveryKind() {
        #expect(ChatkeepError.Kind(wire: "usage") == .usage)
        #expect(ChatkeepError.Kind(wire: "missing_argument") == .missingArgument)
        #expect(ChatkeepError.Kind(wire: "needs_confirmation") == .needsConfirmation)
        #expect(ChatkeepError.Kind(wire: "tool_running") == .toolRunning)
        #expect(ChatkeepError.Kind(wire: "unsupported_in_json") == .unsupportedInJSON)
        #expect(ChatkeepError.Kind(wire: "failed") == .failed)
        #expect(ChatkeepError.Kind(wire: "something_new") == .other("something_new"))
    }

    @Test func blockedQueueCannotBeQueuedAgain() throws {
        let document = Data("""
        {"ok":false,"command":"queue execute","error":{"kind":"tool_running","message":"Cursor is running.","hint":null,"queue_args":null},"results":[]}
        """.utf8)
        #expect(throws: ChatkeepError.self) { try EnvelopeDecoder.decode(document, as: QueueExecution.self) }
        do {
            _ = try EnvelopeDecoder.decode(document, as: QueueExecution.self)
        } catch {
            #expect(error.kind == .toolRunning)
            #expect(error.canQueue == false)
        }
    }

    @Test func partialResultsOfAFailedRunStayReadable() throws {
        let document = Data("""
        {"ok":false,"command":"mv","error":{"kind":"tool_running","message":"Cursor is running.","hint":null,"queue_args":["mv","-y","/a","/b","--tool","cursor"]},"results":[{"tool":"claude","data":{"title":"Move (Claude Code)","warnings":[],"skipped":[],"applied":["move /a -> /b"],"rewritten_keys":[]}}]}
        """.utf8)
        do {
            _ = try EnvelopeDecoder.decode(document, as: Report.self)
            Issue.record("expected an error")
        } catch {
            let partial = error.partialResults(as: Report.self)
            #expect(partial.map(\.tool) == [.claude])
            #expect(partial.first?.data.applied == ["move /a -> /b"])
            #expect(error.queueArgs?.suffix(2) == ["--tool", "cursor"])
        }
    }

    @Test func textThatIsNoDocumentIsUnreadable() {
        do {
            _ = try EnvelopeDecoder.decode(Data("panic: index out of range".utf8), as: Report.self, exitCode: 101)
            Issue.record("expected an error")
        } catch {
            #expect(error.kind == .unreadableOutput)
            #expect(error.exitCode == 101)
        }
    }

    @Test func aDocumentOfTheWrongShapeIsUnreadable() throws {
        let document = try Fixture.data("history")
        do {
            _ = try EnvelopeDecoder.decode(document, as: QueueList.self)
            Issue.record("expected an error")
        } catch {
            #expect(error.kind == .unreadableOutput)
        }
    }
}
