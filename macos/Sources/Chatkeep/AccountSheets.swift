import ChatkeepKit
import SwiftUI

/// `claude accounts cp`: the chats one account lists show up for another account too.
struct CopyAccountChatsSheet: View {
    @State private var from: String
    /// Empty: the account the desktop app is signed in with.
    @State private var to = ""
    @State private var flow = WriteFlow<Report>()
    @Environment(AppModel.self) private var app

    init(from: Account?) {
        _from = State(initialValue: from?.member ?? "")
    }

    private var accounts: [Account] { app.accounts.accounts }

    private var command: WriteCommand<Report>? {
        guard let from = from.trimmedOrNil, from != to else { return nil }
        return CLI.copyAccountChats(from: from, to: to.trimmedOrNil)
    }

    var body: some View {
        WriteSheet(
            title: "Copy Chats to Another Account",
            summary: "Makes the chats one desktop app account lists show up for another account too. The chats stay where they are.",
            applyLabel: "Copy Chats",
            command: command,
            flow: flow
        ) {
            Section {
                Picker("From", selection: $from) {
                    Text("Choose an account").tag("")
                    ForEach(accounts) { account in
                        Text(Self.title(account)).tag(account.member)
                    }
                }
                Picker("To", selection: $to) {
                    Text("The signed-in account").tag("")
                    ForEach(accounts.filter { $0.member != from }) { account in
                        Text(Self.title(account)).tag(account.member)
                    }
                }
            }
        }
        .task {
            if !app.accounts.list.hasLoaded { app.accounts.loadAll() }
        }
    }

    static func title(_ account: Account) -> String {
        let email = account.email.map { " (\($0))" } ?? ""
        return "\(account.label)\(email), \(account.chats) chats" + (account.signedIn ? ", signed in" : "")
    }
}

/// `claude sync`: run one profile or every profile now.
struct SyncNowSheet: View {
    @State private var profile: String
    @State private var flow = WriteFlow<Report>()
    @Environment(AppModel.self) private var app

    init(profile: String?) {
        _profile = State(initialValue: profile ?? "")
    }

    /// The profile to decide for after this sync: the one it ran, or the first that waits.
    private var profileToDecide: String? {
        let waiting = app.accounts.profilesWithPending
        if let chosen = profile.trimmedOrNil, waiting.contains(chosen) { return chosen }
        return profile.trimmedOrNil == nil ? waiting.first : nil
    }

    var body: some View {
        WriteSheet(
            title: "Sync Now",
            summary: "Copies chats between the accounts of a sync profile, so each account lists the same chats. A chat you deleted in one account stays in the others until you decide what happens to it.",
            applyLabel: "Sync",
            command: CLI.sync(profile: profile.trimmedOrNil),
            flow: flow,
            previewAutomatically: true,
            fields: {
                Section {
                    Picker("Profile", selection: $profile) {
                        Text("Every profile").tag("")
                        ForEach(app.accounts.profiles.value?.profiles ?? []) { profile in
                            Text(profile.name).tag(profile.name)
                        }
                    }
                }
            },
            extra: { _ in
                if flow.isFinished, let waiting = profileToDecide {
                    let count = app.accounts.pending(in: waiting).count
                    HStack(spacing: 10) {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Theme.warning)
                        Text(count == 1 ? "1 chat in \(waiting) needs your decision." : "\(count) chats in \(waiting) need your decision.")
                        Spacer(minLength: 0)
                        Button("Decide…") { app.present(.decideSync(profile: waiting)) }
                            .primaryActionStyle()
                    }
                }
            }
        )
    }
}

/// `claude sync decide`: what happens to each chat that one account deleted while another
/// account of the profile still lists it.
struct SyncDecisionsSheet: View {
    enum Choice: Hashable {
        case later
        case decided(SyncDecision)
    }

    let profile: String
    /// The chats as they were when the sheet opened or last changed before a run, so the rows
    /// stay in place after the decisions are applied.
    @State private var chats: [SyncPending.Chat] = []
    @State private var choices: [String: Choice] = [:]
    @State private var flow = WriteFlow<Report>()
    @Environment(AppModel.self) private var app

    private var waiting: [SyncPending.Chat] { app.accounts.pending(in: profile) }

    private func choice(for chat: SyncPending.Chat) -> Choice {
        choices[chat.id] ?? .later
    }

    private var decisions: [String: SyncDecision] {
        var decisions: [String: SyncDecision] = [:]
        for chat in chats {
            if case .decided(let decision) = choice(for: chat) { decisions[chat.id] = decision }
        }
        return decisions
    }

    private var command: WriteCommand<Report>? {
        decisions.isEmpty ? nil : CLI.decideSync(profile, decisions: decisions)
    }

    private var applyLabel: String {
        switch decisions.count {
        case 0: "Apply Decisions"
        case 1: "Apply 1 Decision"
        case let count: "Apply \(count) Decisions"
        }
    }

    var body: some View {
        WriteSheet(
            title: "Deleted Chats in \u{201C}\(profile)\u{201D}",
            summary: "Choose what happens to each chat you deleted in one account while another account still lists it. The chat itself stays on this Mac either way: only the chat lists in the Claude app change.",
            applyLabel: applyLabel,
            command: command,
            flow: flow
        ) {
            Section {
                Self.explanation("Remove", "Goes from the other accounts too.")
                Self.explanation("Keep", "Stays in the other accounts, not in the one you deleted it in. You are not asked again.")
                Self.explanation("Put back", "Returns to the account you deleted it in.")
            }
            Section {
                if chats.isEmpty {
                    Text(app.accounts.pending.hasLoaded ? "No chat waits for a decision in this profile." : "Reading the chats that wait…")
                        .foregroundStyle(.secondary)
                }
                ForEach(chats) { chat in
                    Picker(selection: binding(for: chat)) {
                        Text("Decide later").tag(Choice.later)
                        Text("Remove from \(app.accounts.labels(for: chat.listedIn))")
                            .tag(Choice.decided(.remove))
                            .selectionDisabled(chat.removeWaits)
                        Text("Keep in \(app.accounts.labels(for: chat.listedIn))").tag(Choice.decided(.keep))
                        Text("Put back in \(app.accounts.labels(for: chat.deletedIn))").tag(Choice.decided(.restore))
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(chat.title ?? "Untitled").lineLimit(1)
                            Text("Deleted in \(app.accounts.labels(for: chat.deletedIn)) · still listed in \(app.accounts.labels(for: chat.listedIn))")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            if chat.removeWaits {
                                Text("The Claude app is open with an account that lists it. Quit the app to remove it there.")
                                    .font(.caption)
                                    .foregroundStyle(Theme.warning)
                            }
                        }
                    }
                }
            } header: {
                HStack {
                    Text("Where each chat goes")
                    Spacer()
                    Menu("Set All To") {
                        Button("Remove") { setAll(.decided(.remove)) }
                        Button("Keep") { setAll(.decided(.keep)) }
                        Button("Put Back") { setAll(.decided(.restore)) }
                        Divider()
                        Button("Decide Later") { setAll(.later) }
                    }
                    .fixedSize()
                    .disabled(chats.isEmpty)
                }
            } footer: {
                Text("Change any chat, then preview again before you apply.")
            }
        }
        .onAppear { chats = waiting }
        .onChange(of: waiting) { _, now in
            if !flow.isBusy, !flow.isFinished { chats = now }
        }
        .onChange(of: flow.isBusy) { _, busy in
            // What changed while a preview ran shows once it is done.
            if !busy, !flow.isFinished { chats = waiting }
        }
    }

    private static func explanation(_ name: String, _ text: String) -> some View {
        LabeledContent(name) {
            Text(text).foregroundStyle(.secondary).multilineTextAlignment(.trailing)
        }
    }

    private func binding(for chat: SyncPending.Chat) -> Binding<Choice> {
        Binding(get: { choice(for: chat) }, set: { choices[chat.id] = $0 })
    }

    /// A chat whose removal has to wait keeps its own choice when everything is set to Remove.
    private func setAll(_ choice: Choice) {
        for chat in chats where !(choice == .decided(.remove) && chat.removeWaits) {
            choices[chat.id] = choice
        }
    }
}

/// `claude sync set` (create or replace a profile) and `claude sync add` (add accounts to one).
struct SyncProfileSheet: View {
    let adding: Bool
    let existingName: String?
    @State private var name: String
    @State private var picked: Set<String> = []
    @State private var flow = WriteFlow<Report>()
    @Environment(AppModel.self) private var app

    init(name: String?, adding: Bool) {
        self.adding = adding
        existingName = name
        _name = State(initialValue: name ?? "")
    }

    private var profile: SyncProfiles.Profile? {
        app.accounts.profiles.value?.profiles.first { $0.name == name.trimmedOrNil }
    }

    /// Accounts to choose from. When adding, the ones already in the profile are left out.
    private var choices: [Account] {
        let inProfile = Set(profile?.accounts.map(\.member) ?? [])
        return app.accounts.accounts.filter { !adding || !inProfile.contains($0.member) }
    }

    private var command: WriteCommand<Report>? {
        guard let name = name.trimmedOrNil, !picked.isEmpty else { return nil }
        let members = choices.map(\.member).filter(picked.contains)
        return adding
            ? CLI.addToSyncProfile(name, accounts: members)
            : CLI.setSyncProfile(name, accounts: members)
    }

    var body: some View {
        WriteSheet(
            title: adding ? "Add Accounts to \u{201C}\(name)\u{201D}" : (existingName == nil ? "New Sync Profile" : "Choose Accounts for \u{201C}\(name)\u{201D}"),
            summary: adding
                ? "The accounts you pick start sharing their chats with the profile."
                : "The accounts you pick share their chats. A profile with this name is replaced.",
            applyLabel: adding ? "Add" : "Save Profile",
            command: command,
            flow: flow
        ) {
            if existingName == nil {
                Section {
                    TextField("Name", text: $name, prompt: Text("work"))
                } footer: {
                    if profile != nil {
                        Text("A profile with this name exists. Saving replaces its accounts.")
                            .foregroundStyle(Theme.warning)
                    }
                }
            }
            Section("Accounts") {
                if choices.isEmpty {
                    Text(adding ? "Every account is already in this profile." : "The desktop app has no accounts on this Mac.")
                        .foregroundStyle(.secondary)
                }
                ForEach(choices) { account in
                    Toggle(isOn: Binding(
                        get: { picked.contains(account.member) },
                        set: { if $0 { picked.insert(account.member) } else { picked.remove(account.member) } }
                    )) {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(account.label)
                            Text([account.email, "\(account.chats) chats", account.signedIn ? "signed in" : nil].compactMap(\.self).joined(separator: ", "))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .task {
            if !app.accounts.list.hasLoaded { app.accounts.loadAll() }
        }
        .onAppear {
            if !adding, let profile { picked = Set(profile.accounts.map(\.member)) }
        }
    }
}
