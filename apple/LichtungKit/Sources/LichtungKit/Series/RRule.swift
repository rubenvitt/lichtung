import Foundation

/// Wiederholungsregel nach RFC 5545 (`RRULE`), wie sie von den Anbietern kommt.
///
/// Anders als `RecurrenceRule` (die Teilmenge, die Lichtung selbst schreibt)
/// bildet dieser Typ alles ab, was die Serienexpansion versteht. Unbekannte
/// Teile werden ignoriert; eine kaputte Regel ergibt `nil`.
public struct RRule: Hashable, Sendable {
    public enum Frequency: String, Sendable {
        case secondly = "SECONDLY", minutely = "MINUTELY", hourly = "HOURLY"
        case daily = "DAILY", weekly = "WEEKLY", monthly = "MONTHLY", yearly = "YEARLY"
    }

    /// `UNTIL`: ein Tag, ein UTC-Zeitpunkt (`…Z`) oder eine Wanduhrzeit ohne Zone.
    public enum Until: Hashable, Sendable {
        case date(LocalDate)
        case utc(Date)
        case local(LocalDateTime)
    }

    /// Eintrag in `BYDAY`, z. B. `MO`, `2MO`, `-1FR`.
    public struct WeekdayNum: Hashable, Sendable {
        public var ordinal: Int?
        public var weekday: Weekday

        public init(_ weekday: Weekday, ordinal: Int? = nil) {
            self.weekday = weekday
            self.ordinal = ordinal
        }
    }

    public var frequency: Frequency
    public var interval = 1
    public var count: Int?
    public var until: Until?
    public var byDay: [WeekdayNum] = []
    public var byMonthDay: [Int] = []
    public var byMonth: [Int] = []
    public var byYearDay: [Int] = []
    public var byWeekNo: [Int] = []
    public var bySetPos: [Int] = []
    public var byHour: [Int] = []
    public var byMinute: [Int] = []
    public var bySecond: [Int] = []
    public var weekStart: Weekday = .monday

    /// Liest einen RRULE-Wert (mit oder ohne `RRULE:`-Präfix).
    public init?(_ text: String) {
        var text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.uppercased().hasPrefix("RRULE:") { text.removeFirst(6) }
        var freq: Frequency?
        for part in text.split(separator: ";") {
            let kv = part.split(separator: "=", maxSplits: 1).map {
                $0.trimmingCharacters(in: .whitespaces).uppercased()
            }
            guard kv.count == 2, !kv[1].isEmpty else { return nil }
            let value = kv[1]
            switch kv[0] {
            case "FREQ":
                guard let f = Frequency(rawValue: value) else { return nil }
                freq = f
            case "INTERVAL":
                guard let n = Int(value), n >= 1 else { return nil }
                interval = n
            case "COUNT":
                guard let n = Int(value), n >= 1 else { return nil }
                count = n
            case "UNTIL":
                guard let u = Self.until(value) else { return nil }
                until = u
            case "BYDAY":
                guard let l = Self.list(value, Self.weekdayNum) else { return nil }
                byDay = l
            case "BYMONTHDAY":
                guard let l = Self.ints(value, -31...31, nonZero: true) else { return nil }
                byMonthDay = l
            case "BYMONTH":
                guard let l = Self.ints(value, 1...12) else { return nil }
                byMonth = l
            case "BYYEARDAY":
                guard let l = Self.ints(value, -366...366, nonZero: true) else { return nil }
                byYearDay = l
            case "BYWEEKNO":
                guard let l = Self.ints(value, -53...53, nonZero: true) else { return nil }
                byWeekNo = l
            case "BYSETPOS":
                guard let l = Self.ints(value, -366...366, nonZero: true) else { return nil }
                bySetPos = l
            case "BYHOUR":
                guard let l = Self.ints(value, 0...23) else { return nil }
                byHour = l
            case "BYMINUTE":
                guard let l = Self.ints(value, 0...59) else { return nil }
                byMinute = l
            case "BYSECOND":
                guard let l = Self.ints(value, 0...60) else { return nil }
                bySecond = l
            case "WKST":
                guard let w = Self.weekday(value) else { return nil }
                weekStart = w
            default:
                continue // unbekannte Teile (auch X-…) ignorieren
            }
        }
        guard let freq else { return nil }
        frequency = freq
    }

    /// Die Expansion kann diese Regel abbilden. Sonst gilt nur DTSTART.
    ///
    /// Bewusst nicht: Frequenzen unter einem Tag und `BYWEEKNO`.
    public var isExpandable: Bool {
        switch frequency {
        case .daily, .weekly, .monthly, .yearly: return byWeekNo.isEmpty
        case .hourly, .minutely, .secondly: return false
        }
    }

    // MARK: Lesen

    private static func list<T>(_ value: String, _ item: (String) -> T?) -> [T]? {
        var out: [T] = []
        for raw in value.split(separator: ",") {
            guard let v = item(raw.trimmingCharacters(in: .whitespaces)) else { return nil }
            out.append(v)
        }
        return out.isEmpty ? nil : out
    }

    private static func ints(_ value: String, _ range: ClosedRange<Int>, nonZero: Bool = false) -> [Int]? {
        list(value) { s in
            guard let n = Int(s), range.contains(n), !(nonZero && n == 0) else { return nil }
            return n
        }
    }

    private static func weekday(_ code: String) -> Weekday? {
        Weekday.allCases.first { $0.icalCode == code }
    }

    private static func weekdayNum(_ text: String) -> WeekdayNum? {
        guard text.count >= 2, let wd = weekday(String(text.suffix(2))) else { return nil }
        let prefix = text.dropLast(2)
        if prefix.isEmpty { return WeekdayNum(wd) }
        guard let n = Int(prefix), n != 0, (-53...53).contains(n) else { return nil }
        return WeekdayNum(wd, ordinal: n)
    }

    /// `20261231`, `20261231T225959Z` oder `20261231T225959`.
    private static func until(_ text: String) -> Until? {
        let chars = Array(text)
        func num(_ r: Range<Int>) -> Int? {
            guard r.upperBound <= chars.count else { return nil }
            let s = String(chars[r])
            guard s.allSatisfy(\.isASCII), s.allSatisfy(\.isNumber) else { return nil }
            return Int(s)
        }
        guard let y = num(0..<4), let m = num(4..<6), let d = num(6..<8),
              LocalDate.isValid(year: y, month: m, day: d) else { return nil }
        let date = LocalDate(year: y, month: m, day: d)
        if chars.count == 8 { return .date(date) }
        guard chars.count >= 15, chars[8] == "T",
              let h = num(9..<11), let mi = num(11..<13), let s = num(13..<15),
              (0...23).contains(h), (0...59).contains(mi), (0...60).contains(s) else { return nil }
        let local = LocalDateTime(date: date, hour: h, minute: mi, second: min(s, 59))
        switch chars.count {
        case 15: return .local(local)
        case 16 where chars[15] == "Z": return .utc(local.instant(in: .gregorianUTC))
        default: return nil
        }
    }
}

// MARK: - Expansion in Wanduhrzeit

extension RRule {
    /// Vorkommen der Regel nach `start` (DTSTART selbst nicht), in Wanduhrzeit.
    ///
    /// - `lower`/`upper`: nur Tage in diesem Bereich werden geliefert. Ohne
    ///   `COUNT` springt die Expansion direkt in die Nähe von `lower`; mit
    ///   `COUNT` muss sie ab Beginn zählen, materialisiert aber nur im Bereich.
    /// - `limit`: `UNTIL`, schon in Wanduhrzeit der Serie umgerechnet (inklusive).
    /// - DTSTART zählt für `COUNT` als erstes Vorkommen, auch wenn es nicht auf
    ///   die Regel passt (RFC 5545 §3.8.5.3).
    func instances(
        after start: LocalDateTime, allDay: Bool,
        from lower: LocalDate, through upper: LocalDate,
        until limit: LocalDateTime?, maxPeriods: Int
    ) -> [LocalDateTime] {
        guard isExpandable else { return [] }
        if let count, count <= 1 { return [] }
        let frame = Periods(rule: self, start: start)
        let first = count == nil ? frame.period(containing: lower) : 0
        let times = timesOfDay(start: start, allDay: allDay)
        var produced = 1
        var out: [LocalDateTime] = []
        for k in first..<(first + maxPeriods) {
            guard let firstDay = frame.firstDay(of: k), firstDay <= upper.dayNumber else { break }
            for c in candidates(in: k, frame: frame, times: times) {
                if c <= start { continue }
                if let limit, c > limit { return out }
                if c.date > upper { return out }
                produced += 1
                if c.date >= lower { out.append(c) }
                if let count, produced >= count { return out }
            }
        }
        return out
    }

    private func timesOfDay(start: LocalDateTime, allDay: Bool) -> [(Int, Int, Int)] {
        if allDay { return [(0, 0, 0)] }
        let hours = byHour.isEmpty ? [start.hour] : Set(byHour).sorted()
        let minutes = byMinute.isEmpty ? [start.minute] : Set(byMinute).sorted()
        let seconds = bySecond.isEmpty ? [start.second] : Set(bySecond).sorted()
        return hours.flatMap { h in minutes.flatMap { m in seconds.map { (h, m, min($0, 59)) } } }
    }

    /// Alle Kandidaten einer Periode, sortiert, nach `BYSETPOS`.
    private func candidates(in k: Int, frame: Periods, times: [(Int, Int, Int)]) -> [LocalDateTime] {
        let days = Set(dayNumbers(in: k, frame: frame)).sorted()
        var set = days.flatMap { dn in
            let d = LocalDate(dayNumber: dn)
            return times.map { LocalDateTime(date: d, hour: $0.0, minute: $0.1, second: $0.2) }
        }
        if !bySetPos.isEmpty {
            let n = set.count
            let picked = Set(bySetPos.compactMap { p -> Int? in
                let i = p > 0 ? p - 1 : n + p
                return (0..<n).contains(i) ? i : nil
            })
            set = picked.sorted().map { set[$0] }
        }
        return set
    }

    /// Tage (als Tagesnummer) einer Periode nach den BYxxx-Regeln (RFC 5545,
    /// Tabelle in §3.3.10: „Expand“ bzw. „Limit“ je Frequenz).
    private func dayNumbers(in k: Int, frame: Periods) -> [Int] {
        let start = frame.start.date
        switch frequency {
        case .daily:
            let dn = start.dayNumber + k * interval
            let d = LocalDate(dayNumber: dn)
            guard monthMatches(d), monthDayMatches(d), weekdayMatches(dn), yearDayMatches(d) else { return [] }
            return [dn]

        case .weekly:
            let first = frame.weekStart + 7 * interval * k
            return (first..<first + 7).filter { dn in
                let wd = isoWeekday(dn)
                let dayOK = byDay.isEmpty ? wd == start.weekdayNumber : byDay.contains { $0.weekday.rawValue == wd }
                return dayOK && monthMatches(LocalDate(dayNumber: dn))
            }

        case .monthly:
            let (y, m) = frame.month(of: k)
            guard byMonth.isEmpty || byMonth.contains(m) else { return [] }
            let first = LocalDate(year: y, month: m, day: 1).dayNumber
            let last = first + LocalDate.daysInMonth(year: y, month: m) - 1
            if !byMonthDay.isEmpty {
                let days = monthDays(y, m)
                return byDay.isEmpty ? days : days.filter(Set(expandByDay(first, last)).contains)
            }
            if !byDay.isEmpty { return expandByDay(first, last) }
            guard LocalDate.isValid(year: y, month: m, day: start.day) else { return [] }
            return [LocalDate(year: y, month: m, day: start.day).dayNumber]

        case .yearly:
            let y = start.year + k * interval
            let jan1 = LocalDate(year: y, month: 1, day: 1).dayNumber
            let dec31 = LocalDate(year: y, month: 12, day: 31).dayNumber
            if !byYearDay.isEmpty {
                return byYearDay.compactMap { n -> Int? in
                    let dn = n > 0 ? jan1 + n - 1 : dec31 + n + 1
                    return (jan1...dec31).contains(dn) ? dn : nil
                }.filter { dn in
                    let d = LocalDate(dayNumber: dn)
                    return monthMatches(d) && monthDayMatches(d) && weekdayMatches(dn)
                }
            }
            let months = byMonth.isEmpty ? Array(1...12) : byMonth
            func monthRange(_ m: Int) -> (Int, Int) {
                let f = LocalDate(year: y, month: m, day: 1).dayNumber
                return (f, f + LocalDate.daysInMonth(year: y, month: m) - 1)
            }
            if !byMonthDay.isEmpty {
                let days = months.flatMap { monthDays(y, $0) }
                guard !byDay.isEmpty else { return days }
                let allowed = byMonth.isEmpty
                    ? Set(expandByDay(jan1, dec31))
                    : Set(months.flatMap { m in let r = monthRange(m); return expandByDay(r.0, r.1) })
                return days.filter(allowed.contains)
            }
            if !byDay.isEmpty {
                // Ordinal bezieht sich mit BYMONTH auf den Monat, sonst auf das Jahr.
                if byMonth.isEmpty { return expandByDay(jan1, dec31) }
                return months.flatMap { m in let r = monthRange(m); return expandByDay(r.0, r.1) }
            }
            return (byMonth.isEmpty ? [start.month] : byMonth).compactMap { m in
                LocalDate.isValid(year: y, month: m, day: start.day)
                    ? LocalDate(year: y, month: m, day: start.day).dayNumber : nil
            }

        case .hourly, .minutely, .secondly:
            return []
        }
    }

    private func monthDays(_ y: Int, _ m: Int) -> [Int] {
        let dim = LocalDate.daysInMonth(year: y, month: m)
        let first = LocalDate(year: y, month: m, day: 1).dayNumber
        return byMonthDay.compactMap { n in
            let d = n > 0 ? n : dim + n + 1
            return (1...dim).contains(d) ? first + d - 1 : nil
        }
    }

    /// `BYDAY` als Erweiterung im Bereich `first...last` (Monat oder Jahr).
    private func expandByDay(_ first: Int, _ last: Int) -> [Int] {
        var out: [Int] = []
        for wn in byDay {
            let wd = wn.weekday.rawValue
            let firstMatch = first + (wd - isoWeekday(first) + 7) % 7
            let lastMatch = last - (isoWeekday(last) - wd + 7) % 7
            switch wn.ordinal {
            case nil:
                out.append(contentsOf: stride(from: firstMatch, through: last, by: 7))
            case let n? where n > 0:
                let dn = firstMatch + 7 * (n - 1)
                if dn <= last { out.append(dn) }
            case let n?:
                let dn = lastMatch - 7 * (-n - 1)
                if dn >= first { out.append(dn) }
            }
        }
        return out
    }

    private func monthMatches(_ d: LocalDate) -> Bool { byMonth.isEmpty || byMonth.contains(d.month) }

    private func weekdayMatches(_ dn: Int) -> Bool {
        byDay.isEmpty || byDay.contains { $0.weekday.rawValue == isoWeekday(dn) }
    }

    private func monthDayMatches(_ d: LocalDate) -> Bool {
        guard !byMonthDay.isEmpty else { return true }
        let dim = LocalDate.daysInMonth(year: d.year, month: d.month)
        return byMonthDay.contains { $0 > 0 ? $0 == d.day : dim + $0 + 1 == d.day }
    }

    private func yearDayMatches(_ d: LocalDate) -> Bool {
        guard !byYearDay.isEmpty else { return true }
        let jan1 = LocalDate(year: d.year, month: 1, day: 1).dayNumber
        let len = LocalDate(year: d.year, month: 12, day: 31).dayNumber - jan1 + 1
        let n = d.dayNumber - jan1 + 1
        return byYearDay.contains { $0 > 0 ? $0 == n : len + $0 + 1 == n }
    }
}

/// Perioden einer Regel: Periode `k` ist die k-te Periode im Abstand `INTERVAL`
/// ab der Periode von DTSTART.
private struct Periods {
    let rule: RRule
    let start: LocalDateTime
    /// Erster Tag der Woche von DTSTART (nach `WKST`).
    let weekStart: Int

    init(rule: RRule, start: LocalDateTime) {
        self.rule = rule
        self.start = start
        let dn = start.date.dayNumber
        weekStart = dn - (isoWeekday(dn) - rule.weekStart.rawValue + 7) % 7
    }

    func month(of k: Int) -> (Int, Int) {
        let idx = start.date.year * 12 + start.date.month - 1 + k * rule.interval
        return (idx / 12, idx % 12 + 1)
    }

    /// Erster Tag der Periode `k`, `nil` jenseits des Jahres 9999.
    func firstDay(of k: Int) -> Int? {
        switch rule.frequency {
        case .daily: return start.date.dayNumber + k * rule.interval
        case .weekly: return weekStart + 7 * rule.interval * k
        case .monthly:
            let (y, m) = month(of: k)
            return y > 9999 ? nil : LocalDate(year: y, month: m, day: 1).dayNumber
        case .yearly:
            let y = start.date.year + k * rule.interval
            return y > 9999 ? nil : LocalDate(year: y, month: 1, day: 1).dayNumber
        case .hourly, .minutely, .secondly: return nil
        }
    }

    /// Letzte Periode, die nicht nach `day` beginnt (mindestens 0).
    func period(containing day: LocalDate) -> Int {
        let i = rule.interval
        let diff: Int
        switch rule.frequency {
        case .daily: diff = (day.dayNumber - start.date.dayNumber) / i
        case .weekly: diff = (day.dayNumber - weekStart) / (7 * i)
        case .monthly: diff = ((day.year - start.date.year) * 12 + day.month - start.date.month) / i
        case .yearly: diff = (day.year - start.date.year) / i
        case .hourly, .minutely, .secondly: diff = 0
        }
        return max(0, diff)
    }
}

// MARK: - Tagesnummern

/// ISO-Wochentag (1 = Montag … 7 = Sonntag) einer Tagesnummer.
func isoWeekday(_ dn: Int) -> Int {
    ((dn % 7 + 7 + 3) % 7) + 1 // 1970-01-01 war ein Donnerstag
}

extension LocalDate {
    /// Tage seit 1970-01-01 (proleptisch gregorianisch). Reine Arithmetik ohne
    /// `Calendar`, damit die Expansion langer Serien schnell bleibt
    /// (Algorithmus nach H. Hinnant, „days_from_civil“).
    var dayNumber: Int {
        let y = month <= 2 ? year - 1 : year
        let era = (y >= 0 ? y : y - 399) / 400
        let yoe = y - era * 400
        let doy = (153 * (month + (month > 2 ? -3 : 9)) + 2) / 5 + day - 1
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
        return era * 146_097 + doe - 719_468
    }

    init(dayNumber: Int) {
        let z = dayNumber + 719_468
        let era = (z >= 0 ? z : z - 146_096) / 146_097
        let doe = z - era * 146_097
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
        let mp = (5 * doy + 2) / 153
        let d = doy - (153 * mp + 2) / 5 + 1
        let m = mp < 10 ? mp + 3 : mp - 9
        self.init(year: yoe + era * 400 + (m <= 2 ? 1 : 0), month: m, day: d)
    }

    var weekdayNumber: Int { isoWeekday(dayNumber) }
}
