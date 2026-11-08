// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Ansicht A: Tagesleiste oben, Termine als Liste darunter (Spec §5).
struct AgendaView: View {
    @Environment(AppModel.self) private var model
    /// Wie viele Tage die Liste ab dem ausgewählten Tag zeigt.
    var days = 60
    /// Wie viele Tage vor dem ausgewählten Tag beim Hochscrollen zu sehen sind.
    /// Die Liste öffnet trotzdem beim ausgewählten Tag.
    var pastDays = 30
    var showsTicker = true

    var body: some View {
        let selected = model.selectedDay
        let start = selected.adding(days: -pastDays)
        let count = pastDays + days
        let occurrences = model.occurrences(from: start, days: count)
        let sections = AgendaBuilder.days(
            occurrences, from: start, days: count, today: model.today, calendar: model.calendar, anchor: selected
        )
        VStack(spacing: 0) {
            if showsTicker {
                DayTickerView()
                Divider()
            }
            ScrollViewReader { proxy in
                List {
                    ForEach(sections) { section in
                        Section {
                            if section.items.isEmpty {
                                Label("Nichts geplant", systemImage: "leaf")
                                    .font(.callout)
                                    .foregroundStyle(.tertiary)
                                    .listRowSeparator(.hidden)
                            }
                            ForEach(section.items) { item in
                                AgendaRow(item: item)
                                    .contentShape(Rectangle())
                                    .modifier(OccurrenceTap(occurrence: item.occurrence)) // LIC-24
                                    .contextMenu { OccurrenceMenu(occurrence: item.occurrence) }
                                    .listRowBackground(rowBackground(item.occurrence))
                                    .listRowSeparator(.hidden)
                            }
                        } header: {
                            AgendaSectionHeader(section: section)
                        }
                        .id(section.date)
                    }
                }
                .listStyle(.plain)
                // Oben steht der ausgewählte Tag, vergangene Tage liegen darüber.
                .onAppear { scrollToSelected(proxy) }
                .onChange(of: model.selectedDay) { scrollToSelected(proxy) }
                .onChange(of: model.todayRequests) { scrollToSelected(proxy) }
            }
        }
    }

    /// Springt zum ausgewählten Tag, sobald die Liste mit dem neuen Ausschnitt steht.
    private func scrollToSelected(_ proxy: ScrollViewProxy) {
        let day = model.selectedDay
        DispatchQueue.main.async {
            proxy.scrollTo(day, anchor: .top)
        }
    }

    /// Markierter Termin (Mac, LIC-24) hinterlegt; sonst der Standard der Liste.
    private func rowBackground(_ occ: Occurrence) -> Color? {
        guard model.highlightedOccurrence?.id == occ.id else { return nil }
        return Theme.accent.opacity(0.16)
    }
}

/// Kopf eines Tages: große Tageszahl, darunter „Heute · Fr 23.“.
struct AgendaSectionHeader: View {
    let section: AgendaDay

    var body: some View {
        let tint: Color = section.isToday ? Theme.accent : Color.primary
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(verbatim: "\(section.date.day)")
                .font(Theme.number(.title2, weight: .bold))
                .foregroundStyle(tint)
            Text(section.heading)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(section.isToday ? Theme.accent : Color.secondary)
            Spacer(minLength: 0)
            if !section.items.isEmpty {
                Text(verbatim: "\(section.items.count)")
                    .font(Theme.number(.caption, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Theme.wash))
                    .accessibilityLabel("\(section.items.count) Termine")
            }
        }
        .textCase(nil)
        .padding(.top, 6)
        .padding(.bottom, 2)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

struct AgendaRow: View {
    @Environment(AppModel.self) private var model
    let item: AgendaItem

    var body: some View {
        let occ = item.occurrence
        let color = model.color(for: occ.calendarId)
        let now = Date()
        let running = occ.isRunning(now: now)
        HStack(alignment: .center, spacing: 12) {
            timeColumn
                .frame(width: 52, alignment: .trailing)
            CalendarBar(color: color)
                .frame(height: 34)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(occ.title)
                        .font(.body.weight(.medium))
                        .strikethrough(occ.details.status == .cancelled)
                        .lineLimit(2)
                    if running { NowBadge() }
                }
                if let location = occ.details.location, !location.isEmpty {
                    Label(location, systemImage: "mappin")
                        .labelStyle(.titleAndIcon)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            // Videolink: „Beitreten“ kurz vor und während des Termins (LIC-42).
            JoinButton(occurrence: occ)
            if model.state.isPending(occ.eventId) {
                Image(systemName: "arrow.triangle.2.circlepath")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Noch nicht synchronisiert")
            }
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 8)
        .background {
            // Laufender Termin: zart in Kalenderfarbe hinterlegt.
            if running {
                RoundedRectangle(cornerRadius: 10).fill(color.opacity(0.10))
            }
        }
        // Vergangene Termine treten zurück.
        .opacity(occ.isPast(now: now) ? 0.5 : 1)
        .accessibilityElement(children: .combine)
    }

    /// Beginn kräftig, Ende gedämpft darunter („09:00“ / „09:30“).
    @ViewBuilder
    private var timeColumn: some View {
        let parts = item.timeText.components(separatedBy: "–")
        if parts.count == 2 {
            VStack(alignment: .trailing, spacing: 1) {
                Text(parts[0])
                    .font(Theme.number(.subheadline, weight: .semibold))
                Text(parts[1])
                    .font(Theme.number(.caption))
                    .foregroundStyle(.secondary)
            }
        } else {
            Text(item.timeText)
                .font(Theme.number(.caption, weight: .semibold))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
                .lineLimit(2)
                .minimumScaleFactor(0.8)
        }
    }
}

/// Fantasticals DayTicker: Tage nebeneinander, Farbpunkte je Kalender.
///
/// Virtualisiert über ±100 Jahre (LIC-67): Jede Zelle ist ein Tagesindex
/// (`DayTicker.index(of:)`), gebaut werden nur die sichtbaren. Farbpunkte und
/// Belegung entstehen für einen Ausschnitt um die sichtbaren Zellen
/// (`DayTicker.window`), der beim Wischen nur alle paar Tage neu berechnet wird.
struct DayTickerView: View {
    @Environment(AppModel.self) private var model
    /// Ausschnitt (Indizes) mit Farbpunkten und Belegung; leer bis zur ersten Messung.
    @State private var window: Range<Int> = 0..<0
    /// Sichtbare Zellen. Ein schlichtes Objekt statt `@State`-Wert, damit das
    /// Mitschreiben beim Wischen den Body nicht jedes Mal neu auswertet.
    @State private var visibility = TickerVisibility()

    var body: some View {
        let today = model.today
        let selected = DayTicker.index(of: model.selectedDay)
        // ±100 Jahre um heute und um die Auswahl: Liegt sie (z. B. über den
        // Monat) weiter weg, bleibt auch um sie herum genug Band zum Scrollen.
        let base = DayTicker.indexRange(around: today)
        let around = DayTicker.indexRange(around: model.selectedDay)
        let range = min(base.lowerBound, around.lowerBound)..<max(base.upperBound, around.upperBound)
        let shown = window.isEmpty ? DayTicker.window(current: window, visible: selected...selected) : window
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                TickerStrip(range: range, window: shown, today: today)
            }
            .onScrollTargetVisibilityChange(idType: Int.self, threshold: 0.5) { ids in
                guard let lo = ids.min(), let hi = ids.max() else { return }
                visibility.range = lo...hi
                let next = DayTicker.window(current: window, visible: lo...hi)
                if next != window { window = next }
            }
            .frame(height: TickerCell.height + 16)
            .onAppear { reveal(model.selectedDay, proxy: proxy, animated: false) }
            // Wischen im Raster, Monatsansicht, „Heute“: das Band folgt, auch weit weg.
            .onChange(of: model.selectedDay) { _, day in reveal(day, proxy: proxy, animated: true) }
            .onChange(of: model.todayRequests) { reveal(model.selectedDay, proxy: proxy, animated: true) }
        }
    }

    /// Scrollt den Tag in die Mitte, außer er ist schon ganz zu sehen (Tippen auf eine Zelle).
    private func reveal(_ day: LocalDate, proxy: ScrollViewProxy, animated: Bool) {
        let index = DayTicker.index(of: day)
        if let visible = visibility.range, visible.lowerBound < index, index < visible.upperBound { return }
        // Belegung für das Ziel gleich mitrechnen, nicht erst nach der nächsten Messung.
        let next = DayTicker.window(current: window, visible: index...index)
        if next != window { window = next }
        let near = visibility.range.map { abs($0.lowerBound - index) <= 60 || abs($0.upperBound - index) <= 60 } ?? false
        if animated && near {
            withAnimation(.snappy) { proxy.scrollTo(index, anchor: .center) }
        } else {
            proxy.scrollTo(index, anchor: .center)
        }
    }
}

/// Zuletzt gemessene sichtbare Zellen des Bands (nicht beobachtet).
private final class TickerVisibility {
    var range: ClosedRange<Int>?
}

/// Die Zellen des Bands. Eigene Ansicht, damit ein Wechsel des ausgewählten
/// Tages nur die Zellen neu zeichnet und die Belegung nicht neu rechnet.
private struct TickerStrip: View {
    @Environment(AppModel.self) private var model
    let range: Range<Int>
    let window: Range<Int>
    let today: LocalDate

    var body: some View {
        let start = DayTicker.day(at: window.lowerBound)
        let occurrences = model.occurrences(from: start, days: window.count)
        let summaries = DayTicker.summaries(occurrences, in: window, today: today, calendar: model.calendar)
        LazyHStack(spacing: 4) {
            ForEach(range, id: \.self) { index in
                // Außerhalb des Ausschnitts (schnelles Wischen): Tag ohne Punkte, bis er nachgerechnet ist.
                let day = summaries[index] ?? DayTicker.day(DayTicker.day(at: index), today: today, calendar: model.calendar)
                TickerCell(day: day)
                    .onTapGesture { model.selectedDay = day.date }
            }
        }
        .scrollTargetLayout()
        .padding(.horizontal, 8)
        .padding(.vertical, 8)
    }
}

struct TickerCell: View {
    static let height: CGFloat = 62

    @Environment(AppModel.self) private var model
    let day: TickerDay

    var body: some View {
        let isSelected = day.date == model.selectedDay
        let numberColor: Color = isSelected ? Theme.onAccent : (day.isToday ? Theme.accent : (day.isWeekend ? .secondary : .primary))
        VStack(spacing: 3) {
            Text(day.weekdayShort.uppercased())
                .font(.system(size: 10, weight: .semibold))
                .tracking(0.4)
                .foregroundStyle(isSelected ? Theme.onAccent.opacity(0.85) : (day.isToday ? Theme.accent : Color.secondary))
            Text(verbatim: "\(day.dayNumber)")
                .font(Theme.number(.title3, weight: day.isToday || isSelected ? .bold : .medium))
                .foregroundStyle(numberColor)
            HStack(spacing: 2) {
                ForEach(day.calendarIds.prefix(4), id: \.self) { id in
                    CalendarDot(color: isSelected ? Theme.onAccent : model.color(for: id), size: 5)
                }
            }
            .frame(height: 5)
            Capsule()
                .fill(isSelected ? Theme.onAccent.opacity(0.7) : Theme.accent.opacity(0.45))
                .frame(width: 26 * max(0.1, day.busyFraction), height: 2)
                .opacity(day.busyFraction > 0 ? 1 : 0)
        }
        // Feste Zellgröße des Bands; größere Schrift würde abgeschnitten.
        .dynamicTypeSize(...DynamicTypeSize.large)
        .frame(width: 44, height: Self.height)
        .background {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(isSelected ? Theme.accent : Color.clear)
                .shadow(color: isSelected ? Theme.accent.opacity(0.35) : .clear, radius: 6, y: 3)
        }
        .overlay {
            // Heute, aber nicht ausgewählt: grüner Rahmen.
            if day.isToday && !isSelected {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(Theme.accent.opacity(0.6), lineWidth: 1.5)
            }
        }
        .animation(.snappy(duration: 0.2), value: isSelected)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(day.weekdayShort) \(day.dayNumber)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}
