// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Buchungslinks verwalten (Spec §6, LIC-35): offene Einmal-Links kopieren,
/// teilen und löschen, feste Links (Vorlagen) anlegen und ändern, neuen
/// Einmal-Link erzeugen. Regeln und Texte im Kern (`Features/Openings.swift`).
struct OpeningsSettingsView: View {
    @Environment(AppSession.self) private var session
    /// Zuletzt gewählter Zielkalender, Vorauswahl für neue Links.
    @AppStorage("openingCalendar") private var lastCalendarId = ""
    @State private var creatingOnce = false
    @State private var creatingTemplate = false
    @State private var editing: Opening?
    @State private var deleting: Opening?
    @State private var copiedId: String?

    var body: some View {
        let settings = session.settings
        let list = settings.openingList
        let calendars = settings.calendarsById
        let hasTargets = !settings.openingTargets.isEmpty
        List {
            SettingsNotices()
            Section {
                Button {
                    creatingOnce = true
                } label: {
                    Label("Neuer Einmal-Link", systemImage: "link.badge.plus")
                }
                .disabled(!settings.isAvailable || !hasTargets)
                Button {
                    creatingTemplate = true
                } label: {
                    Label("Neuer fester Link", systemImage: "calendar.badge.plus")
                }
                .disabled(!settings.isAvailable || !hasTargets)
            } footer: {
                if settings.isAvailable, !hasTargets {
                    Text(OpeningTargets.noTargetHint)
                } else {
                    Text("Ein Einmal-Link gilt für genau eine Buchung, ein fester Link beliebig oft. Gäste sehen nur freie Zeiten, nie Termindetails.")
                }
            }

            Section {
                if list.open.isEmpty {
                    Text("Keine offenen Einmal-Links.")
                        .foregroundStyle(.secondary)
                }
                ForEach(list.open) { opening in
                    row(opening, calendars: calendars, editable: false)
                }
            } header: {
                Text("Offene Einmal-Links")
            }

            Section {
                if list.templates.isEmpty {
                    Text("Noch keine festen Links, z. B. „30 Minuten Kennenlernen“ mit Zeitfenstern werktags.")
                        .foregroundStyle(.secondary)
                }
                ForEach(list.templates) { opening in
                    row(opening, calendars: calendars, editable: true)
                }
            } header: {
                Text("Feste Links")
            }

            if !list.booked.isEmpty {
                Section {
                    ForEach(list.booked) { opening in
                        OpeningRow(opening: opening, calendars: calendars, today: session.model.today, calendar: session.model.calendar)
                            .contextMenu {
                                Button("Aus der Liste löschen", systemImage: "trash", role: .destructive) { deleting = opening }
                            }
                    }
                } header: {
                    Text("Kürzlich gebucht")
                }
            }
        }
        .navigationTitle("Buchungslinks")
        .sheet(isPresented: $creatingOnce) {
            OneOffLinkSheet(calendarId: OpeningTargets.preferred(calendars, last: lastCalendarId)) { lastCalendarId = $0 }
                .environment(session)
        }
        .sheet(isPresented: $creatingTemplate) {
            OpeningTemplateEditor(draft: OpeningTemplateDraft(
                calendarId: OpeningTargets.preferred(calendars, last: lastCalendarId), timeZone: session.model.timeZone
            ))
            .environment(session)
        }
        .sheet(item: $editing) { opening in
            OpeningTemplateEditor(draft: OpeningTemplateDraft(editing: opening))
                .environment(session)
        }
        .confirmationDialog(
            "Link löschen?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            titleVisibility: .visible,
            presenting: deleting
        ) { opening in
            Button("„\(opening.title)“ löschen", role: .destructive) {
                Task { await settings.deleteOpening(opening.id) }
            }
            Button("Abbrechen", role: .cancel) {}
        } message: { opening in
            Text(opening.isBooked
                ? "Der gebuchte Termin bleibt im Kalender."
                : "Der Link funktioniert danach nicht mehr. Reservierte Vorschläge verschwinden aus dem Kalender.")
        }
    }

    /// Zeile eines offenen Einmal-Links oder festen Links: Kopieren, Teilen
    /// (iPhone), Löschen per Wischen und Kontextmenü; feste Links öffnen
    /// beim Tippen den Editor.
    @ViewBuilder
    private func row(_ opening: Opening, calendars: [String: CalendarInfo], editable: Bool) -> some View {
        HStack {
            if editable {
                Button {
                    editing = opening
                } label: {
                    OpeningRow(opening: opening, calendars: calendars, today: session.model.today, calendar: session.model.calendar)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            } else {
                OpeningRow(opening: opening, calendars: calendars, today: session.model.today, calendar: session.model.calendar)
            }
            Spacer()
            Button {
                copy(opening)
            } label: {
                Image(systemName: copiedId == opening.id ? "checkmark" : "doc.on.doc")
            }
            .buttonStyle(.borderless)
            .accessibilityLabel("Link kopieren")
            #if os(iOS)
            if let url = URL(string: opening.url) {
                ShareLink(item: url) {
                    Image(systemName: "square.and.arrow.up")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Link teilen")
            }
            #endif
        }
        .swipeActions(edge: .trailing) {
            Button("Löschen", role: .destructive) { deleting = opening }
        }
        .contextMenu {
            Button("Link kopieren", systemImage: "doc.on.doc") { copy(opening) }
            if let url = URL(string: opening.url) {
                ShareLink(item: url) {
                    Label("Teilen …", systemImage: "square.and.arrow.up")
                }
            }
            if editable {
                Button("Bearbeiten", systemImage: "pencil") { editing = opening }
            }
            Divider()
            Button("Löschen", systemImage: "trash", role: .destructive) { deleting = opening }
        }
    }

    private func copy(_ opening: Opening) {
        Clipboard.copy(OpeningText.shareText(opening))
        copiedId = opening.id
    }
}

/// Titel, Kurzfassung, Vorschläge und Hinweis zum Zustand eines Links.
private struct OpeningRow: View {
    let opening: Opening
    let calendars: [String: CalendarInfo]
    let today: LocalDate
    let calendar: Calendar

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(opening.title)
            Text(opening.summary(calendars: calendars))
                .font(.caption)
                .foregroundStyle(.secondary)
            if !opening.isBooked {
                ForEach(opening.suggestions) { suggestion in
                    Text(OpeningText.slot(start: suggestion.start, end: suggestion.end, today: today, calendar: calendar))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if opening.kind == .template {
                    Text(opening.rulesText)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if !opening.bookings.isEmpty {
                        Text(OpeningText.bookingCount(opening.bookings.count))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            // Wer gebucht hat und seine Notiz (LIC-33): nur hier, nicht im Termin.
            if opening.kind == .once, let booking = opening.bookings.first {
                Text(booking.guestText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if let note = booking.noteText {
                    Text(note)
                        .font(.caption)
                        .lineLimit(3)
                }
            }
            if let status = opening.statusText(calendars: calendars, today: today, calendar: calendar) {
                Label(status, systemImage: opening.isBooked ? "checkmark.circle" : "exclamationmark.circle")
                    .font(.caption)
                    .foregroundStyle(opening.isBooked ? Color.secondary : Color.orange)
            }
        }
        .padding(.vertical, 2)
    }
}

/// Gast, gebuchte Zeit und Notiz einer Buchung (LIC-33). Die Notiz steht
/// nur hier und in der Termindetailansicht, nie in der Einladung.
struct OpeningBookingRow: View {
    let booking: Opening.Booking
    let today: LocalDate
    let calendar: Calendar
    var showsTime = true

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(booking.guestName)
            Text(booking.guestEmail)
                .font(.caption)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            if showsTime {
                Text(booking.timeText(today: today, calendar: calendar))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if let note = booking.noteText {
                Text(note)
                    .font(.callout)
                    .textSelection(.enabled)
                    .padding(.top, 2)
            }
        }
        .padding(.vertical, 2)
    }
}

// MARK: - Neuer Einmal-Link

/// Dauer, Zielkalender, optional Titel und Vorschläge; danach der Link zum
/// Kopieren oder Teilen.
private struct OneOffLinkSheet: View {
    /// Ein Vorschlag mit fester Identität für `ForEach`.
    struct Slot: Identifiable {
        let id = UUID()
        var start: Date
    }

    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    @State private var draft: OneOffLinkDraft
    @State private var slots: [Slot] = []
    @State private var created: Opening?
    @State private var error: String?
    @State private var copied = false
    private let onCalendarChosen: (String) -> Void

    init(calendarId: String?, onCalendarChosen: @escaping (String) -> Void) {
        _draft = State(initialValue: OneOffLinkDraft(durationMinutes: 30, calendarId: calendarId))
        self.onCalendarChosen = onCalendarChosen
    }

    /// Der Entwurf mit den Vorschlägen aus der Liste.
    private var current: OneOffLinkDraft {
        var d = draft
        d.suggestions = slots.map(\.start)
        return d
    }

    var body: some View {
        let settings = session.settings
        NavigationStack {
            Form {
                if let created {
                    createdSection(created)
                } else {
                    inputSections(settings)
                }
            }
            .formStyle(.grouped)
            .navigationTitle(created == nil ? "Neuer Einmal-Link" : "Link erstellt")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    if created == nil {
                        Button("Abbrechen") { dismiss() }
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if created == nil {
                        Button("Erstellen") { create() }
                            .disabled(current.problem(now: Date(), calendars: settings.calendarsById) != nil || settings.isWorking)
                    } else {
                        Button("Fertig") { dismiss() }
                    }
                }
            }
        }
        .settingsSheetFrame()
    }

    @ViewBuilder
    private func inputSections(_ settings: SettingsModel) -> some View {
        Section {
            Picker("Dauer", selection: $draft.durationMinutes) {
                ForEach(OpeningText.choices(OpeningText.durationChoices, including: draft.durationMinutes), id: \.self) { minutes in
                    Text(OpeningText.duration(minutes)).tag(minutes)
                }
            }
            Picker("Zielkalender", selection: $draft.calendarId) {
                ForEach(settings.openingTargets) { calendar in
                    Text(calendar.name).tag(String?.some(calendar.id))
                }
            }
            TextField("Titel", text: $draft.title, prompt: Text("Termin (\(draft.durationMinutes) Minuten)"))
        } footer: {
            Text("Zielkalender sind beschreibbare Kalender, die Einladungen verschicken können. Der Gast bekommt die Einladung von dort.")
        }

        Section {
            ForEach($slots) { $slot in
                HStack {
                    DatePicker("Vorschlag", selection: $slot.start, in: Date()..., displayedComponents: [.date, .hourAndMinute])
                    Button {
                        let id = slot.id
                        slots.removeAll { $0.id == id }
                    } label: {
                        Image(systemName: "minus.circle.fill")
                            .foregroundStyle(.red)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel("Vorschlag entfernen")
                }
            }
            Button {
                slots.append(Slot(start: current.nextSuggestion(now: Date(), calendar: session.model.calendar)))
            } label: {
                Label("Vorschlag hinzufügen", systemImage: "plus")
            }
            .disabled(slots.count >= Opening.maxSuggestions)
        } header: {
            Text("Vorschläge (optional)")
        } footer: {
            Text(current.suggestionsHint)
        }

        Section {
            Text(current.summary(calendars: settings.calendarsById))
            if let problem = current.problem(now: Date(), calendars: settings.calendarsById) {
                Text(problem)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
            }
        }
    }

    @ViewBuilder
    private func createdSection(_ opening: Opening) -> some View {
        Section {
            Text(opening.url)
                .font(.body.monospaced())
                .textSelection(.enabled)
            Button {
                Clipboard.copy(OpeningText.shareText(opening))
                copied = true
            } label: {
                Label(copied ? "Kopiert" : "Link kopieren", systemImage: copied ? "checkmark" : "doc.on.doc")
            }
            if let url = URL(string: opening.url) {
                ShareLink(item: url) {
                    Label("Teilen …", systemImage: "square.and.arrow.up")
                }
            }
        } header: {
            Text(opening.title)
        } footer: {
            Text(opening.summary(calendars: session.settings.calendarsById))
        }
        if !opening.suggestions.isEmpty {
            Section("Reserviert") {
                ForEach(opening.suggestions) { suggestion in
                    Text(OpeningText.slot(start: suggestion.start, end: suggestion.end, today: session.model.today, calendar: session.model.calendar))
                }
            }
        }
    }

    private func create() {
        let settings = session.settings
        let draft = current
        guard let body = draft.newOpening(now: Date(), calendars: settings.calendarsById, timeZone: session.model.timeZone) else { return }
        Task {
            if let opening = await settings.createOpening(body) {
                if let id = draft.calendarId { onCalendarChosen(id) }
                Clipboard.copy(OpeningText.shareText(opening))
                copied = true
                error = nil
                created = opening
            } else {
                error = settings.errorMessage
                settings.errorMessage = nil
            }
        }
    }
}

// MARK: - Fester Link

/// Titel, Dauer, Zielkalender, Wochenfenster, Puffer, Vorlauf, Horizont und
/// Videolink eines festen Links; gesendet wird erst mit „Sichern“.
private struct OpeningTemplateEditor: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    @State private var draft: OpeningTemplateDraft
    @State private var error: String?
    @State private var confirmsDelete = false
    @State private var copied = false

    init(draft: OpeningTemplateDraft) {
        _draft = State(initialValue: draft)
    }

    var body: some View {
        let settings = session.settings
        let calendars = settings.calendarsById
        let problem = draft.problem(calendars: calendars)
        NavigationStack {
            Form {
                Section {
                    TextField("Titel", text: $draft.title, prompt: Text("z. B. 30 Minuten Kennenlernen"))
                    Picker("Dauer", selection: $draft.durationMinutes) {
                        ForEach(OpeningText.choices(OpeningText.durationChoices, including: draft.durationMinutes), id: \.self) { minutes in
                            Text(OpeningText.duration(minutes)).tag(minutes)
                        }
                    }
                    Picker("Zielkalender", selection: $draft.calendarId) {
                        ForEach(targets(calendars)) { calendar in
                            Text(calendar.name).tag(String?.some(calendar.id))
                        }
                    }
                }

                Section {
                    ForEach($draft.windows) { $window in
                        HStack {
                            Picker("Tag", selection: $window.weekday) {
                                ForEach(Weekday.allCases, id: \.self) { day in
                                    Text(OpeningText.shortName(day)).tag(day)
                                }
                            }
                            .labelsHidden()
                            .fixedSize()
                            Spacer()
                            DatePicker("Von", selection: clock($window.start), displayedComponents: .hourAndMinute)
                                .labelsHidden()
                            Text("–")
                            DatePicker("Bis", selection: clock($window.end), displayedComponents: .hourAndMinute)
                                .labelsHidden()
                            Button {
                                let id = window.id
                                draft.windows.removeAll { $0.id == id }
                            } label: {
                                Image(systemName: "minus.circle.fill")
                                    .foregroundStyle(.red)
                            }
                            .buttonStyle(.borderless)
                            .accessibilityLabel("Zeitfenster entfernen")
                        }
                    }
                    Button {
                        draft.addWindow()
                    } label: {
                        Label("Zeitfenster hinzufügen", systemImage: "plus")
                    }
                    .disabled(draft.windows.count >= Opening.maxWindows)
                } header: {
                    Text("Zeitfenster")
                } footer: {
                    Text("Gäste buchen innerhalb dieser Zeiten (Zeitzone \(draft.timeZone)), nur wenn frei.")
                }

                Section {
                    minutesPicker("Puffer vorher", selection: $draft.bufferBeforeMinutes, choices: OpeningText.bufferChoices) {
                        $0 == 0 ? "Keiner" : OpeningText.duration($0)
                    }
                    minutesPicker("Puffer nachher", selection: $draft.bufferAfterMinutes, choices: OpeningText.bufferChoices) {
                        $0 == 0 ? "Keiner" : OpeningText.duration($0)
                    }
                    minutesPicker("Mindestvorlauf", selection: $draft.minNoticeMinutes, choices: OpeningText.noticeChoices) {
                        OpeningText.notice($0)
                    }
                    minutesPicker("Buchungshorizont", selection: $draft.horizonDays, choices: OpeningText.horizonChoices) {
                        OpeningText.horizon($0)
                    }
                } header: {
                    Text("Regeln")
                }

                Section {
                    TextField("Videolink", text: $draft.videoURL, prompt: Text("https://meet.example.org/raum"))
                        .plainTextInput()
                        #if os(iOS)
                        .keyboardType(.URL)
                        #endif
                } footer: {
                    Text("Optional. Die Buchung übernimmt ihn in den Termin.")
                }

                if let original = draft.original {
                    Section("Link") {
                        Text(original.url)
                            .font(.callout.monospaced())
                            .textSelection(.enabled)
                        Button {
                            Clipboard.copy(OpeningText.shareText(original))
                            copied = true
                        } label: {
                            Label(copied ? "Kopiert" : "Link kopieren", systemImage: copied ? "checkmark" : "doc.on.doc")
                        }
                    }

                    // Buchungen mit Name, E-Mail-Adresse und Notiz der Gäste (LIC-33).
                    let bookings = settings.opening(original.id)?.bookings ?? original.bookings
                    if !bookings.isEmpty {
                        Section {
                            ForEach(bookings) { booking in
                                OpeningBookingRow(booking: booking, today: session.model.today, calendar: session.model.calendar)
                            }
                        } header: {
                            Text("Buchungen")
                        } footer: {
                            Text("Nur für dich. Die Einladung an den Gast enthält den Titel des Links und den Videolink, nicht seine Notiz.")
                        }
                    }
                }

                if let problem {
                    Section {
                        Text(problem)
                            .foregroundStyle(.secondary)
                    }
                }
                if let error {
                    Section {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                    }
                }
                if !draft.isNew {
                    Section {
                        Button("Link löschen", role: .destructive) { confirmsDelete = true }
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(draft.isNew ? "Neuer fester Link" : "Fester Link")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Sichern") { save() }
                        .disabled(problem != nil || settings.isWorking || !settings.isAvailable)
                }
            }
            .confirmationDialog("Link löschen?", isPresented: $confirmsDelete, titleVisibility: .visible) {
                Button("Löschen", role: .destructive) { delete() }
                Button("Abbrechen", role: .cancel) {}
            } message: {
                Text("Der Link funktioniert danach nicht mehr. Schon gebuchte Termine bleiben.")
            }
        }
        .settingsSheetFrame()
    }

    /// Zielkalender zur Auswahl, dazu der bisherige, falls er nicht mehr taugt
    /// (sonst zeigt die Auswahl nichts an).
    private func targets(_ calendars: [String: CalendarInfo]) -> [CalendarInfo] {
        var list = OpeningTargets.allowed(calendars)
        if let id = draft.calendarId, !list.contains(where: { $0.id == id }), let current = calendars[id] {
            list.append(current)
        }
        return list
    }

    private func minutesPicker(_ title: String, selection: Binding<Int>, choices: [Int], label: @escaping (Int) -> String) -> some View {
        Picker(title, selection: selection) {
            ForEach(OpeningText.choices(choices, including: selection.wrappedValue), id: \.self) { value in
                Text(label(value)).tag(value)
            }
        }
    }

    /// Minuten seit Mitternacht als Uhrzeit für `DatePicker` (an einem Tag
    /// ohne Zeitumstellung).
    private func clock(_ minutes: Binding<Int>) -> Binding<Date> {
        let calendar = Calendar.current
        let reference = calendar.date(from: DateComponents(year: 2026, month: 1, day: 5)) ?? Date(timeIntervalSinceReferenceDate: 0)
        return Binding(
            get: { reference.addingTimeInterval(TimeInterval(minutes.wrappedValue * 60)) },
            set: { date in
                let c = calendar.dateComponents([.hour, .minute], from: date)
                minutes.wrappedValue = (c.hour ?? 0) * 60 + (c.minute ?? 0)
            }
        )
    }

    private func save() {
        let settings = session.settings
        let calendars = settings.calendarsById
        let draft = draft
        Task {
            let ok: Bool
            if let original = draft.original {
                guard let patch = draft.patch(calendars: calendars) else { return }
                ok = await settings.updateOpening(original.id, patch)
            } else {
                guard let body = draft.newOpening(calendars: calendars) else { return }
                ok = await settings.createOpening(body) != nil
            }
            if ok {
                dismiss()
            } else {
                error = settings.errorMessage
                settings.errorMessage = nil
            }
        }
    }

    private func delete() {
        guard let id = draft.original?.id else { return }
        let settings = session.settings
        Task {
            if await settings.deleteOpening(id) {
                dismiss()
            } else {
                error = settings.errorMessage
                settings.errorMessage = nil
            }
        }
    }
}
