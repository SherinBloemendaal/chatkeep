import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The `.chatkeep` archive type the app declares in its Info.plist (a gzip file). Looked up by
/// its extension, so it also resolves under `swift run`, where there is no Info.plist.
extension UTType {
    static let chatkeepArchive = UTType(filenameExtension: "chatkeep", conformingTo: .gzip) ?? .gzip
}

/// A path typed in or picked with the standard open or save panel.
struct PathField: View {
    enum Kind {
        case folder
        case archive
        case newArchive(suggestedName: String)
    }

    let title: String
    @Binding var path: String
    var kind = Kind.folder
    var prompt = "Choose a folder"

    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 6) {
                TextField(title, text: $path, prompt: Text(prompt))
                    .labelsHidden()
                    .truncationMode(.head)
                Button("Choose…") { choose() }
            }
        }
    }

    private func choose() {
        switch kind {
        case .folder:
            if let url = Self.pickFolder(message: title, startingAt: path) { path = url.path(percentEncoded: false) }
        case .archive:
            let panel = NSOpenPanel()
            panel.canChooseFiles = true
            panel.canChooseDirectories = false
            panel.allowedContentTypes = [.chatkeepArchive]
            panel.message = "Choose a .chatkeep archive"
            if panel.runModal() == .OK, let url = panel.url { path = url.path(percentEncoded: false) }
        case .newArchive(let suggestedName):
            let panel = NSSavePanel()
            panel.allowedContentTypes = [.chatkeepArchive]
            panel.nameFieldStringValue = suggestedName
            panel.message = "Where should the archive go?"
            if panel.runModal() == .OK, let url = panel.url { path = url.path(percentEncoded: false) }
        }
    }

    static func pickFolder(message: String, startingAt path: String = "") -> URL? {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.message = message
        if !path.isEmpty {
            panel.directoryURL = URL(filePath: path, directoryHint: .isDirectory).deletingLastPathComponent()
        }
        return panel.runModal() == .OK ? panel.url : nil
    }
}

extension String {
    /// The text without surrounding spaces, or `nil` when nothing is left.
    var trimmedOrNil: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

extension String {
    /// A typed path: trimmed, with `~` expanded to the home folder. `nil` when empty.
    var pathOrNil: String? {
        trimmedOrNil.map { ($0 as NSString).expandingTildeInPath }
    }
}
