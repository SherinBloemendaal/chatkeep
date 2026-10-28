import Foundation

/// `ls`: every project of one tool.
public struct ProjectList: Codable, Sendable, Hashable, ResultPayload {
    /// Cursor: when the index last read the rows. Always `nil` for Claude Code.
    public var indexAsOf: Date?
    /// Cursor: a background refresh of the index runs now.
    public var indexRefreshing: Bool
    /// Missing folders first, then the largest.
    public var projects: [Project]
}

/// `ls ID`: one project with its chats, once per Cursor installation that has it.
public struct ProjectDetails: Codable, Sendable, Hashable, ResultPayload {
    public var indexAsOf: Date?
    public var indexRefreshing: Bool
    public var details: [ProjectDetail]
}

public struct ProjectDetail: Codable, Sendable, Hashable {
    public var project: Project
    /// The newest change of its chats.
    public var lastActive: Date?
    /// Every chat, newest first.
    public var chats: [Chat]
}

/// A project as one tool keeps it: a Cursor workspace or a Claude Code project folder.
public struct Project: Codable, Sendable, Hashable, Identifiable {
    public enum Destination: TolerantValue {
        case present
        case missing
        /// A kind with no folder of its own, or no known path.
        case noFolder
        /// A word this app does not know yet, as the tool wrote it.
        case unknown(String)

        public static let known: [Self] = [.present, .missing, .noFolder]

        public var rawValue: String {
            switch self {
            case .present: "present"
            case .missing: "missing"
            case .noFolder: "none"
            case .unknown(let raw): raw
            }
        }
    }

    public var tool: Tool
    /// The `id` field. Cursor: the workspace hash (or the id of an unsaved or empty window).
    /// Claude Code: the folder name in `~/.claude/projects`.
    public var projectID: String
    public var kind: ProjectKind
    /// The display label, with the home folder as `~`.
    public var name: String
    public var path: String?
    public var uri: String?
    public var storageDir: String
    public var install: String?
    public var profile: String?
    /// `install`, or `install/profile` for another VS Code profile. The value `--profile` takes.
    public var profileLabel: String?
    public var chats: Int
    public var subagents: Int
    public var archived: Int?
    public var sizeBytes: Int64
    /// Cursor: the newest change of its chats, agent chats included. Claude Code: the newest
    /// change of its session transcripts. `nil` without chats.
    public var lastActive: Date?
    public var destination: Destination
    public var destinationMissing: Bool
    /// Claude Code only: files in the project's `memory/` folder.
    public var memoryFiles: Int?

    /// Unique across tools and Cursor installations: the folder the tool keeps the chats in.
    /// (The same Cursor workspace hash can exist in several installations.)
    public var id: String { storageDir }

    public var isUnsaved: Bool { kind == .unsaved }

    private enum CodingKeys: String, CodingKey {
        case tool, kind, name, path, uri, storageDir, install, profile, profileLabel, chats, subagents
        case archived, sizeBytes, lastActive, destination, destinationMissing, memoryFiles
        case projectID = "id"
    }
}

/// What a project is. Cursor: a folder, a `.code-workspace` file, a window never saved to a
/// folder, an empty window, or a remote one. Claude Code: always `project`.
public enum ProjectKind: TolerantValue {
    case folder
    case codeWorkspace
    case unsaved
    case emptyWindow
    case remote
    case project
    /// A kind this app does not know yet, as the tool wrote it.
    case unknown(String)

    public static let known: [Self] = [.folder, .codeWorkspace, .unsaved, .emptyWindow, .remote, .project]

    public var rawValue: String {
        switch self {
        case .folder: "folder"
        case .codeWorkspace: "code-workspace"
        case .unsaved: "unsaved"
        case .emptyWindow: "empty-window"
        case .remote: "remote"
        case .project: "project"
        case .unknown(let raw): raw
        }
    }
}

/// One chat (Cursor composer, or Claude Code session).
public struct Chat: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var title: String?
    /// Cursor only; Claude Code does not record it.
    public var createdAt: Date?
    public var updatedAt: Date?
    /// Cursor only.
    public var archived: Bool?
    /// A chat an agent started (Cursor only).
    public var subagent: Bool
    public var model: String?
    public var tokens: ChatTokens?
    /// Cursor only.
    public var costCents: Int?
    /// Cursor only.
    public var requests: Int?
    /// Claude Code only: subagent transcripts of the session.
    public var subagents: Int?
    /// Claude Code only: the transcript plus its session folder.
    public var sizeBytes: Int64?
    /// Claude Code only: the folder the session started in.
    public var cwd: String?
    /// Claude Code only: the `.jsonl` transcript.
    public var transcript: String?
}

/// Token counts of a chat. Cursor has `context`; Claude Code has the two cache counts.
public struct ChatTokens: Codable, Sendable, Hashable {
    public var input: Int64
    public var output: Int64
    /// Cursor: the context window in use at the last turn.
    public var context: Int64?
    public var cacheWrite: Int64?
    public var cacheRead: Int64?

    /// Every token the chat counted.
    public var total: Int64 {
        input + output + (cacheWrite ?? 0) + (cacheRead ?? 0)
    }
}

/// `--version`.
public struct VersionInfo: Codable, Sendable, Hashable, ResultPayload {
    public var version: String
}
