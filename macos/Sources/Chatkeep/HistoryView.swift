import ChatkeepKit
import SwiftUI

@Observable
final class HistoryModel {
    /// One line of the log; the log has no ids, so rows count from the oldest.
    nonisolated struct Row: Identifiable, Sendable {
        let id: Int
        let entry: History.Entry

        var text: String { ([entry.command ?? ""] + entry.args).joined(separator: " ") }
        var at: Date { entry.at ?? .distantPast }
        var duration: Int64 { entry.durationMs ?? -1 }
    }

    let runner: ChatkeepRunner
    let list = Loadable<History>()
    var search = ""
    var sortOrder: [KeyPathComparator<Row>]

    init(runner: ChatkeepRunner) {
        self.runner = runner
        sortOrder = [KeyPathComparator(\Row.at, order: .reverse)]
    }

    func load() {
        list.load(CLI.history(), with: runner) { $0.data }
    }

    var rows: [Row] {
        let entries = list.value?.entries ?? []
        var rows = entries.enumerated().map { Row(id: $0.offset, entry: $0.element) }
        if let query = search.trimmedOrNil {
            rows = rows.filter { $0.text.localizedCaseInsensitiveContains(query) }
        }
        return rows.sorted(using: sortOrder)
    }
}

struct HistoryView: View {
    @Bindable var model: HistoryModel

    var body: some View {
        Table(model.rows, sortOrder: $model.sortOrder) {
            TableColumn("When", value: \.at) { row in
                Text(Format.date(row.entry.at)).foregroundStyle(.secondary)
            }
            .width(min: 120, ideal: 150)
            TableColumn("Command", value: \.text) { row in
                Text(row.text)
                    .font(.callout.monospaced())
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(row.text)
            }
            .width(min: 260, ideal: 520)
            TableColumn("Result") { row in
                switch row.entry.outcome {
                case .ok: Pill(text: "Done", color: .green, systemImage: "checkmark")
                case .error: Pill(text: "Failed", color: Theme.failure, systemImage: "xmark")
                case .unknown(let raw): Text(raw).foregroundStyle(.secondary)
                case nil: Text("")
                }
            }
            .width(80)
            TableColumn("Took", value: \.duration) { row in
                Text(row.entry.durationMs.map(Format.duration) ?? "").monospacedDigit().foregroundStyle(.secondary)
            }
            .width(70)
        }
        .searchable(text: $model.search, placement: .toolbar, prompt: "Command or path")
        .overlay {
            if model.list.isLoading, model.rows.isEmpty {
                LoadingView(text: "Reading the history…")
            } else if model.list.hasLoaded, model.list.value?.entries.isEmpty == true {
                ContentUnavailableView(
                    "No History Yet",
                    systemImage: "clock",
                    description: Text("Every change chatkeep makes is listed here.")
                )
            } else if model.list.hasLoaded, model.rows.isEmpty {
                ContentUnavailableView.search(text: model.search)
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 0) {
                if let error = model.list.error {
                    ErrorBanner(error: error) { model.list.error = nil }.padding(10)
                }
                if let history = model.list.value, !history.path.isEmpty {
                    HStack {
                        Text(footer(history))
                        Spacer()
                        Button("Show Log File") { Reveal.inFinder(history.path) }
                            .buttonStyle(.link)
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(.bar)
                }
            }
        }
        .toolbar {
            Button("Refresh", systemImage: "arrow.clockwise") { model.load() }
                .help("Read the history again")
        }
        .task { model.load() }
    }

    private func footer(_ history: History) -> String {
        let unreadable = history.unreadable > 0 ? " · \(history.unreadable) lines could not be read" : ""
        return "\(history.entries.count) commands" + unreadable
    }
}
