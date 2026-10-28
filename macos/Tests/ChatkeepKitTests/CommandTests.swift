import ChatkeepKit
import Testing

/// The argv each typed command produces. `--json` always comes first, before any `--`.
struct CommandTests {
    private let cursor = Scope(tool: .cursor, profile: "default/work")

    @Test func reads() {
        #expect(CLI.version().argv == ["--json", "--version"])
        #expect(CLI.projects().argv == ["--json", "ls"])
        #expect(CLI.projects(scope: Scope(tool: .cursor), unsaved: true, fresh: true).argv
            == ["--json", "ls", "--tool", "cursor", "--unsaved", "--fresh"])
        #expect(CLI.project("-Users-me-app", scope: Scope(tool: .claude)).argv
            == ["--json", "ls", "--tool", "claude", "--", "-Users-me-app"])
        #expect(CLI.stats(scope: cursor, fresh: true).argv
            == ["--json", "stats", "--tool", "cursor", "--profile", "default/work", "--fresh"])
        #expect(CLI.history().argv == ["--json", "history"])
        #expect(CLI.queueList().argv == ["--json", "queue", "list"])
        #expect(CLI.queueList(full: true).argv == ["--json", "queue", "list", "--full"])
        #expect(CLI.cursorCacheStats(profile: "default").argv == ["--json", "cursor", "cache", "stats", "--profile", "default"])
        #expect(CLI.cursorCacheCheck().argv == ["--json", "cursor", "cache", "scan", "-n"])
        #expect(CLI.cursorCacheScan(full: true).argv == ["--json", "cursor", "cache", "scan", "--full"])
        #expect(CLI.claudeCacheStats().argv == ["--json", "claude", "cache", "stats"])
        #expect(CLI.accounts().argv == ["--json", "claude", "accounts", "ls"])
        #expect(CLI.accountsDoctor().argv == ["--json", "claude", "accounts", "doctor"])
        #expect(CLI.syncProfiles().argv == ["--json", "claude", "sync", "profiles"])
        #expect(CLI.syncLog(last: 100).argv == ["--json", "claude", "sync", "log", "--last", "100"])
        #expect(CLI.syncAutoStatus().argv == ["--json", "claude", "sync", "auto", "status"])
    }

    @Test func onlyWritesAreNeverStopped() {
        #expect(CLI.projects().changesData == false)
        #expect(CLI.stats().changesData == false)
        #expect(CLI.cursorCacheCheck().changesData == false)
        #expect(CLI.cursorCacheScan().changesData)
        #expect(CLI.queueAdd(["mv", "-y", "/a", "/b"]).changesData)
        #expect(CLI.remove("/a").command(.preview).changesData == false)
        #expect(CLI.remove("/a").command(.apply).changesData)
    }

    @Test func writesWithoutAPreview() {
        #expect(CLI.queueAdd(["mv", "-y", "--", "/a", "/b"]).argv
            == ["--json", "queue", "add", "--", "mv", "-y", "--", "/a", "/b"])
        #expect(CLI.nameAccount("310c27f1", name: "work").argv
            == ["--json", "claude", "accounts", "name", "--", "310c27f1", "work"])
        #expect(CLI.clearAccountName("310c27f1").argv
            == ["--json", "claude", "accounts", "name", "--clear", "--", "310c27f1"])
        #expect(CLI.autoSync(on: true).argv == ["--json", "claude", "sync", "auto", "on"])
        #expect(CLI.autoSync(on: false).argv == ["--json", "claude", "sync", "auto", "off"])
    }

    @Test func previewAndApplyDifferOnlyInTheConfirmationFlag() {
        let move = CLI.move(.project(from: "/old/app", to: "/new/app"))
        #expect(move.command(.preview).argv == ["--json", "mv", "-n", "--", "/old/app", "/new/app"])
        #expect(move.command(.apply).argv == ["--json", "mv", "-y", "--", "/old/app", "/new/app"])
    }

    @Test func moveAndCopy() {
        #expect(CLI.move(.project(from: "/a", to: "/b"), moveFolder: true, scope: cursor, unsaved: true).command(.preview).argv
            == ["--json", "mv", "-n", "--tool", "cursor", "--profile", "default/work", "--unsaved", "--project", "--", "/a", "/b"])
        #expect(CLI.move(.replace(from: "^/Users/old", to: "/Users/new", regex: true)).command(.apply).argv
            == ["--json", "mv", "-y", "--replace", "^/Users/old", "/Users/new", "--regex"])
        #expect(CLI.copy(.project(from: "/a", to: "/b"), copyFolder: true).command(.preview).argv
            == ["--json", "cp", "-n", "--project", "--", "/a", "/b"])
        #expect(CLI.copy(.replace(from: "/x", to: "/y", regex: false), scope: Scope(tool: .claude)).command(.apply).argv
            == ["--json", "cp", "-y", "--tool", "claude", "--replace", "/x", "/y"])
    }

    @Test func removeSplitCombine() {
        #expect(CLI.remove("75b81147", scope: Scope(tool: .cursor)).command(.preview).argv
            == ["--json", "rm", "-n", "--tool", "cursor", "--", "75b81147"])
        #expect(CLI.split(
            "/mono",
            into: ["/mono/api", "/mono/web"],
            assignments: [SplitAssignment(chat: "aaaa", target: "/mono/web"), SplitAssignment(chat: "cccc", target: nil)],
            moveChats: true,
            scope: Scope(tool: .cursor)
        ).command(.apply).argv == [
            "--json", "split", "-y", "--tool", "cursor", "--move",
            "--assign", "aaaa=/mono/web", "--assign", "cccc=",
            "--", "/mono", "/mono/api", "/mono/web",
        ])
        #expect(CLI.combine(into: "/target", from: ["/one", "/two"]).command(.preview).argv
            == ["--json", "combine", "-n", "--copy", "--", "/target", "/one", "/two"])
        #expect(CLI.combine(into: "/target", from: ["/one"], mode: .move, scope: cursor).command(.apply).argv
            == ["--json", "combine", "-y", "--tool", "cursor", "--profile", "default/work", "--move", "--", "/target", "/one"])
    }

    @Test func exportAndImport() {
        #expect(CLI.export("/app", to: "/tmp/app.chatkeep").command(.preview).argv
            == ["--json", "export", "-n", "--", "/app", "/tmp/app.chatkeep"])
        #expect(CLI.importArchive("/tmp/app.chatkeep").command(.preview).argv
            == ["--json", "import", "-n", "--", "/tmp/app.chatkeep"])
        #expect(CLI.importArchive("/tmp/app.chatkeep", into: "/new/app", overwrite: true, scope: Scope(tool: .claude)).command(.apply).argv
            == ["--json", "import", "-y", "--tool", "claude", "--overwrite", "--", "/tmp/app.chatkeep", "/new/app"])
    }

    @Test func cursorCommands() {
        #expect(CLI.saveUnsaved("1762251124970", to: "/new/app", profile: "default").command(.preview).argv
            == ["--json", "cursor", "save", "-n", "--profile", "default", "--", "1762251124970", "/new/app"])
        #expect(CLI.reindex("/app").command(.apply).argv == ["--json", "cursor", "rx", "-y", "--", "/app"])
        #expect(CLI.clearCursorCache().command(.preview).argv == ["--json", "cursor", "cache", "clear", "-n"])
        #expect(CLI.clearCursorCache(profile: "default").command(.apply).argv
            == ["--json", "cursor", "cache", "clear", "-y", "--profile", "default"])
    }

    @Test func queueCommands() {
        #expect(CLI.queueRemove(["de1cd8d8", "3f9c2a1b"]).command(.apply).argv
            == ["--json", "queue", "rm", "-y", "--", "de1cd8d8", "3f9c2a1b"])
        #expect(CLI.queueClear().command(.preview).argv == ["--json", "queue", "clear", "-n"])
        #expect(CLI.queueRetry().command(.apply).argv == ["--json", "queue", "retry", "-y"])
        #expect(CLI.queueRetry(["de1cd8d8"]).command(.preview).argv == ["--json", "queue", "retry", "-n", "--", "de1cd8d8"])
        #expect(CLI.queueExecute().command(.preview).argv == ["--json", "queue", "execute", "-n"])
        #expect(CLI.queueExecute(continueOnError: true).command(.apply).argv
            == ["--json", "queue", "execute", "-y", "--continue-on-error"])
    }

    @Test func claudeCommands() {
        #expect(CLI.copyAccountChats(from: "310c27f1").command(.preview).argv
            == ["--json", "claude", "accounts", "cp", "-n", "--", "310c27f1"])
        #expect(CLI.copyAccountChats(from: "310c27f1", to: "447c80ba").command(.apply).argv
            == ["--json", "claude", "accounts", "cp", "-y", "--", "310c27f1", "447c80ba"])
        #expect(CLI.pruneAccounts().command(.preview).argv == ["--json", "claude", "accounts", "prune", "-n"])
        #expect(CLI.clearClaudeCache().command(.apply).argv == ["--json", "claude", "cache", "clear", "-y"])
        #expect(CLI.sync().command(.preview).argv == ["--json", "claude", "sync", "-n"])
        #expect(CLI.sync(profile: "main").command(.apply).argv == ["--json", "claude", "sync", "-y", "--", "main"])
        #expect(CLI.setSyncProfile("main", accounts: ["a/o", "b/o"]).command(.preview).argv
            == ["--json", "claude", "sync", "set", "-n", "--", "main", "a/o", "b/o"])
        #expect(CLI.addToSyncProfile("main", accounts: ["c/o"]).command(.apply).argv
            == ["--json", "claude", "sync", "add", "-y", "--", "main", "c/o"])
        #expect(CLI.removeFromSyncProfile("main", accounts: ["c/o"]).command(.preview).argv
            == ["--json", "claude", "sync", "rm", "-n", "--", "main", "c/o"])
        #expect(CLI.removeFromSyncProfile("main").command(.apply).argv
            == ["--json", "claude", "sync", "rm", "-y", "--", "main"])
    }

    @Test func anEmptyProfileIsLeftOut() {
        #expect(CLI.stats(scope: Scope(tool: .cursor, profile: "")).argv == ["--json", "stats", "--tool", "cursor"])
    }
}
