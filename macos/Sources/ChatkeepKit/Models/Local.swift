import Foundation

/// `history`: chatkeep's command log.
public struct History: Codable, Sendable, Hashable, ResultPayload {
    public struct Entry: Codable, Sendable, Hashable {
        public enum Outcome: TolerantValue {
            case ok
            case error
            /// A word this app does not know yet, as the tool wrote it.
            case unknown(String)

            public static let known: [Self] = [.ok, .error]

            public var rawValue: String {
                switch self {
                case .ok: "ok"
                case .error: "error"
                case .unknown(let raw): raw
                }
            }
        }

        public var at: Date?
        /// `mv`, `claude accounts`, ...
        public var command: String?
        public var args: [String]
        public var outcome: Outcome?
        public var durationMs: Int64?
    }

    /// The log file.
    public var path: String
    /// Oldest first.
    public var entries: [Entry]
    /// Lines that are not JSON; they are left out.
    public var unreadable: Int
}

/// One stored queue entry.
public struct QueueEntry: Codable, Sendable, Hashable, Identifiable {
    public enum Status: TolerantValue {
        case pending
        case running
        case failed
        case skipped
        /// A word this app does not know yet, as the tool wrote it.
        case unknown(String)

        public static let known: [Self] = [.pending, .running, .failed, .skipped]

        public var rawValue: String {
            switch self {
            case .pending: "pending"
            case .running: "running"
            case .failed: "failed"
            case .skipped: "skipped"
            case .unknown(let raw): raw
            }
        }
    }

    /// 8 hex characters.
    public var id: String
    /// Run order among the pending entries, from 1; `nil` when not pending.
    public var position: Int?
    public var status: Status
    /// Failed because a `queue execute` stopped while it ran.
    public var interrupted: Bool
    /// The stored command, without `chatkeep`.
    public var argv: [String]
    /// `argv` quoted for a shell.
    public var command: String
    public var tool: Tool?
    /// The folder it was queued in.
    public var cwd: String
    public var profile: String?
    public var createdAt: Date?
    /// The chatkeep version that queued it.
    public var toolVersion: String
    public var warnings: [String]
    /// Why it failed or was skipped.
    public var lastError: String?

    public var canRetry: Bool { status == .failed || status == .skipped }
}

/// `queue list`.
public struct QueueList: Codable, Sendable, Hashable, ResultPayload {
    /// In queue order.
    public var entries: [QueueEntry]

    public var pendingCount: Int { entries.count { $0.status == .pending } }
}

/// `queue add`.
public struct QueueAdded: Codable, Sendable, Hashable, ResultPayload {
    public var entry: QueueEntry
}
