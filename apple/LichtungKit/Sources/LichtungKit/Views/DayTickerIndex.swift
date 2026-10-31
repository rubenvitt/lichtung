import Foundation

/// Das Tagesband deckt virtualisiert alle Tage ab (LIC-67): Jede Zelle hat einen
/// ganzzahligen Index (Tage seit 1970-01-01), gebaut werden nur die sichtbaren.
/// Die Belegung entsteht für einen Ausschnitt um die sichtbaren Zellen, nicht
/// vorab für einen festen Zeitraum.
///
/// Der Index ist reine Kalenderrechnung ohne Zeitzone: Ein Tag bleibt derselbe
/// Index, auch über die Zeitumstellung. Erst `summaries` braucht den Kalender,
/// um die Tagesgrenzen (23- oder 25-Stunden-Tage) als Zeitpunkte zu bestimmen.
extension DayTicker {
    // MARK: Index ↔ Datum

    /// Index eines Tages: Tage seit 1970-01-01 (proleptisch gregorianisch).
    public static func index(of day: LocalDate) -> Int {
        // Howard Hinnant, „days_from_civil“: ohne Calendar, damit es pro Zelle billig bleibt.
        let y = day.month <= 2 ? day.year - 1 : day.year
        let era = (y >= 0 ? y : y - 399) / 400
        let yoe = y - era * 400
        let mp = (day.month + 9) % 12
        let doy = (153 * mp + 2) / 5 + day.day - 1
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
        return era * 146_097 + doe - 719_468
    }

    /// Tag zu einem Index, Umkehrung von `index(of:)`.
    public static func day(at index: Int) -> LocalDate {
        let z = index + 719_468
        let era = (z >= 0 ? z : z - 146_096) / 146_097
        let doe = z - era * 146_097
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
        let mp = (5 * doy + 2) / 153
        let d = doy - (153 * mp + 2) / 5 + 1
        let m = mp < 10 ? mp + 3 : mp - 9
        let y = yoe + era * 400 + (m <= 2 ? 1 : 0)
        return LocalDate(year: y, month: m, day: d)
    }

    /// Alle Indizes des Bands: vom 1. Januar `years` Jahre vor bis zum 31. Dezember
    /// `years` Jahre nach dem Jahr von `today`. Ändert sich nur zum Jahreswechsel,
    /// damit die Zellen ihre Identität behalten.
    public static func indexRange(around today: LocalDate, years: Int = 100) -> Range<Int> {
        let first = index(of: LocalDate(year: today.year - years, month: 1, day: 1))
        let end = index(of: LocalDate(year: today.year + years + 1, month: 1, day: 1))
        return first..<end
    }

    // MARK: Ausschnitt mit Belegung

    /// Ausschnitt, für den die Belegung berechnet wird. Bleibt `current`, solange die
    /// sichtbaren Zellen mit mindestens `step` Tagen Abstand darin liegen; sonst ein
    /// neuer, auf `step` gerundeter Ausschnitt mit `2 * step` Tagen Rand je Seite.
    /// So rechnet das Band beim Wischen nur alle paar Tage neu.
    public static func window(current: Range<Int>, visible: ClosedRange<Int>, step: Int = 14) -> Range<Int> {
        let step = max(1, step)
        if !current.isEmpty, current.lowerBound + step <= visible.lowerBound, visible.upperBound < current.upperBound - step {
            return current
        }
        let lo = floorDiv(visible.lowerBound, step) * step - 2 * step
        let hi = (floorDiv(visible.upperBound, step) + 1) * step + 2 * step
        return lo..<hi
    }

    /// Bandtage für alle Indizes in `range`, je Index. Jedes Vorkommen wird nur den
    /// Tagen zugeordnet, die es berührt (kein Durchlauf über alle Vorkommen je Tag).
    public static func summaries(
        _ occurrences: [Occurrence], in range: Range<Int>, today: LocalDate, calendar cal: Calendar
    ) -> [Int: TickerDay] {
        guard !range.isEmpty else { return [:] }
        var buckets: [Int: [Occurrence]] = [:]
        for o in occurrences {
            // Kandidaten: Tag des Beginns bis Tag des Endes; die genaue Prüfung macht `day(_:touching:)`.
            let first = max(range.lowerBound, index(of: LocalDate(o.start, calendar: cal)))
            let last = min(range.upperBound - 1, index(of: LocalDate(max(o.start, o.end), calendar: cal)))
            guard first <= last else { continue }
            for i in first...last { buckets[i, default: []].append(o) }
        }
        var result: [Int: TickerDay] = [:]
        result.reserveCapacity(range.count)
        for i in range {
            result[i] = day(day(at: i), touching: buckets[i] ?? [], today: today, calendar: cal)
        }
        return result
    }

    static func floorDiv(_ a: Int, _ b: Int) -> Int {
        let q = a / b
        return (a % b != 0 && (a < 0) != (b < 0)) ? q - 1 : q
    }
}
