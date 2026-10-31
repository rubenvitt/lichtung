import Foundation

/// Ansicht A: Termine als Liste, Tag für Tag (Spec §5, Entwurf „Tagesleiste + Liste“).
public struct AgendaDay: Hashable, Sendable, Identifiable {
    public var date: LocalDate
    /// „Heute · Fr 23.“, „Morgen · Sa 24.“, „So 25. Okt.“
    public var heading: String
    public var isToday: Bool
    public var items: [AgendaItem]

    public var id: LocalDate { date }
}

public struct AgendaItem: Hashable, Sendable, Identifiable {
    public enum Span: Hashable, Sendable {
        /// Beginnt und endet an diesem Tag.
        case single
        /// Mehrtägig: erster Tag, mittlerer Tag, letzter Tag.
        case first, middle, last
    }

    public var occurrence: Occurrence
    public var span: Span
    /// „09:00–09:30“, „ganztägig“, „ab 22:00“, „bis 06:00“, „14:00“ (ohne Dauer).
    public var timeText: String

    public var id: String { "\(occurrence.id)#\(timeText)" }
}

public enum AgendaBuilder {
    /// Tage ab `start` (`days` Tage), jeder Tag mit seinen Vorkommen.
    /// Leere Tage fehlen, außer heute (für „Keine Termine“).
    public static func days(
        _ occurrences: [Occurrence], from start: LocalDate, days count: Int,
        today: LocalDate, calendar cal: Calendar, includeEmpty: Bool = false
    ) -> [AgendaDay] {
        var result: [AgendaDay] = []
        for offset in 0..<max(0, count) {
            let day = start.adding(days: offset)
            let dayStart = day.startDate(in: cal)
            let dayEnd = day.adding(days: 1).startDate(in: cal)
            var items: [AgendaItem] = []
            for o in occurrences {
                // Termine ohne Dauer zählen an ihrem Beginn.
                let touches = o.start < dayEnd && (o.end > dayStart || (o.end <= o.start && o.start >= dayStart))
                guard touches else { continue }
                items.append(item(o, dayStart: dayStart, dayEnd: dayEnd, cal: cal))
            }
            items.sort { a, b in
                let aAll = a.occurrence.isAllDay || a.span == .middle
                let bAll = b.occurrence.isAllDay || b.span == .middle
                if aAll != bAll { return aAll }
                return CalendarQuery.displayOrder(a.occurrence, b.occurrence)
            }
            guard !items.isEmpty || includeEmpty || day == today else { continue }
            result.append(AgendaDay(date: day, heading: heading(day, today: today), isToday: day == today, items: items))
        }
        return result
    }

    static func item(_ o: Occurrence, dayStart: Date, dayEnd: Date, cal: Calendar) -> AgendaItem {
        let startsHere = o.start >= dayStart
        let endsHere = o.end <= dayEnd
        let span: AgendaItem.Span = switch (startsHere, endsHere) {
        case (true, true): .single
        case (true, false): .first
        case (false, false): .middle
        case (false, true): .last
        }
        let text: String
        if o.isAllDay {
            text = "ganztägig"
        } else {
            switch span {
            case .single: text = o.end > o.start ? "\(clock(o.start, cal))–\(clock(o.end, cal))" : clock(o.start, cal)
            case .first: text = "ab \(clock(o.start, cal))"
            case .middle: text = "ganztägig"
            case .last: text = "bis \(clock(o.end, cal))"
            }
        }
        return AgendaItem(occurrence: o, span: o.isAllDay ? .single : span, timeText: text)
    }

    static func clock(_ d: Date, _ cal: Calendar) -> String {
        let c = cal.dateComponents([.hour, .minute], from: d)
        return String(format: "%02d:%02d", c.hour!, c.minute!)
    }

    /// „Heute · Fr 23.“, „Morgen · Sa 24.“, „Gestern · Do 22.“, sonst „So 25. Okt.“
    public static func heading(_ day: LocalDate, today: LocalDate) -> String {
        let wd = QuickEntryPreview.weekdayShort[day.weekday.rawValue - 1]
        let short = "\(wd) \(day.day)."
        switch today.days(until: day) {
        case 0: return "Heute · \(short)"
        case 1: return "Morgen · \(short)"
        case -1: return "Gestern · \(short)"
        default:
            let month = QuickEntryPreview.monthShort[day.month - 1]
            let year = day.year == today.year ? "" : " \(day.year)"
            return "\(short) \(month)\(year)"
        }
    }
}

/// Die Tagesleiste über der Liste (Fantasticals DayTicker).
public struct TickerDay: Hashable, Sendable, Identifiable {
    public var date: LocalDate
    public var weekdayShort: String
    public var dayNumber: Int
    public var isToday: Bool
    public var isWeekend: Bool
    /// Kalender mit Terminen an diesem Tag, häufigste zuerst (höchstens 4), für die Farbpunkte.
    public var calendarIds: [String]
    /// Anteil des Tages (8–20 Uhr), der belegt ist, 0…1.
    public var busyFraction: Double

    public var id: LocalDate { date }
}

public enum DayTicker {
    /// Feste Folge ab `start` (`count` Tage). Das Band selbst nutzt `summaries(_:in:today:calendar:)`
    /// für den gerade sichtbaren Ausschnitt (LIC-67).
    public static func days(
        _ occurrences: [Occurrence], from start: LocalDate, count: Int, today: LocalDate, calendar cal: Calendar
    ) -> [TickerDay] {
        let first = index(of: start)
        let byIndex = summaries(occurrences, in: first..<(first + max(0, count)), today: today, calendar: cal)
        return (0..<max(0, count)).map { byIndex[first + $0]! }
    }

    /// Ein Tag des Bands aus den Vorkommen, die ihn berühren (`touching` darf mehr enthalten,
    /// gefiltert wird hier). Ohne Vorkommen: leerer Tag, z. B. solange der Ausschnitt nachlädt.
    public static func day(
        _ day: LocalDate, touching occurrences: [Occurrence] = [], today: LocalDate, calendar cal: Calendar
    ) -> TickerDay {
        let wd = day.weekday
        var result = TickerDay(
            date: day, weekdayShort: QuickEntryPreview.weekdayShort[wd.rawValue - 1], dayNumber: day.day,
            isToday: day == today, isWeekend: wd == .saturday || wd == .sunday,
            calendarIds: [], busyFraction: 0
        )
        guard !occurrences.isEmpty else { return result }
        let dayStart = day.startDate(in: cal)
        let dayEnd = day.adding(days: 1).startDate(in: cal)
        let touching = occurrences.filter { $0.start < dayEnd && $0.end > dayStart }
        var counts: [String: Int] = [:]
        for o in touching { counts[o.calendarId, default: 0] += 1 }
        result.calendarIds = counts.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }.prefix(4).map(\.key)
        // Belegung im Kernbereich 8–20 Uhr, Überschneidungen nur einmal gezählt.
        let core = DateInterval(start: day.date(hour: 8, minute: 0, in: cal), end: day.date(hour: 20, minute: 0, in: cal))
        let busy = mergedLength(touching.filter { !$0.isAllDay && $0.details.availability == .busy }
            .compactMap { DateInterval(start: $0.start, end: max($0.start, $0.end)).intersection(with: core) })
        result.busyFraction = min(1, busy / core.duration)
        return result
    }

    static func mergedLength(_ intervals: [DateInterval]) -> TimeInterval {
        var total: TimeInterval = 0
        var current: DateInterval?
        for i in intervals.sorted(by: { $0.start < $1.start }) {
            if let c = current, i.start <= c.end {
                current = DateInterval(start: c.start, end: max(c.end, i.end))
            } else {
                if let c = current { total += c.duration }
                current = i
            }
        }
        return total + (current?.duration ?? 0)
    }
}
