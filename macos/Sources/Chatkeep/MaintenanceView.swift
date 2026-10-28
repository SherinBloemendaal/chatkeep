import ChatkeepKit
import SwiftUI

@Observable
final class MaintenanceModel {
    let runner: ChatkeepRunner
    let cursorIndex = Loadable<CursorCacheStats>()
    let claudeIndex = Loadable<ClaudeCacheStats>()
    let doctor = Loadable<AccountsDoctor>()
    /// `cursor cache scan -n`: what an update would read again.
    let check = Loadable<CursorCacheCheck>()
    /// The last update or rebuild of the Cursor index.
    let scan = Loadable<CursorScan>()

    init(runner: ChatkeepRunner) {
        self.runner = runner
    }

    func load() {
        cursorIndex.load(CLI.cursorCacheStats(), with: runner) { $0.data }
        claudeIndex.load(CLI.claudeCacheStats(), with: runner) { $0.data }
        doctor.load(CLI.accountsDoctor(), with: runner) { $0.data }
    }

    func checkCursorIndex() {
        check.load(CLI.cursorCacheCheck(), with: runner) { $0.data }
    }

    /// Updates (or with `full`, rebuilds) chatkeep's own index of Cursor. It only writes the
    /// index, never Cursor's files, so it runs without a preview.
    func scanCursorIndex(full: Bool, then done: @escaping () -> Void) {
        scan.load(CLI.cursorCacheScan(full: full), with: runner) { response in
            done()
            return response.data
        }
    }
}

struct MaintenanceView: View {
    let model: MaintenanceModel
    @Environment(AppModel.self) private var app

    var body: some View {
        Form {
            cursorSection
            claudeSection
            doctorSection
        }
        .formStyle(.grouped)
        .toolbar {
            Button("Refresh", systemImage: "arrow.clockwise") { model.load() }
                .help("Read everything on this page again")
        }
        .task {
            if !model.cursorIndex.hasLoaded { model.load() }
        }
    }

    // MARK: - Cursor

    @ViewBuilder
    private var cursorSection: some View {
        Section {
            if let error = model.cursorIndex.error {
                ErrorBanner(error: error)
            } else if let stats = model.cursorIndex.value {
                if stats.exists {
                    DetailRow(label: "Size", value: Format.bytes(stats.sizeBytes))
                    DetailRow(label: "Last updated", value: Format.relative(stats.refreshedAt))
                    DetailRow(label: "Background update", value: backgroundState(stats))
                    if let log = stats.lastLog {
                        DetailRow(label: "Last background note", value: log)
                    }
                    ForEach(stats.installs) { install in
                        LabeledContent {
                            VStack(alignment: .trailing, spacing: 2) {
                                Text("\(Format.count(install.workspaces)) workspaces, \(Format.count(install.chats)) chats")
                                Text(staleText(install))
                                    .font(.caption)
                                    .foregroundStyle(install.stale?.sources ?? 0 > 0 ? Theme.warning : .secondary)
                            }
                        } label: {
                            Label(install.name, systemImage: install.found ? "externaldrive" : "externaldrive.badge.questionmark")
                                .help(install.found ? install.root : "This installation is no longer on this Mac")
                        }
                    }
                } else {
                    Text("There is no index yet. It is made the first time chatkeep reads Cursor.")
                        .foregroundStyle(.secondary)
                }
                DetailRow(label: "File", value: Format.path(stats.path))
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
            checkResult
            scanResult
            HStack {
                Button("Check for Changes") { model.checkCursorIndex() }
                    .secondaryActionStyle()
                    .help("See what changed in Cursor since the last update. Reads nothing again.")
                Button("Update Now") { model.scanCursorIndex(full: false) { model.load() } }
                    .primaryActionStyle()
                    .help("Read again only what changed in Cursor")
                Button("Rebuild") { model.scanCursorIndex(full: true) { model.load() } }
                    .secondaryActionStyle()
                    .help("Throw the index away and read all of Cursor again. Slow.")
                Spacer()
                if model.scan.isLoading || model.check.isLoading {
                    ProgressView().controlSize(.small)
                }
                Button("Clear…", role: .destructive) { app.present(.clearCursorIndex) }
            }
            .disabled(model.scan.isLoading)
        } header: {
            Text("Cursor index")
        } footer: {
            Text("chatkeep keeps its own index of Cursor's chats so lists open fast. It never changes Cursor's files.")
        }

        Section {
            HStack {
                Text("Chats missing in Cursor after a move? Repair rebuilds Cursor's references for one project.")
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Repair a Project…") { app.present(.reindex(nil)) }
            }
        } header: {
            Text("Repair")
        }
    }

    @ViewBuilder
    private var checkResult: some View {
        if let error = model.check.error {
            ErrorBanner(error: error) { model.check.error = nil }
        } else if let check = model.check.value, !model.check.isLoading {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(check.installs) { install in
                    let reasons = install.reasons.isEmpty ? "up to date" : install.reasons.joined(separator: ", ")
                    Label(
                        "\(install.name): \(install.scanned ? reasons : "never read yet")",
                        systemImage: install.reasons.isEmpty && install.scanned ? "checkmark.circle" : "arrow.triangle.2.circlepath"
                    )
                    .foregroundStyle(install.reasons.isEmpty && install.scanned ? .secondary : Theme.warning)
                }
            }
            .font(.callout)
        }
    }

    @ViewBuilder
    private var scanResult: some View {
        if let error = model.scan.error {
            ErrorBanner(error: error) { model.scan.error = nil }
        } else if let scan = model.scan.value, !model.scan.isLoading {
            Label(
                "\(scan.full ? "Rebuilt" : "Updated") in \(Format.duration(ms: scan.durationMs)): "
                    + scan.installs.map { "\($0.name) read \($0.chatsRead) chats" }.joined(separator: ", "),
                systemImage: "checkmark.circle.fill"
            )
            .font(.callout)
            .foregroundStyle(Theme.teal)
        }
    }

    private func backgroundState(_ stats: CursorCacheStats) -> String {
        guard let refresh = stats.refresh else { return "Not running" }
        return refresh.alive ? "Running since \(Format.relative(refresh.startedAt))" : "Stopped early"
    }

    private func staleText(_ install: CursorCacheStats.Install) -> String {
        guard install.found else { return "Not on this Mac anymore" }
        guard let stale = install.stale else { return "Read \(Format.relative(install.scannedAt))" }
        return stale.sources == 0 ? "Up to date" : "Changed since: \(stale.reasons.joined(separator: ", "))"
    }

    // MARK: - Claude Code

    @ViewBuilder
    private var claudeSection: some View {
        Section {
            if let error = model.claudeIndex.error {
                ErrorBanner(error: error)
            } else if let stats = model.claudeIndex.value {
                if !stats.enabled {
                    Text("The index is turned off (CHATKEEP_NO_INDEX is set).").foregroundStyle(.secondary)
                } else if !stats.exists {
                    Text("There is no index yet. It is made the first time chatkeep reads Claude Code.").foregroundStyle(.secondary)
                } else {
                    DetailRow(label: "Size", value: Format.bytes(stats.sizeBytes))
                    if let transcripts = stats.transcripts {
                        DetailRow(label: "Transcripts", value: Format.count(transcripts))
                    }
                    DetailRow(label: "Last updated", value: Format.relative(stats.refreshedAt))
                }
                DetailRow(label: "File", value: Format.path(stats.path))
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
            HStack {
                Spacer()
                Button("Clear…", role: .destructive) { app.present(.clearClaudeIndex) }
                    .disabled(model.claudeIndex.value?.exists != true)
            }
        } header: {
            Text("Claude Code index")
        } footer: {
            Text("What every transcript says, so stats and lists open fast. Clearing it is safe: it is made again.")
        }
    }

    // MARK: - Accounts check

    @ViewBuilder
    private var doctorSection: some View {
        Section {
            if let error = model.doctor.error {
                ErrorBanner(error: error)
            } else if let doctor = model.doctor.value {
                if !doctor.desktopApp {
                    Text("The Claude desktop app keeps no data on this Mac.").foregroundStyle(.secondary)
                }
                ForEach(Array(doctor.findings.enumerated()), id: \.offset) { _, finding in
                    Label {
                        Text(finding.text).textSelection(.enabled)
                    } icon: {
                        Image(systemName: Self.symbol(finding.level)).foregroundStyle(Self.color(finding.level))
                    }
                }
                if let desktop = doctor.desktop {
                    DetailRow(label: "Desktop app", value: desktop.label)
                }
                if let session = doctor.session {
                    DetailRow(label: "This Claude Code session", value: session.label)
                }
                if let login = doctor.login {
                    DetailRow(label: "Last login", value: [login.label, login.organizationName].compactMap(\.self).joined(separator: ", "))
                }
                DetailRow(label: "claude command", value: doctor.claudeCommand.map(Format.path) ?? "Not found")
                HStack {
                    Text(doctor.staleCaches == 0
                        ? "Nothing to clean up."
                        : "\(doctor.staleCaches) leftover entries of accounts no longer in use.")
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Clean Up…") { app.present(.pruneAccounts) }
                        .disabled(doctor.staleCaches == 0)
                }
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        } header: {
            Text("Claude accounts check")
        } footer: {
            Text("Which account the desktop app, Claude Code and the last login use. Desktop chats only show for the account they belong to.")
        }
    }

    static func symbol(_ level: AccountsDoctor.Finding.Level) -> String {
        switch level {
        case .ok: "checkmark.circle.fill"
        case .warn: "exclamationmark.triangle.fill"
        case .info: "info.circle.fill"
        case .hint: "lightbulb.fill"
        case .unknown: "circle"
        }
    }

    static func color(_ level: AccountsDoctor.Finding.Level) -> Color {
        switch level {
        case .ok: .green
        case .warn: Theme.warning
        case .info: .blue
        case .hint: Theme.teal
        case .unknown: .secondary
        }
    }
}
