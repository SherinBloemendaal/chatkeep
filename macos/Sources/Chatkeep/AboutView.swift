import AppKit
import ChatkeepKit
import SwiftUI

/// The Settings window: what is installed, where the tool lives, and where the project is.
struct AboutView: View {
    static let repository = URL(string: "https://github.com/SherinBloemendaal/chatkeep")!

    let runner: ChatkeepRunner?
    @Bindable var updater: AppUpdater
    @State private var toolVersion: String?
    @State private var error: ChatkeepError?

    private var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "Development build"
    }

    var body: some View {
        VStack(spacing: 18) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 96, height: 96)
            VStack(spacing: 4) {
                Text("Chatkeep").font(.title.weight(.semibold))
                Text("Keeps your Cursor and Claude Code chats with your projects when folders move.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            Form {
                DetailRow(label: "App version", value: appVersion)
                DetailRow(label: "Tool version", value: toolVersion ?? (error == nil ? "Reading…" : "Unknown"))
                if let runner {
                    LabeledContent("Tool in use") {
                        HStack {
                            Text(Format.path(runner.executable.path(percentEncoded: false)))
                                .font(.callout.monospaced())
                                .textSelection(.enabled)
                                .lineLimit(2)
                                .truncationMode(.middle)
                            Button("Show", systemImage: "magnifyingglass") {
                                Reveal.inFinder(runner.executable.path(percentEncoded: false))
                            }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .help("Show the chatkeep tool in Finder")
                        }
                    }
                }
                if let error {
                    Text(error.message).foregroundStyle(Theme.failure)
                }
                if updater.isAvailable {
                    Section("Updates") {
                        Toggle("Check for updates automatically", isOn: $updater.checksAutomatically)
                        Toggle("Download and install updates automatically", isOn: $updater.downloadsAutomatically)
                            .disabled(!updater.checksAutomatically)
                        LabeledContent("Last checked") {
                            HStack {
                                Text(updater.lastCheck.map { Format.relative($0) } ?? "Never")
                                    .foregroundStyle(.secondary)
                                Button("Check Now") { updater.checkForUpdates() }
                                    .disabled(!updater.canCheckForUpdates)
                            }
                        }
                    }
                }
            }
            .formStyle(.grouped)
            .scrollDisabled(true)
            Link(destination: Self.repository) {
                Label("Chatkeep on GitHub", systemImage: "arrow.up.right.square")
            }
        }
        .padding(24)
        .frame(width: 480)
        .task {
            guard let runner else { return }
            do throws(ChatkeepError) {
                toolVersion = try await runner.run(CLI.version()).data?.version
            } catch {
                self.error = error
            }
        }
    }
}
