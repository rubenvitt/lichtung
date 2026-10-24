// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Einfache Detailansicht. Der Editor folgt mit LIC-20.
struct EventDetailView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let occurrence: Occurrence

    var body: some View {
        let d = occurrence.details
        NavigationStack {
            Form {
                Section {
                    HStack {
                        CalendarDot(color: model.color(for: occurrence.calendarId), size: 10)
                        Text(d.title).font(.title3.weight(.semibold))
                    }
                    Text(AgendaBuilder.heading(LocalDate(occurrence.start, calendar: model.calendar), today: model.today))
                    Text(AgendaBuilder.timeText(occurrence, calendar: model.calendar)).foregroundStyle(.secondary)
                    if occurrence.recurrenceId != nil {
                        Label("Teil einer Serie", systemImage: "repeat").foregroundStyle(.secondary)
                    }
                }
                if let location = d.location, !location.isEmpty {
                    Section("Ort") { Text(location) }
                }
                if let notes = d.notes, !notes.isEmpty {
                    Section("Notiz") { Text(notes).textSelection(.enabled) }
                }
                if !d.attendees.isEmpty {
                    Section("Teilnehmende") {
                        ForEach(d.attendees, id: \.email) { a in
                            Text(a.name ?? a.email)
                        }
                    }
                }
                Section {
                    Button("Duplizieren") {
                        Task { await model.duplicate(occurrence) }
                        dismiss()
                    }
                    Button("Löschen", role: .destructive) {
                        Task { await model.delete(occurrence) }
                        dismiss()
                    }
                }
            }
            .navigationTitle(model.state.calendars[occurrence.calendarId]?.name ?? "Termin")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { dismiss() }
                }
            }
        }
    }
}

extension AgendaBuilder {
    /// Zeittext eines Vorkommens an seinem ersten Tag.
    static func timeText(_ o: Occurrence, calendar cal: Calendar) -> String {
        let day = LocalDate(o.start, calendar: cal)
        return AgendaBuilder.days([o], from: day, days: 1, today: day, calendar: cal).first?.items.first?.timeText ?? ""
    }
}
