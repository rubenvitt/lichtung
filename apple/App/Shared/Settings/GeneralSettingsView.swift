// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Server, Gerät, zweite Zeitzone, Standardkalender (LIC-47), Standarddauer
/// (LIC-53), Abmelden (LIC-43).
struct GeneralSettingsView: View {
    @Environment(AppSession.self) private var session
    /// IANA-ID der zweiten Zeitzone, leer = keine. Das Raster liest sie über
    /// `AppModel.secondaryTimeZone` (siehe `TimeZoneCatalog.zone(fromStored:)`).
    @AppStorage("secondaryTimeZone") private var secondaryTimeZone = ""
    /// Kalender für neue Termine, leer = erster beschreibbarer (LIC-47).
    /// `AppModel.defaultCalendarId` liest ihn beim Anlegen.
    @AppStorage(DefaultCalendar.storageKey) private var defaultCalendarID = ""
    /// Dauer neuer Termine in Minuten, 0 = Standard (LIC-53). `AppModel.newEventMinutes`
    /// und die Schnelleingabe lesen sie über `NewEventDuration.minutes(stored:)`.
    @AppStorage(NewEventDuration.storageKey) private var newEventMinutes = 0
    /// Erinnerungen als Mitteilungen (LIC-68), liest `ReminderScheduler`.
    @AppStorage(ReminderScheduler.enabledKey) private var remindersEnabled = true
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

            mergeSection

            newEventsSection

            remindersSection

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

    /// Erinnerungen der Termine als lokale Mitteilungen (LIC-68).
    private var remindersSection: some View {
        Section {
            Toggle("Erinnerungen als Mitteilungen", isOn: $remindersEnabled)
            if remindersEnabled, ReminderScheduler.shared.permission == .denied {
                Label("Mitteilungen sind für Lichtung in den Systemeinstellungen ausgeschaltet.", systemImage: "bell.slash")
                    .foregroundStyle(.orange)
            }
        } header: {
            Text("Erinnerungen")
        } footer: {
            Text("Lichtung erinnert an die nächsten sieben Tage, mit den Erinnerungen, die am Termin stehen. Termine, die Claude anlegt, erinnern, sobald die App sie einmal geladen hat.")
        }
        .task { await ReminderScheduler.shared.refreshPermission() }
    }

    /// Gleiche Termine und Blocker zusammenfassen (`AppModel.mergeRules`).
    private var mergeSection: some View {
        @Bindable var model = session.model
        let titles = Binding<String>(
            get: { model.mergeRules.blockerTitles.joined(separator: ", ") },
            set: { model.mergeRules.blockerTitles = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty } }
        )
        return Section {
            Toggle("Gleiche Termine zusammenfassen", isOn: $model.mergeRules.mergeIdentical)
            Toggle("Blocker ausblenden", isOn: $model.mergeRules.hideBlockers)
            if model.mergeRules.hideBlockers {
                TextField("Titel von Blockern", text: titles)
            }
        } header: {
            Text("Zusammenfassen")
        } footer: {
            Text("Ein Blocker wird nur ausgeblendet, wenn er zeitgleich mit einem anderen Termin liegt. Am Termin steht, in welchen Kalendern er noch eingetragen ist.")
        }
    }

    /// Standardkalender (LIC-47) und Standarddauer (LIC-53) für neue Termine.
    private var newEventsSection: some View {
        let writable = session.model.writableCalendars
        let minutes = Binding<Int>(
            get: { NewEventDuration.minutes(stored: newEventMinutes) },
            set: { newEventMinutes = $0 }
        )
        return Section {
            Picker("Kalender", selection: $defaultCalendarID) {
                Text("Erster beschreibbarer").tag("")
                ForEach(writable) { calendar in
                    Text(calendar.name).tag(calendar.id)
                }
            }
            Picker("Dauer", selection: minutes) {
                ForEach(NewEventDuration.choices, id: \.self) { m in
                    Text(NewEventDuration.label(m)).tag(m)
                }
            }
        } header: {
            Text("Neue Termine")
        } footer: {
            Text(DefaultCalendar.fallbackNote(stored: defaultCalendarID, writable: writable)
                ?? "Gilt für die Schnelleingabe, das Zeitraster und „Neuer Termin“. Die Dauer gilt, wenn nichts anderes angegeben ist.")
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
