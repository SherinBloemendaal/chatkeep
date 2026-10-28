import Foundation

/// `stats`: the shape differs per tool, so the result's `tool` picks the case.
public enum Stats: Sendable, Hashable, ResultPayload {
    case cursor(CursorStats)
    case claude(ClaudeStats)
    /// Stats of a tool this app does not know yet; their shape is unknown, so they are skipped.
    case unknownTool(String)

    public init(from decoder: any Decoder, tool: Tool?) throws {
        switch tool {
        case .cursor: self = .cursor(try CursorStats(from: decoder))
        case .claude: self = .claude(try ClaudeStats(from: decoder))
        case .unknown(let raw): self = .unknownTool(raw)
        case nil:
            throw DecodingError.dataCorrupted(
                DecodingError.Context(codingPath: decoder.codingPath, debugDescription: "stats result without a tool")
            )
        }
    }

    public var cursor: CursorStats? {
        if case .cursor(let stats) = self { stats } else { nil }
    }

    public var claude: ClaudeStats? {
        if case .claude(let stats) = self { stats } else { nil }
    }
}

public struct CursorStats: Codable, Sendable, Hashable {
    public struct Profile: Codable, Sendable, Hashable {
        public var name: String
        /// The installation's user data folder.
        public var root: String
        public var workspaces: Int
        public var chats: Int
        public var subagents: Int
        /// `nil` for a VS Code profile: it shares the installation's database.
        public var globalDbBytes: Int64?
        public var userProfile: Bool
    }

    public struct KindCount: Codable, Sendable, Hashable {
        public var kind: ProjectKind
        public var workspaces: Int
    }

    public struct Usage: Codable, Sendable, Hashable {
        public var conversations: Int
        public var costCents: Int64
        public var requests: Int64
        public var costChats: Int
        public var contextTokens: Int64
        public var contextChats: Int
        public var inputTokens: Int64
        public var outputTokens: Int64
        public var tokenChats: Int
    }

    public struct ModelCount: Codable, Sendable, Hashable {
        public var model: String
        public var chats: Int
    }

    public struct Month: Codable, Sendable, Hashable {
        /// `YYYY-MM`.
        public var month: String
        public var chats: Int
    }

    public struct Busy: Codable, Sendable, Hashable {
        public var name: String
        public var kind: ProjectKind?
        public var chats: Int
    }

    public struct Database: Codable, Sendable, Hashable {
        public var name: String
        public var kind: ProjectKind?
        public var bytes: Int64
    }

    public var indexAsOf: Date?
    public var indexRefreshing: Bool
    public var installations: Int
    public var userProfiles: Int
    public var profiles: [Profile]
    public var workspaces: Int
    public var byKind: [KindCount]
    public var chats: Int
    public var subagents: Int
    public var archived: Int
    public var usage: Usage
    public var models: [ModelCount]
    public var perMonth: [Month]
    public var busiest: [Busy]
    public var globalDbBytes: Int64?
    public var workspaceDbs: [Database]
}

public struct ClaudeStats: Codable, Sendable, Hashable {
    public struct Tokens: Codable, Sendable, Hashable {
        public var input: Int64
        public var output: Int64
        public var cacheWrite: Int64
        public var cacheRead: Int64

        public var total: Int64 { input + output + cacheWrite + cacheRead }
    }

    public struct ModelCount: Codable, Sendable, Hashable {
        public var model: String
        public var sessions: Int
    }

    public struct Month: Codable, Sendable, Hashable {
        /// `YYYY-MM`.
        public var month: String
        public var sessions: Int
    }

    public struct Busy: Codable, Sendable, Hashable {
        public var name: String
        public var sessions: Int
    }

    public struct Large: Codable, Sendable, Hashable {
        public var name: String
        public var sizeBytes: Int64
    }

    public var projects: Int
    /// Projects whose folder is gone.
    public var missing: Int
    public var sessions: Int
    public var subagents: Int
    public var memoryFiles: Int
    public var sizeBytes: Int64
    public var tokens: Tokens
    public var subagentTokens: Tokens
    public var models: [ModelCount]
    public var perMonth: [Month]
    public var busiest: [Busy]
    public var largest: [Large]
}
