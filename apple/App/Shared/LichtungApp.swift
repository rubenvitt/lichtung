// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

@main
struct LichtungApp: App {
    /// Anmeldung, Demo oder Server (LIC-43). Hält das aktuelle `AppModel`.
    @State private var session = AppSession()

    init() {
        #if os(macOS)
        // Sparkle früh starten, damit er im Hintergrund nach Updates sucht.
        _ = AppUpdater.shared
        #endif
        // Vor dem ersten Tipp auf eine Erinnerung (LIC-68).
        ReminderScheduler.shared.install()
    }

    var body: some Scene {
        WindowGroup {
            SessionRootView()
                .environment(session)
                .tint(Theme.accent)
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
                .tint(Theme.accent)
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
    /// Erinnerungen als Mitteilungen (LIC-68).
    @AppStorage(ReminderScheduler.enabledKey) private var remindersEnabled = true

    var body: some View {
        content
            // SSE nur, solange die App offen ist (Vertrag §5).
            .onChange(of: scenePhase, initial: true) { _, phase in
                session.setActive(phase != .background)
                // Beim Öffnen neu einplanen: Die Zeit ist weitergelaufen (LIC-68).
                if phase == .active { session.model.scheduleReminders() }
            }
    }

    @ViewBuilder
    private var content: some View {
        switch session.stage {
        case .starting:
            VStack(spacing: 16) {
                LichtungMark(size: 64)
                ProgressView().controlSize(.small)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .signedOut:
            LoginView()
        case .demo, .server:
            let model = session.model
            calendarWithBanner
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
                // Erinnerungen (LIC-68): Schalter in den Einstellungen, Tipp auf
                // eine Benachrichtigung öffnet den Termin.
                .onChange(of: remindersEnabled) { _, _ in model.scheduleReminders() }
                .onChange(of: ReminderScheduler.shared.openRequest, initial: true) { _, _ in
                    model.openRequestedReminder()
                }
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

    /// Kalender mit Banner darüber. iPhone: im Stapel statt als Einschub,
    /// sonst liegt der Banner über der Navigationsleiste und verdeckt deren
    /// Knöpfe (LIC-66).
    @ViewBuilder
    private var calendarWithBanner: some View {
        #if os(iOS)
        VStack(spacing: 0) {
            banner
            RootView()
        }
        #else
        RootView()
            .safeAreaInset(edge: .top, spacing: 0) { banner }
        #endif
    }

    @ViewBuilder
    private var banner: some View {
        if session.needsSignIn {
            SessionBanner(
                systemImage: "person.crop.circle.badge.exclamationmark",
                text: "Nicht mehr angemeldet. Änderungen bleiben auf dem Gerät.",
                action: "Neu anmelden",
                tint: Theme.now
            ) { showsLogin = true }
        } else if session.stage == .demo {
            SessionBanner(
                systemImage: "theatermasks",
                text: "Demo mit erfundenen Terminen.",
                action: "Mit Server verbinden",
                tint: Theme.sun
            ) { session.leaveDemo() }
        }
    }
}

/// Schmale Leiste über dem Kalender.
struct SessionBanner: View {
    let systemImage: String
    let text: String
    let action: String
    var tint: Color = Theme.accent
    let perform: @MainActor () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: systemImage)
                .font(.callout.weight(.semibold))
                .foregroundStyle(tint)
            Text(text)
                .lineLimit(2)
            Spacer(minLength: 8)
            Button(action) { perform() }
                .buttonStyle(.borderedProminent)
                .tint(tint)
                .controlSize(.small)
        }
        .font(.callout)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background {
            ZStack {
                Rectangle().fill(.bar)
                Rectangle().fill(tint.opacity(0.12))
            }
        }
        .overlay(alignment: .bottom) {
            Rectangle().fill(tint.opacity(0.3)).frame(height: 0.5)
        }
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
