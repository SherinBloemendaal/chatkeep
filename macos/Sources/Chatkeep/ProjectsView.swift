import ChatkeepKit
import SwiftUI

/// Projects on the left, the selected project and its chats on the right, as in Mail.
///
/// Both panes are SwiftUI views in an `HStack` with flexible frames, not an AppKit split view: a
/// split view hands its panes their ideal width, which left the table as a narrow centered column.
/// Here the list takes between 420 and 720 points and the chats take the rest, so together they
/// always fill the detail area, from the smallest window to full screen.
struct ProjectsView: View {
    @Bindable var model: ProjectsModel
    @Environment(AppModel.self) private var app

    var body: some View {
        HStack(spacing: 0) {
            projectTable
                .frame(minWidth: 420, idealWidth: 560, maxWidth: 720, maxHeight: .infinity)
            Divider()
            ProjectDetailPane(model: model)
                .frame(minWidth: 240, maxWidth: .infinity, maxHeight: .infinity)
        }
        .searchable(text: $model.search, placement: .toolbar, prompt: "Name or path")
        .toolbar { toolbar }
        .navigationSubtitle(subtitle)
        .task {
            if !model.list.hasLoaded, !model.list.isLoading { model.load() }
        }
    }

    // MARK: - Projects

    @ViewBuilder
    private var projectTable: some View {
        Table(of: Project.self, selection: $model.selection, sortOrder: $model.sortOrder) {
            TableColumn("Project", value: \.displayName) { project in
                ProjectNameCell(project: project)
            }
            .width(min: 160, ideal: 280)
            TableColumn("") { project in
                ToolIcon(tool: project.tool)
            }
            .width(22)
            TableColumn("Chats", value: \.chats) { project in
                Text(Format.count(project.chats))
                    .monospacedDigit()
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .help("\(project.subagents) agent chats not counted")
            }
            .width(min: 44, ideal: 52, max: 70)
            TableColumn("Size", value: \.sizeBytes) { project in
                Text(Format.bytes(project.sizeBytes))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .width(min: 60, ideal: 72, max: 96)
            TableColumn("Last Active", value: \.lastActiveOrNever) { project in
                Text(project.lastActive.map { Format.relative($0) } ?? "No chats")
                    .foregroundStyle(project.lastActive == nil ? .tertiary : .secondary)
                    .lineLimit(1)
                    .help(project.lastActive.map { Format.date($0) } ?? "")
            }
            .width(min: 80, ideal: 104, max: 150)
        } rows: {
            ForEach(model.visible) { project in
                TableRow(project)
                    .dropDestination(for: URL.self) { urls in
                        _ = app.drop(urls, on: project)
                    }
                    .contextMenu { ProjectActions(project: project) }
            }
        }
        .overlay { overlay }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if let error = model.error {
                ErrorBanner(error: error) { model.error = nil }
                    .padding(10)
            }
        }
    }

    @ViewBuilder
    private var overlay: some View {
        if model.isLoading, model.visible.isEmpty {
            LoadingView(text: "Reading projects…") { model.stop() }
        } else if !model.isLoading, model.allProjects.isEmpty, model.error == nil {
            ContentUnavailableView(
                model.filter == .unsaved ? "No Unsaved Windows" : "No Projects",
                systemImage: "folder",
                description: Text(model.filter == .unsaved
                    ? "Cursor has no chats in windows that were never saved to a folder."
                    : "Cursor and Claude Code have no chats on this Mac yet.")
            )
        } else if !model.allProjects.isEmpty, model.visible.isEmpty {
            ContentUnavailableView.search(text: model.search)
        }
    }

    private var subtitle: String {
        let projects = model.visible
        guard !projects.isEmpty else { return model.indexNote ?? "" }
        let missing = projects.count { $0.destinationMissing }
        var parts = ["\(projects.count) projects"]
        if missing > 0 { parts.append("\(missing) with a missing folder") }
        if let note = model.indexNote { parts.append(note) }
        return parts.joined(separator: " · ")
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            Picker("Show", selection: $model.filter) {
                ForEach(ProjectsModel.Filter.allCases) { filter in
                    Text(filter.shortTitle).tag(filter)
                }
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .help("Show the projects of one tool, or Cursor windows that were never saved to a folder")
        }
        ToolbarItemGroup(placement: .primaryAction) {
            if model.isLoading {
                ProgressView().controlSize(.small)
            }
            Menu {
                Button("Read Fresh", systemImage: "arrow.clockwise.circle") { model.load(fresh: true) }
                    .help("Read every chat from Cursor and Claude Code again instead of from the index")
            } label: {
                Label("Refresh", systemImage: "arrow.clockwise")
            } primaryAction: {
                model.load()
            }
            .help("Read the projects again (⌘R). Hold the menu for Read Fresh.")
        }
        ToolbarGap()
        ToolbarItem(placement: .primaryAction) {
            Menu {
                if let project = model.selectedProject {
                    ProjectActions(project: project)
                    Divider()
                }
                Button("Rewrite Paths…", systemImage: "character.cursor.ibeam") { app.present(.move(nil)) }
                Button("Import Archive…", systemImage: "square.and.arrow.down") { app.present(.importArchive(file: nil)) }
            } label: {
                Label("Actions", systemImage: "ellipsis.circle")
            }
            .help("Move, copy, split, combine, export or remove chats")
        }
    }
}

/// The folder name first, the whole path below it, cut at the start so the end stays readable.
struct ProjectNameCell: View {
    let project: Project

    var body: some View {
        HStack(spacing: 6) {
            VStack(alignment: .leading, spacing: 1) {
                Text(project.displayName)
                    .fontWeight(.medium)
                    .lineLimit(1)
                Text(project.path.map(Format.path) ?? "No folder")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
            if project.destinationMissing {
                Spacer(minLength: 0)
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(Theme.warning)
                    .help("The project folder is gone. Move its chats to where the folder is now.")
            }
        }
        .padding(.vertical, 2)
        .help(project.path ?? project.name)
    }
}

/// The actions for one project, in its context menu and in the Actions menu.
struct ProjectActions: View {
    let project: Project
    @Environment(AppModel.self) private var app

    var body: some View {
        if project.tool.isKnown {
            changes
        }
        if let path = project.path {
            Button("Show Folder in Finder", systemImage: "folder") { Reveal.inFinder(path) }
        }
        Button("Show Chat Storage in Finder", systemImage: "externaldrive") { Reveal.inFinder(project.storageDir) }
        Button("Copy Path", systemImage: "doc.on.clipboard") { Reveal.copy(project.path ?? project.projectID) }
        if project.tool.isKnown {
            Divider()
            Button("Remove Chats…", systemImage: "trash", role: .destructive) { app.present(.remove(project)) }
        }
    }

    /// Changes only for a tool this app knows; a newer tool's projects can only be looked at.
    @ViewBuilder
    private var changes: some View {
        if project.isUnsaved {
            Button("Save to Folder…", systemImage: "folder.badge.plus") { app.present(.saveUnsaved(project)) }
        }
        Button("Move Chats…", systemImage: "arrow.right.doc.on.clipboard") { app.present(.move(project)) }
        Button("Copy Chats…", systemImage: "doc.on.doc") { app.present(.copy(project)) }
        Button("Split…", systemImage: "arrow.triangle.branch") { app.present(.split(project)) }
        Button("Combine Into This Project…", systemImage: "arrow.triangle.merge") { app.present(.combine(into: project)) }
        Button("Export…", systemImage: "square.and.arrow.up") { app.present(.export(project)) }
        if project.tool == .cursor, !project.isUnsaved {
            Button("Repair…", systemImage: "wrench.adjustable") { app.present(.reindex(project)) }
        }
        Divider()
    }
}

// MARK: - Selected project

/// The right pane: the project's header with its numbers and actions, then its chats.
struct ProjectDetailPane: View {
    let model: ProjectsModel

    var body: some View {
        if let project = model.selectedProject {
            VStack(spacing: 0) {
                ProjectHeader(project: project, model: model)
                Divider()
                ChatList(model: model, project: project)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        } else {
            ContentUnavailableView(
                "No Project Selected",
                systemImage: "bubble.left.and.bubble.right",
                description: Text("Select a project to see its chats. Drop a folder on a project to move or copy its chats there.")
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

struct ProjectHeader: View {
    let project: Project
    let model: ProjectsModel
    @Environment(AppModel.self) private var app

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(project.displayName)
                .font(.title2.weight(.semibold))
                .lineLimit(2)
            Text(project.path.map(Format.path) ?? "No folder of its own")
                .font(.callout)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .truncationMode(.head)
                .textSelection(.enabled)
                .help(project.path ?? "")
            HStack(spacing: 6) {
                ToolTag(tool: project.tool)
                Pill(text: Format.kind(project.kind), color: .secondary)
                if let label = project.profileLabel {
                    Pill(text: label, color: .secondary, systemImage: "externaldrive")
                        .help("Cursor installation")
                }
            }
            Text(facts)
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if project.destinationMissing {
                Label("The project folder is gone. Use Move Chats to point them at its new place.", systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Theme.warning)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if project.tool.isKnown {
                actions
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The buttons when they fit on one line, else one menu.
    private var actions: some View {
        ViewThatFits(in: .horizontal) {
            GlassGroup {
                HStack(spacing: 8) {
                    if project.isUnsaved {
                        Button("Save to Folder…") { app.present(.saveUnsaved(project)) }
                            .primaryActionStyle()
                    }
                    Button("Move…") { app.present(.move(project)) }
                        .secondaryActionStyle()
                    Button("Copy…") { app.present(.copy(project)) }
                        .secondaryActionStyle()
                    Button("Export…") { app.present(.export(project)) }
                        .secondaryActionStyle()
                    moreMenu
                }
            }
            moreMenu
        }
    }

    private var moreMenu: some View {
        Menu("More") { ProjectActions(project: project) }
            .fixedSize()
    }

    private var facts: String {
        var parts = ["\(Format.count(project.chats)) chats"]
        if project.subagents > 0 { parts.append("\(Format.count(project.subagents)) agent chats") }
        if let archived = project.archived, archived > 0 { parts.append("\(Format.count(archived)) archived") }
        parts.append(Format.bytes(project.sizeBytes))
        if let memory = project.memoryFiles, memory > 0 { parts.append("\(memory) memory files") }
        if let date = project.lastActive ?? model.detail.value?.lastActive {
            parts.append("last active \(Format.relative(date))")
        }
        return parts.joined(separator: " · ")
    }
}

// MARK: - Chats

struct ChatList: View {
    let model: ProjectsModel
    let project: Project
    @State private var selection: Chat.ID?
    @Environment(AppModel.self) private var app

    var body: some View {
        let chats = model.detail.value?.chats ?? []
        Table(chats, selection: $selection) {
            TableColumn("Title") { chat in
                HStack(spacing: 6) {
                    Text(chat.title ?? "Untitled")
                        .foregroundStyle(chat.title == nil ? .secondary : .primary)
                        .lineLimit(1)
                    if chat.subagent {
                        Pill(text: "Agent", color: .secondary)
                    }
                    if chat.archived == true {
                        Pill(text: "Archived", color: .secondary, systemImage: "archivebox")
                    }
                }
                .help(chat.title ?? "Untitled")
            }
            .width(min: 160, ideal: 320)
            TableColumn("Updated") { chat in
                Text(Format.relative(chat.updatedAt))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .help(Format.date(chat.updatedAt))
            }
            .width(min: 80, ideal: 110, max: 160)
            TableColumn("Model") { chat in
                Text(chat.model ?? "").foregroundStyle(.secondary).lineLimit(1)
            }
            .width(min: 70, ideal: 120, max: 200)
            TableColumn("Tokens") { chat in
                Text(tokens(chat))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .help(tokenHelp(chat))
            }
            .width(min: 50, ideal: 64, max: 90)
            if project.tool == .claude {
                TableColumn("Size") { chat in
                    Text(chat.sizeBytes.map(Format.bytes) ?? "")
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
                .width(min: 50, ideal: 70, max: 96)
            }
        }
        .contextMenu(forSelectionType: Chat.ID.self) { ids in
            if let id = ids.first, let chat = chats.first(where: { $0.id == id }) {
                Button("Copy Chat ID") { Reveal.copy(chat.id) }
                if let transcript = chat.transcript {
                    Button("Show Transcript in Finder") { Reveal.inFinder(transcript) }
                }
                if project.tool.isKnown {
                    Divider()
                    Button("Remove Chat…", role: .destructive) { app.present(.remove(project, chat: chat)) }
                }
            }
        }
        .overlay {
            if model.detail.isLoading, chats.isEmpty {
                LoadingView(text: "Reading chats…") { model.detail.stop() }
            } else if let error = model.detail.error {
                ErrorBanner(error: error).padding()
            } else if !model.detail.isLoading, chats.isEmpty {
                ContentUnavailableView("No Chats", systemImage: "bubble.left", description: Text("This project has no chats left."))
            }
        }
    }

    private func tokens(_ chat: Chat) -> String {
        guard let tokens = chat.tokens else { return "" }
        if let context = tokens.context, tokens.total == 0 { return Format.tokens(context) }
        return Format.tokens(tokens.total)
    }

    private func tokenHelp(_ chat: Chat) -> String {
        guard let tokens = chat.tokens else { return "Cursor has no token count for this chat" }
        var lines = ["In: \(Format.tokens(tokens.input))", "Out: \(Format.tokens(tokens.output))"]
        if let context = tokens.context { lines.append("Context window in use: \(Format.tokens(context))") }
        if let write = tokens.cacheWrite { lines.append("Cache written: \(Format.tokens(write))") }
        if let read = tokens.cacheRead { lines.append("Cache read: \(Format.tokens(read))") }
        if let cost = chat.costCents, cost > 0 { lines.append("Cost: \(Format.money(cents: Int64(cost)))") }
        return lines.joined(separator: "\n")
    }
}
