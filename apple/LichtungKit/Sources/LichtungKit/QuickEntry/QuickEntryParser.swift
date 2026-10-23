import Foundation

/// Regelbasierter Satz-Parser für die Schnelleingabe (Spec §5).
///
/// Deterministisch und offline: Dasselbe „Jetzt“ und derselbe Satz ergeben
/// immer denselben Entwurf. Der Parser läuft bei jedem Tastendruck, deshalb
/// keine Netzaufrufe, keine KI, keine Locale-abhängigen Formatter.
///
/// Arbeitsweise: Der Satz wird an Leerzeichen in Wörter zerlegt. An jeder
/// Stelle probiert der Parser seine Regeln (Kalender, Wiederholung, Datum,
/// Uhrzeit, Dauer, Ort). Was keine Regel nimmt, wird Titel.
///
/// Regeln für Mehrdeutiges:
/// - Uhrzeiten sind 24-Stunden-Zeiten. „7“ ist 7 Uhr, außer eine Tageszeit
///   sagt etwas anderes („7 abends“). Gesprochene Formen („halb 3“,
///   „viertel nach 4“) gelten unter 7 Uhr als Nachmittag.
/// - Eine nackte Zahl ist keine Uhrzeit („Raum 2“). Uhrzeit wird sie erst mit
///   Minuten („7:30“), „Uhr“ oder „um“.
/// - Tage ohne Jahr liegen in der Zukunft: „3.1.“ im Oktober ist der 3. Januar
///   des nächsten Jahres.
/// - Ein Wochentag ist der nächste solche Tag ab heute. Heute selbst nur, wenn
///   die genannte Uhrzeit noch nicht vorbei ist. „nächsten Dienstag“ ist nie heute.
/// - Nur eine Uhrzeit ohne Tag: heute, oder morgen, wenn sie vorbei ist.
/// - Ein Tag ohne Uhrzeit ergibt einen ganztägigen Termin.
public struct QuickEntryParser: Sendable {
    public var calendar: Calendar
    /// Dauer, wenn der Satz weder Ende noch Dauer nennt.
    public var defaultDurationMinutes: Int
    /// Kalender, die per `/name` gewählt werden können.
    public var calendars: [CalendarChoice]

    public init(
        timeZone: TimeZone = .current,
        defaultDurationMinutes: Int = 60,
        calendars: [CalendarChoice] = []
    ) {
        self.calendar = .lichtung(timeZone: timeZone)
        self.defaultDurationMinutes = defaultDurationMinutes
        self.calendars = calendars
    }

    public func parse(_ text: String, now: Date) -> QuickEntry {
        var run = Run(tokens: Token.split(text))
        run.scan()
        return run.draft.resolve(
            title: run.title(),
            now: now,
            calendar: calendar,
            defaultDuration: defaultDurationMinutes,
            calendars: calendars,
            highlights: run.highlights
        )
    }
}

// MARK: - Wörter

struct Token {
    /// Originaltext, für den Titel.
    let text: String
    /// Kleingeschrieben, Gedankenstriche zu „-“, Satzzeichen am Ende entfernt.
    let lower: String
    /// Wie `lower`, aber mit Großschreibung (für Wochentagskürzel).
    let core: String
    /// Das Wort endete auf ein Komma („Mo,“).
    let trailingComma: Bool
    /// Position in Zeichen.
    let range: Range<Int>

    static func split(_ text: String) -> [Token] {
        var tokens: [Token] = []
        var current = ""
        var start = 0
        var index = 0
        func flush() {
            if !current.isEmpty {
                tokens.append(Token(raw: current, range: start..<index))
                current = ""
            }
        }
        for ch in text {
            if ch.isWhitespace {
                flush()
            } else {
                if current.isEmpty { start = index }
                current.append(ch)
            }
            index += 1
        }
        flush()
        return tokens
    }

    init(raw: String, range: Range<Int>) {
        text = raw
        self.range = range
        var c = raw.replacingOccurrences(of: "–", with: "-").replacingOccurrences(of: "—", with: "-")
        trailingComma = c.hasSuffix(",")
        while let last = c.last, ",;!?".contains(last), c.count > 1 { c.removeLast() }
        core = c
        lower = c.lowercased()
    }
}

// MARK: - Zwischenstand

/// Tag, so wie er im Satz stand, vor dem Bezug auf „heute“.
struct PartialDate: Equatable {
    var day: Int
    var month: Int?
    var year: Int?
}

enum DaySpec: Equatable {
    case absolute(PartialDate)
    case relativeDays(Int)
    case relativeMonths(Int)
    /// Nächster solcher Wochentag. `strict`: nie heute.
    case weekday(Weekday, strict: Bool)
    /// Woche relativ zur aktuellen (1 = nächste), optional ein Wochentag darin.
    case week(offset: Int, weekday: Weekday?)
}

struct Draft {
    var day: DaySpec?
    var lastDay: PartialDate?
    var start: Clock?
    var end: Clock?
    var daypart: Daypart?
    var durationMinutes: Int?
    var allDay = false
    var recurrence: RecurrenceRule?
    var until: PartialDate?
    var location: String?
    var calendarQuery: String?
}

// MARK: - Durchlauf

struct Run {
    let tokens: [Token]
    var draft = Draft()
    var titleIndices: [Int] = []
    var highlights: [Highlight] = []

    init(tokens: [Token]) { self.tokens = tokens }

    mutating func scan() {
        var i = 0
        while i < tokens.count {
            if let (count, kind, next) = firstMatch(at: i, includingLocation: true) {
                draft = next
                highlights.append(Highlight(kind: kind, range: tokens[i].range.lowerBound..<tokens[i + count - 1].range.upperBound))
                i += count
            } else {
                titleIndices.append(i)
                i += 1
            }
        }
    }

    func firstMatch(at i: Int, includingLocation: Bool) -> (Int, Highlight.Kind, Draft)? {
        var d = draft
        if let n = matchCalendar(i, &d) { return (n, .calendar, d) }
        d = draft
        if let n = matchUntil(i, &d) { return (n, .recurrence, d) }
        d = draft
        if let n = matchRecurrence(i, &d) { return (n, .recurrence, d) }
        d = draft
        if let n = matchDate(i, &d) { return (n, .date, d) }
        d = draft
        if let n = matchTimeRange(i, &d) { return (n, .time, d) }
        d = draft
        if let n = matchTime(i, &d) { return (n, .time, d) }
        d = draft
        if let n = matchDaypart(i, &d) { return (n, .time, d) }
        d = draft
        if let n = matchDuration(i, &d) { return (n, .duration, d) }
        d = draft
        if let n = matchAllDay(i, &d) { return (n, .time, d) }
        if includingLocation {
            d = draft
            if let n = matchLocation(i, &d) { return (n, .location, d) }
        }
        return nil
    }

    func title() -> String {
        let words = titleIndices.map { tokens[$0].text }
        var title = words.joined(separator: " ")
        let trim = CharacterSet.whitespaces.union(CharacterSet(charactersIn: ",;:-–—"))
        title = title.trimmingCharacters(in: trim)
        return title
    }

    // MARK: Hilfen

    func lower(_ i: Int) -> String? { i < tokens.count ? tokens[i].lower : nil }

    func weekday(at i: Int) -> Weekday? {
        guard i < tokens.count else { return nil }
        let t = tokens[i]
        if let w = GermanLexicon.weekdayNames[t.lower] { return w }
        var abbr = t.core
        let dotted = abbr.hasSuffix(".")
        if dotted { abbr.removeLast() }
        guard let w = GermanLexicon.weekdayAbbreviations[abbr.lowercased()] else { return nil }
        if dotted || abbr.first?.isUppercase == true { return w }
        return nil
    }

    /// Wochentage mit „und“ oder Komma verbunden: „Mo, Mi und Fr“.
    /// `plural` erlaubt „montags und donnerstags“.
    func weekdayList(at i: Int, plural: Bool) -> ([Weekday], Int)? {
        func one(_ j: Int) -> Weekday? {
            if let w = weekday(at: j) { return w }
            if plural, j < tokens.count { return GermanLexicon.weekdayPlurals[tokens[j].lower] }
            return nil
        }
        guard let first = one(i) else { return nil }
        var days = [first]
        var j = i + 1
        while j < tokens.count {
            if tokens[j - 1].trailingComma, let w = one(j) {
                days.append(w); j += 1
            } else if ["und", "&", "sowie", ","].contains(tokens[j].lower), let w = one(j + 1) {
                days.append(w); j += 2
            } else {
                break
            }
        }
        return (days, j - i)
    }

    // MARK: Kalender

    func matchCalendar(_ i: Int, _ d: inout Draft) -> Int? {
        let t = tokens[i]
        guard d.calendarQuery == nil, t.core.hasPrefix("/"), t.core.count > 1 else { return nil }
        d.calendarQuery = String(t.core.dropFirst())
        return 1
    }

    // MARK: Wiederholung

    func matchRecurrence(_ i: Int, _ d: inout Draft) -> Int? {
        guard d.recurrence == nil else { return nil }
        let w = tokens[i].lower
        var rule: RecurrenceRule
        var used: Int

        switch w {
        case "täglich": rule = .daily; used = 1
        case "werktags", "wochentags": rule = .workdays; used = 1
        case "wöchentlich": rule = RecurrenceRule(frequency: .weekly); used = 1
        case "zweiwöchentlich", "14-tägig", "vierzehntägig": rule = RecurrenceRule(frequency: .weekly, interval: 2); used = 1
        case "monatlich": rule = RecurrenceRule(frequency: .monthly); used = 1
        case "jährlich": rule = RecurrenceRule(frequency: .yearly); used = 1
        case "jeden", "jede", "jedes", "jeder":
            guard let (r, n) = recurrenceAfterEvery(i + 1) else { return nil }
            rule = r; used = 1 + n
        case "alle":
            guard let n = lower(i + 1).flatMap(GermanLexicon.number), n >= 1,
                  let unit = lower(i + 2).flatMap(Self.frequency(plural:)) else { return nil }
            rule = RecurrenceRule(frequency: unit, interval: n); used = 3
        default:
            guard let (days, n) = weekdayList(at: i, plural: true),
                  GermanLexicon.weekdayPlurals[w] != nil else { return nil }
            rule = RecurrenceRule(frequency: .weekly, weekdays: days); used = n
        }

        // „jede Woche am Mo“, „alle 2 Wochen Di und Do“
        if rule.frequency == .weekly, rule.weekdays.isEmpty {
            var j = i + used
            if lower(j) == "am" { j += 1 }
            if let (days, n) = weekdayList(at: j, plural: true) {
                rule.weekdays = days.sortedUnique()
                used = j + n - i
            }
        }
        d.recurrence = rule
        return used
    }

    private func recurrenceAfterEvery(_ j: Int) -> (RecurrenceRule, Int)? {
        guard let w = lower(j) else { return nil }
        switch w {
        case "tag": return (.daily, 1)
        case "werktag": return (.workdays, 1)
        case "woche": return (RecurrenceRule(frequency: .weekly), 1)
        case "monat": return (RecurrenceRule(frequency: .monthly), 1)
        case "jahr": return (RecurrenceRule(frequency: .yearly), 1)
        default: break
        }
        // „jede zweite Woche“, „jeden 2. Tag“
        let ordinal = GermanLexicon.ordinalWords[w] ?? (w.hasSuffix(".") ? Int(w.dropLast()) : nil)
        if let n = ordinal, n >= 1, let unit = lower(j + 1).flatMap(Self.frequency(singular:)) {
            return (RecurrenceRule(frequency: unit, interval: n), 2)
        }
        if let (days, n) = weekdayList(at: j, plural: false) {
            return (RecurrenceRule(frequency: .weekly, weekdays: days), n)
        }
        return nil
    }

    static func frequency(plural w: String) -> RecurrenceRule.Frequency? {
        switch w {
        case "tage": .daily
        case "wochen": .weekly
        case "monate": .monthly
        case "jahre": .yearly
        default: nil
        }
    }

    static func frequency(singular w: String) -> RecurrenceRule.Frequency? {
        switch w {
        case "tag": .daily
        case "woche": .weekly
        case "monat": .monthly
        case "jahr": .yearly
        default: nil
        }
    }

    /// „… bis 30.11.“ nach einer Wiederholung.
    func matchUntil(_ i: Int, _ d: inout Draft) -> Int? {
        guard d.recurrence != nil, d.until == nil, tokens[i].lower == "bis" else { return nil }
        var j = i + 1
        if ["zum", "einschließlich"].contains(lower(j)) { j += 1 }
        guard let (date, n) = partialDate(at: j, allowDayOnly: true) else { return nil }
        d.until = date
        return j + n - i
    }

    // MARK: Datum

    func matchDate(_ i: Int, _ d: inout Draft) -> Int? {
        guard d.day == nil else { return nil }
        var j = i
        let prefix = tokens[i].lower
        let hasPrefix = ["am", "vom", "ab"].contains(prefix)
        if hasPrefix { j += 1 }
        guard j < tokens.count else { return nil }
        let w = tokens[j].lower

        if !hasPrefix {
            switch w {
            case "heute": d.day = .relativeDays(0); return 1
            case "morgen", "morgen.":
                d.day = .relativeDays(1); return 1
            case "übermorgen": d.day = .relativeDays(2); return 1
            case "gestern": d.day = .relativeDays(-1); return 1
            default: break
            }
            if w == "in", let n = lower(i + 1).flatMap(GermanLexicon.number), let unit = lower(i + 2) {
                switch unit {
                case "tag", "tagen": d.day = .relativeDays(n); return 3
                case "woche", "wochen": d.day = .relativeDays(7 * n); return 3
                case "monat", "monaten": d.day = .relativeMonths(n); return 3
                default: break
                }
            }
            if let n = matchRelativeWeek(i, &d) { return n }
        }

        // Wochentag, auch „Dienstagabend“, optional mit Datum dahinter („Di 3.11.“)
        if let wd = weekday(at: j) ?? compoundWeekday(at: j, &d) {
            if let (date, n) = partialDate(at: j + 1, allowDayOnly: false) {
                d.day = .absolute(date)
                return j + 1 + n - i
            }
            d.day = .weekday(wd, strict: false)
            return j + 1 - i
        }

        // Kalenderdatum, ggf. als Spanne
        if let (first, last, n) = dateRange(at: j) {
            d.day = .absolute(first)
            d.lastDay = last
            d.allDay = true
            return j + n - i
        }
        guard let (date, n) = partialDate(at: j, allowDayOnly: hasPrefix) else { return nil }
        d.day = .absolute(date)
        return j + n - i
    }

    /// „nächste Woche Mi“, „nächsten Dienstag“, „diese Woche“.
    private func matchRelativeWeek(_ i: Int, _ d: inout Draft) -> Int? {
        let w = tokens[i].lower
        let offset: Int
        switch w {
        case "nächste", "nächsten", "nächster", "kommende", "kommenden": offset = 1
        case "übernächste", "übernächsten": offset = 2
        case "diese", "diesen", "dieser": offset = 0
        default: return nil
        }
        if lower(i + 1) == "woche" {
            if let wd = weekday(at: i + 2) {
                d.day = .week(offset: offset, weekday: wd); return 3
            }
            d.day = .week(offset: offset, weekday: nil); return 2
        }
        if let wd = weekday(at: i + 1) {
            d.day = offset == 2 ? .week(offset: 2, weekday: wd) : .weekday(wd, strict: offset == 1)
            return 2
        }
        return nil
    }

    /// „Dienstagabend“, „Freitagnachmittag“: Wochentag plus Tageszeit.
    private func compoundWeekday(at j: Int, _ d: inout Draft) -> Weekday? {
        guard j < tokens.count else { return nil }
        let w = tokens[j].lower
        for (name, wd) in GermanLexicon.weekdayNames where w.hasPrefix(name) {
            let rest = String(w.dropFirst(name.count))
            if let (_, part) = GermanLexicon.daypartSuffixes.first(where: { $0.0 == rest || $0.0 + "s" == rest }) {
                if d.daypart == nil { d.daypart = part }
                return wd
            }
        }
        return nil
    }

    /// Ein Datum aus einem oder mehreren Wörtern: „3.11.“, „3.11.2026“,
    /// „3. November“, „3 Nov 2027“, „2026-11-03“; mit `allowDayOnly` auch „3.“.
    func partialDate(at j: Int, allowDayOnly: Bool) -> (PartialDate, Int)? {
        guard j < tokens.count else { return nil }
        let w = tokens[j].lower
        if let iso = Self.isoDate(w) { return (iso, 1) }
        if let p = Self.numericDate(w), p.month != nil { return (p, 1) }

        // „3.“ oder „3“ gefolgt von einem Monatsnamen
        let dayText = w.hasSuffix(".") ? String(w.dropLast()) : w
        if let day = Int(dayText), (1...31).contains(day) {
            if let month = monthName(at: j + 1) {
                var p = PartialDate(day: day, month: month)
                if let y = lower(j + 2).flatMap(Self.longYear) {
                    p.year = y
                    return (p, 3)
                }
                return (p, 2)
            }
            if allowDayOnly, w.hasSuffix(".") { return (PartialDate(day: day), 1) }
        }
        return nil
    }

    private func monthName(at j: Int) -> Int? {
        guard var w = lower(j) else { return nil }
        if w.hasSuffix(".") { w.removeLast() }
        return GermanLexicon.months[w]
    }

    /// „3.–7.11.“, „3.11.-7.11.“, „3. bis 7. November“, „vom 3. bis 7.11.“
    func dateRange(at j: Int) -> (PartialDate, PartialDate, Int)? {
        guard j < tokens.count else { return nil }
        let w = tokens[j].lower

        // in einem Wort
        let halves = w.split(separator: "-", omittingEmptySubsequences: false)
        if halves.count == 2, !halves[0].isEmpty, Self.isoDate(w) == nil,
           var first = Self.numericDate(String(halves[0]), allowDayOnly: true) {
            if !halves[1].isEmpty, var last = Self.numericDate(String(halves[1]), allowDayOnly: true) {
                var used = 1
                if last.month == nil, let m = monthName(at: j + 1) {
                    last.month = m
                    used = 2
                    if let y = lower(j + 2).flatMap(Self.longYear) { last.year = y; used = 3 }
                }
                guard last.month != nil else { return nil }
                first.month = first.month ?? last.month
                first.year = first.year ?? last.year
                return (first, last, used)
            }
            // „3.-“ „7.11.“
            if halves[1].isEmpty, let (last, n) = partialDate(at: j + 1, allowDayOnly: false) {
                first.month = first.month ?? last.month
                first.year = first.year ?? last.year
                return (first, last, 1 + n)
            }
            return nil
        }

        // über mehrere Wörter: Anfang, „-“ oder „bis“, Ende
        guard let (start, n1) = partialDate(at: j, allowDayOnly: true) else { return nil }
        guard let sep = lower(j + n1), sep == "-" || sep == "bis" else { return nil }
        guard let (last, n2) = partialDate(at: j + n1 + 1, allowDayOnly: false) else { return nil }
        var first = start
        first.month = first.month ?? last.month
        first.year = first.year ?? last.year
        return (first, last, n1 + 1 + n2)
    }

    static func numericDate(_ w: String, allowDayOnly: Bool = false) -> PartialDate? {
        let parts = w.split(separator: ".", omittingEmptySubsequences: false).map(String.init)
        // „3.“ → ["3", ""]; „3.11.“ → ["3","11",""]; „3.11.2026“ → ["3","11","2026"]; „3.11“ → ["3","11"]
        guard parts.count >= 2, parts.count <= 3, let day = Int(parts[0]), (1...31).contains(day) else { return nil }
        if parts.count == 2 {
            if parts[1].isEmpty { return allowDayOnly ? PartialDate(day: day) : nil }
            guard let month = Int(parts[1]), (1...12).contains(month), parts[1].count <= 2 else { return nil }
            return PartialDate(day: day, month: month)
        }
        guard let month = Int(parts[1]), (1...12).contains(month), parts[1].count <= 2 else { return nil }
        if parts[2].isEmpty { return PartialDate(day: day, month: month) }
        guard let y = year(parts[2]) else { return nil }
        return PartialDate(day: day, month: month, year: y)
    }

    static func isoDate(_ w: String) -> PartialDate? {
        let parts = w.split(separator: "-").map(String.init)
        guard parts.count == 3, parts[0].count == 4, let y = Int(parts[0]),
              let m = Int(parts[1]), let d = Int(parts[2]), (1...12).contains(m), (1...31).contains(d) else { return nil }
        return PartialDate(day: d, month: m, year: y)
    }

    /// Nach einem Monatsnamen nur vierstellig: „5. November 10 Uhr“ ist kein Jahr 2010.
    static func longYear(_ w: String) -> Int? {
        w.count == 4 ? year(w) : nil
    }

    static func year(_ w: String) -> Int? {
        guard let n = Int(w) else { return nil }
        switch w.count {
        case 2: return 2000 + n
        case 4 where n >= 1900: return n
        default: return nil
        }
    }

    // MARK: Uhrzeit

    /// „14“, „14:30“, „14.30“ (Punkt nur, wenn `dotAllowed`), „14uhr“.
    static func clock(_ raw: String, dotAllowed: Bool) -> (Clock, hadUhr: Bool, hadMinutes: Bool)? {
        var w = raw
        var hadUhr = false
        if w.hasSuffix("uhr"), w.count > 3 { w.removeLast(3); hadUhr = true }
        let separators: [Character] = dotAllowed || hadUhr ? [":", "."] : [":"]
        if let sep = w.firstIndex(where: { separators.contains($0) }) {
            let h = String(w[..<sep]), m = String(w[w.index(after: sep)...])
            guard let hour = Int(h), let minute = Int(m), h.count <= 2, m.count == 2,
                  (0...24).contains(hour), (0...59).contains(minute), hour < 24 || minute == 0 else { return nil }
            return (Clock(hour: hour, minute: minute), hadUhr, true)
        }
        guard let hour = Int(w), w.count <= 2, (0...24).contains(hour) else { return nil }
        return (Clock(hour: hour, minute: 0), hadUhr, false)
    }

    /// „9–9:30“, „9:00-10:30“, „von 9 bis 10 Uhr“, „14-15 Uhr“.
    func matchTimeRange(_ i: Int, _ d: inout Draft) -> Int? {
        guard d.start == nil else { return nil }
        var j = i
        let hasPrefix = ["von", "um", "ab"].contains(tokens[i].lower)
        if hasPrefix { j += 1 }
        guard j < tokens.count else { return nil }

        var a: (Clock, hadUhr: Bool, hadMinutes: Bool)
        var b: (Clock, hadUhr: Bool, hadMinutes: Bool)
        var next: Int
        let halves = tokens[j].lower.split(separator: "-", omittingEmptySubsequences: false).map(String.init)
        if halves.count == 2, !halves[0].isEmpty {
            guard let x = Self.clock(halves[0], dotAllowed: true) else { return nil }
            a = x
            if halves[1].isEmpty {
                guard let y = lower(j + 1).flatMap({ Self.clock($0, dotAllowed: true) }) else { return nil }
                b = y; next = j + 2
            } else {
                guard let y = Self.clock(halves[1], dotAllowed: true) else { return nil }
                b = y; next = j + 1
            }
        } else {
            guard let x = Self.clock(tokens[j].lower, dotAllowed: true),
                  let sep = lower(j + 1), sep == "-" || sep == "bis",
                  let y = lower(j + 2).flatMap({ Self.clock($0, dotAllowed: true) }) else { return nil }
            a = x; b = y; next = j + 3
        }
        var hadUhr = a.hadUhr || b.hadUhr
        if lower(next) == "uhr" { hadUhr = true; next += 1 }
        guard hasPrefix || hadUhr || a.hadMinutes || b.hadMinutes || daypartFollows(next) else { return nil }
        d.start = a.0
        d.end = b.0
        return next - i
    }

    /// „14 Uhr“, „um 14“, „7:30“, „um halb 8“, „viertel nach 3“.
    func matchTime(_ i: Int, _ d: inout Draft) -> Int? {
        // „20 Uhr bis 2 Uhr“: Ende nach bereits erkanntem Anfang
        if d.start != nil, d.end == nil, tokens[i].lower == "bis", let w = lower(i + 1),
           let (c, hadUhr, hadMinutes) = Self.clock(w, dotAllowed: true) {
            let nextIsUhr = lower(i + 2) == "uhr"
            guard hadUhr || hadMinutes || nextIsUhr else { return nil }
            d.end = c
            return 2 + (nextIsUhr ? 1 : 0)
        }
        guard d.start == nil else { return nil }
        var j = i
        let hasPrefix = ["um", "gegen", "ab"].contains(tokens[i].lower)
        if hasPrefix { j += 1 }
        guard let w = lower(j) else { return nil }

        // Gesprochene Uhrzeiten sind 12-Stunden-Zeiten: „halb 3“ ist 14:30,
        // „halb 8“ ist 7:30. Unter 7 Uhr gilt der Nachmittag.
        func spoken(_ c: Clock) -> Clock {
            c.hour < 7 ? Clock(hour: c.hour + 12, minute: c.minute) : c
        }
        if w == "halb", let h = lower(j + 1).flatMap(GermanLexicon.number), (1...12).contains(h) {
            d.start = spoken(Clock(hour: h - 1, minute: 30))
            return j + 2 + (lower(j + 2) == "uhr" ? 1 : 0) - i
        }
        if w == "viertel", let dir = lower(j + 1), dir == "nach" || dir == "vor",
           let h = lower(j + 2).flatMap(GermanLexicon.number), (1...12).contains(h) {
            d.start = spoken(dir == "nach" ? Clock(hour: h, minute: 15) : Clock(hour: h - 1, minute: 45))
            return j + 3 + (lower(j + 3) == "uhr" ? 1 : 0) - i
        }

        let nextIsUhr = lower(j + 1) == "uhr"
        let parsed = Self.clock(w, dotAllowed: hasPrefix || nextIsUhr)
            ?? (hasPrefix ? GermanLexicon.numberWords[w].map { (Clock(hour: $0, minute: 0), false, false) } : nil)
        guard let (c, hadUhr, hadMinutes) = parsed, c.hour < 24 else { return nil }
        guard hasPrefix || hadUhr || nextIsUhr || hadMinutes || daypartFollows(j + 1) else { return nil }
        d.start = c
        return j + 1 + (nextIsUhr ? 1 : 0) - i
    }

    /// „8 abends“: Eine Tageszeit macht aus der Zahl davor eine Uhrzeit.
    func daypartFollows(_ j: Int) -> Bool {
        lower(j).map { GermanLexicon.dayparts[$0] != nil } ?? false
    }

    func matchDaypart(_ i: Int, _ d: inout Draft) -> Int? {
        guard d.daypart == nil else { return nil }
        var j = i
        if tokens[i].lower == "am" { j += 1 }
        guard let w = lower(j) else { return nil }
        // „am Morgen“ ist eine Tageszeit, „morgen“ allein ist der Tag danach.
        guard let part = GermanLexicon.dayparts[w] ?? (j > i && w == "morgen" ? .early : nil) else { return nil }
        d.daypart = part
        return j + 1 - i
    }

    // MARK: Dauer

    /// „für 2 Stunden“, „90 min“, „1,5h“, „eine halbe Stunde“.
    func matchDuration(_ i: Int, _ d: inout Draft) -> Int? {
        guard d.durationMinutes == nil else { return nil }
        var j = i
        if tokens[i].lower == "für" { j += 1 }
        guard let w = lower(j) else { return nil }

        let phrases: [([String], Int)] = [
            (["eine", "halbe", "stunde"], 30), (["halbe", "stunde"], 30),
            (["eine", "viertelstunde"], 15), (["viertelstunde"], 15),
            (["eine", "dreiviertelstunde"], 45), (["dreiviertelstunde"], 45),
            (["anderthalb", "stunden"], 90), (["eineinhalb", "stunden"], 90),
        ]
        for (words, minutes) in phrases where (0..<words.count).allSatisfy({ lower(j + $0) == words[$0] }) {
            d.durationMinutes = minutes
            return j + words.count - i
        }

        // „90min“, „2h“, „1,5std“
        if let (value, unit) = Self.splitNumberUnit(w), let minutes = Self.minutes(value, unit: unit) {
            d.durationMinutes = minutes
            return j + 1 - i
        }
        // „2 Stunden“, „zwei Stunden“, „1,5 Std.“
        if let value = Self.decimal(w) ?? GermanLexicon.numberWords[w].map(Double.init),
           let unit = lower(j + 1), let minutes = Self.minutes(value, unit: unit) {
            d.durationMinutes = minutes
            return j + 2 - i
        }
        return nil
    }

    static func decimal(_ w: String) -> Double? {
        Double(w.replacingOccurrences(of: ",", with: "."))
    }

    static func splitNumberUnit(_ w: String) -> (Double, String)? {
        guard let idx = w.firstIndex(where: { $0.isLetter }), idx != w.startIndex,
              let value = decimal(String(w[..<idx])) else { return nil }
        return (value, String(w[idx...]))
    }

    static func minutes(_ value: Double, unit: String) -> Int? {
        let u = unit.hasSuffix(".") ? String(unit.dropLast()) : unit
        let factor: Double
        switch u {
        case "h", "std", "stunde", "stunden": factor = 60
        case "min", "minute", "minuten": factor = 1
        default: return nil
        }
        let minutes = Int((value * factor).rounded())
        return minutes > 0 ? minutes : nil
    }

    func matchAllDay(_ i: Int, _ d: inout Draft) -> Int? {
        guard ["ganztägig", "ganztags"].contains(tokens[i].lower) else { return nil }
        d.allDay = true
        return 1
    }

    // MARK: Ort

    /// „in Raum 2“, „im Café Sonne“, „@Büro“. Endet am nächsten erkannten Teil.
    func matchLocation(_ i: Int, _ d: inout Draft) -> Int? {
        guard d.location == nil else { return nil }
        let t = tokens[i]
        var words: [String] = []
        var j = i + 1
        if t.lower.hasPrefix("@"), t.text.count > 1 {
            words.append(String(t.text.dropFirst()))
            if t.trailingComma { d.location = words.joined(); return 1 }
        } else if t.lower == "in" || t.lower == "im" || t.lower == "@" {
            if t.lower == "in", let a = lower(j), GermanLexicon.articles.contains(a) { j += 1 }
        } else {
            return nil
        }
        while j < tokens.count {
            let w = tokens[j]
            if GermanLexicon.locationStops.contains(w.lower) { break }
            if firstMatch(at: j, includingLocation: false) != nil { break }
            words.append(w.text.trimmingCharacters(in: CharacterSet(charactersIn: ",;")))
            j += 1
            if w.trailingComma { break }
        }
        guard !words.isEmpty else { return nil }
        d.location = words.joined(separator: " ")
        return j - i
    }
}
