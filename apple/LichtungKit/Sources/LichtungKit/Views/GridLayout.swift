import Foundation

/// Senkrechte Achse des Zeitrasters (LIC-41): Wanduhr, für jeden Tag gleich.
///
/// Das Raster zeichnet immer 24 Stundenzeilen 00–23. Ein Zeitpunkt liegt
/// dort, wo seine Wanduhrzeit steht: Minuten seit Mitternacht laut Uhr,
/// 0…1440. Ein Termin um 12:00 steht damit auch an Umstellungstagen auf der
/// Linie „12“, obwohl seit Mitternacht 11 oder 13 echte Stunden vergangen sind.
///
/// Umstellungstage:
/// - **Vorstellen (Frühjahr, 28.3.2027 in Berlin):** 02:00–03:00 gibt es
///   nicht. Das Band bleibt leer, kein Zeitpunkt liegt darin. Ein Termin
///   01:30–03:30 (eine echte Stunde) reicht optisch über zwei Stunden, so wie
///   die Uhr es anzeigt. Eine Lage im Band ergibt beim Zurückrechnen den
///   ersten gültigen Zeitpunkt danach (03:00 Sommerzeit).
/// - **Zurückstellen (Herbst, 25.10.2026 in Berlin):** 02:00–03:00 läuft
///   zweimal ab. Beide Durchgänge teilen sich das eine Band 02–03: Die Achse
///   läuft in diesen zwei echten Stunden halb so schnell (02:00 Sommerzeit
///   liegt auf „02“, 02:00 Winterzeit auf „02:30“, 03:00 Winterzeit auf „03“).
///   So bleibt die Achse streng monoton, Blöcke bekommen keine negative Höhe,
///   und jede Lage im Band gehört zu genau einem Zeitpunkt (wichtig fürs
///   Ziehen, `GridInteraction`). Außerhalb dieser zwei Stunden stimmt die
///   Lage exakt mit der Wanduhr überein.
///
/// Andere Zonen und Umstellungszeiten (etwa um Mitternacht) folgen denselben
/// Regeln; die Übergänge werden aus `secondsFromGMT` bestimmt.
public struct WallClockAxis: Sendable {
    public static let minutesPerDay = 1440.0

    public let day: LocalDate
    public let calendar: Calendar
    public let dayStart: Date
    public let dayEnd: Date

    /// Mitternacht des Tages in Sekunden seit 1970, so gerechnet, als liefe
    /// die Wanduhr in UTC. Wanduhr eines Zeitpunkts = `t + Abstand(t) − wallMidnight`.
    private let wallMidnight: TimeInterval
    private let transitions: [Transition]

    /// Wechsel des Abstands zu UTC bei `at` von `before` auf `after` Sekunden.
    private struct Transition: Sendable {
        var at: Date
        var before: Int
        var after: Int
        /// Länge der doppelten (> 0) bzw. fehlenden (< 0) Zeit in Sekunden.
        var repeated: TimeInterval { TimeInterval(before - after) }
    }

    public init(day: LocalDate, calendar: Calendar) {
        self.day = day
        self.calendar = calendar
        dayStart = day.startDate(in: calendar)
        dayEnd = day.adding(days: 1).startDate(in: calendar)
        wallMidnight = day.startDate(in: .gregorianUTC).timeIntervalSince1970
        transitions = Self.transitions(
            in: calendar.timeZone,
            from: dayStart.addingTimeInterval(-6 * 3600), to: dayEnd.addingTimeInterval(6 * 3600)
        )
    }

    /// Lage von `instant` in Minuten auf der Achse, begrenzt auf 0…1440.
    public func minutes(at instant: Date) -> Double {
        if instant <= dayStart { return 0 }
        if instant >= dayEnd { return Self.minutesPerDay }
        return min(Self.minutesPerDay, max(0, wallSeconds(at: instant) / 60))
    }

    /// Lage als Anteil der Spaltenhöhe, 0…1.
    public func fraction(at instant: Date) -> Double {
        minutes(at: instant) / Self.minutesPerDay
    }

    /// Ob es die Wanduhrzeit `m` (Minuten seit Mitternacht) an diesem Tag
    /// gibt: nicht in der Lücke beim Vorstellen.
    public func exists(minutes m: Double) -> Bool {
        guard m >= 0, m <= Self.minutesPerDay else { return false }
        let w = m * 60
        for t in transitions where t.repeated < 0 {
            let gapStart = wall(t.at, offset: t.before)
            if w >= gapStart && w < gapStart - t.repeated { return false }
        }
        return w >= wallSeconds(at: dayStart)
    }

    /// Zeitpunkt zur Lage `m` in Minuten (Umkehrung von `minutes(at:)`). In
    /// der Lücke beim Vorstellen der erste gültige Zeitpunkt danach.
    public func instant(atMinutes m: Double) -> Date {
        if m >= Self.minutesPerDay { return dayEnd }
        let w = max(0, m) * 60
        if w <= wallSeconds(at: dayStart) { return dayStart }
        for t in transitions {
            let atWall = wall(t.at, offset: t.before)
            if t.repeated > 0 {
                // Doppelte Zeit: halbe Geschwindigkeit über [at − g, at + g].
                let bandStart = atWall - t.repeated
                if w >= bandStart && w <= atWall {
                    return t.at.addingTimeInterval(-t.repeated + 2 * (w - bandStart))
                }
            } else if t.repeated < 0, w >= atWall, w < atWall - t.repeated {
                return t.at
            }
        }
        let tz = calendar.timeZone
        var offsets = [tz.secondsFromGMT(for: dayStart), tz.secondsFromGMT(for: dayEnd.addingTimeInterval(-1))]
        offsets += transitions.flatMap { [$0.before, $0.after] }
        for offset in offsets {
            let t = Date(timeIntervalSince1970: wallMidnight + w - TimeInterval(offset))
            if tz.secondsFromGMT(for: t) == offset { return t }
        }
        return dayStart.addingTimeInterval(w)
    }

    private func wall(_ t: Date, offset: Int) -> TimeInterval {
        t.timeIntervalSince1970 + TimeInterval(offset) - wallMidnight
    }

    /// Lage in Sekunden ab Mitternacht, mit gestauchter doppelter Stunde.
    private func wallSeconds(at t: Date) -> TimeInterval {
        for tr in transitions where tr.repeated > 0 {
            let g = tr.repeated
            let bandStart = tr.at.addingTimeInterval(-g)
            if t >= bandStart && t <= tr.at.addingTimeInterval(g) {
                return wall(tr.at, offset: tr.before) - g + t.timeIntervalSince(bandStart) / 2
            }
        }
        return wall(t, offset: calendar.timeZone.secondsFromGMT(for: t))
    }

    /// Übergänge zwischen `from` und `to`: stündlich abtasten, dann auf die
    /// Sekunde eingrenzen. So unabhängig von `nextDaylightSavingTimeTransition`.
    private static func transitions(in tz: TimeZone, from: Date, to: Date) -> [Transition] {
        var result: [Transition] = []
        var a = from
        var offsetA = tz.secondsFromGMT(for: a)
        while a < to {
            let b = min(to, a.addingTimeInterval(3600))
            let offsetB = tz.secondsFromGMT(for: b)
            if offsetB != offsetA {
                var lo = a, hi = b
                while hi.timeIntervalSince(lo) > 0.25 {
                    let mid = lo.addingTimeInterval(hi.timeIntervalSince(lo) / 2)
                    if tz.secondsFromGMT(for: mid) == offsetA { lo = mid } else { hi = mid }
                }
                let at = Date(timeIntervalSince1970: hi.timeIntervalSince1970.rounded())
                result.append(Transition(at: at, before: offsetA, after: offsetB))
            }
            a = b
            offsetA = offsetB
        }
        return result
    }
}

/// Ein Termin im Zeitraster (Ansicht B/C): Lage im Tag und Spalte bei
/// Überschneidungen.
public struct TimelineBlock: Hashable, Sendable, Identifiable {
    public var occurrence: Occurrence
    /// Oberkante als Anteil der Spalte, 0…1, auf der Wanduhr-Achse
    /// (`WallClockAxis`): Minuten seit Mitternacht laut Uhr / 1440, auch an
    /// Umstellungstagen.
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
    /// überschneiden. `minimumMinutes` ist die kleinste dargestellte Höhe in
    /// Minuten der Achse; kurze Termine überlappen damit auch optisch nicht.
    /// Lage und Überschneidung gelten auf der Wanduhr-Achse (`WallClockAxis`).
    public static func blocks(_ occurrences: [Occurrence], day: LocalDate, calendar cal: Calendar, minimumMinutes: Double = 15) -> [TimelineBlock] {
        let axis = WallClockAxis(day: day, calendar: cal)
        let dayStart = axis.dayStart
        let dayEnd = axis.dayEnd
        let full = WallClockAxis.minutesPerDay

        // `s` und `shownEnd` in Minuten der Achse.
        struct Item { var o: Occurrence; var s: Double; var shownEnd: Double }
        let items = occurrences
            .filter { !$0.isAllDay && $0.start < dayEnd && ($0.end > dayStart || ($0.end <= $0.start && $0.start >= dayStart)) }
            .map { o -> Item in
                let s = axis.minutes(at: max(o.start, dayStart))
                let e = axis.minutes(at: min(max(o.end, o.start), dayEnd))
                return Item(o: o, s: s, shownEnd: min(full, max(e, s + minimumMinutes)))
            }
            .sorted { a, b in
                if a.s != b.s { return a.s < b.s }
                if a.shownEnd != b.shownEnd { return a.shownEnd > b.shownEnd }
                return CalendarQuery.displayOrder(a.o, b.o)
            }

        var result: [TimelineBlock] = []
        var cluster: [(Item, Int)] = []
        var clusterEnd = -Double.infinity

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
                    top: item.s / full,
                    height: (item.shownEnd - item.s) / full,
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

    /// Höhe der aktuellen Uhrzeit-Linie als Anteil der Spalte (Wanduhr-Achse),
    /// `nil` an anderen Tagen.
    public static func nowLine(_ now: Date, day: LocalDate, calendar cal: Calendar) -> Double? {
        let axis = WallClockAxis(day: day, calendar: cal)
        guard now >= axis.dayStart, now < axis.dayEnd else { return nil }
        return axis.fraction(at: now)
    }

    /// Beschriftung einer zweiten Zeitzone für die 24 Stundenzeilen des
    /// Rasters (LIC-42). `TimeZoneDisplay.hourLabels` zählt echte Stunden
    /// (23 oder 25 an Umstellungstagen der Heimatzone); hier wird jede Zeile
    /// der Wanduhr-Achse ihrer echten Stunde zugeordnet. Zeilen, die es am Tag
    /// nicht gibt (Lücke beim Vorstellen), bleiben `nil`; beim Zurückstellen
    /// zeigt die Zeile „02“ den ersten Durchgang.
    public static func secondaryHourLabels(for zone: TimeZone, home: TimeZone, day: LocalDate) -> [String?] {
        let labels = TimeZoneDisplay.hourLabels(for: zone, home: home, day: day)
        let axis = WallClockAxis(day: day, calendar: .lichtung(timeZone: home))
        return (0..<24).map { hour -> String? in
            let m = Double(hour * 60)
            guard axis.exists(minutes: m) else { return nil }
            let realHours = axis.instant(atMinutes: m).timeIntervalSince(axis.dayStart) / 3600
            let index = Int(realHours.rounded(.down))
            return labels.indices.contains(index) ? labels[index] : nil
        }
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
