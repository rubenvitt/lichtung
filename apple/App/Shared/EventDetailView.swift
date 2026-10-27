// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Einfache Detailansicht. „Bearbeiten“ öffnet den Editor (LIC-20).
struct EventDetailView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let occurrence: Occurrence
    /// Frage nach dem Geltungsbereich beim Löschen einer Serie (LIC-50).
    @State private var confirmsSeriesDelete = false

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
                    // Zeitzone und Reisezeit (LIC-42)
                    if let zoneHint = TimeZoneDisplay.originalZoneHint(for: d, home: model.timeZone) {
                        Label(zoneHint, systemImage: "globe")
                            .foregroundStyle(.secondary)
                            .accessibilityLabel("In anderer Zeitzone angelegt: \(zoneHint)")
                    }
                    if let travel = TravelTime.detailText(for: occurrence, calendar: model.calendar) {
                        Label(travel, systemImage: "car").foregroundStyle(.secondary)
                    }
                }
                // Terminanfrage: Status und Antworten (LIC-30)
                if InvitationActions.isInvitation(occurrence) {
                    InvitationDetailSection(occurrence: occurrence)
                }
                // Videolink (LIC-42)
                if let link = VideoLinkDetector.detect(in: d) {
                    Section("Videokonferenz") {
                        HStack {
                            Label(link.provider.displayName, systemImage: "video")
                            Spacer(minLength: 8)
                            JoinButton(occurrence: occurrence, link: link, prominent: true)
                        }
                        if let meetingID = link.meetingID {
                            LabeledContent("Meeting-ID", value: meetingID).textSelection(.enabled)
                        }
                        if let passcode = link.passcode {
                            LabeledContent("Kenncode", value: passcode).textSelection(.enabled)
                        }
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
                // Über einen Buchungslink gebucht (LIC-33): Gast und Notiz
                // stehen nur in Lichtung, nicht im Termin.
                if let found = model.state.openingBooking(forEvent: occurrence.eventId) {
                    Section {
                        OpeningBookingRow(booking: found.booking, today: model.today, calendar: model.calendar, showsTime: false)
                    } header: {
                        Text("Gebucht über „\(found.opening.title)“")
                    }
                }
                Section {
                    Button("Duplizieren") {
                        Task { await model.duplicate(occurrence) }
                        dismiss()
                    }
                    // Einfügen danach am Tag: ⌘V bzw. Kontextmenü (Mac),
                    // Toolbar oder Monatstag (iPhone, LIC-50).
                    Button("Kopieren") {
                        Pasteboard.write(model.copy(occurrence))
                        dismiss()
                    }
                    // Wie ⌫ und Kontextmenü (LIC-50): Einzeltermine sofort
                    // (Rückgängig geht), bei Serien erst nach der Frage.
                    Button(deleteTitle, role: .destructive) {
                        if OccurrenceActions.deleteScopes(occurrence).isEmpty {
                            Task { await model.delete(occurrence, scope: .all) }
                            dismiss()
                        } else {
                            confirmsSeriesDelete = true
                        }
                    }
                    .disabled(!model.canEdit(occurrence))
                }
            }
            .modifier(SeriesDeleteDialog(
                model: model,
                isPresented: $confirmsSeriesDelete,
                occurrence: occurrence,
                onDelete: { dismiss() }
            ))
            .navigationTitle(model.state.calendars[occurrence.calendarId]?.name ?? "Termin")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { dismiss() }
                }
                // MARK: Editor (LIC-20)
                ToolbarItem(placement: .primaryAction) {
                    Button("Bearbeiten") { model.edit(occurrence) }
                        .disabled(!model.canEdit(occurrence))
                        .keyboardShortcut("e", modifiers: .command)
                }
            }
        }
    }
}

extension EventDetailView {
    /// Mit Auslassungspunkten, wenn noch eine Frage folgt (wie im Kontextmenü).
    private var deleteTitle: String {
        OccurrenceActions.isSeries(occurrence) ? "Löschen …" : "Löschen"
    }
}

extension AgendaBuilder {
    /// Zeittext eines Vorkommens an seinem ersten Tag.
    static func timeText(_ o: Occurrence, calendar cal: Calendar) -> String {
        let day = LocalDate(o.start, calendar: cal)
        return AgendaBuilder.days([o], from: day, days: 1, today: day, calendar: cal).first?.items.first?.timeText ?? ""
    }
}
