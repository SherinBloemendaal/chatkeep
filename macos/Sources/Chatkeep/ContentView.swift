import ChatkeepKit
import SwiftUI

struct ContentView: View {
    @Bindable var app: AppModel
    /// The sidebar shows from the start and comes back when the window grows again.
    @State private var columns = NavigationSplitViewVisibility.all

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            List(selection: Binding(get: { app.screen }, set: { if let screen = $0 { app.screen = screen } })) {
                Section {
                    row(.projects)
                    row(.queue)
                        .badge(app.queue.pendingCount)
                    row(.history)
                    row(.stats)
                }
                Section("Claude Desktop App") {
                    row(.accounts)
                }
                Section {
                    row(.maintenance)
                }
            }
            .navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 300)
        } detail: {
            detail
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .navigationTitle(app.screen.title)
        }
        .navigationSplitViewStyle(.balanced)
        .tint(Theme.teal)
        .environment(app)
        .dropDestination(for: URL.self) { urls, _ in
            app.open(urls)
        }
        .sheet(item: $app.presented) { presented in
            ActionSheetView(action: presented.action)
                .environment(app)
        }
        .confirmationDialog(
            dropTitle,
            isPresented: Binding(get: { app.dropChoice != nil }, set: { if !$0 { app.dropChoice = nil } }),
            titleVisibility: .visible,
            presenting: app.dropChoice
        ) { choice in
            Button("Move Chats There…") { app.present(.move(choice.project, to: choice.url)) }
            Button("Copy Chats There…") { app.present(.copy(choice.project, to: choice.url)) }
            if choice.project.isUnsaved {
                Button("Save Window to This Folder…") { app.present(.saveUnsaved(choice.project, to: choice.url)) }
            }
            Button("Combine Its Chats Into This Project…") {
                app.present(.combine(into: choice.project, sources: [choice.url.path(percentEncoded: false)]))
            }
            Button("Cancel", role: .cancel) {}
        } message: { choice in
            Text("What should happen with the chats of \(choice.project.name) and \(Format.path(choice.url.path(percentEncoded: false)))?")
        }
        .errorAlert($app.notice)
        .task {
            app.queue.load()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            // The queue changes from the terminal too; keep the badge right.
            app.queue.load()
        }
    }

    private var dropTitle: String {
        guard let choice = app.dropChoice else { return "" }
        return "\(choice.url.lastPathComponent) on \(choice.project.name)"
    }

    private func row(_ screen: Screen) -> some View {
        Label(screen.title, systemImage: screen.systemImage).tag(screen)
    }

    @ViewBuilder
    private var detail: some View {
        switch app.screen {
        case .projects: ProjectsView(model: app.projects)
        case .queue: QueueView(model: app.queue)
        case .history: HistoryView(model: app.history)
        case .stats: StatsView(model: app.stats)
        case .accounts: AccountsView(model: app.accounts)
        case .maintenance: MaintenanceView(model: app.maintenance)
        }
    }
}

/// Shown instead of the window when the app carries no chatkeep tool.
struct MissingToolView: View {
    let error: ChatkeepError

    var body: some View {
        ContentUnavailableView {
            Label("Chatkeep Cannot Start", systemImage: "exclamationmark.triangle")
        } description: {
            Text(error.message)
        } actions: {
            Link("Download Chatkeep", destination: AboutView.repository.appending(path: "releases"))
        }
        .frame(minWidth: 520, minHeight: 360)
    }
}
