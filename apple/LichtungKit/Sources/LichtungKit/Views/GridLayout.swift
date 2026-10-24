import Foundation

/// Ein Termin im Zeitraster (Ansicht B/C): Lage im Tag und Spalte bei
/// Überschneidungen.
public struct TimelineBlock: Hashable, Sendable, Identifiable {
    public var occurrence: Occurrence
    /// Oberkante als Anteil des Tages, 0…1. Der Tag hat an Umstellungstagen
    /// 23 oder 25 Stunden; die Anteile beziehen sich auf die echte Länge.
    public var top: Double
    public var height: Double
    /// Spalte in der Gruppe sich überschneidender Termine (0-basiert).
    public var column: Int
    /// Spalten der Gruppe.
    public var columns: Int
    /// Über wie viele Spalten der Block reicht, wenn rechts daneben frei ist.
    public var span: Int
    /// Beginnt vor diesem Tag bzw. endet danach (für abgeschnittene Kanten).
    public var continuesBefore: Bool
    public var continuesAfter: Bool

    public var id: String { occurrence.id }

    /// Linke Kante und Breite als Anteil der Tagesspalte.
    public var leading: Double { Double(column) / Double(columns) }
    public var width: Double { Double(span) / Double(columns) }
}

public enum TimelineLayout {
    /// Zeitgebundene Vorkommen an `day`, nebeneinander gelegt, wo sie sich
    /// überschneiden. `minimumMinutes` ist die kleinste dargestellte Höhe;
    /// kurze Termine überlappen damit auch optisch nicht.
    public static func blocks(_ occurrences: [Occurrence], day: LocalDate, calendar cal: Calendar, minimumMinutes: Double = 15) -> [TimelineBlock] {
        let dayStart = day.startDate(in: cal)
        let dayEnd = day.adding(days: 1).startDate(in: cal)
        let length = dayEnd.timeIntervalSince(dayStart)
        let minimum = minimumMinutes * 60

        struct Item { var o: Occurrence; var s: Date; var e: Date; var shownEnd: Date }
        let items = occurrences
            .filter { !$0.isAllDay && $0.start < dayEnd && ($0.end > dayStart || ($0.end <= $0.start && $0.start >= dayStart)) }
            .map { o -> Item in
                let s = max(o.start, dayStart)
                let e = min(max(o.end, o.start), dayEnd)
                return Item(o: o, s: s, e: e, shownEnd: min(dayEnd, max(e, s.addingTimeInterval(minimum))))
            }
            .sorted { a, b in
                if a.s != b.s { return a.s < b.s }
                if a.shownEnd != b.shownEnd { return a.shownEnd > b.shownEnd }
                return CalendarQuery.displayOrder(a.o, b.o)
            }

        var result: [TimelineBlock] = []
        var cluster: [(Item, Int)] = []
        var clusterEnd = Date.distantPast

        func flush() {
            guard !cluster.isEmpty else { return }
            let columns = (cluster.map(\.1).max() ?? 0) + 1
            for (item, col) in cluster {
                // Nach rechts ausdehnen, solange keine Spalte daneben belegt ist.
                var span = 1
                while col + span < columns, !cluster.contains(where: { other in
                    other.1 == col + span && other.0.s < item.shownEnd && item.s < other.0.shownEnd
                }) { span += 1 }
                result.append(TimelineBlock(
                    occurrence: item.o,
                    top: item.s.timeIntervalSince(dayStart) / length,
                    height: item.shownEnd.timeIntervalSince(item.s) / length,
                    column: col, columns: columns, span: span,
                    continuesBefore: item.o.start < dayStart, continuesAfter: item.o.end > dayEnd
                ))
            }
            cluster = []
        }

        for item in items {
            if item.s >= clusterEnd { flush() }
            // Erste Spalte, deren letzter Termin vorbei ist.
            var col = 0
            while cluster.contains(where: { $0.1 == col && $0.0.shownEnd > item.s }) { col += 1 }
            cluster.append((item, col))
            clusterEnd = max(clusterEnd, item.shownEnd)
        }
        flush()
        return result
    }

    /// Höhe der aktuellen Uhrzeit-Linie als Anteil des Tages, `nil` an anderen Tagen.
    public static func nowLine(_ now: Date, day: LocalDate, calendar cal: Calendar) -> Double? {
        let s = day.startDate(in: cal), e = day.adding(days: 1).startDate(in: cal)
        guard now >= s, now < e else { return nil }
        return now.timeIntervalSince(s) / e.timeIntervalSince(s)
    }
}

/// Ein Balken über mehrere Tagesspalten: ganztägige Zeile im Wochenraster,
/// Termine im Monatsraster.
public struct LaneBar: Hashable, Sendable, Identifiable {
    public var occurrence: Occurrence
    /// Erste und letzte Spalte (inklusive) im sichtbaren Bereich.
    public var firstColumn: Int
    public var lastColumn: Int
    public var lane: Int
    public var continuesBefore: Bool
    public var continuesAfter: Bool

    public var id: String { "\(occurrence.id)@\(firstColumn)" }
}

public enum LaneLayout {
    /// Legt Vorkommen über die Tage `days` in Bahnen, längere zuerst.
    /// Ein Vorkommen belegt jeden Tag, den es berührt (Ende exklusiv).
    public static func bars(_ occurrences: [Occurrence], days: [LocalDate], calendar cal: Calendar) -> [LaneBar] {
        guard let first = days.first, let last = days.last else { return [] }
        let rangeStart = first.startDate(in: cal)
        let rangeEnd = last.adding(days: 1).startDate(in: cal)

        struct Item { var o: Occurrence; var a: Int; var b: Int }
        let items: [Item] = occurrences.compactMap { o in
            let end = o.end > o.start ? o.end : o.start.addingTimeInterval(1)
            guard o.start < rangeEnd, end > rangeStart else { return nil }
            let a = max(0, first.days(until: LocalDate(max(o.start, rangeStart), calendar: cal)))
            let lastInstant = min(end, rangeEnd).addingTimeInterval(-1)
            let b = min(days.count - 1, first.days(until: LocalDate(lastInstant, calendar: cal)))
            return Item(o: o, a: a, b: max(a, b))
        }
        .sorted { x, y in
            if x.a != y.a { return x.a < y.a }
            if (x.b - x.a) != (y.b - y.a) { return (x.b - x.a) > (y.b - y.a) }
            return CalendarQuery.displayOrder(x.o, y.o)
        }

        var laneEnds: [Int] = []
        var result: [LaneBar] = []
        for item in items {
            let lane = laneEnds.firstIndex { $0 < item.a } ?? laneEnds.count
            if lane == laneEnds.count { laneEnds.append(item.b) } else { laneEnds[lane] = item.b }
            result.append(LaneBar(
                occurrence: item.o, firstColumn: item.a, lastColumn: item.b, lane: lane,
                continuesBefore: item.o.start < rangeStart, continuesAfter: item.o.end > rangeEnd
            ))
        }
        return result
    }

    /// Wie viele Balken je Spalte nicht mehr angezeigt werden, wenn nur
    /// `visibleLanes` Bahnen Platz haben (für „+2“ im Monatsraster).
    public static func overflow(_ bars: [LaneBar], columns: Int, visibleLanes: Int) -> [Int] {
        var hidden = Array(repeating: 0, count: columns)
        for bar in bars where bar.lane >= visibleLanes {
            for c in bar.firstColumn...bar.lastColumn where c < columns { hidden[c] += 1 }
        }
        return hidden
    }
}

/// Ansicht D (Monat) und Jahr.
public enum MonthGrid {
    /// Wochen des Monats als Zeilen zu je 7 Tagen, Montag zuerst; Tage aus
    /// Vor- und Folgemonat füllen die erste und letzte Woche. `fixedRows`
    /// liefert immer 6 Zeilen (ruhiges Layout beim Blättern).
    public static func weeks(year: Int, month: Int, fixedRows: Bool = false) -> [[LocalDate]] {
        let first = LocalDate(year: year, month: month, day: 1)
        let start = first.adding(days: -(first.weekday.rawValue - 1))
        let last = LocalDate(year: year, month: month, day: LocalDate.daysInMonth(year: year, month: month))
        var rows = (start.days(until: last) / 7) + 1
        if fixedRows { rows = 6 }
        return (0..<rows).map { w in (0..<7).map { start.adding(days: w * 7 + $0) } }
    }

    /// Termine je Tag (für die Dichte im Jahr und Punkte im Mini-Monat).
    public static func counts(_ occurrences: [Occurrence], days: [LocalDate], calendar cal: Calendar) -> [LocalDate: Int] {
        var result: [LocalDate: Int] = [:]
        for bar in LaneLayout.bars(occurrences, days: days, calendar: cal) {
            for c in bar.firstColumn...bar.lastColumn { result[days[c], default: 0] += 1 }
        }
        return result
    }

    /// „Oktober 2026“
    public static func title(year: Int, month: Int) -> String {
        let names = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"]
        return "\(names[month - 1]) \(year)"
    }

    /// ISO-Kalenderwoche des Tages (für die KW-Spalte).
    public static func isoWeek(_ d: LocalDate) -> Int {
        // Donnerstag der Woche bestimmt das Jahr.
        let thursday = d.adding(days: 4 - d.weekday.rawValue)
        let jan1 = LocalDate(year: thursday.year, month: 1, day: 1)
        return jan1.days(until: thursday) / 7 + 1
    }
}
