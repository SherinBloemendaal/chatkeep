@testable import ChatkeepKit
import Foundation
import Testing

/// Every example of docs/json.md, copied into Fixtures, decoded into the app's models.
enum Fixture {
    static func data(_ name: String) throws -> Data {
        let url = try #require(Bundle.module.url(forResource: name, withExtension: "json", subdirectory: "Fixtures"))
        return try Data(contentsOf: url)
    }

    static func decode<Payload: ResultPayload>(_ name: String, as payload: Payload.Type) throws -> Response<Payload> {
        try EnvelopeDecoder.decode(data(name), as: payload)
    }
}

/// Accepts any `data`, for the envelope examples whose data is left empty.
struct AnyPayload: ResultPayload {
    init(from decoder: any Decoder, tool: Tool?) throws {}
}

private func date(_ text: String) -> Date {
    ISO8601DateFormatter().date(from: text)!
}

struct DecodingTests {
    @Test func envelopeKeepsTheToolOrderAndTheDryRunFlag() throws {
        let response = try Fixture.decode("envelope-success", as: AnyPayload.self)

        #expect(response.command == "ls")
        #expect(response.dryRun == false)
        #expect(response.results.map(\.tool) == [.claude, .cursor])
    }

    @Test func projectDetailsOfBothTools() throws {
        let response = try Fixture.decode("ls-detail", as: ProjectDetails.self)

        let claude = try #require(response.data(for: .claude))
        #expect(claude.indexAsOf == nil)
        let claudeDetail = try #require(claude.details.first)
        #expect(claudeDetail.project.projectID == "-Users-me-projects-resolved-api")
        #expect(claudeDetail.project.tool == .claude)
        #expect(claudeDetail.project.sizeBytes == 439_708_356)
        #expect(claudeDetail.project.memoryFiles == 17)
        #expect(claudeDetail.project.archived == nil)
        #expect(claudeDetail.project.destination == .present)
        #expect(claudeDetail.lastActive == date("2026-10-27T23:22:02Z"))
        #expect(claudeDetail.project.lastActive == claudeDetail.lastActive)
        #expect(claudeDetail.project.kind == .project)
        let session = try #require(claudeDetail.chats.first)
        #expect(session.title == "Fix login redirect")
        #expect(session.createdAt == nil)
        #expect(session.tokens?.cacheRead == 28_345_607)
        #expect(session.tokens?.context == nil)
        #expect(session.subagents == 8)
        #expect(session.sizeBytes == 10_830_749)
        #expect(session.transcript?.hasSuffix(".jsonl") == true)

        let cursor = try #require(response.data(for: .cursor))
        #expect(cursor.indexAsOf == date("2026-10-27T23:21:38Z"))
        let workspace = try #require(cursor.details.first?.project)
        #expect(workspace.projectID == "0fe570bb46aa058cdd9bdfcf83698bfb")
        #expect(workspace.profileLabel == "default")
        #expect(workspace.archived == 0)
        #expect(workspace.memoryFiles == nil)
        #expect(workspace.id == workspace.storageDir)
        #expect(workspace.kind == .folder)
        #expect(workspace.lastActive == date("2026-09-24T13:33:37Z"))
        let chat = try #require(cursor.details.first?.chats.first)
        #expect(chat.archived == false)
        #expect(chat.model == "grok-4.7")
        #expect(chat.tokens?.context == 156_861)
        #expect(chat.costCents == 0)
        #expect(chat.cwd == nil)
    }

    @Test func statsPickTheShapeByTool() throws {
        let response = try Fixture.decode("stats", as: Stats.self)

        let claude = try #require(response.data(for: .claude)?.claude)
        #expect(claude.projects == 40)
        #expect(claude.subagentTokens.cacheRead == 18_135_669_495)
        #expect(claude.perMonth == [ClaudeStats.Month(month: "2026-09", sessions: 5)])
        #expect(claude.largest.first?.sizeBytes == 659_715_219)

        let cursor = try #require(response.data(for: .cursor)?.cursor)
        #expect(cursor.installations == 8)
        #expect(cursor.profiles.first?.globalDbBytes == 34_357_874_688)
        #expect(cursor.usage.contextTokens == 1_620_491_356)
        #expect(cursor.busiest.first?.kind == .codeWorkspace)
        #expect(cursor.globalDbBytes == 158_142_390_272)
        #expect(cursor.workspaceDbs.first?.bytes == 19_136_512)
        #expect(response.data(for: .cursor)?.claude == nil)
    }

    @Test func history() throws {
        let response = try Fixture.decode("history", as: History.self)

        #expect(response.results.first?.tool == nil)
        let history = try #require(response.data)
        #expect(history.path == "/Users/me/.chatkeep/history.jsonl")
        #expect(history.unreadable == 0)
        let entry = try #require(history.entries.first)
        #expect(entry.at == date("2026-09-24T10:04:28Z"))
        #expect(entry.command == "mv")
        #expect(entry.outcome == .ok)
        #expect(entry.durationMs == 162)
    }

    @Test func queueListAndAdd() throws {
        let list = try #require(try Fixture.decode("queue-list", as: QueueList.self).data)
        let failed = try #require(list.entries.first)
        #expect(failed.status == .failed)
        #expect(failed.position == nil)
        #expect(failed.tool == .cursor)
        #expect(failed.profile == "resolved")
        #expect(failed.canRetry)
        #expect(failed.lastError?.contains("rolled back") == true)
        #expect(list.pendingCount == 0)

        let added = try #require(try Fixture.decode("queue-add", as: QueueAdded.self).data)
        #expect(added.entry.position == 1)
        #expect(added.entry.status == .pending)
        #expect(added.entry.argv == ["mv", "-y", "/Users/me/old/app", "/Users/me/new/app"])
        #expect(added.entry.tool == nil)
    }

    @Test func cursorCache() throws {
        let stats = try #require(try Fixture.decode("cursor-cache-stats", as: CursorCacheStats.self).data)
        #expect(stats.sizeBytes == 7_409_664)
        #expect(stats.refresh == nil)
        let install = try #require(stats.installs.first)
        #expect(install.scanDurationMs == 77)
        #expect(install.dirtyAt == nil)
        #expect(install.stale?.reasons == ["global db changed"])

        let preview = try Fixture.decode("cursor-cache-scan-preview", as: CursorCacheCheck.self)
        #expect(preview.dryRun)
        #expect(preview.data?.installs.first?.staleSources == 1)

        let scan = try #require(try Fixture.decode("cursor-cache-scan", as: CursorScan.self).data)
        #expect(scan.full == false)
        #expect(scan.durationMs == 86)
        #expect(scan.installs.first?.chatsRead == 2)
    }

    @Test func claudeCacheStats() throws {
        let stats = try #require(try Fixture.decode("claude-cache-stats", as: ClaudeCacheStats.self).data)
        #expect(stats.enabled)
        #expect(stats.transcripts == 851)
        #expect(stats.refreshedAt == date("2026-10-27T23:22:04Z"))
    }

    @Test func accountsAndDoctor() throws {
        let accounts = try #require(try Fixture.decode("claude-accounts-ls", as: AccountList.self).data)
        let account = try #require(accounts.accounts.first)
        #expect(account.label == "310c27f1")
        #expect(account.name == nil)
        #expect(account.onDisk == 79)
        #expect(account.signedIn == false)
        #expect(account.id == account.member)

        let doctor = try #require(try Fixture.decode("claude-accounts-doctor", as: AccountsDoctor.self).data)
        #expect(doctor.desktopApp)
        #expect(doctor.desktop?.organizations.count == 1)
        #expect(doctor.login?.organizationName == "Example Organization")
        #expect(doctor.session?.organizationName == nil)
        #expect(doctor.staleCaches == 7)
        #expect(doctor.findings.map(\.level) == [.ok, .hint])
    }

    @Test func syncProfilesLogAndAutoStatus() throws {
        let profiles = try #require(try Fixture.decode("claude-sync-profiles", as: SyncProfiles.self).data)
        #expect(profiles.profiles.map(\.name) == ["main"])
        #expect(profiles.profiles.first?.accounts.first?.organization == "d68212da-94ca-4311-9743-1c72b0f9362b")
        #expect(profiles.auto.state == .stopped)

        let log = try #require(try Fixture.decode("claude-sync-log", as: SyncLog.self).data)
        let entry = try #require(log.entries.first)
        #expect(entry.action == .update)
        #expect(entry.source == .watch)
        #expect(entry.changed == ["lastActivityAt"])
        #expect(entry.times == 1)

        let status = try #require(try Fixture.decode("claude-sync-auto-status", as: AutoSyncStatus.self).data)
        #expect(status.installed)
        #expect(status.running == false)
        #expect(status.logPath == "/Users/me/.chatkeep/sync-log.jsonl")
    }

    @Test func version() throws {
        let response = try Fixture.decode("version", as: VersionInfo.self)
        #expect(response.command == "version")
        #expect(response.data?.version == "1.2.0")
    }

    @Test func bareReport() throws {
        let report = try JSONDecoder.chatkeep.decode(Report.self, from: Fixture.data("report"))
        #expect(report.title == "Move")
        #expect(report.skipped == ["9602d48d5e4595972fd32e1bb8f58cf0"])
        #expect(report.isEmpty == false)
    }

    @Test func movePreview() throws {
        let response = try Fixture.decode("mv-preview", as: Report.self)
        #expect(response.dryRun)
        #expect(response.data(for: .cursor)?.warnings.first?.hasPrefix("skipped") == true)
        #expect(response.data?.applied.isEmpty == true)
    }

    @Test func splitPreviewCarriesThePlanPerChat() throws {
        let response = try Fixture.decode("split-preview", as: SplitReport.self)
        let split = try #require(response.data(for: .cursor))
        #expect(split.report.title == "Split")
        #expect(split.report.applied.count == 5)
        #expect(split.chats.map(\.id) == [
            "aaaaaaaa-0000-4000-8000-000000000001",
            "bbbbbbbb-0000-4000-8000-000000000002",
            "cccccccc-0000-4000-8000-000000000003",
        ])
        #expect(split.chats[0].suggested == ["/Users/me/mono/api"])
        #expect(split.chats[2].suggested.isEmpty)
        #expect(split.chats[2].targets.isEmpty)
    }

    @Test func queueExecutePreview() throws {
        let execution = try #require(try Fixture.decode("queue-execute-preview", as: QueueExecution.self).data)
        #expect(execution.report.title == "Queue execute")
        let entry = try #require(execution.entries.first)
        #expect(entry.plan == .ok)
        #expect(entry.status == .planned)
        #expect(entry.results.isEmpty)
    }

    @Test func autoSyncReportHoldsTheStatusAfterwards() throws {
        let document = Data("""
        {"ok":true,"command":"claude sync auto off","dry_run":false,"results":[{"tool":"claude","data":{"title":"Background sync off","warnings":[],"skipped":["background sync was not on"],"applied":[],"rewritten_keys":[],"auto":{"state":"off","installed":false,"running":false,"log_path":"/Users/me/.chatkeep/sync-log.jsonl"}}}]}
        """.utf8)
        let report = try #require(try EnvelopeDecoder.decode(document, as: AutoSyncReport.self).data)
        #expect(report.report.skipped == ["background sync was not on"])
        #expect(report.auto.state == .off)
    }

    @Test func entryReportsOfAnExecutedQueueKeepTheirDryRunFlag() throws {
        let document = Data("""
        {"ok":true,"command":"queue execute","dry_run":false,"results":[{"tool":null,"data":{"title":"Queue execute","warnings":[],"skipped":[],"applied":["b970b2f4 mv -y /a /b"],"rewritten_keys":[],"entries":[{"id":"b970b2f4","argv":["mv","-y","/a","/b"],"command":"mv -y /a /b","plan":"ok","plan_warnings":[],"plan_error":null,"status":"done","error":null,"results":[{"title":"Move","warnings":[],"skipped":[],"applied":["move /a -> /b"],"rewritten_keys":["k"],"dry_run":false}]}]}}]}
        """.utf8)
        let entry = try #require(try EnvelopeDecoder.decode(document, as: QueueExecution.self).data?.entries.first)
        #expect(entry.status == .done)
        #expect(entry.results.first?.dryRun == false)
        #expect(entry.results.first?.report.rewrittenKeys == ["k"])
    }
}
