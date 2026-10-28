import ChatkeepKit
import Foundation
import Testing

/// The debug build of the real tool (`cargo build` at the repository root), when there is one.
private let debugTool: URL? = {
    let url = URL(filePath: #filePath)
        .deletingLastPathComponent() // ChatkeepKitTests
        .deletingLastPathComponent() // Tests
        .deletingLastPathComponent() // macos
        .deletingLastPathComponent() // repository root
        .appending(path: "target/debug/chatkeep")
    return FileManager.default.isExecutableFile(atPath: url.path) ? url : nil
}()

/// Runs read-only commands against the real tool and this machine's data. Nothing here writes:
/// every command is a read the contract lists as not logged and not changing data.
@Suite(.enabled(if: debugTool != nil, "Run cargo build at the repository root first"), .serialized)
struct IntegrationTests {
    private var runner: ChatkeepRunner { ChatkeepRunner(executable: debugTool!) }

    @Test func version() async throws {
        let response = try await runner.run(CLI.version())

        #expect(response.command == "version")
        #expect(response.results.first?.tool == nil)
        #expect(response.data?.version.isEmpty == false)
    }

    @Test func listsProjects() async throws {
        let response = try await runner.run(CLI.projects())

        #expect(response.command == "ls")
        #expect(response.dryRun == false)
        for result in response.results {
            #expect(result.data.projects.allSatisfy { $0.tool == result.tool })
            // Claude Code knows when every session last changed.
            if result.tool == .claude {
                #expect(result.data.projects.filter { $0.chats > 0 }.allSatisfy { $0.lastActive != nil })
            }
        }
    }

    @Test func showsOneProjectByItsId() async throws {
        let list = try await runner.run(CLI.projects())
        let picked = list.results.lazy.compactMap { result in
            result.data.projects.first { $0.chats > 0 && !$0.isUnsaved }
        }.first
        // A machine without chats has nothing to show here.
        guard let project = picked else { return }

        let response = try await runner.run(CLI.project(
            project.projectID,
            scope: Scope(tool: project.tool, profile: project.profileLabel)
        ))

        let detail = try #require(response.data(for: project.tool)?.details.first)
        #expect(detail.project.projectID == project.projectID)
        #expect(detail.chats.isEmpty == false)
    }

    @Test func stats() async throws {
        let response = try await runner.run(CLI.stats())

        #expect(response.command == "stats")
        for result in response.results {
            switch (result.tool, result.data) {
            case (.cursor, .cursor), (.claude, .claude): break
            default: Issue.record("stats of \(String(describing: result.tool)) decoded as the other tool")
            }
        }
    }

    @Test func queueList() async throws {
        let response = try await runner.run(CLI.queueList())

        #expect(response.command == "queue list")
        #expect(response.results.count == 1)
    }

    @Test func history() async throws {
        let response = try await runner.run(CLI.history())

        #expect(response.command == "history")
        #expect(response.data?.path.hasSuffix("history.jsonl") == true)
    }

    @Test func anUnknownProjectIsAReadableError() async throws {
        do {
            _ = try await runner.run(CLI.project("/nonexistent/chatkeep-integration-test"))
            Issue.record("expected an error")
        } catch {
            #expect(error.kind == .failed)
            #expect(error.message.contains("/nonexistent/chatkeep-integration-test"))
        }
    }
}
