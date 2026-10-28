import ChatkeepKit
import SwiftUI

@Observable
final class QueueModel {
    let runner: ChatkeepRunner
    let list = Loadable<QueueList>()
    var selection: Set<QueueEntry.ID> = []

    init(runner: ChatkeepRunner) {
        self.runner = runner
    }

    func load() {
        list.load(CLI.queueList(), with: runner) { $0.data }
    }

    var entries: [QueueEntry] { list.value?.entries ?? [] }
    var pendingCount: Int { list.value?.pendingCount ?? 0 }
    var selectedEntries: [QueueEntry] { entries.filter { selection.contains($0.id) } }
}

struct QueueView: View {
    @Bindable var model: QueueModel
    @State private var showsDetails = true
    @Environment(AppModel.self) private var app

    var body: some View {
        Table(model.entries, selection: $model.selection) {
            TableColumn("#") { entry in
                Text(entry.position.map(String.init) ?? "").monospacedDigit().foregroundStyle(.secondary)
            }
            .width(28)
            TableColumn("Status") { entry in
                Pill(text: Self.label(entry), color: Self.color(entry.status), systemImage: Self.symbol(entry.status))
            }
            .width(min: 90, ideal: 110)
            TableColumn("Command") { entry in
                Text(entry.command)
                    .font(.callout.monospaced())
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(entry.command)
            }
            .width(min: 240, ideal: 420)
            TableColumn("Tool") { entry in
                if let tool = entry.tool { ToolTag(tool: tool) } else { Text("Decided when it runs").foregroundStyle(.tertiary) }
            }
            .width(min: 90, ideal: 130)
            TableColumn("Added") { entry in
                Text(Format.date(entry.createdAt)).foregroundStyle(.secondary)
            }
            .width(min: 110, ideal: 140)
        }
        .contextMenu(forSelectionType: QueueEntry.ID.self) { ids in
            let picked = model.entries.filter { ids.contains($0.id) }
            if picked.contains(where: \.canRetry) {
                Button("Try Again…") { app.present(.retryQueue(picked.filter(\.canRetry).map(\.id))) }
            }
            Button("Copy Command") { Reveal.copy(picked.map(\.command).joined(separator: "\n")) }
            Divider()
            Button("Remove…", role: .destructive) { app.present(.removeFromQueue(picked.map(\.id))) }
        }
        .overlay {
            if model.list.isLoading, model.entries.isEmpty {
                LoadingView(text: "Reading the queue…")
            } else if model.list.hasLoaded, model.entries.isEmpty {
                ContentUnavailableView(
                    "The Queue Is Empty",
                    systemImage: "tray",
                    description: Text("When a change cannot run because Cursor or Claude Code is open, you can add it here and run it later.")
                )
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if let error = model.list.error {
                ErrorBanner(error: error) { model.list.error = nil }.padding(10)
            }
        }
        .inspector(isPresented: $showsDetails) {
            QueueEntryDetails(entries: model.selectedEntries)
                .inspectorColumnWidth(min: 240, ideal: 300, max: 420)
        }
        .navigationSubtitle(subtitle)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button("Refresh", systemImage: "arrow.clockwise") { model.load() }
                    .help("Read the queue again")
                Button("Run Queue", systemImage: "play.fill") { app.present(.runQueue) }
                    .disabled(model.pendingCount == 0)
                    .help("Run every waiting change, once Cursor and Claude Code are closed")
                Button("Try Again", systemImage: "arrow.uturn.forward") {
                    let picked = model.selectedEntries.filter(\.canRetry).map(\.id)
                    app.present(.retryQueue(picked))
                }
                .disabled(!model.entries.contains(where: \.canRetry))
                .help("Put the selected failed entries back in line, or every failed one when none is selected")
                Button("Remove", systemImage: "minus.circle") { app.present(.removeFromQueue(Array(model.selection))) }
                    .disabled(model.selection.isEmpty)
                    .help("Take the selected entries out of the queue")
                Button("Clear", systemImage: "trash") { app.present(.clearQueue) }
                    .disabled(model.entries.isEmpty)
                    .help("Remove every entry")
                Button("Details", systemImage: "sidebar.trailing") { showsDetails.toggle() }
            }
        }
        .task { model.load() }
    }

    private var subtitle: String {
        let entries = model.entries
        guard !entries.isEmpty else { return "" }
        let failed = entries.count { $0.status == .failed || $0.status == .skipped }
        var parts = ["\(model.pendingCount) waiting"]
        if failed > 0 { parts.append("\(failed) failed") }
        return parts.joined(separator: " · ")
    }

    static func label(_ entry: QueueEntry) -> String {
        switch entry.status {
        case .pending: "Waiting"
        case .running: "Running"
        case .failed: entry.interrupted ? "Stopped" : "Failed"
        case .skipped: "Skipped"
        case .unknown(let raw): raw
        }
    }

    static func color(_ status: QueueEntry.Status) -> Color {
        switch status {
        case .pending: Theme.teal
        case .running: .blue
        case .failed: Theme.failure
        case .skipped: Theme.warning
        case .unknown: .secondary
        }
    }

    static func symbol(_ status: QueueEntry.Status) -> String {
        switch status {
        case .pending: "clock"
        case .running: "play.fill"
        case .failed: "xmark"
        case .skipped: "forward.fill"
        case .unknown: "questionmark"
        }
    }
}

struct QueueEntryDetails: View {
    let entries: [QueueEntry]

    var body: some View {
        if entries.count == 1, let entry = entries.first {
            Form {
                Section {
                    Text(entry.command)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                    if let error = entry.lastError {
                        Label(error, systemImage: "xmark.octagon")
                            .foregroundStyle(Theme.failure)
                            .textSelection(.enabled)
                    }
                    ForEach(entry.warnings, id: \.self) { warning in
                        Label(warning, systemImage: "exclamationmark.triangle").foregroundStyle(Theme.warning)
                    }
                }
                Section {
                    DetailRow(label: "Status", value: QueueView.label(entry))
                    if let position = entry.position { DetailRow(label: "Place in line", value: "\(position)") }
                    DetailRow(label: "Added", value: Format.date(entry.createdAt))
                    DetailRow(label: "Run from", value: Format.path(entry.cwd))
                    if let profile = entry.profile { DetailRow(label: "Installation", value: profile) }
                    DetailRow(label: "Added by chatkeep", value: entry.toolVersion)
                    DetailRow(label: "ID", value: entry.id, monospaced: true)
                }
            }
            .formStyle(.grouped)
        } else {
            ContentUnavailableView(
                entries.isEmpty ? "No Entry Selected" : "\(entries.count) Entries Selected",
                systemImage: "tray"
            )
        }
    }
}
