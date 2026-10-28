import ChatkeepKit
import Foundation
import Observation

@Observable
final class ProjectsModel {
    enum Filter: String, CaseIterable, Identifiable {
        case all
        case cursor
        case claude
        case unsaved

        var id: Self { self }

        var title: String {
            switch self {
            case .all: "All"
            case .cursor: "Cursor"
            case .claude: "Claude Code"
            case .unsaved: "Unsaved Cursor Windows"
            }
        }

        /// For the segmented control in the toolbar.
        var shortTitle: String {
            switch self {
            case .unsaved: "Unsaved"
            default: title
            }
        }
    }

    let runner: ChatkeepRunner
    var filter = Filter.all {
        didSet {
            if filter == .unsaved, !unsavedList.hasLoaded, !unsavedList.isLoading { loadUnsaved(fresh: false) }
        }
    }
    var search = ""
    /// Newest first; projects without chats (no last change) at the end.
    var sortOrder: [KeyPathComparator<Project>]
    var selection: Project.ID? {
        didSet { if selection != oldValue { loadDetail() } }
    }

    let list = Loadable<[ToolResult<ProjectList>]>()
    let unsavedList = Loadable<[ToolResult<ProjectList>]>()
    let detail = Loadable<ProjectDetail>()

    init(runner: ChatkeepRunner) {
        self.runner = runner
        sortOrder = [KeyPathComparator(\Project.lastActiveOrNever, order: .reverse)]
    }

    // MARK: - Loading

    func load(fresh: Bool = false) {
        list.load(CLI.projects(fresh: fresh), with: runner) { $0.results }
        if unsavedList.hasLoaded || filter == .unsaved { loadUnsaved(fresh: fresh) }
    }

    private func loadUnsaved(fresh: Bool) {
        unsavedList.load(CLI.projects(scope: Scope(tool: .cursor), unsaved: true, fresh: fresh), with: runner) { $0.results }
    }

    /// Reads the lists and the open project again, for instance after a change.
    func reload() {
        guard list.hasLoaded || list.isLoading else { return }
        load()
        loadDetail()
    }

    private func loadDetail() {
        guard let project = selectedProject else {
            detail.clear()
            return
        }
        let scope = Scope(tool: project.tool, profile: project.profileLabel)
        if detail.value?.project.id != project.id { detail.clear() }
        detail.load(CLI.project(project.projectID, scope: scope, unsaved: project.isUnsaved), with: runner) { response in
            let details = response.data(for: project.tool)?.details ?? []
            return details.first { $0.project.id == project.id } ?? details.first
        }
    }

    var isLoading: Bool {
        filter == .unsaved ? unsavedList.isLoading : list.isLoading
    }

    var error: ChatkeepError? {
        get { filter == .unsaved ? unsavedList.error : list.error }
        set {
            if filter == .unsaved { unsavedList.error = newValue } else { list.error = newValue }
        }
    }

    func stop() {
        list.stop()
        unsavedList.stop()
    }

    // MARK: - What the table shows

    private var results: [ToolResult<ProjectList>] {
        (filter == .unsaved ? unsavedList.value : list.value) ?? []
    }

    var allProjects: [Project] {
        results.flatMap(\.data.projects)
    }

    var visible: [Project] {
        var projects = allProjects.filter { project in
            switch filter {
            case .all, .unsaved: true
            case .cursor: project.tool == .cursor
            case .claude: project.tool == .claude
            }
        }
        if let query = search.trimmedOrNil {
            projects = projects.filter {
                $0.name.localizedCaseInsensitiveContains(query)
                    || ($0.path ?? "").localizedCaseInsensitiveContains(query)
                    || $0.projectID.localizedCaseInsensitiveContains(query)
            }
        }
        if !sortOrder.isEmpty { projects.sort(using: sortOrder) }
        return projects
    }

    var selectedProject: Project? {
        guard let selection else { return nil }
        return (list.value ?? []).flatMap(\.data.projects).first { $0.id == selection }
            ?? (unsavedList.value ?? []).flatMap(\.data.projects).first { $0.id == selection }
    }

    /// When Cursor's rows came from chatkeep's index, and whether it is being refreshed now.
    var indexNote: String? {
        guard let cursor = results.first(where: { $0.tool == .cursor })?.data else { return nil }
        if cursor.indexRefreshing { return "Cursor list is being refreshed in the background" }
        guard let asOf = cursor.indexAsOf else { return nil }
        return "Cursor list as of \(Format.relative(asOf))"
    }

    /// The Cursor installations (and VS Code profiles) seen in the list, for `--profile`.
    var cursorProfiles: [String] {
        Array(Set(allProjects.compactMap(\.profileLabel))).sorted()
    }

    var missingCount: Int {
        allProjects.count { $0.destinationMissing }
    }

    /// Selects the project kept for `folder`. False when no tool keeps chats for it.
    func reveal(folder: URL) -> Bool {
        let path = folder.standardizedFileURL.path(percentEncoded: false)
        let trimmed = path.hasSuffix("/") ? String(path.dropLast()) : path
        guard let project = (list.value ?? []).flatMap(\.data.projects).first(where: { $0.path == trimmed }) else {
            return false
        }
        filter = .all
        search = ""
        selection = project.id
        return true
    }
}

nonisolated extension Project {
    /// The path, or empty, so the table can sort on it.
    var pathText: String { path ?? "" }

    /// The name to show first: the folder's own name, the file name of a `.code-workspace`, or
    /// for a Claude Code worktree the project it belongs to and the worktree's name.
    var displayName: String {
        if isUnsaved { return "Unsaved window" }
        guard let path else { return name }
        let url = URL(filePath: path)
        if kind == .codeWorkspace { return url.deletingPathExtension().lastPathComponent }
        let parts = url.pathComponents
        if let index = parts.lastIndex(of: "worktrees"), index >= 2, index + 1 < parts.count,
           parts[index - 1] == ".claude" {
            return "\(parts[index - 2]) (worktree \(parts[index + 1]))"
        }
        return url.lastPathComponent
    }

    /// The last change, or the earliest date for a project without chats, so those sort last
    /// when the newest come first.
    var lastActiveOrNever: Date { lastActive ?? .distantPast }
}
