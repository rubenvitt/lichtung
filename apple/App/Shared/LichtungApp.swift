// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

@main
struct LichtungApp: App {
    /// Bis zur Anmeldung (LIC-21) läuft die App mit erfundenen Daten.
    @State private var model = AppModel.demo()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(model)
                .task { await model.observe() }
                // Rückgängig-Verlauf für Knöpfe und Beschriftung (LIC-42).
                .task { await model.observeUndo() }
        }
        #if os(macOS)
        .commands { LichtungCommands(model: model) }
        #endif
    }
}

struct RootView: View {
    var body: some View {
        #if os(macOS)
        MacRootView()
        #else
        PhoneRootView()
        #endif
    }
}
