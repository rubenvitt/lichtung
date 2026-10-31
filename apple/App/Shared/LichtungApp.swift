// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

@main
struct LichtungApp: App {
    /// Anmeldung, Demo oder Server (LIC-43). Hält das aktuelle `AppModel`.
    @State private var session = AppSession()

    #if os(macOS)
    init() {
        // Sparkle früh starten, damit er im Hintergrund nach Updates sucht.
        _ = AppUpdater.shared
    }
    #endif

    var body: some Scene {
        WindowGroup {
            SessionRootView()
                .environment(session)
        }
        #if os(macOS)
        .commands {
            LichtungCommands(model: session.model)
            CheckForUpdatesCommand()
        }
        #endif

        #if os(macOS)
        // Mac: Einstellungen als eigenes Fenster (⌘,). Auf dem iPhone öffnet
        // die Toolbar sie als Blatt (PhoneSettingsButton).
        Settings {
            SettingsView()
                .environment(session)
                .environment(session.model)
        }
        #endif
    }
}

/// Weiche beim Start: Anmeldung oder Kalender (LIC-43).
struct SessionRootView: View {
    @Environment(AppSession.self) private var session
    @Environment(\.scenePhase) private var scenePhase
    @State private var showsLogin = false
    /// IANA-ID der zweiten Zeitzone aus den Einstellungen, leer heißt keine.
    @AppStorage("secondaryTimeZone") private var secondaryZoneID = ""

    var body: some View {
        content
            // SSE nur, solange die App offen ist (Vertrag §5).
            .onChange(of: scenePhase, initial: true) { _, phase in
                session.setActive(phase != .background)
            }
    }

    @ViewBuilder
    private var content: some View {
        switch session.stage {
        case .starting:
            ProgressView()
        case .signedOut:
            LoginView()
        case .demo, .server:
            let model = session.model
            RootView()
                .eventEditorSheet() // Termin-Editor (LIC-20)
                .occurrenceDeleteDialog() // Serie löschen aus Menüs und Tastatur (LIC-24)
                .environment(model)
                .task(id: ObjectIdentifier(model)) { await model.observe() }
                // Rückgängig-Verlauf für Knöpfe und Beschriftung (LIC-42).
                .task(id: ObjectIdentifier(model)) { await model.observeUndo() }
                // Zweite Zeitzone im Raster (LIC-42). Ein neues Modell liest
                // sie beim Entstehen selbst, hier folgen nur Änderungen.
                .onChange(of: secondaryZoneID) { _, id in
                    model.secondaryTimeZone = TimeZoneCatalog.zone(fromStored: id)
                }
                .safeAreaInset(edge: .top, spacing: 0) { banner }
                .sheet(isPresented: $showsLogin) {
                    LoginView(isReauthentication: true)
                        .environment(session)
                }
                // Nach erfolgreicher Anmeldung schließt sich das Blatt.
                .onChange(of: session.needsSignIn) { _, needs in
                    if !needs { showsLogin = false }
                }
        }
    }

    @ViewBuilder
    private var banner: some View {
        if session.needsSignIn {
            SessionBanner(
                systemImage: "person.crop.circle.badge.exclamationmark",
                text: "Nicht mehr angemeldet. Änderungen bleiben auf dem Gerät.",
                action: "Neu anmelden"
            ) { showsLogin = true }
        } else if session.stage == .demo {
            SessionBanner(
                systemImage: "theatermasks",
                text: "Demo mit erfundenen Terminen.",
                action: "Mit Server verbinden"
            ) { session.leaveDemo() }
        }
    }
}

/// Schmale Leiste über dem Kalender.
struct SessionBanner: View {
    let systemImage: String
    let text: String
    let action: String
    let perform: @MainActor () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: systemImage)
            Text(text)
                .lineLimit(2)
            Spacer(minLength: 8)
            Button(action) { perform() }
                .buttonStyle(.bordered)
                .controlSize(.small)
        }
        .font(.callout)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(.bar)
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
