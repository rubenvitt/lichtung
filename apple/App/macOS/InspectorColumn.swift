import LichtungKit
import SwiftUI

/// Rechte Spalte im Mac-Fenster (Spec §5), etwa 240 pt, ohne Kartenrahmen:
/// oben der markierte Termin (sonst der nächste von heute), darunter
/// „Heute noch“ und, nur bei Bedarf, „Offene Einladungen“.
struct InspectorColumn: View {
    @Environment(AppModel.self) private var model

    static let width: CGFloat = 240

    var body: some View {
        TimelineView(.everyMinute) { context in
            content(now: context.date)
        }
        .frame(width: Self.width)
        .background(Theme.shade)
    }

    private func content(now: Date) -> some View {
        let rest = model.occurrences(from: model.today, days: 1)
            // Abgesagte zählen nicht (wie `AgendaBuilder.nextSentence`).
            .filter { !DayGridView.isAllDayRow($0) && $0.details.status != .cancelled && $0.end > now }
            .sorted { $0.start < $1.start }
        let shown = model.highlightedOccurrence ?? rest.first
        let invitations = model.invitations.items
        return ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if let shown {
                    InspectorEvent(occurrence: shown, now: now)
                        .padding(.bottom, 28)
                }
                heading("Heute noch")
                if let free = rest.map(\.end).max() {
                    ForEach(rest) { occ in
                        Button { model.highlight(occ) } label: {
                            InspectorRow(time: DayGridView.clockText(occ.start, calendar: model.calendar), title: occ.title,
                                         color: model.color(for: occ.calendarId))
                        }
                        .buttonStyle(.plain)
                    }
                    InspectorRow(time: DayGridView.clockText(free, calendar: model.calendar), title: "danach frei", color: nil)
                } else {
                    Text("Heute nichts mehr")
                        .foregroundStyle(Theme.text2)
                }
                if !invitations.isEmpty {
                    heading("Offene Einladungen")
                        .padding(.top, 28)
                    ForEach(invitations) { invitation in
                        let occ = invitation.occurrence
                        let day = LocalDate(occ.start, calendar: model.calendar)
                        Button { model.invitationsListRequests += 1 } label: {
                            InspectorRow(time: "\(QuickEntryPreview.weekdayShort[day.weekday.rawValue - 1]) \(day.day).",
                                         title: occ.title, color: model.color(for: occ.calendarId))
                        }
                        .buttonStyle(.plain)
                        .accessibilityHint("Öffnet die Terminanfragen")
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 14)
            .padding(.leading, 22)
            .padding(.trailing, 20)
            .padding(.bottom, 20)
        }
        .scrollIndicators(.never)
        .modifier(NoTopScrollEdgeEffect())
    }

    private func heading(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(Theme.text1)
            .padding(.bottom, 8)
            .accessibilityAddTraits(.isHeader)
    }
}

/// Ein Termin oben in der Spalte: Titel, Zeit als Satz, Ort, Kalender,
/// „Steht auch in …“, Beitreten und Zu-/Absage.
private struct InspectorEvent: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    let occurrence: Occurrence
    let now: Date

    var body: some View {
        let occ = occurrence
        let calendar = model.state.calendars[occ.calendarId]
        // Rechnet den Tag neu (`merged`), deshalb nur für diesen einen Termin.
        let alsoIn = model.alsoIn(occ)
        VStack(alignment: .leading, spacing: 4) {
            Text(occ.title)
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(Theme.text1)
                .strikethrough(occ.details.status == .cancelled)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 2)
            Group {
                Text(AgendaBuilder.sentence(occ, today: model.today, calendar: model.calendar))
                    .monospacedDigit()
                if let location = occ.details.location, !location.isEmpty {
                    Text(location)
                }
                if occ.isRunning(now: now) {
                    Text("läuft gerade")
                }
                if let calendar {
                    Text("\(Text("●").foregroundStyle(model.color(for: calendar.id))) \(calendar.name)")
                }
                if !alsoIn.isEmpty {
                    Text("Steht auch in \(Self.calendarList(alsoIn, model: model))")
                }
            }
            .foregroundStyle(Theme.text2)
            .fixedSize(horizontal: false, vertical: true)
            if let link = VideoLinkDetector.detect(in: occ.details), !occ.isPast(now: now) {
                Button { JoinButton.join(link, with: openURL) } label: {
                    Text("Beitreten").frame(maxWidth: .infinity)
                }
                .buttonStyle(Theme.PrimaryButtonStyle())
                .help("\(link.provider.displayName) beitreten")
                .accessibilityLabel("\(link.provider.displayName) beitreten")
                .padding(.top, 10)
            }
            if InvitationActions.isInvitation(occ), model.canRespond(occ) {
                let status = model.responseStatus(occ) ?? occ.details.selfStatus
                InvitationDetailButtons(occurrence: occ, current: InvitationResponse(status))
                    .padding(.top, 10)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// „● Arbeit, ● DRK“ mit Farbpunkten.
    static func calendarList(_ calendars: [CalendarInfo], model: AppModel) -> Text {
        calendars.enumerated().reduce(Text(verbatim: "")) { text, item in
            let dot = Text("●").foregroundStyle(model.color(for: item.element.id))
            return Text("\(text)\(item.offset > 0 ? ", " : "")\(dot) \(item.element.name)")
        }
    }
}

/// Zeile mit Uhrzeit (oder Tag) und Titel an einer Farbkante, wie im Entwurf.
private struct InspectorRow: View {
    let time: String
    let title: String
    /// `nil`: ohne Kante (Zeile „danach frei“).
    let color: Color?

    var body: some View {
        HStack(spacing: 10) {
            Text(time)
                .font(Theme.digits(.body))
                .foregroundStyle(Theme.text2)
                .frame(width: 44, alignment: .leading)
            Text(title)
                .foregroundStyle(color == nil ? Theme.text2 : Theme.text1)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.leading, 8)
                .overlay(alignment: .leading) {
                    if let color {
                        Rectangle().fill(color).frame(width: 3)
                    }
                }
        }
        .padding(.vertical, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// Ohne den Schleier, den macOS 26 oben über einen ScrollView unter der
/// Toolbar legt: Sonst wirkt die Toolbar über der Spalte anders getönt als
/// über dem Raster, dessen Kopf nicht scrollt.
private struct NoTopScrollEdgeEffect: ViewModifier {
    func body(content: Content) -> some View {
        // Die CI baut mit dem SDK von macOS 15; dort gibt es den Aufruf nicht.
        #if compiler(>=6.2)
        if #available(macOS 26, *) {
            content.scrollEdgeEffectHidden(true, for: .top)
        } else {
            content
        }
        #else
        content
        #endif
    }
}
