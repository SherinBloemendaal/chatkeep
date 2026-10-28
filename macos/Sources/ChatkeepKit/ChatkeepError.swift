import Foundation

/// Why a chatkeep run did not succeed: an error document from the tool, or a problem running it.
public struct ChatkeepError: Error, Sendable, Equatable, LocalizedError {
    public enum Kind: Sendable, Hashable {
        /// The arguments could not be parsed.
        case usage
        /// Something a terminal would have asked for is missing.
        case missingArgument
        /// A write got neither `-n` nor `-y`.
        case needsConfirmation
        /// Cursor or Claude Code still uses the data; `queueArgs` can queue the command.
        case toolRunning
        /// The command has no JSON output.
        case unsupportedInJSON
        /// Anything else the tool refused or could not do.
        case failed
        /// A kind this app does not know yet.
        case other(String)

        /// The chatkeep tool is not in the app and not on PATH.
        case toolNotFound
        /// The tool could not be started.
        case couldNotStart
        /// The tool printed something that is not a chatkeep document.
        case unreadableOutput
        /// The run was stopped before it finished.
        case cancelled

        public init(wire: String) {
            self = switch wire {
            case "usage": .usage
            case "missing_argument": .missingArgument
            case "needs_confirmation": .needsConfirmation
            case "tool_running": .toolRunning
            case "unsupported_in_json": .unsupportedInJSON
            case "failed": .failed
            default: .other(wire)
            }
        }
    }

    public var kind: Kind
    public var message: String
    public var hint: String?
    /// With `toolRunning`: the arguments `queue add` would store. `nil` when it cannot be queued.
    public var queueArgs: [String]?
    /// The command's words, as the tool named them.
    public var command: String?
    public var exitCode: Int32?
    /// The whole document, to read the results that finished before the error.
    public var document: Data?

    public init(
        kind: Kind,
        message: String,
        hint: String? = nil,
        queueArgs: [String]? = nil,
        command: String? = nil,
        exitCode: Int32? = nil,
        document: Data? = nil
    ) {
        self.kind = kind
        self.message = message
        self.hint = hint
        self.queueArgs = queueArgs
        self.command = command
        self.exitCode = exitCode
        self.document = document
    }

    public var errorDescription: String? { message }
    public var recoverySuggestion: String? { hint }

    /// Whether the command can be stored in the queue to run once the tools are closed.
    public var canQueue: Bool { kind == .toolRunning && !(queueArgs ?? []).isEmpty }

    /// The results that finished before the error, such as the Claude Code part of a command
    /// whose Cursor part failed, or every entry of a `queue execute` that stopped early.
    public func partialResults<Payload: ResultPayload>(as payload: Payload.Type = Payload.self) -> [ToolResult<Payload>] {
        guard let document, let partial = try? JSONDecoder.chatkeep.decode(PartialResults<Payload>.self, from: document) else {
            return []
        }
        return partial.results
    }

    static func unreadable(document: Data, exitCode: Int32?, reason: (any Error)?) -> ChatkeepError {
        let text = String(decoding: document.prefix(400), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        return ChatkeepError(
            kind: .unreadableOutput,
            message: text.isEmpty
                ? "The chatkeep tool stopped without an answer."
                : "The chatkeep tool gave an answer this app cannot read.",
            hint: reason.map { "Details: \($0.localizedDescription)" } ?? (text.isEmpty ? nil : text),
            exitCode: exitCode,
            document: document
        )
    }

    static let cancelled = ChatkeepError(kind: .cancelled, message: "Stopped.")
}

/// The `results` of an error document.
private struct PartialResults<Payload: ResultPayload>: Decodable {
    var results: [ToolResult<Payload>]
}
