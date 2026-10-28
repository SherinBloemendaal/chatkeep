import AppKit
import ChatkeepKit
import SwiftUI

@main
struct ChatkeepApp: App {
    @NSApplicationDelegateAdaptor private var appDelegate: AppDelegate

    var body: some Scene {
        Window("Chatkeep", id: "main") {
            Group {
                switch appDelegate.setup {
                case .success(let app): ContentView(app: app)
                case .failure(let error): MissingToolView(error: error)
                }
            }
            .frame(minWidth: 900, minHeight: 560)
        }
        .defaultSize(width: 1240, height: 780)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { appDelegate.updater.checkForUpdates() }
                    .disabled(!appDelegate.updater.canCheckForUpdates)
            }
            if case .success(let app) = appDelegate.setup {
                MenuCommands(app: app)
            }
        }

        Settings {
            AboutView(runner: try? appDelegate.setup.get().runner, updater: appDelegate.updater)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Started at launch, so the daily check runs even before a window opens.
    let updater = AppUpdater()

    /// The app, or why it cannot run: without the chatkeep tool there is nothing to show.
    let setup: Result<AppModel, ChatkeepError> = Result { () throws(ChatkeepError) in
        AppModel(runner: try ChatkeepRunner())
    }

    /// `.chatkeep` archives opened from Finder (the app owns the type, see Info.plist) or dropped
    /// on the Dock icon. A file opened at launch arrives before the window shows; the Import sheet
    /// then opens with the window.
    func application(_ application: NSApplication, open urls: [URL]) {
        guard case .success(let app) = setup else { return }
        app.open(urls)
        NSApp.activate()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

struct MenuCommands: Commands {
    let app: AppModel

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Import Archive…") { app.present(.importArchive(file: nil)) }
                .keyboardShortcut("o")
            Button("Rewrite Paths…") { app.present(.move(nil)) }
        }
        CommandGroup(after: .toolbar) {
            Button("Refresh") { refresh(fresh: false) }
                .keyboardShortcut("r")
            Button("Read Fresh") { refresh(fresh: true) }
                .keyboardShortcut("r", modifiers: [.command, .shift])
                .disabled(app.screen != .projects && app.screen != .stats)
            Divider()
            ForEach(Array(Screen.allCases.enumerated()), id: \.element) { index, screen in
                Button(screen.title) { app.screen = screen }
                    .keyboardShortcut(KeyEquivalent(Character(String(index + 1))), modifiers: .command)
            }
            Divider()
        }
        CommandMenu("Queue") {
            Button("Run Queue…") { app.present(.runQueue) }
                .disabled(app.queue.pendingCount == 0)
            Button("Try Every Failed Entry Again…") { app.present(.retryQueue([])) }
            Button("Clear Queue…") { app.present(.clearQueue) }
        }
    }

    private func refresh(fresh: Bool) {
        switch app.screen {
        case .projects: app.projects.load(fresh: fresh)
        case .queue: app.queue.load()
        case .history: app.history.load()
        case .stats: app.stats.load(fresh: fresh)
        case .accounts: app.accounts.loadAll()
        case .maintenance: app.maintenance.load()
        }
    }
}
