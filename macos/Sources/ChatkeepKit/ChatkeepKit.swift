import Foundation

/// Finds the `chatkeep` command line tool that the app runs for every action.
///
/// A built app carries its own copy in `Contents/Resources/chatkeep` (see build-app.sh), so the
/// app and the engine always come from the same release. Under `swift run` there is no app
/// bundle, so it falls back to the first `chatkeep` on PATH.
public struct ChatkeepCLI: Sendable {
    public static let executableName = "chatkeep"

    public enum LocateError: Error, Equatable, LocalizedError {
        case notFound

        public var errorDescription: String? {
            "The chatkeep tool was not found in the app or on your PATH. Reinstall Chatkeep."
        }
    }

    /// The folder that holds the bundled tool: `Contents/Resources` of the running app.
    public var resourceDirectory: URL?
    /// A PATH-style list of folders, searched in order when the app carries no tool.
    public var searchPath: String?

    public init(
        resourceDirectory: URL? = Bundle.main.resourceURL,
        searchPath: String? = ProcessInfo.processInfo.environment["PATH"]
    ) {
        self.resourceDirectory = resourceDirectory
        self.searchPath = searchPath
    }

    /// The tool to run: the bundled copy when there is one, otherwise the first one on PATH.
    public func locate() throws(LocateError) -> URL {
        let bundled = resourceDirectory.map { [$0.appending(path: Self.executableName)] } ?? []
        let onPath = (searchPath ?? "")
            .split(separator: ":")
            .map { URL(filePath: String($0), directoryHint: .isDirectory).appending(path: Self.executableName) }

        guard let found = (bundled + onPath).first(where: Self.isExecutableFile) else {
            throw .notFound
        }
        return found
    }

    private static func isExecutableFile(_ url: URL) -> Bool {
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory), !isDirectory.boolValue else {
            return false
        }
        return FileManager.default.isExecutableFile(atPath: url.path)
    }
}
