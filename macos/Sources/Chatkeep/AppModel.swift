import AppKit
import ChatkeepKit
import Observation
import SwiftUI

/// The screens in the sidebar.
enum Screen: String, Hashable, CaseIterable, Identifiable {
    case projects
    case queue
    case history
    case stats
    case accounts
    case maintenance

    var id: Self { self }

    var title: String {
        switch self {
        case .projects: "Projects"
        case .queue: "Queue"
        case .history: "History"
        case .stats: "Stats"
        case .accounts: "Accounts & Sync"
        case .maintenance: "Maintenance"
        }
    }

    var systemImage: String {
        switch self {
        case .projects: "folder"
        case .queue: "tray.full"
        case .history: "clock.arrow.circlepath"
        case .stats: "chart.bar"
        case .accounts: "person.2"
        case .maintenance: "wrench.and.screwdriver"
        }
    }
}

/// Every action that changes data. Each one opens a sheet: fill in, preview, then apply.
enum Action {
    case move(Project?, to: URL? = nil)
    case copy(Project?, to: URL? = nil)
    case remove(Project, chat: Chat? = nil)
    case split(Project)
    case combine(into: Project?, sources: [String] = [])
    case export(Project)
    case importArchive(file: URL?, into: URL? = nil)
    case saveUnsaved(Project, to: URL? = nil)
    case reindex(Project?)
    case clearCursorIndex
    case clearClaudeIndex
    case pruneAccounts
    case runQueue
    case retryQueue([String])
    case removeFromQueue([String])
    case clearQueue
    case copyAccountChats(from: Account?)
    case syncNow(profile: String?)
    case editSyncProfile(name: String?, adding: Bool)
    case removeFromSyncProfile(name: String, members: [String])
}

/// One open action sheet. A new id per opening, so the sheet starts fresh every time.
struct PresentedAction: Identifiable {
    let id = UUID()
    let action: Action
}

/// The result of a read the window shows, loaded off the main thread and stoppable.
@Observable
final class Loadable<Value> {
    private(set) var value: Value?
    var error: ChatkeepError?
    private(set) var isLoading = false
    private var task: Task<Void, Never>?

    // Swift 6.3 (Xcode 26.6) crashes while optimizing the implicit deinit of this generic
    // main-actor class in a release build. Writing one out avoids it.
    deinit {}

    var hasLoaded: Bool { value != nil }

    /// Runs `command` and keeps what `transform` makes of the answer; `nil` counts as an error.
    func load<Payload>(
        _ command: Command<Payload>,
        with runner: ChatkeepRunner,
        _ transform: @escaping (Response<Payload>) -> Value?
    ) {
        task?.cancel()
        isLoading = true
        error = nil
        task = Task {
            do throws(ChatkeepError) {
                let response = try await runner.run(command)
                guard !Task.isCancelled else { return }
                if let loaded = transform(response) {
                    value = loaded
                } else {
                    error = ChatkeepError(kind: .unreadableOutput, message: "The chatkeep tool gave no result for \(response.command).")
                }
            } catch {
                guard !Task.isCancelled else { return }
                if error.kind != .cancelled { self.error = error }
            }
            isLoading = false
        }
    }

    /// Stops a read. A write keeps running in the tool; only its answer is no longer shown.
    func stop() {
        task?.cancel()
        task = nil
        isLoading = false
    }

    func clear() {
        stop()
        value = nil
        error = nil
    }
}

/// A folder or file dropped somewhere it can mean more than one thing.
struct DropChoice: Identifiable {
    let id = UUID()
    let url: URL
    let project: Project
}

@Observable
final class AppModel {
    let runner: ChatkeepRunner
    var screen: Screen = .projects
    var presented: PresentedAction?
    var dropChoice: DropChoice?
    /// A message for the whole window, such as a drop that matched nothing.
    var notice: ChatkeepError?

    let projects: ProjectsModel
    let queue: QueueModel
    let history: HistoryModel
    let stats: StatsModel
    let accounts: AccountsModel
    let maintenance: MaintenanceModel

    init(runner: ChatkeepRunner) {
        self.runner = runner
        projects = ProjectsModel(runner: runner)
        queue = QueueModel(runner: runner)
        history = HistoryModel(runner: runner)
        stats = StatsModel(runner: runner)
        accounts = AccountsModel(runner: runner)
        maintenance = MaintenanceModel(runner: runner)
    }

    func present(_ action: Action) {
        presented = PresentedAction(action: action)
    }

    /// After a write: read again what it may have changed. Screens not opened yet load when shown.
    func dataChanged() {
        projects.reload()
        queue.load()
        if history.list.hasLoaded { history.load() }
        if accounts.list.hasLoaded { accounts.loadAll() }
        if maintenance.cursorIndex.hasLoaded || maintenance.claudeIndex.hasLoaded { maintenance.load() }
    }

    // MARK: - Drag and drop

    /// Files and folders dropped on the window. A `.chatkeep` archive opens the import; a folder
    /// shows the project kept for it.
    @discardableResult
    func open(_ urls: [URL]) -> Bool {
        if let archive = urls.first(where: Self.isArchive) {
            present(.importArchive(file: archive))
            return true
        }
        guard let folder = urls.first(where: Self.isFolder) else { return false }
        screen = .projects
        if !projects.reveal(folder: folder) {
            notice = ChatkeepError(
                kind: .failed,
                message: "No chats are kept for \(Format.path(folder.path(percentEncoded: false))).",
                hint: "Drop the folder on a project instead to move or copy that project's chats there."
            )
        }
        return true
    }

    /// A folder or archive dropped on one project.
    func drop(_ urls: [URL], on project: Project) -> Bool {
        guard project.tool.isKnown else { return false }
        if let archive = urls.first(where: Self.isArchive) {
            present(.importArchive(file: archive, into: project.path.map { URL(filePath: $0, directoryHint: .isDirectory) }))
            return true
        }
        guard let folder = urls.first(where: Self.isFolder) else { return false }
        dropChoice = DropChoice(url: folder, project: project)
        return true
    }

    static func isArchive(_ url: URL) -> Bool {
        url.pathExtension.lowercased() == "chatkeep"
    }

    static func isFolder(_ url: URL) -> Bool {
        (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true
    }
}
