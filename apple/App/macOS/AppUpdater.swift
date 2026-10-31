// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import Sparkle
import SwiftUI

/// Auto-Update der Mac-App über Sparkle. Feed und Schlüssel stehen in der
/// Info.plist (project.yml: SUFeedURL, SUPublicEDKey); der Feed ist das
/// `appcast.xml` am neuesten GitHub Release (scripts/release-macos.sh).
@MainActor
final class AppUpdater {
    static let shared = AppUpdater()

    private let controller = SPUStandardUpdaterController(
        startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil
    )

    private init() {}

    /// Menü „Nach Updates suchen …“. Sparkle zeigt den Fortschritt selbst.
    func checkForUpdates() {
        controller.checkForUpdates(nil)
    }
}

/// Eintrag im App-Menü unter „Über Lichtung“.
struct CheckForUpdatesCommand: Commands {
    var body: some Commands {
        CommandGroup(after: .appInfo) {
            Button("Nach Updates suchen …") { AppUpdater.shared.checkForUpdates() }
        }
    }
}
