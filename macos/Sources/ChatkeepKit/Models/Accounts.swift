import Foundation

/// `claude accounts ls`: one row per organization of each Claude desktop app account.
public struct AccountList: Codable, Sendable, Hashable, ResultPayload {
    public var accounts: [Account]
}

public struct Account: Codable, Sendable, Hashable, Identifiable {
    /// The account id.
    public var account: String
    /// Its name, else its e-mail address, else the start of its id.
    public var label: String
    /// The name given with `claude accounts name`.
    public var name: String?
    public var email: String?
    /// The organization id.
    public var organization: String
    /// `account/organization`: the value sync profiles store and accept.
    public var member: String
    /// Chats the app lists for it.
    public var chats: Int
    /// Of those, chats whose transcript is on this machine.
    public var onDisk: Int
    public var signedIn: Bool
    /// The folder the app keeps the list in.
    public var dir: String

    public var id: String { member }
}

/// `claude accounts doctor`.
public struct AccountsDoctor: Codable, Sendable, Hashable, ResultPayload {
    /// The account the desktop app is signed in with.
    public struct Desktop: Codable, Sendable, Hashable {
        public var account: String
        public var label: String
        public var name: String?
        public var email: String?
        public var organizations: [String]
    }

    /// An account seen in a Claude Code session or a login.
    public struct Seen: Codable, Sendable, Hashable {
        public var account: String
        public var label: String
        public var email: String?
        public var organization: String?
        public var organizationName: String?
    }

    public struct Finding: Codable, Sendable, Hashable {
        public enum Level: TolerantValue {
            case ok
            case warn
            case info
            case hint
            /// A word this app does not know yet, as the tool wrote it.
            case unknown(String)

            public static let known: [Self] = [.ok, .warn, .info, .hint]

            public var rawValue: String {
                switch self {
                case .ok: "ok"
                case .warn: "warn"
                case .info: "info"
                case .hint: "hint"
                case .unknown(let raw): raw
                }
            }
        }

        public var level: Level
        public var text: String
    }

    /// The desktop app keeps data on this machine.
    public var desktopApp: Bool
    public var desktop: Desktop?
    /// The account of the Claude Code session chatkeep runs in.
    public var session: Seen?
    /// The last login in `.claude.json`.
    public var login: Seen?
    /// The `claude` command on PATH.
    public var claudeCommand: String?
    /// Entries `claude accounts prune` would remove.
    public var staleCaches: Int
    public var findings: [Finding]
}

/// `claude sync profiles`.
public struct SyncProfiles: Codable, Sendable, Hashable, ResultPayload {
    public struct Profile: Codable, Sendable, Hashable, Identifiable {
        public var name: String
        public var accounts: [Member]

        public var id: String { name }
    }

    public struct Member: Codable, Sendable, Hashable, Identifiable {
        public var account: String
        public var label: String
        public var name: String?
        public var email: String?
        public var organization: String?
        /// The stored `account/organization`.
        public var member: String
        public var signedIn: Bool

        public var id: String { member }
    }

    /// Sorted by name.
    public var profiles: [Profile]
    public var auto: AutoSyncStatus
}

/// `claude sync auto status`: whether the background sync runs.
public struct AutoSyncStatus: Codable, Sendable, Hashable, ResultPayload {
    public enum State: TolerantValue {
        /// Installed and running.
        case on
        /// Installed but not running.
        case stopped
        case off
        /// A word this app does not know yet, as the tool wrote it.
        case unknown(String)

        public static let known: [Self] = [.on, .stopped, .off]

        public var rawValue: String {
            switch self {
            case .on: "on"
            case .stopped: "stopped"
            case .off: "off"
            case .unknown(let raw): raw
            }
        }
    }

    public var state: State
    public var installed: Bool
    public var running: Bool
    public var logPath: String
}

/// `claude sync log`: the newest changes, oldest first.
public struct SyncLog: Codable, Sendable, Hashable, ResultPayload {
    public struct Entry: Codable, Sendable, Hashable {
        public enum Action: TolerantValue {
            case add
            case update
            case remove
            /// A message from the watcher; `title` holds its text.
            case note
            /// A word this app does not know yet, as the tool wrote it.
            case unknown(String)

            public static let known: [Self] = [.add, .update, .remove, .note]

            public var rawValue: String {
                switch self {
                case .add: "add"
                case .update: "update"
                case .remove: "remove"
                case .note: "note"
                case .unknown(let raw): raw
                }
            }
        }

        public enum Source: TolerantValue {
            /// A sync someone ran.
            case manual
            /// The background sync.
            case watch
            /// A word this app does not know yet, as the tool wrote it.
            case unknown(String)

            public static let known: [Self] = [.manual, .watch]

            public var rawValue: String {
                switch self {
                case .manual: "manual"
                case .watch: "watch"
                case .unknown(let raw): raw
                }
            }
        }

        /// When (the last time, for a repeated change).
        public var at: Date?
        public var profile: String?
        public var action: Action
        public var chat: String?
        /// The chat title, or the text of a note.
        public var title: String?
        /// The `account/organization` whose list changed.
        public var account: String?
        public var source: Source
        /// For an update: what differed.
        public var changed: [String]
        /// How often this happened in a row.
        public var times: Int
    }

    public var entries: [Entry]
}
