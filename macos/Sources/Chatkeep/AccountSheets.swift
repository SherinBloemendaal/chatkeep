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

    var body: some View {
        WriteSheet(
            title: "Sync Now",
            summary: "Copies chats between the accounts of a sync profile, so each account lists the same chats. A chat you deleted in one account stays in the others: this app never removes chats from an account.",
            applyLabel: "Sync",
            command: CLI.sync(profile: profile.trimmedOrNil),
            flow: flow,
            previewAutomatically: true
        ) {
            Section {
                Picker("Profile", selection: $profile) {
                    Text("Every profile").tag("")
                    ForEach(app.accounts.profiles.value?.profiles ?? []) { profile in
                        Text(profile.name).tag(profile.name)
                    }
                }
            }
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
