// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Schnelleingabe per Satz mit Vorschau beim Tippen (Spec §5, LIC-17).
struct QuickEntryBar: View {
    @Environment(AppModel.self) private var model
    @State private var text = ""
    @FocusState private var focused: Bool
    /// Dauer, wenn der Satz keine nennt (LIC-53, `NewEventDuration`).
    @AppStorage(NewEventDuration.storageKey) private var storedMinutes = 0

    private var newEventMinutes: Int { NewEventDuration.minutes(stored: storedMinutes) }

    private var choices: [CalendarChoice] {
        model.writableCalendars.map { CalendarChoice(id: $0.id, name: $0.name) }
    }

    private func parse(now: Date) -> QuickEntry {
        QuickEntryParser(timeZone: model.timeZone, defaultDurationMinutes: newEventMinutes, calendars: choices).parse(text, now: now)
    }

    var body: some View {
        let now = Date()
        let entry = parse(now: now)
        let preview = QuickEntryPreview(entry, now: now, timeZone: model.timeZone, calendars: choices)
        VStack(alignment: .leading, spacing: 6) {
            if !text.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Text(preview.title).font(.headline)
                    Label(preview.when, systemImage: "clock")
                    if let r = preview.recurrence { Label(r, systemImage: "repeat") }
                    if let l = preview.location { Label(l, systemImage: "mappin.and.ellipse") }
                    Label(preview.calendarName ?? model.writableCalendars.first?.name ?? "Kein Kalender", systemImage: "calendar")
                }
                .font(.callout)
                .foregroundStyle(.secondary)
                .transition(.opacity)
            }
            HStack {
                TextField("Zahnarzt Di 14 Uhr …", text: $text)
                    .textFieldStyle(.roundedBorder)
                    .focused($focused)
                    .onSubmit(submit)
                    .accessibilityLabel("Neuer Termin als Satz")
                Button(action: submit) {
                    Image(systemName: "plus.circle.fill").font(.title2)
                }
                .buttonStyle(.plain)
                .disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
                .keyboardShortcut(.return, modifiers: .command)
                .accessibilityLabel("Termin anlegen")
            }
        }
        .padding(10)
        .animation(.default, value: text.isEmpty)
    }

    private func submit() {
        let entry = parse(now: Date())
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        text = ""
        Task { await model.create(from: entry) }
    }
}
