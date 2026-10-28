import ChatkeepKit
import SwiftUI

struct ProjectsView: View {
    @Bindable var model: ProjectsModel
    @Environment(AppModel.self) private var app

    var body: some View {
        VSplitView {
            projectTable
                .frame(minHeight: 200, idealHeight: 380)
            ChatList(model: model)
                .frame(minHeight: 160, idealHeight: 260)
        }
        .searchable(text: $model.search, placement: .toolbar, prompt: "Name or path")
        .inspector(isPresented: $model.showsInspector) {
            ProjectInspector(model: model)
                .inspectorColumnWidth(min: 260, ideal: 300, max: 420)
        }
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
            TableColumn("Name", value: \.name) { project in
                HStack(spacing: 6) {
                    Image(systemName: project.isUnsaved ? "macwindow.badge.plus" : "folder")
                        .foregroundStyle(Theme.color(for: project.tool))
                    Text(project.name)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if project.destinationMissing {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(Theme.warning)
                            .help("The project folder is gone. Move its chats to where the folder is now.")
                    }
                }
            }
            .width(min: 200, ideal: 300)
            TableColumn("Tool") { project in
                ToolTag(tool: project.tool)
            }
            .width(110)
            TableColumn("Chats", value: \.chats) { project in
                Text(Format.count(project.chats)).monospacedDigit()
                    .help("\(project.subagents) agent chats not counted")
            }
            .width(60)
            TableColumn("Size", value: \.sizeBytes) { project in
                Text(Format.bytes(project.sizeBytes)).monospacedDigit()
            }
            .width(80)
            TableColumn("Last Active", value: \.lastActiveOrNever) { project in
                Text(project.lastActive.map { Format.relative($0) } ?? "No chats")
                    .foregroundStyle(project.lastActive == nil ? .tertiary : .secondary)
                    .help(project.lastActive.map { Format.date($0) } ?? "")
            }
            .width(min: 90, ideal: 120)
            TableColumn("Path", value: \.pathText) { project in
                Text(project.path.map(Format.path) ?? "None")
                    .foregroundStyle(project.path == nil ? .tertiary : .secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
            .width(min: 160, ideal: 280)
            TableColumn("Installation") { project in
                Text(project.profileLabel ?? "")
                    .foregroundStyle(.secondary)
            }
            .width(min: 60, ideal: 90)
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
        ToolbarItem(placement: .navigation) {
            Picker("Show", selection: $model.filter) {
                ForEach(ProjectsModel.Filter.allCases) { filter in
                    Text(filter.title).tag(filter)
                }
            }
            .pickerStyle(.menu)
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
            Button("Details", systemImage: "sidebar.trailing") { model.showsInspector.toggle() }
                .help("Show or hide the project details")
        }
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

// MARK: - Chats

struct ChatList: View {
    @Bindable var model: ProjectsModel
    @State private var selection: Chat.ID?
    @Environment(AppModel.self) private var app

    var body: some View {
        Group {
            if let project = model.selectedProject {
                chats(of: project)
            } else {
                ContentUnavailableView(
                    "No Project Selected",
                    systemImage: "bubble.left.and.bubble.right",
                    description: Text("Select a project to see its chats. Drop a folder on a project to move or copy its chats there.")
                )
            }
        }
    }

    @ViewBuilder
    private func chats(of project: Project) -> some View {
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
            }
            .width(min: 200, ideal: 340)
            TableColumn("Updated") { chat in
                Text(Format.date(chat.updatedAt)).foregroundStyle(.secondary)
            }
            .width(min: 110, ideal: 140)
            TableColumn("Model") { chat in
                Text(chat.model ?? "").foregroundStyle(.secondary)
            }
            .width(min: 90, ideal: 140)
            TableColumn("Tokens") { chat in
                Text(tokens(chat)).monospacedDigit().foregroundStyle(.secondary)
                    .help(tokenHelp(chat))
            }
            .width(min: 60, ideal: 80)
            TableColumn("Size") { chat in
                Text(chat.sizeBytes.map(Format.bytes) ?? "").monospacedDigit().foregroundStyle(.secondary)
            }
            .width(min: 50, ideal: 70)
        }
        .contextMenu(forSelectionType: Chat.ID.self) { ids in
            if let id = ids.first, let chat = chats.first(where: { $0.id == id }) {
                Button("Copy Chat ID") { Reveal.copy(chat.id) }
                if let transcript = chat.transcript {
                    Button("Show Transcript in Finder") { Reveal.inFinder(transcript) }
                }
                Divider()
                Button("Remove Chat…", role: .destructive) { app.present(.remove(project, chat: chat)) }
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

// MARK: - Details

struct ProjectInspector: View {
    let model: ProjectsModel
    @Environment(AppModel.self) private var app

    var body: some View {
        if let project = model.selectedProject {
            Form {
                Section {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(project.name)
                            .font(.headline)
                            .lineLimit(3)
                            .truncationMode(.middle)
                        HStack {
                            ToolTag(tool: project.tool)
                            Pill(text: Format.kind(project.kind), color: .secondary)
                        }
                    }
                    if project.destinationMissing {
                        Label("The project folder is gone. Use Move Chats to point them at its new place.", systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(Theme.warning)
                    }
                }
                Section("Chats") {
                    DetailRow(label: "Chats", value: Format.count(project.chats))
                    DetailRow(label: "Agent chats", value: Format.count(project.subagents))
                    if let archived = project.archived {
                        DetailRow(label: "Archived", value: Format.count(archived))
                    }
                    if let memory = project.memoryFiles {
                        DetailRow(label: "Memory files", value: Format.count(memory))
                    }
                    DetailRow(label: "Size", value: Format.bytes(project.sizeBytes))
                    DetailRow(label: "Last active", value: lastActive)
                }
                Section("Where") {
                    DetailRow(label: "Folder", value: project.path.map(Format.path) ?? "None")
                    DetailRow(label: "Chat storage", value: Format.path(project.storageDir))
                    if let label = project.profileLabel {
                        DetailRow(label: "Installation", value: label)
                    }
                    DetailRow(label: "ID", value: project.projectID, monospaced: true)
                }
                if project.tool.isKnown {
                    actionSection(project)
                }
            }
            .formStyle(.grouped)
        } else {
            ContentUnavailableView("No Project Selected", systemImage: "info.circle")
        }
    }

    private func actionSection(_ project: Project) -> some View {
        Section {
            HStack {
                Button("Move…") { app.present(.move(project)) }
                Button("Copy…") { app.present(.copy(project)) }
                Button("Export…") { app.present(.export(project)) }
            }
            if project.isUnsaved {
                Button("Save to Folder…") { app.present(.saveUnsaved(project)) }
            }
        }
    }

    private var lastActive: String {
        guard let project = model.selectedProject else { return "Unknown" }
        guard let date = project.lastActive ?? model.detail.value?.lastActive else { return "No chats" }
        return Format.date(date)
    }
}
