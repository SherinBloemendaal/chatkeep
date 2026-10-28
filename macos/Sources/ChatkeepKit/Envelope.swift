import Foundation

/// The AI coding tools whose chats chatkeep looks after.
public enum Tool: TolerantValue, CaseIterable, Identifiable {
    case cursor
    case claude
    /// A tool this app does not know yet, as the tool wrote it.
    case unknown(String)

    public static let known: [Self] = [.cursor, .claude]
    /// The tools a command can be limited to with `--tool`.
    public static let allCases: [Self] = known

    public var id: String { rawValue }

    public var rawValue: String {
        switch self {
        case .cursor: "cursor"
        case .claude: "claude"
        case .unknown(let raw): raw
        }
    }

    /// The name people know the tool by.
    public var label: String {
        switch self {
        case .cursor: "Cursor"
        case .claude: "Claude Code"
        case .unknown(let raw): raw
        }
    }
}

/// The `data` of one result. A type decodes itself from the decoder, and may look at the tool the
/// result is about when the shape differs per tool (as with `stats`).
public protocol ResultPayload: Sendable {
    init(from decoder: any Decoder, tool: Tool?) throws
}

extension ResultPayload where Self: Decodable {
    public init(from decoder: any Decoder, tool: Tool?) throws {
        try self.init(from: decoder)
    }
}

/// One entry of `results`: the tool it is about (`nil` for chatkeep's own state) and its data.
public struct ToolResult<Payload: ResultPayload>: Sendable, Decodable {
    public var tool: Tool?
    public var data: Payload

    public init(tool: Tool?, data: Payload) {
        self.tool = tool
        self.data = data
    }

    private enum CodingKeys: String, CodingKey {
        case tool
        case data
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        tool = try container.decodeIfPresent(Tool.self, forKey: .tool)
        data = try Payload(from: container.superDecoder(forKey: .data), tool: tool)
    }
}

/// A successful run: the command's words, whether it was a dry run, and one result per tool.
public struct Response<Payload: ResultPayload>: Sendable, Decodable {
    public var command: String
    public var dryRun: Bool
    public var results: [ToolResult<Payload>]

    public init(command: String, dryRun: Bool, results: [ToolResult<Payload>]) {
        self.command = command
        self.dryRun = dryRun
        self.results = results
    }

    /// The data of the first result, for commands that always give exactly one.
    public var data: Payload? { results.first?.data }

    public func data(for tool: Tool) -> Payload? {
        results.first { $0.tool == tool }?.data
    }
}

/// The parts every document has, read before deciding how to decode the rest.
struct EnvelopeHead: Decodable {
    struct Failure: Decodable {
        var kind: String
        var message: String
        var hint: String?
        var queueArgs: [String]?
    }

    var ok: Bool
    var command: String?
    var error: Failure?
}

extension JSONDecoder {
    /// Reads chatkeep's JSON: snake_case keys, ISO 8601 times in UTC.
    public static var chatkeep: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }
}

/// Decodes one document of `chatkeep --json`: the typed response, or the error it describes.
public enum EnvelopeDecoder {
    public static func decode<Payload: ResultPayload>(
        _ document: Data,
        as payload: Payload.Type = Payload.self,
        exitCode: Int32? = nil
    ) throws(ChatkeepError) -> Response<Payload> {
        let decoder = JSONDecoder.chatkeep
        let head: EnvelopeHead
        do {
            head = try decoder.decode(EnvelopeHead.self, from: document)
        } catch {
            throw ChatkeepError.unreadable(document: document, exitCode: exitCode, reason: error)
        }
        if !head.ok {
            guard let failure = head.error else {
                throw ChatkeepError.unreadable(document: document, exitCode: exitCode, reason: nil)
            }
            throw ChatkeepError(
                kind: ChatkeepError.Kind(wire: failure.kind),
                message: failure.message,
                hint: failure.hint,
                queueArgs: failure.queueArgs,
                command: head.command,
                exitCode: exitCode,
                document: document
            )
        }
        do {
            return try decoder.decode(Response<Payload>.self, from: document)
        } catch {
            throw ChatkeepError.unreadable(document: document, exitCode: exitCode, reason: error)
        }
    }
}
