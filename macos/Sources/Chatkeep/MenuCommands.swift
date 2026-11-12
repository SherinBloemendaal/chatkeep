import AppKit
import ChatkeepKit
import SwiftUI

/// The menu bar: every action of the app, in the menu of the thing it works on. An item that
/// cannot work right now is greyed out rather than hidden, and each one opens the same sheet or
/// runs the same code as its button on the page.
struct MenuCommands: Commands {
    let app: AppModel

    /// The project the Project and File menus act on: the selected one, while Projects shows.
    private var project: Project? {
        app.screen == .projects ? app.projects.selectedProject : nil
    }

    /// A project of a tool this app knows; a newer tool's projects can only be looked at.
    private var editable: Project? {
        guard let project, project.tool.isKnown else { return nil }
        return project
    }

    var body: some Commands {
        fileMenu
        editMenu
        viewMenu
        projectMenu
        queueMenu
        accountsMenu
        toolsMenu
        helpMenu
    }

    // MARK: - File

    private var fileMenu: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Import Archive…") { app.present(.importArchive(file: nil)) }
                .keyboardShortcut("o")
            Button("Export Chats…") {
                if let editable { app.present(.export(editable)) }
            }
            .keyboardShortcut("e", modifiers: [.command, .shift])
            .disabled(editable == nil)
            Divider()
            Button("Show Folder in Finder") {
                if let path = project?.path { Reveal.inFinder(path) }
            }
            .disabled(project?.path == nil)
            Button("Show Chat Storage in Finder") {
                if let project { Reveal.inFinder(project.storageDir) }
            }
            .disabled(project == nil)
            Button("Copy Path") {
                if let project { Reveal.copy(project.path ?? project.projectID) }
            }
            .keyboardShortcut("c", modifiers: [.command, .option])
            .disabled(project == nil)
            Divider()
            Button("Show History Log File") {
                if let path = app.history.list.value?.path { Reveal.inFinder(path) }
            }
            .disabled(app.history.list.value?.path.isEmpty ?? true)
        }
    }

    // MARK: - Edit

    private var editMenu: some Commands {
        CommandGroup(after: .textEditing) {
            Button("Find…") { app.searchRequests += 1 }
                .keyboardShortcut("f")
                .disabled(app.screen != .projects && app.screen != .history)
        }
    }

    // MARK: - View

    private var viewMenu: some Commands {
        CommandGroup(after: .toolbar) {
            Button("Refresh") { refresh(fresh: false) }
                .keyboardShortcut("r")
            Button("Read Fresh") { refresh(fresh: true) }
                .keyboardShortcut("r", modifiers: [.command, .shift])
                .disabled(app.screen != .projects && app.screen != .stats)
            Divider()
            ForEach(Array(Screen.allCases.enumerated()), id: \.element) { index, screen in
                Button(screen.title) { app.screen = screen }
                    .keyboardShortcut(KeyEquivalent(Character(String(index + 1))), modifiers: .command)
            }
            Divider()
            Picker("Projects of", selection: Binding(get: { app.projects.filter }, set: { show($0) })) {
                ForEach(ProjectsModel.Filter.allCases) { Text($0.title).tag($0) }
            }
            Picker("Accounts & Sync Page", selection: Binding(get: { app.accounts.tab }, set: { show($0) })) {
                ForEach(AccountsModel.Tab.allCases) { Text($0.title).tag($0) }
            }
            Picker("Stats of", selection: Binding(get: { app.stats.tool }, set: { show($0) })) {
                Text(Tool.claude.label).tag(Tool.claude)
                Text(Tool.cursor.label).tag(Tool.cursor)
            }
            Divider()
        }
    }

    // MARK: - Project

    private var projectMenu: some Commands {
        CommandMenu("Project") {
            Button("Move Chats…") {
                if let editable { app.present(.move(editable)) }
            }
            .keyboardShortcut("m", modifiers: [.command, .shift])
            .disabled(editable == nil)
            Button("Copy Chats…") {
                if let editable { app.present(.copy(editable)) }
            }
            .disabled(editable == nil)
            Button("Split…") {
                if let editable { app.present(.split(editable)) }
            }
            .disabled(editable == nil)
            Button("Combine Into This Project…") {
                if let editable { app.present(.combine(into: editable)) }
            }
            .disabled(editable == nil)
            Divider()
            Button("Save Window to Folder…") {
                if let editable { app.present(.saveUnsaved(editable)) }
            }
            .disabled(editable?.isUnsaved != true)
            Button("Repair…") {
                if let editable { app.present(.reindex(editable)) }
            }
            .disabled(!(editable?.tool == .cursor && editable?.isUnsaved == false))
            Divider()
            Button("Rewrite Paths…") { app.present(.move(nil)) }
            Button("Combine Chats…") { app.present(.combine(into: nil)) }
            Divider()
            Button("Remove Chats…") {
                if let editable { app.present(.remove(editable)) }
            }
            .disabled(editable == nil)
        }
    }

    // MARK: - Queue

    private var queueMenu: some Commands {
        CommandMenu("Queue") {
            Button("Run Queue…") { app.present(.runQueue) }
                .keyboardShortcut(.return, modifiers: [.command, .shift])
                .disabled(app.queue.pendingCount == 0)
            Divider()
            Button("Try Selected Entries Again…") {
                app.present(.retryQueue(app.queue.selectedEntries.filter(\.canRetry).map(\.id)))
            }
            .disabled(app.screen != .queue || !app.queue.selectedEntries.contains(where: \.canRetry))
            Button("Try Every Failed Entry Again…") { app.present(.retryQueue([])) }
                .disabled(!app.queue.entries.contains(where: \.canRetry))
            Divider()
            Button("Remove Selected Entries…") {
                app.present(.removeFromQueue(app.queue.selectedEntries.map(\.id)))
            }
            .disabled(app.screen != .queue || app.queue.selectedEntries.isEmpty)
            Button("Clear Queue…") { app.present(.clearQueue) }
                .disabled(app.queue.entries.isEmpty)
        }
    }

    // MARK: - Accounts

    private var accountsMenu: some Commands {
        CommandMenu("Accounts") {
            Button("Sync Now…") { app.present(.syncNow(profile: nil)) }
                .keyboardShortcut("s", modifiers: [.command, .shift])
            Menu("Decide on Deleted Chats") {
                ForEach(app.accounts.profilesWithPending, id: \.self) { profile in
                    Button("\(profile) (\(app.accounts.pending(in: profile).count))…") {
                        app.present(.decideSync(profile: profile))
                    }
                }
            }
            .disabled(app.accounts.pendingCount == 0)
            Toggle("Background Sync", isOn: Binding(
                get: { app.accounts.auto.value?.state == .on },
                set: { app.accounts.setAuto($0) }
            ))
            .disabled(app.accounts.auto.value == nil || app.accounts.isSwitchingAuto)
            Divider()
            Button("New Sync Profile…") { app.present(.editSyncProfile(name: nil, adding: false)) }
            Divider()
            Button("Name Account…") {
                if let account = selectedAccount { app.accounts.startRenaming(account) }
            }
            .disabled(selectedAccount == nil)
            Button("Copy Chats to Another Account…") { app.present(.copyAccountChats(from: selectedAccount)) }
            Button("Show Chat List in Finder") {
                if let account = selectedAccount { Reveal.inFinder(account.dir) }
            }
            .disabled(selectedAccount == nil)
            Divider()
            Button("Clean Up Old Accounts…") { app.present(.pruneAccounts) }
        }
    }

    /// The account the Accounts menu acts on: the selected one, while the accounts list shows.
    private var selectedAccount: Account? {
        app.screen == .accounts && app.accounts.tab == .accounts ? app.accounts.selectedAccount : nil
    }

    // MARK: - Tools

    private var toolsMenu: some Commands {
        CommandMenu("Tools") {
            Button("Check Cursor Index for Changes") {
                app.screen = .maintenance
                app.maintenance.checkCursorIndex()
            }
            .disabled(isScanning)
            Button("Update Cursor Index") { scanCursorIndex(full: false) }
                .disabled(isScanning)
            Button("Rebuild Cursor Index") { scanCursorIndex(full: true) }
                .disabled(isScanning)
            Button("Clear Cursor Index…") { app.present(.clearCursorIndex) }
                .disabled(isScanning)
            Divider()
            Button("Clear Claude Code Index…") { app.present(.clearClaudeIndex) }
            Divider()
            Button("Repair a Cursor Project…") { app.present(.reindex(nil)) }
        }
    }

    private var isScanning: Bool {
        app.maintenance.scan.isLoading
    }

    // MARK: - Help

    private var helpMenu: some Commands {
        CommandGroup(replacing: .help) {
            Button("Chatkeep Help") { open(AboutView.repository.appending(path: "blob/main/README.md")) }
            Button("Release Notes") { open(AboutView.repository.appending(path: "releases")) }
            Divider()
            Button("Report a Problem…") { open(AboutView.repository.appending(path: "issues/new")) }
            Button("Chatkeep on GitHub") { open(AboutView.repository) }
        }
    }

    // MARK: - What the items do

    private func open(_ url: URL) {
        NSWorkspace.shared.open(url)
    }

    private func refresh(fresh: Bool) {
        switch app.screen {
        case .projects: app.projects.load(fresh: fresh)
        case .queue: app.queue.load()
        case .history: app.history.load()
        case .stats: app.stats.load(fresh: fresh)
        case .accounts: app.accounts.loadAll()
        case .maintenance: app.maintenance.load()
        }
    }

    /// Choosing what a page shows also goes to that page.
    private func show(_ filter: ProjectsModel.Filter) {
        app.screen = .projects
        app.projects.filter = filter
    }

    private func show(_ tab: AccountsModel.Tab) {
        app.screen = .accounts
        app.accounts.tab = tab
    }

    private func show(_ tool: Tool) {
        app.screen = .stats
        app.stats.tool = tool
    }

    /// The result shows on the Maintenance page, so the page opens with it.
    private func scanCursorIndex(full: Bool) {
        app.screen = .maintenance
        app.maintenance.scanCursorIndex(full: full) { app.maintenance.load() }
    }
}
