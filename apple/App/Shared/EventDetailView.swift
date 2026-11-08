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
                    EventDetailHeader(occurrence: occurrence)
                        .listRowBackground(model.color(for: occurrence.calendarId).opacity(0.12))
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
                    // LIC-69
                    if d.availability == .free {
                        Label("Als frei angezeigt", systemImage: "circle.dashed").foregroundStyle(.secondary)
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
                    Section("Ort") {
                        Label(location, systemImage: "mappin.and.ellipse")
                            .textSelection(.enabled)
                    }
                }
                // Link am Termin (LIC-69)
                // Ohne Schema (z. B. „example.org/raum“ aus einem anderen
                // Programm) wie im Editor mit https:// davor; was sich nicht
                // öffnen lässt, steht als Text da.
                if let text = d.url?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty {
                    Section("Link") {
                        if let url = EventDraft.effectiveURL(text).flatMap(URL.init(string:)), url.scheme != nil {
                            Link(destination: url) {
                                Text(text).lineLimit(2).truncationMode(.middle)
                            }
                        } else {
                            Text(text).textSelection(.enabled)
                        }
                    }
                }
                if let notes = d.notes, !notes.isEmpty {
                    Section("Notiz") { Text(notes).textSelection(.enabled) }
                }
                if !d.attendees.isEmpty {
                    Section("Teilnehmende (\(d.attendees.count))") {
                        ForEach(d.attendees, id: \.email) { a in
                            AttendeeRow(name: a.name ?? a.email, detail: a.name == nil ? nil : a.email)
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
                    Button {
                        Task { await model.duplicate(occurrence) }
                        dismiss()
                    } label: {
                        Label("Duplizieren", systemImage: "plus.square.on.square")
                    }
                    // Einfügen danach am Tag: ⌘V bzw. Kontextmenü (Mac),
                    // Toolbar oder Monatstag (iPhone, LIC-50).
                    Button {
                        Pasteboard.write(model.copy(occurrence))
                        dismiss()
                    } label: {
                        Label("Kopieren", systemImage: "doc.on.doc")
                    }
                    // Wie ⌫ und Kontextmenü (LIC-50): Einzeltermine sofort
                    // (Rückgängig geht), bei Serien erst nach der Frage.
                    Button(role: .destructive) {
                        if OccurrenceActions.deleteScopes(occurrence).isEmpty {
                            Task { await model.delete(occurrence, scope: .all) }
                            dismiss()
                        } else {
                            confirmsSeriesDelete = true
                        }
                    } label: {
                        Label(deleteTitle, systemImage: "trash")
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

/// Kopf der Detailansicht: Kalender, Titel groß, Tag und Uhrzeit.
private struct EventDetailHeader: View {
    @Environment(AppModel.self) private var model
    let occurrence: Occurrence

    var body: some View {
        let color = model.color(for: occurrence.calendarId)
        let day = LocalDate(occurrence.start, calendar: model.calendar)
        let calendarName: String = model.state.calendars[occurrence.calendarId]?.name ?? "Kalender"
        HStack(alignment: .top, spacing: 12) {
            CalendarBar(color: color, width: 5)
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    CalendarDot(color: color, size: 7)
                    Text(calendarName.uppercased())
                        .font(.caption2.weight(.bold))
                        .tracking(0.6)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    TimelineView(.everyMinute) { context in
                        if occurrence.isRunning(now: context.date) { NowBadge() }
                    }
                }
                Text(occurrence.title)
                    .font(Theme.display(.title2))
                    .strikethrough(occurrence.details.status == .cancelled)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                VStack(alignment: .leading, spacing: 2) {
                    Text(AgendaBuilder.heading(day, today: model.today))
                        .font(.subheadline.weight(.semibold))
                    Text(AgendaBuilder.timeText(occurrence, calendar: model.calendar))
                        .font(Theme.number(.subheadline))
                        .foregroundStyle(.secondary)
                    // Mehrtägig: auch das Ende nennen, sonst fehlt es im Kopf.
                    if let until = untilText {
                        Text(until)
                            .font(Theme.number(.subheadline))
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 6)
        .accessibilityElement(children: .combine)
    }
}

extension EventDetailHeader {
    /// „bis Mo 26. Okt. 10:00“ (ganztägig ohne Uhrzeit), wenn der Termin an
    /// einem späteren Tag endet; sonst `nil`.
    fileprivate var untilText: String? {
        let cal = model.calendar
        let startDay = LocalDate(occurrence.start, calendar: cal)
        // Ganztägig ist das Ende exklusiv (Mitternacht danach).
        let lastMoment: Date = occurrence.isAllDay ? occurrence.end.addingTimeInterval(-1) : occurrence.end
        let endDay = LocalDate(lastMoment, calendar: cal)
        guard endDay > startDay else { return nil }
        let heading = AgendaBuilder.heading(endDay, today: model.today)
        if occurrence.isAllDay { return "bis \(heading)" }
        let c = cal.dateComponents([.hour, .minute], from: occurrence.end)
        return "bis \(heading), " + String(format: "%02d:%02d", c.hour ?? 0, c.minute ?? 0)
    }
}

/// Teilnehmende mit Initialen-Kreis.
private struct AttendeeRow: View {
    let name: String
    let detail: String?

    var body: some View {
        HStack(spacing: 10) {
            Text(initials)
                .font(.caption.weight(.bold))
                .foregroundStyle(Theme.accent)
                .frame(width: 28, height: 28)
                .background(Circle().fill(Theme.accent.opacity(0.14)))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                // Lange Namen und Adressen umbrechen statt abschneiden.
                Text(name)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                if let detail {
                    Text(detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                }
            }
        }
    }

    private var initials: String {
        let parts = name.split(whereSeparator: { $0 == " " || $0 == "." || $0 == "@" })
        let letters = parts.prefix(2).compactMap { $0.first }.map { String($0) }
        return letters.joined().uppercased()
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
