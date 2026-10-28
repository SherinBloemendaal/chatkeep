import ChatkeepKit
import SwiftUI

/// Opens the sheet of one action.
struct ActionSheetView: View {
    let action: Action

    var body: some View {
        switch action {
        case .move(let project, let to): PathChangeSheet(copy: false, project: project, destination: to)
        case .copy(let project, let to): PathChangeSheet(copy: true, project: project, destination: to)
        case .remove(let project, let chat): RemoveSheet(project: project, chat: chat)
        case .split(let project): SplitSheet(project: project)
        case .combine(let target, let sources): CombineSheet(target: target, initialSources: sources)
        case .export(let project): ExportSheet(project: project)
        case .importArchive(let file, let folder): ImportSheet(file: file, folder: folder)
        case .saveUnsaved(let project, let folder): SaveUnsavedSheet(project: project, folder: folder)
        case .reindex(let project): ReindexSheet(project: project)
        case .clearCursorIndex: ClearCursorIndexSheet()
        case .clearClaudeIndex: SimpleWriteSheet(
                title: "Clear the Claude Code Index",
                summary: "Deletes chatkeep's index of what every transcript says. The next list or stats reads every transcript again. Your chats are not touched.",
                applyLabel: "Clear Index",
                command: CLI.clearClaudeCache()
            )
        case .pruneAccounts: SimpleWriteSheet(
                title: "Clean Up Old Accounts",
                summary: "Removes what Claude Code still keeps in .claude.json for accounts that are no longer in use.",
                applyLabel: "Clean Up",
                command: CLI.pruneAccounts()
            )
        case .runQueue: RunQueueSheet()
        case .retryQueue(let ids): SimpleWriteSheet(
                title: ids.isEmpty ? "Try Every Failed Entry Again" : "Try Again",
                summary: "Puts failed or skipped entries back in line, so the next run of the queue picks them up.",
                applyLabel: "Try Again",
                command: CLI.queueRetry(ids)
            )
        case .removeFromQueue(let ids): SimpleWriteSheet(
                title: ids.count == 1 ? "Remove Queue Entry" : "Remove \(ids.count) Queue Entries",
                summary: "The commands are taken out of the queue and will not run.",
                applyLabel: "Remove",
                command: CLI.queueRemove(ids)
            )
        case .clearQueue: SimpleWriteSheet(
                title: "Clear the Queue",
                summary: "Removes every entry, also the ones that are waiting to run.",
                applyLabel: "Clear Queue",
                command: CLI.queueClear()
            )
        case .copyAccountChats(let from): CopyAccountChatsSheet(from: from)
        case .syncNow(let profile): SyncNowSheet(profile: profile)
        case .editSyncProfile(let name, let adding): SyncProfileSheet(name: name, adding: adding)
        case .removeFromSyncProfile(let name, let members): SimpleWriteSheet(
                title: members.isEmpty ? "Remove Sync Profile \u{201C}\(name)\u{201D}" : "Remove from \u{201C}\(name)\u{201D}",
                summary: members.isEmpty
                    ? "The profile goes away. The chat lists of its accounts stay as they are."
                    : "The accounts stop syncing with this profile. Their chat lists stay as they are.",
                applyLabel: "Remove",
                command: CLI.removeFromSyncProfile(name, accounts: members)
            )
        }
    }
}

/// A write with nothing to fill in: it shows its preview right away.
struct SimpleWriteSheet: View {
    let title: String
    let summary: String
    let applyLabel: String
    let command: WriteCommand<Report>
    @State private var flow = WriteFlow<Report>()

    var body: some View {
        WriteSheet(
            title: title,
            summary: summary,
            applyLabel: applyLabel,
            command: command,
            flow: flow,
            previewAutomatically: true
        ) {
            EmptyView()
        }
    }
}

/// Which tools a command covers.
enum ToolChoice: Hashable, CaseIterable, Identifiable {
    case every
    case cursor
    case claude

    var id: Self { self }

    init(_ tool: Tool?) {
        self = switch tool {
        case .cursor: .cursor
        case .claude: .claude
        case .unknown, nil: .every
        }
    }

    var tool: Tool? {
        switch self {
        case .every: nil
        case .cursor: .cursor
        case .claude: .claude
        }
    }

    var title: String {
        switch self {
        case .every: "Cursor and Claude Code"
        case .cursor: "Cursor only"
        case .claude: "Claude Code only"
        }
    }
}

/// The tools picker, and the Cursor installation when Cursor is part of it.
struct ScopeFields: View {
    @Binding var tools: ToolChoice
    @Binding var installation: String
    var allowsEvery = true
    @Environment(AppModel.self) private var app

    var body: some View {
        Picker("Tools", selection: $tools) {
            ForEach(ToolChoice.allCases.filter { allowsEvery || $0 != .every }) { choice in
                Text(choice.title).tag(choice)
            }
        }
        if tools != .claude {
            InstallationPicker(installation: $installation)
        }
    }
}

/// A Cursor installation (and VS Code profile), or every one. Needed when a folder is open in
/// several installations.
struct InstallationPicker: View {
    @Binding var installation: String
    var title = "Cursor installation"
    @Environment(AppModel.self) private var app

    var body: some View {
        let known = app.projects.cursorProfiles
        Picker(title, selection: $installation) {
            Text("Any").tag("")
            ForEach(known, id: \.self) { Text($0).tag($0) }
            if !installation.isEmpty, !known.contains(installation) {
                Text(installation).tag(installation)
            }
        }
        .help("Only needed when the folder is open in more than one Cursor installation")
    }
}

extension Project {
    /// The value a command for every tool takes: the folder, so both tools match it. A Cursor
    /// window without a folder of its own only has its id.
    var reference: String {
        tool == .cursor && (isUnsaved || path == nil) ? projectID : (path ?? projectID)
    }

    /// The value a command for this project's own tool takes: the exact Cursor workspace id, or
    /// the Claude Code project's folder.
    var ownReference: String {
        tool == .cursor ? projectID : (path ?? projectID)
    }
}

// MARK: - Move and copy

struct PathChangeSheet: View {
    enum Mode: Hashable {
        case project
        case replace
    }

    let copy: Bool
    let project: Project?
    @State private var mode: Mode
    @State private var from: String
    @State private var to: String
    @State private var alsoFolder = false
    @State private var pattern = ""
    @State private var replacement = ""
    @State private var regex = false
    @State private var tools: ToolChoice
    @State private var installation: String
    @State private var flow = WriteFlow<Report>()

    init(copy: Bool, project: Project?, destination: URL?) {
        self.copy = copy
        self.project = project
        _mode = State(initialValue: project == nil ? .replace : .project)
        _from = State(initialValue: project?.reference ?? "")
        _to = State(initialValue: destination?.path(percentEncoded: false) ?? "")
        // A project without a folder is only known to its own tool.
        _tools = State(initialValue: project.map { $0.path == nil || $0.isUnsaved ? ToolChoice($0.tool) : .every } ?? .every)
        _installation = State(initialValue: project?.isUnsaved == true ? project?.profileLabel ?? "" : "")
    }

    private var command: WriteCommand<Report>? {
        let scope = Scope(tool: tools.tool, profile: tools == .claude ? nil : installation.trimmedOrNil)
        let unsaved = project?.isUnsaved == true && mode == .project
        let change: PathChange
        switch mode {
        case .project:
            guard let from = from.pathOrNil, let to = to.pathOrNil else { return nil }
            change = .project(from: from, to: to)
        case .replace:
            guard let pattern = pattern.trimmedOrNil, let replacement = replacement.trimmedOrNil else { return nil }
            change = .replace(from: pattern, to: replacement, regex: regex)
        }
        return copy
            ? CLI.copy(change, copyFolder: alsoFolder, scope: scope, unsaved: unsaved)
            : CLI.move(change, moveFolder: alsoFolder, scope: scope, unsaved: unsaved)
    }

    var body: some View {
        WriteSheet(
            title: copy ? "Copy Chats" : "Move Chats",
            summary: copy
                ? "Copies a project's chats to another folder, so both folders have them."
                : "Points a project's chats at the folder where the project is now, for example after you moved or renamed it.",
            applyLabel: copy ? "Copy Chats" : "Move Chats",
            command: command,
            flow: flow
        ) {
            Section {
                Picker("Change", selection: $mode) {
                    Text("One project").tag(Mode.project)
                    Text("Every matching path").tag(Mode.replace)
                }
                .pickerStyle(.segmented)
            }
            if mode == .project {
                Section {
                    PathField(title: "From", path: $from, prompt: "Project folder or id")
                    PathField(title: "To", path: $to, prompt: "The new folder")
                    Toggle(copy ? "Also copy the project folder itself" : "Also move the project folder itself", isOn: $alsoFolder)
                        .help(copy
                            ? "Copies the real folder with your files to the new place too"
                            : "Moves the real folder with your files to the new place too. Leave this off when you already moved it.")
                }
            } else {
                Section {
                    TextField("Replace", text: $pattern, prompt: Text("/Users/me/old"))
                    TextField("With", text: $replacement, prompt: Text("/Users/me/new"))
                    Toggle("Treat as a pattern (regular expression)", isOn: $regex)
                    Toggle(copy ? "Also copy the project folders themselves" : "Also move the project folders themselves", isOn: $alsoFolder)
                } footer: {
                    Text("Every project whose path starts with or matches the text gets the new path.")
                }
            }
            Section {
                ScopeFields(tools: $tools, installation: $installation)
            }
        }
    }
}

// MARK: - Remove

struct RemoveSheet: View {
    let project: Project
    let chat: Chat?
    @State private var tools: ToolChoice
    @State private var flow = WriteFlow<Report>()

    init(project: Project, chat: Chat?) {
        self.project = project
        self.chat = chat
        _tools = State(initialValue: ToolChoice(project.tool))
    }

    private var command: WriteCommand<Report> {
        let tool = chat == nil ? tools.tool : project.tool
        let profile = tool == .claude ? nil : project.profileLabel
        return CLI.remove(chat?.id ?? (tools == .every ? project.reference : project.ownReference), scope: Scope(tool: tool, profile: profile))
    }

    var body: some View {
        WriteSheet(
            title: chat == nil ? "Remove Chats" : "Remove Chat",
            summary: chat == nil
                ? "Removes every chat chatkeep finds for \(project.name). The project folder with your files stays."
                : "Removes the chat \u{201C}\(chat?.title ?? "Untitled")\u{201D} from \(project.name).",
            applyLabel: "Remove",
            command: command,
            flow: flow
        ) {
            if chat == nil {
                Section {
                    Picker("Tools", selection: $tools) {
                        Text(ToolChoice(project.tool).title).tag(ToolChoice(project.tool))
                        if project.path != nil {
                            Text(ToolChoice.every.title).tag(ToolChoice.every)
                        }
                    }
                } footer: {
                    Text("The preview lists exactly what goes. Check it before you remove.")
                }
            } else {
                Section {
                    DetailRow(label: "Chat", value: chat?.title ?? "Untitled")
                    DetailRow(label: "ID", value: chat?.id ?? "", monospaced: true)
                }
            }
        }
    }
}

// MARK: - Combine

struct CombineSheet: View {
    let target: Project?
    @State private var into: String
    @State private var sources: [String]
    @State private var mode = TransferMode.copy
    @State private var tools: ToolChoice
    @State private var installation: String
    @State private var flow = WriteFlow<Report>()
    @Environment(AppModel.self) private var app

    init(target: Project?, initialSources: [String]) {
        self.target = target
        _into = State(initialValue: target?.reference ?? "")
        _sources = State(initialValue: initialSources)
        _tools = State(initialValue: target.map { ToolChoice($0.tool) } ?? .every)
        _installation = State(initialValue: target?.profileLabel ?? "")
    }

    private var command: WriteCommand<Report>? {
        guard let into = into.pathOrNil else { return nil }
        let picked = sources.compactMap(\.pathOrNil)
        guard !picked.isEmpty else { return nil }
        let scope = Scope(tool: tools.tool, profile: tools == .claude ? nil : installation.trimmedOrNil)
        return CLI.combine(into: into, from: picked, mode: mode, scope: scope)
    }

    /// Other projects of the same tool, to pick sources from.
    private var candidates: [Project] {
        app.projects.allProjects.filter { project in
            reference(for: project) != into && !sources.contains(reference(for: project))
                && (tools.tool == nil || project.tool == tools.tool)
                && project.chats > 0
        }
    }

    private func reference(for project: Project) -> String {
        tools == .every ? project.reference : project.ownReference
    }

    var body: some View {
        WriteSheet(
            title: "Combine Chats",
            summary: "Brings the chats of other projects into one project.",
            applyLabel: mode == .move ? "Move Chats" : "Copy Chats",
            command: command,
            flow: flow
        ) {
            Section {
                PathField(title: "Into", path: $into, prompt: "Project folder or id")
            }
            Section("From") {
                ForEach(sources.indices, id: \.self) { index in
                    HStack {
                        TextField("Source", text: $sources[index], prompt: Text("Project folder, id or chat id"))
                            .labelsHidden()
                        Button("Remove", systemImage: "minus.circle") { sources.remove(at: index) }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                    }
                }
                HStack {
                    Menu("Add Project") {
                        ForEach(candidates) { project in
                            Button("\(project.name) (\(project.chats) chats)") { sources.append(reference(for: project)) }
                        }
                    }
                    .disabled(candidates.isEmpty)
                    .fixedSize()
                    Button("Add Folder…") {
                        if let url = PathField.pickFolder(message: "Choose a project folder to take chats from") {
                            sources.append(url.path(percentEncoded: false))
                        }
                    }
                }
            }
            Section {
                Picker("Chats in the sources", selection: $mode) {
                    Text("Keep them (copy)").tag(TransferMode.copy)
                    Text("Remove them (move)").tag(TransferMode.move)
                }
                ScopeFields(tools: $tools, installation: $installation)
            }
        }
        .task {
            if !app.projects.list.hasLoaded { app.projects.load() }
        }
    }
}

// MARK: - Export and import

struct ExportSheet: View {
    let project: Project
    @State private var file: String
    @State private var tools: ToolChoice
    @State private var flow = WriteFlow<Report>()

    init(project: Project) {
        self.project = project
        let name = URL(filePath: project.path ?? project.projectID).lastPathComponent
        let folder = FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first
        _file = State(initialValue: folder?.appending(path: "\(name).chatkeep").path(percentEncoded: false) ?? "")
        _tools = State(initialValue: project.path == nil ? ToolChoice(project.tool) : .every)
    }

    private var command: WriteCommand<Report>? {
        guard let file = file.pathOrNil else { return nil }
        let reference = tools == .every ? project.reference : project.ownReference
        let profile = tools == .claude ? nil : project.profileLabel
        return CLI.export(reference, to: file, scope: Scope(tool: tools.tool, profile: tools == .every ? nil : profile))
    }

    var body: some View {
        WriteSheet(
            title: "Export Chats",
            summary: "Writes the chats of \(project.name) into one .chatkeep file, to keep or to bring to another Mac.",
            applyLabel: "Export",
            command: command,
            flow: flow
        ) {
            Section {
                PathField(
                    title: "Save as",
                    path: $file,
                    kind: .newArchive(suggestedName: URL(filePath: file).lastPathComponent),
                    prompt: "Archive file"
                )
                Picker("Tools", selection: $tools) {
                    Text(ToolChoice(project.tool).title).tag(ToolChoice(project.tool))
                    if project.path != nil {
                        Text(ToolChoice.every.title).tag(ToolChoice.every)
                    }
                }
            }
        }
    }
}

struct ImportSheet: View {
    @State private var file: String
    @State private var folder: String
    @State private var overwrite = false
    @State private var tools = ToolChoice.every
    @State private var flow = WriteFlow<Report>()

    init(file: URL?, folder: URL?) {
        _file = State(initialValue: file?.path(percentEncoded: false) ?? "")
        _folder = State(initialValue: folder?.path(percentEncoded: false) ?? "")
    }

    private var command: WriteCommand<Report>? {
        guard let file = file.pathOrNil else { return nil }
        return CLI.importArchive(file, into: folder.pathOrNil, overwrite: overwrite, scope: Scope(tool: tools.tool))
    }

    var body: some View {
        WriteSheet(
            title: "Import Chats",
            summary: "Restores the chats from a .chatkeep archive.",
            applyLabel: "Import",
            command: command,
            flow: flow
        ) {
            Section {
                PathField(title: "Archive", path: $file, kind: .archive, prompt: "A .chatkeep file")
                PathField(title: "Into folder", path: $folder, prompt: "Optional: the folder the project is in now")
            } footer: {
                Text("Leave the folder empty to restore the chats for the path they had when they were exported.")
            }
            Section {
                Toggle("Replace chats that already exist", isOn: $overwrite)
                    .help("Off: chats that are already there are left alone")
                Picker("Tools", selection: $tools) {
                    ForEach(ToolChoice.allCases) { Text($0.title).tag($0) }
                }
            }
        }
    }
}

// MARK: - Cursor

struct SaveUnsavedSheet: View {
    let project: Project
    @State private var folder: String
    @State private var flow = WriteFlow<Report>()

    init(project: Project, folder: URL?) {
        self.project = project
        _folder = State(initialValue: folder?.path(percentEncoded: false) ?? "")
    }

    private var command: WriteCommand<Report>? {
        guard let folder = folder.pathOrNil else { return nil }
        return CLI.saveUnsaved(project.projectID, to: folder, profile: project.profileLabel)
    }

    var body: some View {
        WriteSheet(
            title: "Save Window to a Folder",
            summary: "Attaches the chats of an unsaved Cursor window to a real folder, so they show up when you open that folder.",
            applyLabel: "Save",
            command: command,
            flow: flow
        ) {
            Section {
                DetailRow(label: "Window", value: project.name)
                PathField(title: "Folder", path: $folder, prompt: "The folder to attach the chats to")
            }
        }
    }
}

struct ReindexSheet: View {
    @State private var target: String
    @State private var installation: String
    @State private var flow = WriteFlow<Report>()

    init(project: Project?) {
        _target = State(initialValue: project?.ownReference ?? "")
        _installation = State(initialValue: project?.profileLabel ?? "")
    }

    private var command: WriteCommand<Report>? {
        guard let target = target.pathOrNil else { return nil }
        return CLI.reindex(target, profile: installation.trimmedOrNil)
    }

    var body: some View {
        WriteSheet(
            title: "Repair a Cursor Project",
            summary: "Rebuilds Cursor's references to the project's chats, fixes paths left over from an old place, and clears Cursor's caches for it. Use it when chats are missing in Cursor after a move.",
            applyLabel: "Repair",
            command: command,
            flow: flow
        ) {
            Section {
                PathField(title: "Project", path: $target, prompt: "Project folder or workspace id")
                InstallationPicker(installation: $installation)
            }
        }
    }
}

struct ClearCursorIndexSheet: View {
    @State private var installation = ""
    @State private var flow = WriteFlow<Report>()

    var body: some View {
        WriteSheet(
            title: "Clear the Cursor Index",
            summary: "Deletes chatkeep's index of Cursor's chats, or only one installation's rows. The next list reads Cursor again. Your chats are not touched.",
            applyLabel: "Clear Index",
            command: CLI.clearCursorCache(profile: installation.trimmedOrNil),
            flow: flow
        ) {
            Section {
                InstallationPicker(installation: $installation, title: "Installation")
            }
        }
    }
}

// MARK: - Queue

struct RunQueueSheet: View {
    @State private var continueOnError = false
    @State private var flow = WriteFlow<QueueExecution>()

    var body: some View {
        WriteSheet(
            title: "Run the Queue",
            summary: "Runs every waiting change in order. Close Cursor and Claude Code (and the Claude desktop app) first.",
            applyLabel: "Run Queue",
            command: CLI.queueExecute(continueOnError: continueOnError),
            flow: flow,
            previewAutomatically: true
        ) {
            Section {
                Toggle("Keep going when one fails", isOn: $continueOnError)
                    .help("Off: the queue stops at the first change that fails")
            }
        } extra: { response in
            if let execution = response.data {
                QueueRunEntries(entries: execution.entries)
            }
        }
    }
}

/// What a queue run did with each entry.
struct QueueRunEntries: View {
    let entries: [QueueExecution.Entry]

    var body: some View {
        if !entries.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                Text("Entries").font(.headline)
                ForEach(entries) { entry in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(entry.id).font(.callout.monospaced()).foregroundStyle(.secondary)
                            Pill(text: Self.label(entry.status), color: Self.color(entry.status))
                            if entry.plan == .skip { Pill(text: "Plan failed", color: Theme.failure) }
                        }
                        Text(entry.command)
                            .font(.callout.monospaced())
                            .textSelection(.enabled)
                            .lineLimit(3)
                            .truncationMode(.middle)
                        ForEach(entry.planWarnings, id: \.self) { warning in
                            Label(warning, systemImage: "exclamationmark.triangle").font(.callout).foregroundStyle(Theme.warning)
                        }
                        if let message = entry.planError ?? entry.error {
                            Label(message, systemImage: "xmark.octagon").font(.callout).foregroundStyle(Theme.failure)
                        }
                        ForEach(Array(entry.results.enumerated()), id: \.offset) { _, result in
                            ReportView(report: result.report, tool: nil)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
                }
            }
        }
    }

    static func label(_ status: QueueExecution.Entry.Status) -> String {
        switch status {
        case .planned: "Ready"
        case .done: "Done"
        case .failed: "Failed"
        case .skipped: "Skipped"
        case .blocked: "Waiting for a tool to close"
        case .unknown(let raw): raw
        }
    }

    static func color(_ status: QueueExecution.Entry.Status) -> Color {
        switch status {
        case .planned: Theme.teal
        case .done: .green
        case .failed: Theme.failure
        case .skipped, .blocked: Theme.warning
        case .unknown: .secondary
        }
    }
}
