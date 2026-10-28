import Combine
import Foundation
import Sparkle

/// Sparkle, which checks the GitHub releases for a newer Chatkeep once a day and installs it.
///
/// The feed and the public key that every update must be signed with are in Info.plist. A build
/// that is not an app bundle (`swift run`) has neither, so the updater only starts inside one.
@Observable
final class AppUpdater {
    let isAvailable = Bundle.main.bundleURL.pathExtension == "app"
    private(set) var canCheckForUpdates = false
    private(set) var lastCheck: Date?

    @ObservationIgnored private let controller: SPUStandardUpdaterController
    @ObservationIgnored private var subscriptions: Set<AnyCancellable> = []

    init() {
        controller = SPUStandardUpdaterController(
            startingUpdater: isAvailable,
            updaterDelegate: nil,
            userDriverDelegate: nil
        )
        guard isAvailable else { return }
        let updater = controller.updater
        updater.publisher(for: \.canCheckForUpdates)
            .sink { [weak self] in self?.canCheckForUpdates = $0 }
            .store(in: &subscriptions)
        updater.publisher(for: \.lastUpdateCheckDate)
            .sink { [weak self] in self?.lastCheck = $0 }
            .store(in: &subscriptions)
    }

    /// Shows Sparkle's window: up to date, or what is new with Install and Skip.
    func checkForUpdates() {
        controller.checkForUpdates(nil)
    }

    var checksAutomatically: Bool {
        get {
            access(keyPath: \.checksAutomatically)
            return controller.updater.automaticallyChecksForUpdates
        }
        set {
            withMutation(keyPath: \.checksAutomatically) {
                controller.updater.automaticallyChecksForUpdates = newValue
            }
        }
    }

    var downloadsAutomatically: Bool {
        get {
            access(keyPath: \.downloadsAutomatically)
            return controller.updater.automaticallyDownloadsUpdates
        }
        set {
            withMutation(keyPath: \.downloadsAutomatically) {
                controller.updater.automaticallyDownloadsUpdates = newValue
            }
        }
    }
}
