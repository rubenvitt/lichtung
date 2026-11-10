import LichtungKit
import SwiftUI

/// Trefferliste der Terminsuche (Spec §1 Kriterium 3, LIC-42). iPhone: statt
/// der Ansicht, solange gesucht wird. Der Mac zeigt Treffer im Feld der
/// Toolbar (`CommandField`).
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
        // Vorbei wie in der Liste: Titel in Text 2, Farbkante 40 %; kein Abdunkeln (Kontrast 4,5:1).
        let isPast = occurrence.end <= Date()
        HStack(alignment: .center, spacing: 12) {
            CalendarBar(color: model.color(for: occurrence.calendarId).opacity(isPast ? 0.4 : 1))
                .frame(height: 34)
            VStack(alignment: .leading, spacing: 2) {
                Text(occurrence.title)
                    .font(.body.weight(.medium))
                    .foregroundStyle(isPast ? Theme.text2 : Theme.text1)
                    .strikethrough(occurrence.details.status == .cancelled)
                if let date = hit.dateText(today: model.today, calendar: model.calendar) {
                    Text(date)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(Theme.text2)
                }
                if let location = occurrence.details.location, !location.isEmpty {
                    Text(location)
                        .font(.caption)
                        .foregroundStyle(Theme.text2)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}
