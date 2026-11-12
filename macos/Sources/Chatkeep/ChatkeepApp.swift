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
