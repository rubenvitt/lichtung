import Foundation

/// Text der Vorschau unter dem Eingabefeld, z. B.
/// „Di, 27. Okt. · 14:00–15:00“ und „jeden Montag“.
///
/// Bewusst ohne `DateFormatter`: Die Vorschau ist Teil der Tabellentests und
/// soll auf jeder Plattform und mit jeder System-Locale gleich aussehen.
public struct QuickEntryPreview: Equatable, Sendable {
    public var title: String
    public var when: String
    public var recurrence: String?
    public var location: String?
    public var calendarName: String?

    public init(_ entry: QuickEntry, now: Date, timeZone: TimeZone, calendars: [CalendarChoice] = []) {
        let cal = Calendar.lichtung(timeZone: timeZone)
        let today = LocalDate(now, calendar: cal)
        title = entry.title.isEmpty ? "Neuer Termin" : entry.title
        location = entry.location
        calendarName = entry.calendarID.flatMap { id in calendars.first { $0.id == id }?.name }
            ?? entry.calendarQuery.map { "„\($0)“ (unbekannt)" }

        switch entry.timing {
        case .unspecified:
            when = "Tag offen"
        case let .allDay(first, last):
            let span = first == last
                ? Self.day(first, today: today)
                : "\(Self.day(first, today: today)) – \(Self.day(last, today: today))"
            when = "\(span) · ganztägig"
        case let .timed(start, end):
            let startDay = LocalDate(start, calendar: cal)
            // Endet um Mitternacht: gehört noch zum Vortag.
            let endDay = LocalDate(end.addingTimeInterval(-1), calendar: cal)
            let from = Self.clock(start, cal)
            let to = Self.clock(end, cal)
            if startDay == endDay {
                when = "\(Self.day(startDay, today: today)) · \(from)–\(to)"
            } else {
                when = "\(Self.day(startDay, today: today)) \(from) – \(Self.day(endDay, today: today)) \(to)"
            }
        }
        recurrence = entry.recurrence.map(Self.describe)
    }

    static let weekdayShort = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]
    static let weekdayLong = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"]
    static let monthShort = ["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.", "Dez."]

    static func day(_ d: LocalDate, today: LocalDate) -> String {
        let base = "\(weekdayShort[d.weekday.rawValue - 1]), \(d.day). \(monthShort[d.month - 1])"
        let prefix: String? = switch today.days(until: d) {
        case 0: "Heute"
        case 1: "Morgen"
        default: nil
        }
        let year = d.year == today.year ? "" : " \(d.year)"
        return (prefix.map { "\($0), " } ?? "") + base + year
    }

    static func clock(_ date: Date, _ cal: Calendar) -> String {
        let c = cal.dateComponents([.hour, .minute], from: date)
        return String(format: "%02d:%02d", c.hour!, c.minute!)
    }

    static func describe(_ rule: RecurrenceRule) -> String {
        var text: String
        let days = rule.weekdays.map { weekdayLong[$0.rawValue - 1] }
        let dayList = days.count <= 1
            ? days.first ?? ""
            : days.dropLast().joined(separator: ", ") + " und " + days.last!
        switch (rule.frequency, rule.interval) {
        case (.daily, 1): text = "täglich"
        case (.daily, let n): text = "alle \(n) Tage"
        case (.weekly, 1) where rule.weekdays == Weekday.workdays: text = "werktags"
        case (.weekly, 1): text = days.isEmpty ? "wöchentlich" : "jeden \(dayList)"
        case (.weekly, let n): text = "alle \(n) Wochen" + (days.isEmpty ? "" : " am \(dayList)")
        case (.monthly, 1): text = "monatlich"
        case (.monthly, let n): text = "alle \(n) Monate"
        case (.yearly, 1): text = "jährlich"
        case (.yearly, let n): text = "alle \(n) Jahre"
        }
        if let u = rule.until {
            text += " bis \(u.day). \(monthShort[u.month - 1]) \(u.year)"
        }
        return text
    }
}
