// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Server, Gerät, zweite Zeitzone, Standardkalender (LIC-47), Abmelden (LIC-43).
struct GeneralSettingsView: View {
    @Environment(AppSession.self) private var session
    /// IANA-ID der zweiten Zeitzone, leer = keine. Das Raster liest sie über
    /// `AppModel.secondaryTimeZone` (siehe `TimeZoneCatalog.zone(fromStored:)`).
    @AppStorage("secondaryTimeZone") private var secondaryTimeZone = ""
    /// Kalender für neue Termine, leer = erster beschreibbarer (LIC-47).
    /// `AppModel.defaultCalendarId` liest ihn beim Anlegen.
    @AppStorage(DefaultCalendar.storageKey) private var defaultCalendarID = ""
    @State private var choosingZone = false
    @State private var confirmsSignOut = false

    var body: some View {
        let settings = session.settings
        Form {
            SettingsNotices()
            if let url = session.serverURL {
                Section("Server") {
                    LabeledContent("Adresse", value: ServerAddress.displayName(url))
                    if let me = settings.me {
                        LabeledContent("Angemeldet als", value: me.name ?? me.email ?? "–")
                        if let email = me.email, me.name != nil {
                            LabeledContent("E-Mail", value: email)
                        }
                    }
                    LabeledContent("Dieses Gerät", value: settings.me?.device?.name ?? session.deviceName)
                    if let server = settings.me?.serverDescription {
                        LabeledContent("Server-Version", value: server)
                    }
                    if session.needsSignIn {
                        Label("Nicht mehr angemeldet. Bitte im Hauptfenster neu anmelden.", systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                    }
                }
            }

            Section {
                Button {
                    choosingZone = true
                } label: {
                    LabeledContent("Zweite Zeitzone", value: zoneTitle)
                }
                .buttonStyle(.plain)
                if !secondaryTimeZone.isEmpty {
                    Button("Keine zweite Zeitzone") { secondaryTimeZone = "" }
                }
            } header: {
                Text("Anzeige")
            } footer: {
                Text("Erscheint als zusätzliche Uhrzeitspalte im Zeitraster.")
            }

            newEventsSection

            if session.stage == .server {
                Section {
                    Button("Abmelden …", role: .destructive) { confirmsSignOut = true }
                } footer: {
                    Text("Meldet dieses Gerät beim Server ab und löscht den lokalen Stand. Die Server-Adresse bleibt für die nächste Anmeldung stehen.")
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Allgemein")
        .task { await settings.loadMe() }
        .sheet(isPresented: $choosingZone) {
            TimeZonePicker(selection: $secondaryTimeZone)
        }
        .confirmationDialog("Abmelden?", isPresented: $confirmsSignOut, titleVisibility: .visible) {
            Button("Abmelden", role: .destructive) {
                Task { await session.signOut() }
            }
            Button("Abbrechen", role: .cancel) {}
        } message: {
            Text(signOutMessage)
        }
    }

    /// Standardkalender für neue Termine (LIC-47).
    private var newEventsSection: some View {
        let writable = session.model.writableCalendars
        return Section {
            Picker("Kalender", selection: $defaultCalendarID) {
                Text("Erster beschreibbarer").tag("")
                ForEach(writable) { calendar in
                    Text(calendar.name).tag(calendar.id)
                }
            }
        } header: {
            Text("Neue Termine")
        } footer: {
            Text(DefaultCalendar.fallbackNote(stored: defaultCalendarID, writable: writable)
                ?? "Gilt für die Schnelleingabe, das Zeitraster und „Neuer Termin“.")
        }
    }

    private var zoneTitle: String {
        guard let zone = TimeZoneCatalog.zone(fromStored: secondaryTimeZone) else { return "Keine" }
        return TimeZoneDisplay.cityName(for: zone)
    }

    private var signOutMessage: String {
        let pending = session.pendingChangeCount
        switch pending {
        case 0: return "Termine bleiben beim Anbieter; auf diesem Gerät wird der Stand gelöscht."
        case 1: return "Eine Änderung ist noch nicht beim Server angekommen und geht verloren."
        default: return "\(pending) Änderungen sind noch nicht beim Server angekommen und gehen verloren."
        }
    }
}

/// Auswahl mit Suche („York“, „Wien“).
private struct TimeZonePicker: View {
    @Environment(\.dismiss) private var dismiss
    @Binding var selection: String
    @State private var query = ""
    @State private var choices: [TimeZoneCatalog.Choice] = []

    init(selection: Binding<String>) {
        _selection = selection
    }

    var body: some View {
        NavigationStack {
            List {
                Button {
                    selection = ""
                    dismiss()
                } label: {
                    row(title: "Keine", detail: nil, selected: selection.isEmpty)
                }
                .buttonStyle(.plain)
                ForEach(TimeZoneCatalog.filter(choices, query: query)) { choice in
                    Button {
                        selection = choice.id
                        dismiss()
                    } label: {
                        row(title: choice.city, detail: "\(choice.offset) · \(choice.id)", selected: selection == choice.id)
                    }
                    .buttonStyle(.plain)
                }
            }
            .searchable(text: $query, prompt: "Stadt oder Zone")
            .navigationTitle("Zweite Zeitzone")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen") { dismiss() }
                }
            }
        }
        .onAppear {
            if choices.isEmpty { choices = TimeZoneCatalog.choices(home: .current, at: Date()) }
        }
        .settingsSheetFrame()
    }

    private func row(title: String, detail: String?, selected: Bool) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                if let detail {
                    Text(detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            if selected {
                Image(systemName: "checkmark")
                    .foregroundStyle(.tint)
            }
        }
        .contentShape(Rectangle())
    }
}
