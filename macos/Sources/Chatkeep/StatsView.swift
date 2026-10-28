import Charts
import ChatkeepKit
import SwiftUI

@Observable
final class StatsModel {
    let runner: ChatkeepRunner
    let list = Loadable<[ToolResult<Stats>]>()
    var tool = Tool.claude

    init(runner: ChatkeepRunner) {
        self.runner = runner
    }

    func load(fresh: Bool = false) {
        list.load(CLI.stats(fresh: fresh), with: runner) { $0.results }
    }

    var tools: [Tool] { (list.value ?? []).compactMap(\.tool) }

    var cursor: CursorStats? { list.value?.lazy.compactMap(\.data.cursor).first }
    var claude: ClaudeStats? { list.value?.lazy.compactMap(\.data.claude).first }
}

struct StatsView: View {
    @Bindable var model: StatsModel

    var body: some View {
        Group {
            if model.list.isLoading, !model.list.hasLoaded {
                LoadingView(text: "Counting chats and tokens… This can take a while the first time.") { model.list.stop() }
            } else if let error = model.list.error, !model.list.hasLoaded {
                VStack(spacing: 14) {
                    ErrorBanner(error: error)
                    Button("Try Again") { model.load() }
                        .primaryActionStyle()
                }
                .padding(40)
                .frame(maxWidth: 560)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if model.list.hasLoaded, model.tools.isEmpty {
                ContentUnavailableView("No Stats", systemImage: "chart.bar", description: Text("Neither Cursor nor Claude Code keeps chats on this Mac."))
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 20) {
                        switch model.tool {
                        case .claude:
                            if let stats = model.claude { ClaudeDashboard(stats: stats) } else { missing(.claude) }
                        case .cursor:
                            if let stats = model.cursor { CursorDashboard(stats: stats) } else { missing(.cursor) }
                        case .unknown:
                            missing(model.tool)
                        }
                    }
                    .padding(24)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .navigationSubtitle(subtitle)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("Tool", selection: $model.tool) {
                    ForEach(Tool.allCases.reversed()) { tool in
                        Label(tool.label, systemImage: Theme.symbol(for: tool)).tag(tool)
                    }
                }
                .pickerStyle(.segmented)
                .labelStyle(.titleAndIcon)
                .fixedSize()
            }
            ToolbarItemGroup(placement: .primaryAction) {
                if model.list.isLoading, model.list.hasLoaded {
                    ProgressView().controlSize(.small)
                    Button("Stop", systemImage: "stop.fill") { model.list.stop() }
                }
                Menu {
                    Button("Read Fresh", systemImage: "arrow.clockwise.circle") { model.load(fresh: true) }
                        .help("Read every chat again instead of from the index. Slow.")
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                } primaryAction: {
                    model.load()
                }
                .disabled(model.list.isLoading)
                .help("Count again (⌘R). Hold the menu for Read Fresh.")
            }
        }
        .task {
            if !model.list.hasLoaded, !model.list.isLoading { model.load() }
        }
    }

    private var subtitle: String {
        guard model.tool == .cursor, let stats = model.cursor else { return "" }
        if stats.indexRefreshing { return "Being refreshed in the background" }
        return stats.indexAsOf.map { "As of \(Format.relative($0))" } ?? ""
    }

    private func missing(_ tool: Tool) -> some View {
        ContentUnavailableView("No \(tool.label) Chats", systemImage: Theme.symbol(for: tool), description: Text("\(tool.label) keeps no chats on this Mac."))
    }
}

// MARK: - Claude Code

struct ClaudeDashboard: View {
    let stats: ClaudeStats

    var body: some View {
        StatGrid {
            StatCard(title: "Projects", value: Format.count(stats.projects), detail: stats.missing > 0 ? "\(stats.missing) with a missing folder" : nil, symbol: "folder")
            StatCard(title: "Sessions", value: Format.count(stats.sessions), detail: "\(Format.count(stats.subagents)) agent transcripts", symbol: "bubble.left.and.bubble.right")
            StatCard(title: "Tokens", value: Format.tokens(stats.tokens.total), detail: "\(Format.tokens(stats.subagentTokens.total)) more in agents", symbol: "number")
            StatCard(title: "On Disk", value: Format.bytes(stats.sizeBytes), detail: "\(Format.count(stats.memoryFiles)) memory files", symbol: "internaldrive")
        }
        HStack(alignment: .top, spacing: 16) {
            Card(title: "Tokens of the sessions") { TokenBreakdown(tokens: stats.tokens) }
            Card(title: "Tokens of the agents") { TokenBreakdown(tokens: stats.subagentTokens) }
        }
        Card(title: "Sessions per month") {
            MonthChart(months: stats.perMonth.map { ($0.month, $0.sessions) }, unit: "Sessions")
        }
        Card(title: "Models") {
            RankChart(rows: stats.models.map { ($0.model, Double($0.sessions)) }, unit: "Sessions", format: { Format.count(Int($0)) })
        }
        HStack(alignment: .top, spacing: 16) {
            Card(title: "Busiest projects") {
                RankList(rows: stats.busiest.map { ($0.name, Format.count($0.sessions)) })
            }
            Card(title: "Largest projects") {
                RankList(rows: stats.largest.map { ($0.name, Format.bytes($0.sizeBytes)) })
            }
        }
    }
}

struct TokenBreakdown: View {
    let tokens: ClaudeStats.Tokens

    var body: some View {
        VStack(spacing: 6) {
            DetailRow(label: "Read from cache", value: Format.tokens(tokens.cacheRead))
            DetailRow(label: "Written to cache", value: Format.tokens(tokens.cacheWrite))
            DetailRow(label: "Sent", value: Format.tokens(tokens.input))
            DetailRow(label: "Answered", value: Format.tokens(tokens.output))
        }
    }
}

// MARK: - Cursor

struct CursorDashboard: View {
    let stats: CursorStats

    var body: some View {
        StatGrid {
            StatCard(title: "Workspaces", value: Format.count(stats.workspaces), detail: "in \(stats.installations) installations", symbol: "folder")
            StatCard(title: "Chats", value: Format.count(stats.chats), detail: "\(Format.count(stats.subagents)) agent chats", symbol: "bubble.left.and.bubble.right")
            StatCard(title: "Archived", value: Format.count(stats.archived), detail: nil, symbol: "archivebox")
            StatCard(title: "Databases", value: stats.globalDbBytes.map(Format.bytes) ?? "Unknown", detail: "Cursor's own chat storage", symbol: "internaldrive")
        }
        Card(title: "Usage Cursor recorded") {
            let usage = stats.usage
            VStack(spacing: 6) {
                DetailRow(label: "Conversations", value: Format.count(usage.conversations))
                DetailRow(label: "Cost", value: "\(Format.money(cents: usage.costCents)) in \(Format.count(usage.costChats)) chats")
                DetailRow(label: "Requests", value: Format.count(Int(usage.requests)))
                DetailRow(label: "Context in use", value: "\(Format.tokens(usage.contextTokens)) tokens in \(Format.count(usage.contextChats)) chats")
                DetailRow(label: "Sent and answered", value: "\(Format.tokens(usage.inputTokens)) / \(Format.tokens(usage.outputTokens)) tokens in \(Format.count(usage.tokenChats)) chats")
            }
        }
        Card(title: "Chats per month") {
            MonthChart(months: stats.perMonth.map { ($0.month, $0.chats) }, unit: "Chats")
        }
        Card(title: "Models") {
            RankChart(rows: stats.models.map { ($0.model, Double($0.chats)) }, unit: "Chats", format: { Format.count(Int($0)) })
        }
        HStack(alignment: .top, spacing: 16) {
            Card(title: "Busiest workspaces") {
                RankList(rows: stats.busiest.map { ($0.name, Format.count($0.chats)) })
            }
            Card(title: "Largest workspace databases") {
                RankList(rows: stats.workspaceDbs.map { ($0.name, Format.bytes($0.bytes)) })
            }
        }
        HStack(alignment: .top, spacing: 16) {
            Card(title: "Installations") {
                VStack(spacing: 6) {
                    ForEach(stats.profiles, id: \.name) { profile in
                        DetailRow(
                            label: profile.userProfile ? "\(profile.name) (profile)" : profile.name,
                            value: "\(Format.count(profile.workspaces)) workspaces, \(Format.count(profile.chats)) chats"
                        )
                        .help(profile.root)
                    }
                }
            }
            Card(title: "Kinds of workspaces") {
                RankList(rows: stats.byKind.map { (Format.kind($0.kind), Format.count($0.workspaces)) })
            }
        }
    }
}

// MARK: - Building blocks

struct StatGrid<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        GlassGroup(spacing: 16) {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 200), spacing: 16)], spacing: 16) {
                content
            }
        }
    }
}

struct StatCard: View {
    let title: String
    let value: String
    let detail: String?
    let symbol: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(title, systemImage: symbol)
                .font(.callout)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(size: 28, weight: .semibold, design: .rounded))
                .monospacedDigit()
                .foregroundStyle(Theme.teal)
            Text(detail ?? " ")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .chatkeepGlass(in: RoundedRectangle(cornerRadius: 18), tint: Theme.brandTeal.opacity(0.08))
    }
}

struct Card<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline)
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .background(.quinary, in: RoundedRectangle(cornerRadius: 14))
    }
}

/// Counts per month as bars.
struct MonthChart: View {
    let months: [(month: String, count: Int)]
    let unit: String

    var body: some View {
        if months.isEmpty {
            Text("No dates recorded.").foregroundStyle(.secondary)
        } else {
            Chart(months, id: \.month) { row in
                BarMark(
                    x: .value("Month", Format.monthDate(row.month) ?? .distantPast, unit: .month),
                    y: .value(unit, row.count)
                )
                .foregroundStyle(Theme.teal.gradient)
                .cornerRadius(3)
            }
            .chartXAxis {
                AxisMarks(values: .stride(by: .month, count: months.count > 18 ? 3 : 1)) { _ in
                    AxisGridLine()
                    AxisValueLabel(format: .dateTime.month(.abbreviated).year(.twoDigits))
                }
            }
            .frame(height: 200)
        }
    }
}

/// The largest rows as horizontal bars, at most ten.
struct RankChart: View {
    let rows: [(name: String, value: Double)]
    let unit: String
    let format: (Double) -> String

    var body: some View {
        let top = Array(rows.prefix(10))
        if top.isEmpty {
            Text("Nothing recorded.").foregroundStyle(.secondary)
        } else {
            Chart(top, id: \.name) { row in
                BarMark(x: .value(unit, row.value), y: .value("Name", row.name))
                    .foregroundStyle(Theme.teal.gradient)
                    .cornerRadius(3)
                    .annotation(position: .trailing) {
                        Text(format(row.value)).font(.caption).foregroundStyle(.secondary)
                    }
            }
            .chartXAxis(.hidden)
            .frame(height: CGFloat(top.count) * 26 + 10)
        }
    }
}

/// Name and value rows, at most ten.
struct RankList: View {
    let rows: [(name: String, value: String)]

    var body: some View {
        if rows.isEmpty {
            Text("Nothing recorded.").foregroundStyle(.secondary)
        } else {
            VStack(spacing: 6) {
                ForEach(Array(rows.prefix(10).enumerated()), id: \.offset) { _, row in
                    HStack {
                        Text(row.name)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .help(row.name)
                        Spacer()
                        Text(row.value).monospacedDigit().foregroundStyle(.secondary)
                    }
                    .font(.callout)
                }
            }
        }
    }
}
