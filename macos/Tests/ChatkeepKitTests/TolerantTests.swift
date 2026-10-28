@testable import ChatkeepKit
import Foundation
import Testing

private func decode<Payload: ResultPayload>(_ json: String, as payload: Payload.Type) throws -> Response<Payload> {
    try EnvelopeDecoder.decode(Data(json.utf8), as: payload)
}

/// A word from a newer chatkeep decodes to `unknown` with the word kept, never to an error.
struct TolerantTests {
    @Test func knownWordsMapToTheirCaseAndBack() throws {
        #expect(QueueEntry.Status(rawValue: "pending") == .pending)
        #expect(ProjectKind(rawValue: "code-workspace") == .codeWorkspace)
        #expect(Project.Destination(rawValue: "none") == .noFolder)
        #expect(Tool(rawValue: "claude") == .claude)
        #expect(ProjectKind.codeWorkspace.rawValue == "code-workspace")
        #expect(Tool.claude.isKnown)

        let encoded = try JSONEncoder().encode([ProjectKind.emptyWindow, .unknown("dev-container")])
        #expect(String(decoding: encoded, as: UTF8.self) == #"["empty-window","dev-container"]"#)
        #expect(try JSONDecoder().decode([ProjectKind].self, from: encoded) == [.emptyWindow, .unknown("dev-container")])
    }

    @Test func unknownWordsKeepTheirText() {
        #expect(QueueEntry.Status(rawValue: "paused") == .unknown("paused"))
        #expect(QueueEntry.Status.unknown("paused").rawValue == "paused")
        #expect(QueueEntry.Status.unknown("paused").isKnown == false)
        #expect(Tool.allCases == [.cursor, .claude])
    }

    @Test func aProjectListWithNewToolKindAndDestination() throws {
        let response = try decode("""
        {"ok":true,"command":"ls","dry_run":false,"results":[{"tool":"windsurf","data":{"index_as_of":null,"index_refreshing":false,"projects":[{"tool":"windsurf","id":"x","kind":"dev-container","name":"~/x","path":"/x","uri":null,"storage_dir":"/s/x","install":null,"profile":null,"profile_label":null,"chats":1,"subagents":0,"archived":null,"size_bytes":10,"last_active":null,"destination":"elsewhere","destination_missing":false}]}}]}
        """, as: ProjectList.self)

        #expect(response.results.first?.tool == .unknown("windsurf"))
        let project = try #require(response.data?.projects.first)
        #expect(project.tool == .unknown("windsurf"))
        #expect(project.tool.label == "windsurf")
        #expect(project.kind == .unknown("dev-container"))
        #expect(project.destination == .unknown("elsewhere"))
        #expect(project.lastActive == nil)
    }

    @Test func statsOfANewToolAreSkippedNotFailed() throws {
        let response = try decode("""
        {"ok":true,"command":"stats","dry_run":false,"results":[{"tool":"windsurf","data":{"anything":1}}]}
        """, as: Stats.self)

        #expect(response.data == .unknownTool("windsurf"))
    }

    @Test func queueEntriesWithNewStatuses() throws {
        let list = try decode("""
        {"ok":true,"command":"queue list","dry_run":false,"results":[{"tool":null,"data":{"entries":[{"id":"3f9c2a1b","position":null,"status":"paused","interrupted":false,"argv":["mv"],"command":"mv","tool":"windsurf","cwd":"/","profile":null,"created_at":null,"tool_version":"9.0.0","warnings":[],"last_error":null}]}}]}
        """, as: QueueList.self)
        let entry = try #require(list.data?.entries.first)
        #expect(entry.status == .unknown("paused"))
        #expect(entry.tool == .unknown("windsurf"))
        #expect(entry.canRetry == false)

        let execution = try decode("""
        {"ok":true,"command":"queue execute","dry_run":true,"results":[{"tool":null,"data":{"title":"Queue execute","warnings":[],"skipped":[],"applied":[],"rewritten_keys":[],"entries":[{"id":"b970b2f4","argv":["mv"],"command":"mv","plan":"deferred","plan_warnings":[],"plan_error":null,"status":"waiting","error":null,"results":[]}]}}]}
        """, as: QueueExecution.self)
        let run = try #require(execution.data?.entries.first)
        #expect(run.plan == .unknown("deferred"))
        #expect(run.status == .unknown("waiting"))
    }

    @Test func historySyncAndDoctorWithNewWords() throws {
        let history = try decode("""
        {"ok":true,"command":"history","dry_run":false,"results":[{"tool":null,"data":{"path":"/h","entries":[{"at":null,"command":"mv","args":[],"outcome":"partial","duration_ms":null}],"unreadable":0}}]}
        """, as: History.self)
        #expect(history.data?.entries.first?.outcome == .unknown("partial"))

        let log = try decode("""
        {"ok":true,"command":"claude sync log","dry_run":false,"results":[{"tool":"claude","data":{"entries":[{"at":null,"profile":null,"action":"rename","chat":null,"title":null,"account":null,"source":"schedule","changed":[],"times":1}]}}]}
        """, as: SyncLog.self)
        #expect(log.data?.entries.first?.action == .unknown("rename"))
        #expect(log.data?.entries.first?.source == .unknown("schedule"))

        let status = try decode("""
        {"ok":true,"command":"claude sync auto status","dry_run":false,"results":[{"tool":"claude","data":{"state":"paused","installed":true,"running":false,"log_path":"/l"}}]}
        """, as: AutoSyncStatus.self)
        #expect(status.data?.state == .unknown("paused"))

        let doctor = try decode("""
        {"ok":true,"command":"claude accounts doctor","dry_run":false,"results":[{"tool":"claude","data":{"desktop_app":false,"desktop":null,"session":null,"login":null,"claude_command":null,"stale_caches":0,"findings":[{"level":"error","text":"Something new."}]}}]}
        """, as: AccountsDoctor.self)
        #expect(doctor.data?.findings.first?.level == .unknown("error"))
    }

    @Test func cursorStatsWithANewWorkspaceKind() throws {
        let kinds = try JSONDecoder.chatkeep.decode(
            [CursorStats.KindCount].self,
            from: Data(#"[{"kind":"folder","workspaces":3},{"kind":"dev-container","workspaces":1}]"#.utf8)
        )
        #expect(kinds.map(\.kind) == [.folder, .unknown("dev-container")])
    }
}
