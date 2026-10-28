import ChatkeepKit
import SwiftUI

@Observable
final class AccountsModel {
    enum Tab: String, CaseIterable, Identifiable {
        case accounts
        case profiles
        case log

        var id: Self { self }

        var title: String {
            switch self {
            case .accounts: "Accounts"
            case .profiles: "Sync Profiles"
            case .log: "Sync Log"
            }
        }
    }

    let runner: ChatkeepRunner
    var tab = Tab.accounts
    var selection: Account.ID?
    let list = Loadable<AccountList>()
    let profiles = Loadable<SyncProfiles>()
    let log = Loadable<SyncLog>()
    let auto = Loadable<AutoSyncStatus>()
    private(set) var isSwitchingAuto = false
    var actionError: ChatkeepError?

    init(runner: ChatkeepRunner) {
        self.runner = runner
    }

    func loadAll() {
        list.load(CLI.accounts(), with: runner) { $0.data }
        profiles.load(CLI.syncProfiles(), with: runner) { $0.data }
        log.load(CLI.syncLog(last: 200), with: runner) { $0.data }
        loadAuto()
    }

    func loadAuto() {
        auto.load(CLI.syncAutoStatus(), with: runner) { $0.data }
    }

    var accounts: [Account] { list.value?.accounts ?? [] }
    var selectedAccount: Account? { accounts.first { $0.id == selection } }

    /// The best name for an `account/organization` value, as the accounts list shows it.
    func label(for member: String?) -> String {
        guard let member else { return "" }
        return accounts.first { $0.member == member }?.label ?? Format.shortID(member)
    }

    /// `claude sync auto on` or `off`: it writes without a preview, as at a terminal.
    func setAuto(_ on: Bool) {
        isSwitchingAuto = true
        Task {
            do throws(ChatkeepError) {
                let response = try await runner.run(CLI.autoSync(on: on))
                if let report = response.data {
                    if !report.report.warnings.isEmpty {
                        actionError = ChatkeepError(kind: .failed, message: report.report.warnings.joined(separator: "\n"))
                    }
                }
            } catch {
                actionError = error
            }
            isSwitchingAuto = false
            loadAuto()
        }
    }

    /// `claude accounts name`: an empty name removes the name.
    func rename(_ account: Account, to name: String) {
        let command = name.trimmedOrNil.map { CLI.nameAccount(account.account, name: $0) }
            ?? CLI.clearAccountName(account.account)
        Task {
            do throws(ChatkeepError) {
                _ = try await runner.run(command)
            } catch {
                actionError = error
            }
            loadAll()
        }
    }
}

struct AccountsView: View {
    @Bindable var model: AccountsModel
    @State private var renaming: Account?
    @State private var newName = ""
    @Environment(AppModel.self) private var app

    var body: some View {
        Group {
            switch model.tab {
            case .accounts: accounts
            case .profiles: profiles
            case .log: SyncLogTable(model: model)
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if let error = model.actionError ?? model.list.error ?? model.profiles.error ?? model.log.error {
                ErrorBanner(error: error) {
                    model.actionError = nil
                    model.list.error = nil
                    model.profiles.error = nil
                    model.log.error = nil
                }
                .padding(10)
            }
        }
        .toolbar { toolbar }
        .alert("Name This Account", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $newName)
            Button("Save") {
                if let renaming { model.rename(renaming, to: newName) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The name shows instead of the account id, here and in the chatkeep tool. Leave it empty to remove the name.")
        }
        .task {
            if !model.list.hasLoaded { model.loadAll() }
        }
    }

    // MARK: - Accounts

    private var accounts: some View {
        Table(model.accounts, selection: $model.selection) {
            TableColumn("Account") { account in
                HStack(spacing: 6) {
                    Image(systemName: account.signedIn ? "person.crop.circle.fill.badge.checkmark" : "person.crop.circle")
                        .foregroundStyle(account.signedIn ? Theme.teal : .secondary)
                    Text(account.label)
                    if account.signedIn {
                        Pill(text: "Signed in", color: Theme.teal)
                    }
                }
            }
            .width(min: 160, ideal: 220)
            TableColumn("E-mail") { account in
                Text(account.email ?? "Not known yet").foregroundStyle(account.email == nil ? .tertiary : .secondary)
            }
            .width(min: 140, ideal: 220)
            TableColumn("Organization") { account in
                Text(Format.shortID(account.organization)).font(.callout.monospaced()).foregroundStyle(.secondary)
                    .help(account.organization)
            }
            .width(min: 80, ideal: 100)
            TableColumn("Chats") { account in
                Text(Format.count(account.chats)).monospacedDigit()
            }
            .width(60)
            TableColumn("On This Mac") { account in
                Text(Format.count(account.onDisk)).monospacedDigit().foregroundStyle(.secondary)
                    .help("Chats whose transcript is on this Mac")
            }
            .width(90)
        }
        .contextMenu(forSelectionType: Account.ID.self) { ids in
            if let account = model.accounts.first(where: { ids.contains($0.id) }) {
                Button("Name…") { startRenaming(account) }
                Button("Copy Chats to Another Account…") { app.present(.copyAccountChats(from: account)) }
                Divider()
                Button("Copy Account ID") { Reveal.copy(account.member) }
                Button("Show Chat List in Finder") { Reveal.inFinder(account.dir) }
            }
        }
        .overlay {
            if model.list.isLoading, model.accounts.isEmpty {
                LoadingView(text: "Reading accounts…")
            } else if model.list.hasLoaded, model.accounts.isEmpty {
                ContentUnavailableView(
                    "No Desktop App Accounts",
                    systemImage: "person.2",
                    description: Text("The Claude desktop app keeps no chat lists on this Mac.")
                )
            }
        }
    }

    private func startRenaming(_ account: Account) {
        newName = account.name ?? ""
        renaming = account
    }

    // MARK: - Sync profiles

    @ViewBuilder
    private var profiles: some View {
        let profiles = model.profiles.value?.profiles ?? []
        List {
            ForEach(profiles) { profile in
                Section {
                    ForEach(profile.accounts) { member in
                        HStack {
                            Image(systemName: member.signedIn ? "person.crop.circle.fill.badge.checkmark" : "person.crop.circle")
                                .foregroundStyle(member.signedIn ? Theme.teal : .secondary)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(member.label)
                                Text(member.email ?? Format.shortID(member.member))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button("Remove", systemImage: "minus.circle") {
                                app.present(.removeFromSyncProfile(name: profile.name, members: [member.member]))
                            }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .help("Stop syncing this account with \(profile.name)")
                        }
                    }
                } header: {
                    HStack {
                        Text(profile.name).font(.headline)
                        Text("\(profile.accounts.count) accounts").foregroundStyle(.secondary)
                        Spacer()
                        Button("Sync Now…") { app.present(.syncNow(profile: profile.name)) }
                        Button("Add Accounts…") { app.present(.editSyncProfile(name: profile.name, adding: true)) }
                        Menu("More") {
                            Button("Choose Accounts Again…") { app.present(.editSyncProfile(name: profile.name, adding: false)) }
                            Button("Remove Profile…", role: .destructive) {
                                app.present(.removeFromSyncProfile(name: profile.name, members: []))
                            }
                        }
                        .fixedSize()
                    }
                    .buttonStyle(.borderless)
                }
            }
        }
        .overlay {
            if model.profiles.isLoading, profiles.isEmpty {
                LoadingView(text: "Reading sync profiles…")
            } else if model.profiles.hasLoaded, profiles.isEmpty {
                ContentUnavailableView {
                    Label("No Sync Profiles", systemImage: "arrow.triangle.2.circlepath")
                } description: {
                    Text("A sync profile keeps the chat lists of several accounts the same, so every account shows every chat.")
                } actions: {
                    Button("New Sync Profile…") { app.present(.editSyncProfile(name: nil, adding: false)) }
                }
            }
        }
    }

    // MARK: - Toolbar

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            Picker("Show", selection: $model.tab) {
                ForEach(AccountsModel.Tab.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented)
            .fixedSize()
        }
        ToolbarItemGroup(placement: .primaryAction) {
            AutoSyncToggle(model: model)
            switch model.tab {
            case .accounts:
                Button("Name", systemImage: "pencil") {
                    if let account = model.selectedAccount { startRenaming(account) }
                }
                .disabled(model.selectedAccount == nil)
                .help("Give the selected account a name")
                Button("Copy Chats", systemImage: "arrow.right.doc.on.clipboard") {
                    app.present(.copyAccountChats(from: model.selectedAccount))
                }
                .help("Make one account's chats show up for another account too")
            case .profiles:
                Button("New Profile", systemImage: "plus") { app.present(.editSyncProfile(name: nil, adding: false)) }
                    .help("Choose accounts that should share their chats")
                Button("Sync Now", systemImage: "arrow.triangle.2.circlepath") { app.present(.syncNow(profile: nil)) }
                    .disabled(model.profiles.value?.profiles.isEmpty ?? true)
                    .help("Sync every profile now")
            case .log:
                EmptyView()
            }
            Button("Refresh", systemImage: "arrow.clockwise") { model.loadAll() }
                .help("Read accounts, profiles and the log again")
        }
    }
}

/// The background sync, switched with `claude sync auto on` and `off`.
struct AutoSyncToggle: View {
    let model: AccountsModel

    var body: some View {
        let status = model.auto.value
        Toggle(isOn: Binding(
            get: { status?.state == .on },
            set: { model.setAuto($0) }
        )) {
            Label(title(status), systemImage: "arrow.triangle.2.circlepath.circle")
        }
        .toggleStyle(.switch)
        .disabled(status == nil || model.isSwitchingAuto)
        .help(help(status))
    }

    private func title(_ status: AutoSyncStatus?) -> String {
        switch status?.state {
        case .on: "Background sync on"
        case .stopped: "Background sync stopped"
        case .off, nil: "Background sync off"
        case .unknown(let raw): "Background sync: \(raw)"
        }
    }

    private func help(_ status: AutoSyncStatus?) -> String {
        switch status?.state {
        case .on: "Syncs the profiles whenever a chat list changes, from login on. Switch off to stop it."
        case .stopped: "Set up, but not running right now. Switch it on to start it again."
        case .off, nil: "Switch on to sync the profiles whenever a chat list changes, from login on."
        case .unknown: "This version of the app does not know this state. Switch on or off to set it."
        }
    }
}

struct SyncLogTable: View {
    let model: AccountsModel

    private struct Row: Identifiable {
        let id: Int
        let entry: SyncLog.Entry
    }

    var body: some View {
        let rows = (model.log.value?.entries ?? []).enumerated().reversed().map { Row(id: $0.offset, entry: $0.element) }
        Table(rows) {
            TableColumn("When") { row in
                Text(Format.date(row.entry.at)).foregroundStyle(.secondary)
            }
            .width(min: 110, ideal: 140)
            TableColumn("What") { row in
                Pill(text: Self.action(row.entry.action), color: Self.color(row.entry.action))
            }
            .width(80)
            TableColumn("Chat") { row in
                Text(row.entry.title ?? row.entry.chat.map(Format.shortID) ?? "")
                    .lineLimit(1)
                    .help(row.entry.changed.isEmpty ? "" : "Changed: \(row.entry.changed.joined(separator: ", "))")
            }
            .width(min: 180, ideal: 300)
            TableColumn("Account") { row in
                Text(model.label(for: row.entry.account)).foregroundStyle(.secondary)
            }
            .width(min: 100, ideal: 160)
            TableColumn("Profile") { row in
                Text(row.entry.profile ?? "").foregroundStyle(.secondary)
            }
            .width(min: 60, ideal: 90)
            TableColumn("By") { row in
                Text(Self.source(row.entry.source)).foregroundStyle(.secondary)
            }
            .width(80)
            TableColumn("Times") { row in
                Text(row.entry.times > 1 ? "\(row.entry.times)×" : "").monospacedDigit().foregroundStyle(.secondary)
            }
            .width(50)
        }
        .overlay {
            if model.log.isLoading, rows.isEmpty {
                LoadingView(text: "Reading the sync log…")
            } else if model.log.hasLoaded, rows.isEmpty {
                ContentUnavailableView("Nothing Synced Yet", systemImage: "list.bullet.rectangle", description: Text("Every chat a sync adds, updates or removes is listed here."))
            }
        }
    }

    static func source(_ source: SyncLog.Entry.Source) -> String {
        switch source {
        case .manual: "You"
        case .watch: "Background"
        case .unknown(let raw): raw
        }
    }

    static func action(_ action: SyncLog.Entry.Action) -> String {
        switch action {
        case .add: "Added"
        case .update: "Updated"
        case .remove: "Removed"
        case .note: "Note"
        case .unknown(let raw): raw
        }
    }

    static func color(_ action: SyncLog.Entry.Action) -> Color {
        switch action {
        case .add: .green
        case .update: Theme.teal
        case .remove: Theme.failure
        case .note, .unknown: .secondary
        }
    }
}
