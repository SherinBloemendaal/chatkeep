import Foundation

/// A result that carries a write's report.
public protocol ReportPayload: ResultPayload {
    var report: Report { get }
}

/// What a write did, or in a preview would do. Every write command's `data` is (or holds) one.
public struct Report: Codable, Sendable, Hashable, ReportPayload {
    /// What ran, for display only.
    public var title: String
    public var warnings: [String]
    /// Items left alone (ids or lines).
    public var skipped: [String]
    /// One line per change made or planned.
    public var applied: [String]
    /// Cursor storage keys that were rewritten.
    public var rewrittenKeys: [String]

    public init(title: String, warnings: [String] = [], skipped: [String] = [], applied: [String] = [], rewrittenKeys: [String] = []) {
        self.title = title
        self.warnings = warnings
        self.skipped = skipped
        self.applied = applied
        self.rewrittenKeys = rewrittenKeys
    }

    public var report: Report { self }

    /// Nothing changed and nothing was left alone.
    public var isEmpty: Bool { applied.isEmpty && skipped.isEmpty }
}

/// `split`: the report plus, per chat, where the suggestion put it and where it goes.
public struct SplitReport: Sendable, Hashable, Decodable, ReportPayload {
    public struct ChatPlan: Codable, Sendable, Hashable, Identifiable {
        /// The `CHAT` of `--assign`.
        public var id: String
        public var title: String?
        /// Targets the suggestion picked, absolute; empty for none.
        public var suggested: [String]
        /// Targets it goes to in this run, after `--assign`; empty for none.
        public var targets: [String]
    }

    public var report: Report
    /// One per top-level chat of the source, sorted by id.
    public var chats: [ChatPlan]

    private enum CodingKeys: String, CodingKey {
        case chats
    }

    public init(from decoder: any Decoder) throws {
        report = try Report(from: decoder)
        chats = try decoder.container(keyedBy: CodingKeys.self).decode([ChatPlan].self, forKey: .chats)
    }
}

/// `claude sync auto on` and `off`: the report plus the status afterwards.
public struct AutoSyncReport: Sendable, Hashable, Decodable, ReportPayload {
    public var report: Report
    public var auto: AutoSyncStatus

    private enum CodingKeys: String, CodingKey {
        case auto
    }

    public init(from decoder: any Decoder) throws {
        report = try Report(from: decoder)
        auto = try decoder.container(keyedBy: CodingKeys.self).decode(AutoSyncStatus.self, forKey: .auto)
    }
}

/// `queue execute`: the summary report plus one entry per pending queue entry.
public struct QueueExecution: Sendable, Hashable, Decodable, ReportPayload {
    public struct Entry: Decodable, Sendable, Hashable, Identifiable {
        public enum Plan: TolerantValue {
            case ok
            case warnings
            /// The plan failed; the entry is skipped.
            case skip
            /// A word this app does not know yet, as the tool wrote it.
            case unknown(String)

            public static let known: [Self] = [.ok, .warnings, .skip]

            public var rawValue: String {
                switch self {
                case .ok: "ok"
                case .warnings: "warnings"
                case .skip: "skip"
                case .unknown(let raw): raw
                }
            }
        }

        public enum Status: TolerantValue {
            /// A preview, or the run stopped before it.
            case planned
            case done
            case failed
            case skipped
            /// A tool was still open; it waits again.
            case blocked
            /// A word this app does not know yet, as the tool wrote it.
            case unknown(String)

            public static let known: [Self] = [.planned, .done, .failed, .skipped, .blocked]

            public var rawValue: String {
                switch self {
                case .planned: "planned"
                case .done: "done"
                case .failed: "failed"
                case .skipped: "skipped"
                case .blocked: "blocked"
                case .unknown(let raw): raw
                }
            }
        }

        public var id: String
        public var argv: [String]
        public var command: String
        public var plan: Plan
        public var planWarnings: [String]
        public var planError: String?
        public var status: Status
        public var error: String?
        /// One report per tool it ran for.
        public var results: [EntryReport]
    }

    /// A report of one queued command, with whether it was a preview.
    public struct EntryReport: Sendable, Hashable, Decodable {
        public var report: Report
        public var dryRun: Bool

        private enum CodingKeys: String, CodingKey {
            case dryRun
        }

        public init(from decoder: any Decoder) throws {
            report = try Report(from: decoder)
            dryRun = try decoder.container(keyedBy: CodingKeys.self).decode(Bool.self, forKey: .dryRun)
        }
    }

    public var report: Report
    public var entries: [Entry]

    private enum CodingKeys: String, CodingKey {
        case entries
    }

    public init(from decoder: any Decoder) throws {
        report = try Report(from: decoder)
        entries = try decoder.container(keyedBy: CodingKeys.self).decode([Entry].self, forKey: .entries)
    }
}
