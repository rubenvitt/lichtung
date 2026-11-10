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
    /// Rahmenlinien links vom Block: höchster Index der Rahmen, die ihn
    /// zeitlich überdecken, plus 1 (`TimelineLayout.day`).
    public var insetByFrames = 0

    public var id: String { occurrence.id }

    /// Linke Kante und Breite als Anteil der Tagesspalte.
    public var leading: Double { Double(column) / Double(columns) }
    public var width: Double { Double(span) / Double(columns) }
}

/// Langer Termin als Randlinie neben der Tagesspalte (`TimelineLayout.day`).
public struct TimelineFrame: Hashable, Sendable, Identifiable {
    public var occurrence: Occurrence
    /// Oberkante und Höhe als Anteil der Spalte, 0…1.
    public var top: Double
    public var height: Double
    /// 0 = ganz links.
    public var index: Int

    public var id: String { occurrence.id }
}

/// Feld „+n“ für Blöcke, die über die erlaubten Spalten hinausgehen.
public struct TimelineOverflow: Hashable, Sendable, Identifiable {
    public var hidden: [Occurrence]
    public var top: Double
    public var height: Double
    public var column: Int
    public var columns: Int
    public var id: String
    /// Wie `TimelineBlock.insetByFrames`.
    public var insetByFrames = 0
}

/// Trefferfläche eines Rahmens in einem Zeitabschnitt (`TimelineLayout.frameHitAreas`).
public struct FrameHitArea: Hashable, Sendable {
    public var frameId: String
    /// Linker Rand und Breite in Punkten ab dem linken Spaltenrand.
    public var x: Double
    public var width: Double
    /// Oberkante und Höhe als Anteil der Spalte.
    public var top: Double
    public var height: Double

    public init(frameId: String, x: Double, width: Double, top: Double, height: Double) {
        self.frameId = frameId
        self.x = x
        self.width = width
        self.top = top
        self.height = height
    }
}

public struct TimelineDay: Hashable, Sendable {
    public var blocks: [TimelineBlock]
    public var frames: [TimelineFrame]
    public var overflows: [TimelineOverflow]
}

public enum TimelineLayout {
    /// Zeitgebundene Vorkommen an `day`, nebeneinander gelegt, wo sie sich
    /// überschneiden. `minimumMinutes` ist die kleinste dargestellte Höhe in
    /// Minuten der Achse; kurze Termine überlappen damit auch optisch nicht.
    /// Lage und Überschneidung gelten auf der Wanduhr-Achse (`WallClockAxis`).
    public static func blocks(_ occurrences: [Occurrence], day: LocalDate, calendar cal: Calendar, minimumMinutes: Double = 15) -> [TimelineBlock] {
        let axis = WallClockAxis(day: day, calendar: cal)
        return columnLayout(items(occurrences, axis: axis, minimumMinutes: minimumMinutes), axis: axis, maxColumns: nil).blocks
    }

    /// Ab dieser Länge (Minuten am Tag) kann ein Termin zum Rahmen werden.
    public static let frameMinimumMinutes = 240.0

    /// Raster eines Tages: lange Termine, die kürzere überdecken, werden zu
    /// Rahmen (Randlinie) und fallen aus der Spaltenrechnung. Hat eine Gruppe
    /// mehr als `maxColumns` Spalten, sammelt ein Feld „+n“ in der letzten
    /// Spalte die Blöcke ab Spalte `maxColumns - 1`.
    public static func day(_ occurrences: [Occurrence], day: LocalDate, calendar cal: Calendar,
                           maxColumns: Int, minimumMinutes: Double = 15) -> TimelineDay {
        let axis = WallClockAxis(day: day, calendar: cal)
        let all = items(occurrences, axis: axis, minimumMinutes: minimumMinutes)
        let long = all.filter { $0.e - $0.s >= frameMinimumMinutes }
        let short = all.filter { $0.e - $0.s < frameMinimumMinutes }
        let frameItems = long.filter { f in short.contains { $0.s < f.shownEnd && f.s < $0.shownEnd } }
        let frameIds = Set(frameItems.map(\.o.id))
        let rest = all.filter { !frameIds.contains($0.o.id) }

        // Jeder Rahmen nimmt die erste Linie, die kein zeitgleicher Rahmen hat.
        var frames: [TimelineFrame] = []
        for f in frameItems {
            let used = Set(zip(frames, frameItems).filter { $0.1.s < f.shownEnd && f.s < $0.1.shownEnd }.map(\.0.index))
            let index = (0...).first { !used.contains($0) } ?? 0
            frames.append(TimelineFrame(occurrence: f.o, top: f.s / WallClockAxis.minutesPerDay,
                                        height: (f.shownEnd - f.s) / WallClockAxis.minutesPerDay, index: index))
        }
        /// Rechts der höchsten Linie der Rahmen, die `s..<e` überdecken.
        func inset(_ s: Double, _ e: Double) -> Int {
            (zip(frames, frameItems).filter { $0.1.s < e && s < $0.1.shownEnd }.map(\.0.index).max() ?? -1) + 1
        }
        let byId = Dictionary(rest.map { ($0.o.id, $0) }, uniquingKeysWith: { a, _ in a })
        var (blocks, overflows) = columnLayout(rest, axis: axis, maxColumns: maxColumns)
        for i in blocks.indices {
            guard let b = byId[blocks[i].id] else { continue }
            blocks[i].insetByFrames = inset(b.s, b.shownEnd)
        }
        for i in overflows.indices {
            let o = overflows[i]
            overflows[i].insetByFrames = inset(o.top * WallClockAxis.minutesPerDay, (o.top + o.height) * WallClockAxis.minutesPerDay)
        }
        return TimelineDay(blocks: blocks, frames: frames, overflows: overflows)
    }

    /// Linie von Rahmen `index` in Punkten ab dem linken Spaltenrand (je 5 pt).
    public static func frameLineX(_ index: Int) -> Double { 3 + 5 * Double(index) }

    /// Trefferflächen der Rahmen: ein gemeinsames Band von mindestens
    /// `minimumWidth` pt am linken Rand. Wo sich Rahmen zeitlich überdecken,
    /// teilt es sich auf halbem Weg zwischen ihren Linien; wo einer allein
    /// ist, gehört ihm das ganze Band. So trifft jeder Klick genau einen Rahmen.
    public static func frameHitAreas(_ frames: [TimelineFrame], minimumWidth: Double = 16) -> [FrameHitArea] {
        let edges = Set(frames.flatMap { [$0.top, $0.top + $0.height] }).sorted()
        var result: [FrameHitArea] = []
        for (a, b) in zip(edges, edges.dropFirst()) {
            let active = frames.filter { $0.top <= a && $0.top + $0.height >= b }.sorted { $0.index < $1.index }
            guard let last = active.last else { continue }
            let lines = active.map { frameLineX($0.index) }
            let band = max(minimumWidth, frameLineX(last.index) + minimumWidth / 2)
            for (i, f) in active.enumerated() {
                let left = i == 0 ? 0 : (lines[i - 1] + lines[i]) / 2
                let right = i == active.count - 1 ? band : (lines[i] + lines[i + 1]) / 2
                // Gleiche Fläche direkt darüber: verlängern statt neu anlegen.
                if let j = result.lastIndex(where: { $0.frameId == f.id }), result[j].x == left,
                   result[j].width == right - left, result[j].top + result[j].height == a {
                    result[j].height = b - result[j].top
                } else {
                    result.append(FrameHitArea(frameId: f.id, x: left, width: right - left, top: a, height: b - a))
                }
            }
        }
        return result.sorted { ($0.top, $0.x) < ($1.top, $1.x) }
    }

    /// Zeile des Titels je Rahmen: Titel, die näher als `lineHeight` (Anteil
    /// der Spalte) beieinander begännen, rutschen je eine Zeile nach unten.
    /// Liegt ein Block über dieser Zeile, steht der Titel stattdessen über
    /// der Anfangslinie (Zeile -1, bei mehreren -2 …), sofern dort Platz ist.
    public static func frameTitleRows(_ frames: [TimelineFrame], lineHeight: Double, blocks: [TimelineBlock] = []) -> [String: Int] {
        var placed: [Double] = []
        var rows: [String: Int] = [:]
        for f in frames.sorted(by: { ($0.top, $0.index) < ($1.top, $1.index) }) {
            func titleFree(_ row: Int) -> Bool {
                !placed.contains { abs($0 - (f.top + Double(row) * lineHeight)) < lineHeight }
            }
            func blockFree(_ row: Int) -> Bool {
                let top = f.top + Double(row) * lineHeight, bottom = f.top + Double(row + 1) * lineHeight
                return top >= 0 && !blocks.contains { $0.top < bottom && top < $0.top + $0.height }
            }
            var row = 0
            while !titleFree(row) { row += 1 }
            if !blockFree(row), let above = (1...max(1, frames.count)).map({ -$0 }).first(where: { titleFree($0) && blockFree($0) }) {
                row = above
            }
            placed.append(f.top + Double(row) * lineHeight)
            rows[f.id] = row
        }
        return rows
    }

    /// `s`, `e` und `shownEnd` in Minuten der Achse; `e` ohne Mindesthöhe.
    fileprivate struct Item { var o: Occurrence; var s: Double; var e: Double; var shownEnd: Double }

    fileprivate static func items(_ occurrences: [Occurrence], axis: WallClockAxis, minimumMinutes: Double) -> [Item] {
        let dayStart = axis.dayStart
        let dayEnd = axis.dayEnd
        return occurrences
            .filter { !$0.isAllDay && $0.start < dayEnd && ($0.end > dayStart || ($0.end <= $0.start && $0.start >= dayStart)) }
            .map { o -> Item in
                let s = axis.minutes(at: max(o.start, dayStart))
                let e = axis.minutes(at: min(max(o.end, o.start), dayEnd))
                return Item(o: o, s: s, e: e, shownEnd: min(WallClockAxis.minutesPerDay, max(e, s + minimumMinutes)))
            }
            .sorted { a, b in
                if a.s != b.s { return a.s < b.s }
                if a.shownEnd != b.shownEnd { return a.shownEnd > b.shownEnd }
                return CalendarQuery.displayOrder(a.o, b.o)
            }
    }

    /// Spaltenrechnung: Gruppen sich überschneidender Termine, je Gruppe
    /// Spalten und Ausdehnung. Mit `maxColumns` wandern Blöcke ab der letzten
    /// erlaubten Spalte in ein Überlauffeld.
    fileprivate static func columnLayout(_ items: [Item], axis: WallClockAxis, maxColumns: Int?) -> (blocks: [TimelineBlock], overflows: [TimelineOverflow]) {
        let dayStart = axis.dayStart
        let dayEnd = axis.dayEnd
        let full = WallClockAxis.minutesPerDay

        var result: [TimelineBlock] = []
        var overflows: [TimelineOverflow] = []
        var cluster: [(Item, Int)] = []
        var clusterEnd = -Double.infinity

        func flush() {
            guard !cluster.isEmpty else { return }
            let groupColumns = (cluster.map(\.1).max() ?? 0) + 1
            let collapses = maxColumns.map { groupColumns > $0 } ?? false
            // Mit Überlauf bleiben die Spalten davor; das Feld sitzt in der letzten.
            let visibleColumns = collapses ? maxColumns! - 1 : groupColumns
            let columns = collapses ? maxColumns! : groupColumns
            for (item, col) in cluster where col < visibleColumns {
                // Nach rechts ausdehnen, solange keine Spalte daneben belegt ist.
                var span = 1
                while col + span < visibleColumns, !cluster.contains(where: { other in
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
            if collapses {
                let hidden = cluster.filter { $0.1 >= visibleColumns }.map(\.0)
                let top = hidden.map(\.s).min() ?? 0
                let bottom = hidden.map(\.shownEnd).max() ?? top
                overflows.append(TimelineOverflow(
                    hidden: hidden.map(\.o), top: top / full, height: (bottom - top) / full,
                    column: visibleColumns, columns: columns, id: "+\(hidden[0].o.id)"
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
        return (result, overflows)
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

    /// Zweite Zeitzone über mehrere Tage (LIC-47). Die Spalte links trägt die
    /// Beschriftung des ersten Tages. Wo ein Tag davon abweicht (eine der
    /// Zonen stellt in dieser Woche die Uhr um), steht in `perDay` die eigene
    /// Beschriftung dieser Zeilen, sonst `nil`. Ein Tag ganz ohne Abweichung
    /// ist `nil`.
    public static func secondaryHourLabels(for zone: TimeZone, home: TimeZone, days: [LocalDate]) -> SecondaryZoneLabels {
        guard let first = days.first else { return SecondaryZoneLabels(gutter: Array(repeating: nil, count: 24), perDay: []) }
        let gutter = secondaryHourLabels(for: zone, home: home, day: first)
        let perDay: [[String?]?] = days.map { day in
            guard day != first else { return nil }
            let own = secondaryHourLabels(for: zone, home: home, day: day)
            let differing = zip(own, gutter).map { mine, left in mine == left ? nil : mine }
            return differing.allSatisfy { $0 == nil } ? nil : differing
        }
        return SecondaryZoneLabels(gutter: gutter, perDay: perDay)
    }
}

/// Beschriftung der zweiten Zeitzone für die sichtbaren Tage
/// (`TimelineLayout.secondaryHourLabels(for:home:days:)`).
public struct SecondaryZoneLabels: Hashable, Sendable {
    /// 24 Zeilen für die Spalte links (erster Tag).
    public var gutter: [String?]
    /// Je Tag die 24 Zeilen, die von der Spalte links abweichen; `nil`, wo
    /// alles gleich ist.
    public var perDay: [[String?]?]

    public init(gutter: [String?], perDay: [[String?]?]) {
        self.gutter = gutter
        self.perDay = perDay
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
