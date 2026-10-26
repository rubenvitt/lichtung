// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Termin-Editor (LIC-20, Spec §5): Titel, Ort, Zeit, Wiederholung,
/// Erinnerungen, Kalender, Notiz. Die Logik steckt in `EventDraft`
/// (LichtungKit, getestet); hier ist nur die Form. Teilnehmende folgen mit E3.
struct EventEditorView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var draft: EventDraft
    @State private var askingSaveScope = false
    @State private var askingDelete = false

    init(draft: EventDraft) {
        _draft = State(initialValue: draft)
    }

    var body: some View {
        NavigationStack {
            Form {
                titleSection
                timeSection
                repeatSection
                alarmSection
                calendarSection
                if !draft.isNew {
                    deleteSection
                }
            }
            .formStyle(.grouped)
            // Datumsauswahlen zeigen die Wanduhrzeit des Termins (siehe EventDraft).
            .environment(\.timeZone, draft.deviceTimeZone)
            .navigationTitle(draft.isNew ? "Neuer Termin" : "Termin bearbeiten")
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Sichern") { save() }
                        .disabled(!draft.canSave)
                }
            }
            .confirmationDialog("Serientermin ändern", isPresented: $askingSaveScope, titleVisibility: .visible) {
                ForEach(draft.saveScopes, id: \.self) { scope in
                    Button(scope.editorLabel) { commit(scope) }
                }
                Button("Abbrechen", role: .cancel) {}
            } message: {
                Text("Für welche Termine der Serie gilt die Änderung?")
            }
        }
        // Wischen verwirft nichts: Mit Änderungen (und bei neuen Terminen)
        // schließt nur „Abbrechen“.
        .interactiveDismissDisabled(draft.hasChanges)
        .editorFrame()
    }

    // MARK: Abschnitte

    private var titleSection: some View {
        Section {
            TextField("Titel", text: $draft.title)
                .font(.title3)
            TextField("Ort", text: $draft.location)
            if draft.isSeriesOccurrence {
                Label("Teil einer Serie", systemImage: "repeat")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var dateComponents: DatePickerComponents {
        draft.isAllDay ? [.date] : [.date, .hourAndMinute]
    }

    private var timeSection: some View {
        Section {
            Toggle("Ganztägig", isOn: $draft.allDay)
            DatePicker("Beginn", selection: $draft.startPickerDate, displayedComponents: dateComponents)
            DatePicker("Ende", selection: $draft.endPickerDate, displayedComponents: dateComponents)
            if !draft.isAllDay {
                Picker("Zeitzone", selection: $draft.timeZoneSelection) {
                    ForEach(draft.timeZoneChoices, id: \.self) { id in
                        Text(EventDraft.timeZoneLabel(id)).tag(id)
                    }
                    if draft.timeZoneID == nil {
                        Text(EventDraft.timeZoneLabel(nil)).tag("")
                    }
                }
            }
            if let message = draft.validationMessage {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.red)
            }
        }
    }

    private var repeatSection: some View {
        Section("Wiederholung") {
            Picker("Wiederholen", selection: $draft.repeatFrequency) {
                ForEach(draft.frequencyChoices) { f in
                    Text(f.label).tag(f)
                }
            }
            switch draft.repeatFrequency {
            case .none:
                EmptyView()
            case .custom:
                Text("Eigene Regel des Anbieters. Sie bleibt, wie sie ist, solange hier nichts anderes gewählt wird.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            case .daily, .weekly, .monthly, .yearly:
                ruleRows
            }
        }
    }

    @ViewBuilder
    private var ruleRows: some View {
        Stepper(draft.repeatFrequency.intervalText(draft.repeatInterval), value: $draft.repeatInterval, in: 1...99)
        if draft.repeatFrequency == .weekly {
            weekdayRow
        }
        Picker("Endet", selection: $draft.repeatEnd) {
            ForEach(EventDraft.RepeatEnd.allCases) { end in
                Text(end.label).tag(end)
            }
        }
        if draft.repeatEnd == .until {
            DatePicker("Letzter Tag", selection: $draft.repeatUntilPickerDate, displayedComponents: [.date])
        }
        if draft.repeatEnd == .count {
            Stepper("\(draft.repeatCount) Mal", value: $draft.repeatCount, in: 1...999)
        }
    }

    private var weekdayRow: some View {
        HStack(spacing: 4) {
            ForEach(Weekday.allCases, id: \.self) { day in
                let selected = draft.effectiveWeekdays.contains(day)
                Button {
                    draft.toggleWeekday(day)
                } label: {
                    Text(QuickEntryPreview.weekdayShort[day.rawValue - 1])
                        .frame(maxWidth: .infinity, minHeight: 28)
                        .background(
                            selected ? Color.accentColor.opacity(0.25) : Color.secondary.opacity(0.1),
                            in: RoundedRectangle(cornerRadius: 6)
                        )
                }
                .buttonStyle(.borderless)
                .accessibilityLabel(QuickEntryPreview.weekdayLong[day.rawValue - 1])
                .accessibilityAddTraits(selected ? .isSelected : [])
            }
        }
    }

    private var alarmSection: some View {
        Section("Erinnerungen") {
            ForEach(draft.alarms, id: \.self) { minutes in
                HStack {
                    Text(EventDraft.alarmLabel(minutes, allDay: draft.isAllDay))
                    Spacer()
                    Button {
                        draft.removeAlarm(minutes)
                    } label: {
                        Image(systemName: "minus.circle.fill")
                            .foregroundStyle(.red)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel("Erinnerung entfernen")
                }
            }
            if !draft.fixedAlarms.isEmpty {
                Text(fixedAlarmText)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            if !draft.alarmChoices.isEmpty {
                Menu("Erinnerung hinzufügen") {
                    ForEach(draft.alarmChoices, id: \.self) { minutes in
                        Button(EventDraft.alarmLabel(minutes, allDay: draft.isAllDay)) {
                            draft.addAlarm(minutes)
                        }
                    }
                }
            }
        }
    }

    private var fixedAlarmText: String {
        let n = draft.fixedAlarms.count
        return n == 1 ? "1 Erinnerung zu einer festen Zeit bleibt erhalten." : "\(n) Erinnerungen zu festen Zeiten bleiben erhalten."
    }

    private var deleteMessage: String {
        draft.deleteScopes.isEmpty
            ? "Der Termin wird auch beim Anbieter gelöscht."
            : "Welche Termine der Serie sollen gelöscht werden?"
    }

    private var calendarSection: some View {
        Section {
            Picker("Kalender", selection: $draft.calendarId) {
                ForEach(model.editorCalendars(for: draft)) { cal in
                    Text(cal.name).tag(cal.id)
                }
            }
            TextField("Notiz", text: $draft.notes, axis: .vertical)
                .lineLimit(3...10)
        }
    }

    private var deleteSection: some View {
        Section {
            Button("Termin löschen", role: .destructive) {
                askingDelete = true
            }
            .confirmationDialog("Termin löschen", isPresented: $askingDelete, titleVisibility: .visible) {
                if draft.deleteScopes.isEmpty {
                    Button("Löschen", role: .destructive) { remove(.all) }
                } else {
                    ForEach(draft.deleteScopes, id: \.self) { scope in
                        Button(scope.editorLabel, role: .destructive) { remove(scope) }
                    }
                }
                Button("Abbrechen", role: .cancel) {}
            } message: {
                Text(deleteMessage)
            }
        }
    }

    // MARK: Aktionen

    private func save() {
        guard draft.hasChanges else {
            dismiss()
            return
        }
        if draft.saveScopes.isEmpty {
            commit(.all)
        } else {
            askingSaveScope = true
        }
    }

    private func commit(_ scope: EditScope) {
        let current = draft
        Task { await model.save(current, scope: scope) }
        dismiss()
    }

    private func remove(_ scope: EditScope) {
        let current = draft
        Task { await model.delete(current, scope: scope) }
        dismiss()
    }
}

/// Zeigt den Editor, sobald `AppModel.editorDraft` gesetzt ist (Einstieg
/// über „+“, ⌘N oder „Bearbeiten“ in der Detailansicht).
struct EventEditorPresenter: ViewModifier {
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        @Bindable var model = model
        content.sheet(item: $model.editorDraft) { draft in
            EventEditorView(draft: draft)
        }
    }
}

extension View {
    func eventEditorSheet() -> some View {
        modifier(EventEditorPresenter())
    }
}

private extension View {
    /// Mac: Sheet mit festem Rahmen; iPhone: volle Höhe.
    @ViewBuilder
    func editorFrame() -> some View {
        #if os(macOS)
        frame(width: 480, height: 640)
        #else
        self
        #endif
    }

    @ViewBuilder
    func inlineNavigationTitle() -> some View {
        #if os(iOS)
        navigationBarTitleDisplayMode(.inline)
        #else
        self
        #endif
    }
}
