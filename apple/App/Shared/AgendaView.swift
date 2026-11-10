import LichtungKit
import SwiftUI

/// Ansicht A: Termine als Liste; der Wochenstreifen steht darüber im Kopf (`PhoneRootView`).
struct AgendaView: View {
    @Environment(AppModel.self) private var model
    /// Wie viele Tage die Liste ab dem ausgewählten Tag zeigt.
    var days = 60
    /// Wie viele Tage vor dem ausgewählten Tag beim Hochscrollen zu sehen sind.
    /// Die Liste öffnet trotzdem beim ausgewählten Tag.
    var pastDays = 30

    var body: some View {
        let selected = model.selectedDay
        let start = selected.adding(days: -pastDays)
        let count = pastDays + days
        let occurrences = model.occurrences(from: start, days: count)
        let sections = AgendaBuilder.days(
            occurrences, from: start, days: count, today: model.today, calendar: model.calendar, anchor: selected
        )
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
            // Der erste Stand kommt oft erst nach dem Erscheinen; ohne erneuten Sprung
            // blieben die davor eingefügten Tage oben stehen.
            .onChange(of: occurrences.isEmpty) { scrollToSelected(proxy) }
            .onChange(of: model.selectedDay) { scrollToSelected(proxy) }
            .onChange(of: model.todayRequests) { scrollToSelected(proxy) }
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
        return Theme.light.opacity(0.12)
    }
}

/// Kopf eines Tages: große Tageszahl, darunter „Heute · Fr 23.“.
struct AgendaSectionHeader: View {
    let section: AgendaDay

    var body: some View {
        let tint: Color = section.isToday ? Theme.light : Theme.text1
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(verbatim: "\(section.date.day)")
                .font(Theme.digits(.title2, weight: .bold))
                .foregroundStyle(tint)
            Text(section.heading)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Theme.text2)
            Spacer(minLength: 0)
            if !section.items.isEmpty {
                Text(verbatim: "\(section.items.count)")
                    .font(Theme.digits(.caption, weight: .semibold))
                    .foregroundStyle(Theme.text2)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Theme.line))
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
        // Jede Minute neu, damit „Jetzt“ und das Dämpfen vergangener Termine
        // auch bei offener Liste mitgehen.
        TimelineView(.everyMinute) { context in
            row(now: context.date)
        }
    }

    private func row(now: Date) -> some View {
        let occ = item.occurrence
        let color = model.color(for: occ.calendarId)
        let running = occ.isRunning(now: now)
        return HStack(alignment: .center, spacing: 12) {
            timeColumn
                .frame(width: 52, alignment: .trailing)
            CalendarBar(color: color)
                .frame(height: 34)
            VStack(alignment: .leading, spacing: 3) {
                Text(occ.title)
                    .font(.body.weight(.medium))
                    .strikethrough(occ.details.status == .cancelled)
                    .lineLimit(2)
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
        .accessibilityValue(EventBlockView.accessibilityValue(Theme.state(of: occ, now: now)))
    }

    /// Beginn kräftig, Ende gedämpft darunter („09:00“ / „09:30“).
    @ViewBuilder
    private var timeColumn: some View {
        let parts = item.timeText.components(separatedBy: "–")
        if parts.count == 2 {
            VStack(alignment: .trailing, spacing: 1) {
                Text(parts[0])
                    .font(Theme.digits(.subheadline, weight: .semibold))
                Text(parts[1])
                    .font(Theme.digits(.caption))
                    .foregroundStyle(.secondary)
            }
        } else {
            Text(item.timeText)
                .font(Theme.digits(.caption, weight: .semibold))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
                .lineLimit(2)
                .minimumScaleFactor(0.8)
        }
    }
}

/// Wochenstreifen (Spec §5 iPhone): Mo bis So über die ganze Breite, Farbpunkte
/// je Kalender. Wischen blättert wochenweise, ein Tipp macht den Tag zum Fokus-Tag.
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
        // Beginnt an einem Montag, damit jede Seite eine ganze Woche ist.
        let first = Self.monday(min(base.lowerBound, around.lowerBound))
        let range = first..<max(base.upperBound, around.upperBound)
        let shown = window.isEmpty ? DayTicker.window(current: window, visible: selected...selected) : window
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                TickerStrip(range: range, window: shown, today: today)
            }
            .scrollTargetBehavior(.paging)
            .onScrollTargetVisibilityChange(idType: Int.self, threshold: 0.5) { ids in
                guard let lo = ids.min(), let hi = ids.max() else { return }
                visibility.range = lo...hi
                let next = DayTicker.window(current: window, visible: lo...hi)
                if next != window { window = next }
            }
            .frame(height: TickerCell.height + 8)
            .onAppear { reveal(model.selectedDay, proxy: proxy, animated: false) }
            // Wischen im Raster, Monatsansicht, „Heute“: das Band folgt, auch weit weg.
            .onChange(of: model.selectedDay) { _, day in reveal(day, proxy: proxy, animated: true) }
            .onChange(of: model.todayRequests) { reveal(model.selectedDay, proxy: proxy, animated: true) }
        }
    }

    /// Index des Montags der Woche von `index`.
    private static func monday(_ index: Int) -> Int {
        index - (DayTicker.day(at: index).weekday.rawValue - 1)
    }

    /// Blättert zur Woche des Tages, außer er ist schon zu sehen (Tippen auf eine Zelle).
    private func reveal(_ day: LocalDate, proxy: ScrollViewProxy, animated: Bool) {
        let index = DayTicker.index(of: day)
        if let visible = visibility.range, visible.contains(index) { return }
        // Belegung für das Ziel gleich mitrechnen, nicht erst nach der nächsten Messung.
        let next = DayTicker.window(current: window, visible: index...index)
        if next != window { window = next }
        let near = visibility.range.map { abs($0.lowerBound - index) <= 60 || abs($0.upperBound - index) <= 60 } ?? false
        let monday = Self.monday(index)
        if animated && near {
            withAnimation(.snappy) { proxy.scrollTo(monday, anchor: .leading) }
        } else {
            proxy.scrollTo(monday, anchor: .leading)
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
        LazyHStack(spacing: 0) {
            ForEach(range, id: \.self) { index in
                // Außerhalb des Ausschnitts (schnelles Wischen): Tag ohne Punkte, bis er nachgerechnet ist.
                let day = summaries[index] ?? DayTicker.day(DayTicker.day(at: index), today: today, calendar: model.calendar)
                TickerCell(day: day)
                    .containerRelativeFrame(.horizontal, count: 7, spacing: 0)
                    .contentShape(Rectangle())
                    .onTapGesture { model.focus(day.date) }
            }
        }
        .scrollTargetLayout()
        .padding(.vertical, 4)
    }
}

struct TickerCell: View {
    static let height: CGFloat = 62

    @Environment(AppModel.self) private var model
    let day: TickerDay

    var body: some View {
        let isSelected = day.date == model.selectedDay
        // Heute in Licht, am Wochenende Text 2; der gewählte Tag liegt auf einer stillen Fläche.
        let numberColor: Color = day.isToday ? Theme.light : (day.isWeekend ? Theme.text2 : Theme.text1)
        VStack(spacing: 3) {
            Text(day.weekdayShort)
                .font(.caption)
                .foregroundStyle(Theme.text2)
            Text(verbatim: "\(day.dayNumber)")
                .font(Theme.digits(.body, weight: day.isToday || isSelected ? .bold : .regular))
                .foregroundStyle(numberColor)
            HStack(spacing: 2) {
                ForEach(day.calendarIds.prefix(4), id: \.self) { id in
                    CalendarDot(color: model.color(for: id), size: 5)
                }
            }
            .frame(height: 5)
            Capsule()
                .fill(Theme.text2.opacity(0.6))
                .frame(width: 26 * max(0.1, day.busyFraction), height: 2)
                .opacity(day.busyFraction > 0 ? 1 : 0)
        }
        // Feste Zellgröße des Bands; größere Schrift würde abgeschnitten.
        .dynamicTypeSize(...DynamicTypeSize.large)
        .frame(width: 44, height: Self.height)
        .background {
            RoundedRectangle(cornerRadius: Theme.buttonRadius, style: .continuous)
                .fill(isSelected ? Theme.line : Color.clear)
        }
        .frame(maxWidth: .infinity)
        .animation(.snappy(duration: 0.2), value: isSelected)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(day.weekdayShort) \(day.dayNumber)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}
