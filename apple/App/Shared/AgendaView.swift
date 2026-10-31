// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Ansicht A: Tagesleiste oben, Termine als Liste darunter (Spec §5).
struct AgendaView: View {
    @Environment(AppModel.self) private var model
    /// Wie viele Tage die Liste ab dem ausgewählten Tag zeigt.
    var days = 60
    var showsTicker = true

    var body: some View {
        let start = model.selectedDay
        let occurrences = model.occurrences(from: start, days: days)
        let sections = AgendaBuilder.days(occurrences, from: start, days: days, today: model.today, calendar: model.calendar)
        VStack(spacing: 0) {
            if showsTicker {
                DayTickerView()
                Divider()
            }
            List {
                ForEach(sections) { section in
                    Section {
                        if section.items.isEmpty {
                            Text("Keine Termine").foregroundStyle(.secondary)
                        }
                        ForEach(section.items) { item in
                            AgendaRow(item: item)
                                .contentShape(Rectangle())
                                .modifier(OccurrenceTap(occurrence: item.occurrence)) // LIC-24
                                .contextMenu { OccurrenceMenu(occurrence: item.occurrence) }
                                .listRowBackground(rowBackground(item.occurrence))
                        }
                    } header: {
                        Text(section.heading)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(section.isToday ? Color.accentColor : Color.primary)
                    }
                }
            }
            .listStyle(.plain)
        }
    }

    /// Markierter Termin (Mac, LIC-24) hinterlegt; sonst der Standard der Liste.
    private func rowBackground(_ occ: Occurrence) -> Color? {
        guard model.highlightedOccurrence?.id == occ.id else { return nil }
        return Color.accentColor.opacity(0.18)
    }
}

struct AgendaRow: View {
    @Environment(AppModel.self) private var model
    let item: AgendaItem

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            CalendarDot(color: model.color(for: item.occurrence.calendarId))
            Text(item.timeText)
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: 96, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.occurrence.title)
                    .strikethrough(item.occurrence.details.status == .cancelled)
                if let location = item.occurrence.details.location, !location.isEmpty {
                    Text(location).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
            // Videolink: „Beitreten“ kurz vor und während des Termins (LIC-42).
            JoinButton(occurrence: item.occurrence)
            if model.state.isPending(item.occurrence.eventId) {
                Image(systemName: "arrow.triangle.2.circlepath")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Noch nicht synchronisiert")
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
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
        // ±100 Jahre um heute; liegt die Auswahl (z. B. über den Monat) weiter
        // weg, wächst der Bereich bis zu ihr, damit das Band sie zeigen kann.
        let base = DayTicker.indexRange(around: today)
        let range = min(base.lowerBound, selected)..<max(base.upperBound, selected + 1)
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
            .frame(height: TickerCell.height + 12)
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
        .padding(.vertical, 6)
    }
}

struct TickerCell: View {
    static let height: CGFloat = 58

    @Environment(AppModel.self) private var model
    let day: TickerDay

    var body: some View {
        let isSelected = day.date == model.selectedDay
        VStack(spacing: 2) {
            Text(day.weekdayShort).font(.caption2)
            Text("\(day.dayNumber)").font(.callout.weight(day.isToday ? .bold : .regular))
            HStack(spacing: 2) {
                ForEach(day.calendarIds, id: \.self) { id in
                    CalendarDot(color: model.color(for: id), size: 4)
                }
            }
            .frame(height: 4)
            Capsule()
                .fill(Color.secondary.opacity(0.4))
                .frame(width: 28 * max(0.08, day.busyFraction), height: 2)
                .opacity(day.busyFraction > 0 ? 1 : 0)
        }
        // Feste Zellgröße des Bands; größere Schrift würde abgeschnitten.
        .dynamicTypeSize(...DynamicTypeSize.large)
        .frame(width: 40, height: Self.height)
        .foregroundStyle(isSelected ? Color.white : (day.isWeekend ? Color.secondary : Color.primary))
        .background(RoundedRectangle(cornerRadius: 8).fill(isSelected ? Color.accentColor : Color.clear))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(day.weekdayShort) \(day.dayNumber)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}
