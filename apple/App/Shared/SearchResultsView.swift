// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Trefferliste der Terminsuche (Spec §1 Kriterium 3, LIC-42). iPhone: statt
/// der Ansicht, solange gesucht wird. Mac: in der Seitenleiste statt der
/// nächsten Termine.
struct SearchResultsView: View {
    @Environment(AppModel.self) private var model
    let text: String
    /// Ein Treffer wurde gewählt (Sprung zum Tag, Detail öffnen).
    let onSelect: @MainActor (Occurrence) -> Void

    var body: some View {
        let hits = model.search(text)
        if hits.isEmpty {
            ContentUnavailableView(
                "Keine Treffer",
                systemImage: "magnifyingglass",
                description: Text("Nichts gefunden für „\(text)“. Gesucht wird in Titel, Ort, Notiz und Teilnehmenden.")
            )
        } else {
            List(hits) { hit in
                if let occurrence = hit.occurrence {
                    Button {
                        onSelect(occurrence)
                    } label: {
                        SearchResultRow(hit: hit, occurrence: occurrence)
                    }
                    .buttonStyle(.plain)
                }
            }
            .listStyle(.plain)
        }
    }
}

struct SearchResultRow: View {
    @Environment(AppModel.self) private var model
    let hit: EventSearch.Hit
    let occurrence: Occurrence

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            CalendarDot(color: model.color(for: occurrence.calendarId))
            VStack(alignment: .leading, spacing: 2) {
                Text(occurrence.title)
                    .strikethrough(occurrence.details.status == .cancelled)
                if let date = hit.dateText(today: model.today, calendar: model.calendar) {
                    Text(date)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                if let location = occurrence.details.location, !location.isEmpty {
                    Text(location)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 2)
        // Vergangene Treffer treten zurück.
        .opacity(occurrence.end <= Date() ? 0.6 : 1)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}
