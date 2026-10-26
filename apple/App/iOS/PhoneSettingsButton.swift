// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Zahnrad in der Toolbar, öffnet die Einstellungen als Blatt (LIC-43).
/// Eigener Modifier, damit `PhoneRootView` nur eine Zeile dazubekommt.
private struct PhoneSettingsButton: ViewModifier {
    @Environment(AppSession.self) private var session
    @Environment(AppModel.self) private var model
    @State private var showsSettings = false

    func body(content: Content) -> some View {
        content
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showsSettings = true
                    } label: {
                        Image(systemName: "gearshape")
                    }
                    .accessibilityLabel("Einstellungen")
                }
            }
            .sheet(isPresented: $showsSettings) {
                SettingsView()
                    .environment(session)
                    .environment(model)
            }
    }
}

extension View {
    func phoneSettingsButton() -> some View {
        modifier(PhoneSettingsButton())
    }
}
