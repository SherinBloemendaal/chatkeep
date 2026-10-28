import Foundation

/// A field with a fixed set of words, such as a status or a kind. A word this app does not know
/// yet (from a newer chatkeep) decodes to `unknown` with the word as written, never to an error,
/// so one new value cannot hide a whole list.
public protocol TolerantValue: Codable, Sendable, Hashable {
    /// Every case except `unknown`.
    static var known: [Self] { get }
    /// A word this app does not know, as the tool wrote it.
    static func unknown(_ raw: String) -> Self
    /// The word the tool writes for this case.
    var rawValue: String { get }
}

extension TolerantValue {
    public init(rawValue: String) {
        self = Self.known.first { $0.rawValue == rawValue } ?? .unknown(rawValue)
    }

    public init(from decoder: any Decoder) throws {
        self.init(rawValue: try decoder.singleValueContainer().decode(String.self))
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }

    /// Whether the word is one this app knows.
    public var isKnown: Bool { Self.known.contains(self) }
}
