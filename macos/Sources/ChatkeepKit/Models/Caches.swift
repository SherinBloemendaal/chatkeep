import Foundation

/// `cursor cache stats`: chatkeep's index of Cursor's chats.
public struct CursorCacheStats: Codable, Sendable, Hashable, ResultPayload {
    /// Who holds the refresh lock.
    public struct Refresh: Codable, Sendable, Hashable {
        public var pid: Int32
        public var startedAt: Date?
        public var alive: Bool
    }

    public struct Stale: Codable, Sendable, Hashable {
        public var sources: Int
        public var reasons: [String]
    }

    public struct Install: Codable, Sendable, Hashable, Identifiable {
        public var name: String
        /// The installation's user data folder.
        public var root: String
        /// False for rows of an installation that is no longer on this machine.
        public var found: Bool
        public var scannedAt: Date?
        public var dirtyAt: Date?
        public var scanDurationMs: Int64?
        public var workspaces: Int
        public var chats: Int
        public var subagents: Int
        /// `nil` when unknown.
        public var stale: Stale?

        public var id: String { name }
    }

    /// `index.db`.
    public var path: String
    public var exists: Bool
    /// With `-wal` and `-shm`.
    public var sizeBytes: Int64
    public var schema: Int?
    public var refreshedAt: Date?
    public var spawnedAt: Date?
    public var refresh: Refresh?
    /// The last line of the background refresh log.
    public var lastLog: String?
    public var installs: [Install]
}

/// `cursor cache scan -n`: what a scan would read again.
public struct CursorCacheCheck: Codable, Sendable, Hashable, ResultPayload {
    public struct Install: Codable, Sendable, Hashable, Identifiable {
        public var name: String
        public var root: String
        /// Whether the installation was ever scanned.
        public var scanned: Bool
        public var staleSources: Int?
        /// What changed; empty when up to date.
        public var reasons: [String]

        public var id: String { name }
    }

    public var installs: [Install]
}

/// `cursor cache scan`: what a scan read again.
public struct CursorScan: Codable, Sendable, Hashable, ResultPayload {
    public struct Install: Codable, Sendable, Hashable, Identifiable {
        public var name: String
        public var workspaces: Int
        public var chats: Int
        public var subagents: Int
        public var changedSources: Int
        public var chatsRead: Int
        public var durationMs: Int64

        public var id: String { name }
    }

    public var full: Bool
    public var durationMs: Int64
    public var path: String
    public var installs: [Install]
}

/// `claude cache stats`: chatkeep's index of what every Claude Code transcript says.
public struct ClaudeCacheStats: Codable, Sendable, Hashable, ResultPayload {
    /// False when the index is turned off (`CHATKEEP_NO_INDEX`).
    public var enabled: Bool
    public var exists: Bool
    public var path: String
    public var sizeBytes: Int64
    public var transcripts: Int?
    public var refreshedAt: Date?
    public var schema: Int?
}
