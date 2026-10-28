import Foundation

/// One run of the tool: its arguments (without `--json`) and the type its results decode to.
public struct Command<Payload: ResultPayload>: Sendable, Hashable {
    public let arguments: [String]
    /// Whether the run changes data. Such a run is never stopped halfway.
    public let changesData: Bool

    init(_ arguments: Arguments, changesData: Bool) {
        self.arguments = arguments.argv
        self.changesData = changesData
    }

    /// The full argument list: `--json` first, so it stands before any `--`.
    public var argv: [String] { ["--json"] + arguments }
}

/// How a write runs: `-n` shows what it would do, `-y` does it.
public enum Confirmation: Sendable, Hashable {
    case preview
    case apply

    var flag: String {
        switch self {
        case .preview: "-n"
        case .apply: "-y"
        }
    }
}

/// A command that changes data. It runs twice: first as a preview, then for real.
public struct WriteCommand<Payload: ResultPayload>: Sendable, Hashable {
    let arguments: Arguments

    public func command(_ confirmation: Confirmation) -> Command<Payload> {
        var arguments = arguments
        arguments.options.insert(confirmation.flag, at: 0)
        return Command(arguments, changesData: confirmation == .apply)
    }
}

/// Which installation of a tool a command works on. Both empty: every tool chatkeep finds.
public struct Scope: Sendable, Hashable {
    public var tool: Tool?
    /// A Cursor installation, or `installation/profile` for one VS Code profile in it.
    public var profile: String?

    public init(tool: Tool? = nil, profile: String? = nil) {
        self.tool = tool
        self.profile = profile
    }

    public static let everyTool = Scope()

    var options: [String] {
        var options: [String] = []
        if let tool { options += ["--tool", tool.rawValue] }
        if let profile, !profile.isEmpty { options += ["--profile", profile] }
        return options
    }
}

/// What `mv` and `cp` rewrite: one project, or every path that matches.
public enum PathChange: Sendable, Hashable {
    /// A project (folder path, Cursor workspace hash, or Claude Code project folder name) to a new folder.
    case project(from: String, to: String)
    /// Every matching path from `from` to `to`; `from` is a regular expression when `regex` is set.
    case replace(from: String, to: String, regex: Bool)
}

/// Whether `combine` keeps the chats in the sources.
public enum TransferMode: Sendable, Hashable {
    case copy
    case move
}

/// Where `split` sends one chat instead of the suggestion.
public struct SplitAssignment: Sendable, Hashable {
    public var chat: String
    /// The target folder; `nil` keeps the chat out of the split.
    public var target: String?

    public init(chat: String, target: String?) {
        self.chat = chat
        self.target = target
    }

    var option: String { "\(chat)=\(target ?? "")" }
}

/// The words, then the options, then `--` and the positional values, so a value that starts
/// with a dash (a Claude Code project folder name does) is never read as a flag.
struct Arguments: Sendable, Hashable {
    var words: [String]
    var options: [String] = []
    var positionals: [String] = []

    var argv: [String] {
        words + options + (positionals.isEmpty ? [] : ["--"] + positionals)
    }
}

/// Every command of the JSON contract (docs/json.md), typed, so no caller builds argv by hand.
public enum CLI {
    // MARK: - Reads

    public static func version() -> Command<VersionInfo> {
        Command(Arguments(words: ["--version"]), changesData: false)
    }

    public static func projects(scope: Scope = .everyTool, unsaved: Bool = false, fresh: Bool = false) -> Command<ProjectList> {
        Command(Arguments(words: ["ls"], options: listOptions(scope, unsaved: unsaved, fresh: fresh)), changesData: false)
    }

    /// One project and its chats: a folder path, a Cursor workspace hash, or a Claude Code project folder name.
    public static func project(_ id: String, scope: Scope = .everyTool, unsaved: Bool = false, fresh: Bool = false) -> Command<ProjectDetails> {
        Command(
            Arguments(words: ["ls"], options: listOptions(scope, unsaved: unsaved, fresh: fresh), positionals: [id]),
            changesData: false
        )
    }

    public static func stats(scope: Scope = .everyTool, fresh: Bool = false) -> Command<Stats> {
        Command(Arguments(words: ["stats"], options: scope.options + (fresh ? ["--fresh"] : [])), changesData: false)
    }

    public static func history() -> Command<History> {
        Command(Arguments(words: ["history"]), changesData: false)
    }

    public static func queueList(full: Bool = false) -> Command<QueueList> {
        Command(Arguments(words: ["queue", "list"], options: full ? ["--full"] : []), changesData: false)
    }

    public static func cursorCacheStats(profile: String? = nil) -> Command<CursorCacheStats> {
        Command(Arguments(words: ["cursor", "cache", "stats"], options: Scope(profile: profile).options), changesData: false)
    }

    /// `cursor cache scan -n`: what a scan would read again. Reads nothing again itself.
    public static func cursorCacheCheck(profile: String? = nil) -> Command<CursorCacheCheck> {
        Command(Arguments(words: ["cursor", "cache", "scan"], options: ["-n"] + Scope(profile: profile).options), changesData: false)
    }

    /// Refreshes chatkeep's own index of Cursor; `full` rebuilds it from scratch. It writes the
    /// index, so it is never stopped halfway.
    public static func cursorCacheScan(full: Bool = false, profile: String? = nil) -> Command<CursorScan> {
        Command(
            Arguments(words: ["cursor", "cache", "scan"], options: (full ? ["--full"] : []) + Scope(profile: profile).options),
            changesData: true
        )
    }

    public static func claudeCacheStats() -> Command<ClaudeCacheStats> {
        Command(Arguments(words: ["claude", "cache", "stats"]), changesData: false)
    }

    public static func accounts() -> Command<AccountList> {
        Command(Arguments(words: ["claude", "accounts", "ls"]), changesData: false)
    }

    public static func accountsDoctor() -> Command<AccountsDoctor> {
        Command(Arguments(words: ["claude", "accounts", "doctor"]), changesData: false)
    }

    public static func syncProfiles() -> Command<SyncProfiles> {
        Command(Arguments(words: ["claude", "sync", "profiles"]), changesData: false)
    }

    public static func syncLog(last: Int = 30) -> Command<SyncLog> {
        Command(Arguments(words: ["claude", "sync", "log"], options: ["--last", String(max(last, 1))]), changesData: false)
    }

    public static func syncAutoStatus() -> Command<AutoSyncStatus> {
        Command(Arguments(words: ["claude", "sync", "auto", "status"]), changesData: false)
    }

    // MARK: - Writes without a preview

    /// Stores a command that was blocked by an open tool; `argv` is the error's `queue_args`.
    public static func queueAdd(_ argv: [String]) -> Command<QueueAdded> {
        Command(Arguments(words: ["queue", "add"], positionals: argv), changesData: true)
    }

    public static func nameAccount(_ account: String, name: String) -> Command<Report> {
        Command(Arguments(words: ["claude", "accounts", "name"], positionals: [account, name]), changesData: true)
    }

    public static func clearAccountName(_ account: String) -> Command<Report> {
        Command(Arguments(words: ["claude", "accounts", "name"], options: ["--clear"], positionals: [account]), changesData: true)
    }

    public static func autoSync(on: Bool) -> Command<AutoSyncReport> {
        Command(Arguments(words: ["claude", "sync", "auto", on ? "on" : "off"]), changesData: true)
    }

    // MARK: - Writes

    /// `mv`: repath chats after a project folder moved. `moveFolder` also moves the real folder.
    public static func move(_ change: PathChange, moveFolder: Bool = false, scope: Scope = .everyTool, unsaved: Bool = false) -> WriteCommand<Report> {
        WriteCommand(arguments: pathArguments("mv", change, alsoFolder: moveFolder, scope: scope, unsaved: unsaved))
    }

    /// `cp`: copy a project's chats to another folder. `copyFolder` also copies the real folder.
    public static func copy(_ change: PathChange, copyFolder: Bool = false, scope: Scope = .everyTool, unsaved: Bool = false) -> WriteCommand<Report> {
        WriteCommand(arguments: pathArguments("cp", change, alsoFolder: copyFolder, scope: scope, unsaved: unsaved))
    }

    /// `rm`: a project path, a Cursor workspace hash, or one chat id.
    public static func remove(_ target: String, scope: Scope = .everyTool) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["rm"], options: scope.options, positionals: [target]))
    }

    public static func split(
        _ source: String,
        into targets: [String],
        assignments: [SplitAssignment] = [],
        moveChats: Bool = false,
        scope: Scope = .everyTool
    ) -> WriteCommand<SplitReport> {
        var options = scope.options
        if moveChats { options.append("--move") }
        for assignment in assignments {
            options += ["--assign", assignment.option]
        }
        return WriteCommand(arguments: Arguments(words: ["split"], options: options, positionals: [source] + targets))
    }

    public static func combine(into target: String, from sources: [String], mode: TransferMode = .copy, scope: Scope = .everyTool) -> WriteCommand<Report> {
        let flag = mode == .move ? "--move" : "--copy"
        return WriteCommand(arguments: Arguments(words: ["combine"], options: scope.options + [flag], positionals: [target] + sources))
    }

    public static func export(_ project: String, to file: String, scope: Scope = .everyTool) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["export"], options: scope.options, positionals: [project, file]))
    }

    /// `import`: restore an archive, into `folder` instead of the original path when given.
    public static func importArchive(_ file: String, into folder: String? = nil, overwrite: Bool = false, scope: Scope = .everyTool) -> WriteCommand<Report> {
        let positionals = [file] + (folder.map { [$0] } ?? [])
        return WriteCommand(
            arguments: Arguments(words: ["import"], options: scope.options + (overwrite ? ["--overwrite"] : []), positionals: positionals)
        )
    }

    /// `cursor save`: attach an unsaved session (its number or hash) to a folder.
    public static func saveUnsaved(_ id: String, to folder: String, profile: String? = nil) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["cursor", "save"], options: Scope(profile: profile).options, positionals: [id, folder]))
    }

    /// `cursor rx`: rebuild registry references, rewrite leftover paths and clear caches.
    public static func reindex(_ target: String, profile: String? = nil) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["cursor", "rx"], options: Scope(profile: profile).options, positionals: [target]))
    }

    public static func clearCursorCache(profile: String? = nil) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["cursor", "cache", "clear"], options: Scope(profile: profile).options))
    }

    public static func queueRemove(_ ids: [String]) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["queue", "rm"], positionals: ids))
    }

    public static func queueClear() -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["queue", "clear"]))
    }

    /// Resets failed or skipped entries to pending; no ids means every one of them.
    public static func queueRetry(_ ids: [String] = []) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["queue", "retry"], positionals: ids))
    }

    public static func queueExecute(continueOnError: Bool = false) -> WriteCommand<QueueExecution> {
        WriteCommand(arguments: Arguments(words: ["queue", "execute"], options: continueOnError ? ["--continue-on-error"] : []))
    }

    /// Make the chats `from` lists show up for `to` too; `to` defaults to the signed-in account.
    public static func copyAccountChats(from: String, to: String? = nil) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["claude", "accounts", "cp"], positionals: [from] + (to.map { [$0] } ?? [])))
    }

    public static func pruneAccounts() -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["claude", "accounts", "prune"]))
    }

    public static func clearClaudeCache() -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["claude", "cache", "clear"]))
    }

    /// Run a sync now: one profile, or every profile when `profile` is `nil`.
    public static func sync(profile: String? = nil) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["claude", "sync"], positionals: profile.map { [$0] } ?? []))
    }

    /// Create or replace a sync profile with exactly these accounts.
    public static func setSyncProfile(_ name: String, accounts: [String]) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["claude", "sync", "set"], positionals: [name] + accounts))
    }

    public static func addToSyncProfile(_ name: String, accounts: [String]) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["claude", "sync", "add"], positionals: [name] + accounts))
    }

    /// Take accounts out of a profile, or remove the whole profile when `accounts` is empty.
    public static func removeFromSyncProfile(_ name: String, accounts: [String] = []) -> WriteCommand<Report> {
        WriteCommand(arguments: Arguments(words: ["claude", "sync", "rm"], positionals: [name] + accounts))
    }

    // MARK: - Shared options

    private static func listOptions(_ scope: Scope, unsaved: Bool, fresh: Bool) -> [String] {
        scope.options + (unsaved ? ["--unsaved"] : []) + (fresh ? ["--fresh"] : [])
    }

    private static func pathArguments(_ word: String, _ change: PathChange, alsoFolder: Bool, scope: Scope, unsaved: Bool) -> Arguments {
        var options = scope.options
        if unsaved { options.append("--unsaved") }
        if alsoFolder { options.append("--project") }
        switch change {
        case .project(let from, let to):
            return Arguments(words: [word], options: options, positionals: [from, to])
        case .replace(let from, let to, let regex):
            options += ["--replace", from, to]
            if regex { options.append("--regex") }
            return Arguments(words: [word], options: options)
        }
    }
}
