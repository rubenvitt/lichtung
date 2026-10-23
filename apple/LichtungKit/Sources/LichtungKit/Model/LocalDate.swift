import Foundation

/// Ein Kalendertag ohne Uhrzeit und ohne Zeitzone (iCalendar `VALUE=DATE`).
///
/// Ganztägige Termine hängen an solchen Tagen, nicht an einem `Date`: Ein
/// Urlaub vom 3. bis 7. November bleibt auch in New York der 3. bis 7. November.
public struct LocalDate: Hashable, Comparable, Sendable, Codable, CustomStringConvertible {
    public var year: Int
    public var month: Int
    public var day: Int

    public init(year: Int, month: Int, day: Int) {
        self.year = year
        self.month = month
        self.day = day
    }

    /// Kalendertag eines Zeitpunkts in der Zeitzone des Kalenders.
    public init(_ date: Date, calendar: Calendar) {
        let parts = calendar.dateComponents([.year, .month, .day], from: date)
        self.init(year: parts.year!, month: parts.month!, day: parts.day!)
    }

    /// Prüft, ob es den Tag gibt (kein 31. April, 29. Februar nur im Schaltjahr).
    public static func isValid(year: Int, month: Int, day: Int) -> Bool {
        guard (1...12).contains(month), day >= 1 else { return false }
        return day <= daysInMonth(year: year, month: month)
    }

    public static func daysInMonth(year: Int, month: Int) -> Int {
        switch month {
        case 1, 3, 5, 7, 8, 10, 12: return 31
        case 4, 6, 9, 11: return 30
        default:
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
            return leap ? 29 : 28
        }
    }

    /// Mitternacht dieses Tages in der Zeitzone des Kalenders.
    public func startDate(in calendar: Calendar) -> Date {
        calendar.date(from: DateComponents(year: year, month: month, day: day))!
    }

    /// Zeitpunkt an diesem Tag zur Wanduhrzeit `hour:minute`.
    ///
    /// Fällt die Uhrzeit in die Lücke der Sommerzeit-Umstellung, liefert
    /// `Calendar` den nächsten gültigen Zeitpunkt.
    public func date(hour: Int, minute: Int, in calendar: Calendar) -> Date {
        calendar.date(from: DateComponents(year: year, month: month, day: day, hour: hour, minute: minute))!
    }

    public func adding(days: Int) -> LocalDate {
        // Rechnen in UTC, damit Sommerzeit nichts verschiebt.
        let cal = Calendar.gregorianUTC
        let shifted = cal.date(byAdding: .day, value: days, to: startDate(in: cal))!
        return LocalDate(shifted, calendar: cal)
    }

    public func adding(months: Int) -> LocalDate {
        var m = month - 1 + months
        var y = year + m / 12
        m %= 12
        if m < 0 { m += 12; y -= 1 }
        let d = min(day, LocalDate.daysInMonth(year: y, month: m + 1))
        return LocalDate(year: y, month: m + 1, day: d)
    }

    /// Tage von `self` bis `other` (positiv, wenn `other` später liegt).
    public func days(until other: LocalDate) -> Int {
        let cal = Calendar.gregorianUTC
        return cal.dateComponents([.day], from: startDate(in: cal), to: other.startDate(in: cal)).day!
    }

    public var weekday: Weekday {
        let cal = Calendar.gregorianUTC
        return Weekday(calendarWeekday: cal.component(.weekday, from: startDate(in: cal)))
    }

    public static func < (lhs: LocalDate, rhs: LocalDate) -> Bool {
        (lhs.year, lhs.month, lhs.day) < (rhs.year, rhs.month, rhs.day)
    }

    /// ISO-Form `2026-11-03`.
    public var description: String {
        String(format: "%04d-%02d-%02d", year, month, day)
    }

    /// iCalendar-Form `20261103`.
    public var icalString: String {
        String(format: "%04d%02d%02d", year, month, day)
    }
}

/// Wochentag in ISO-Reihenfolge (Montag zuerst).
public enum Weekday: Int, CaseIterable, Comparable, Sendable, Codable {
    case monday = 1, tuesday, wednesday, thursday, friday, saturday, sunday

    /// Aus `Calendar.component(.weekday)` (1 = Sonntag … 7 = Samstag).
    public init(calendarWeekday: Int) {
        self = calendarWeekday == 1 ? .sunday : Weekday(rawValue: calendarWeekday - 1)!
    }

    public static let workdays: [Weekday] = [.monday, .tuesday, .wednesday, .thursday, .friday]

    /// Kürzel für `BYDAY` in einer RRULE.
    public var icalCode: String {
        ["MO", "TU", "WE", "TH", "FR", "SA", "SU"][rawValue - 1]
    }

    public static func < (lhs: Weekday, rhs: Weekday) -> Bool { lhs.rawValue < rhs.rawValue }
}

extension Calendar {
    /// Gregorianischer Kalender in UTC, für reine Tagesrechnung.
    static let gregorianUTC: Calendar = {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        return cal
    }()

    /// Gregorianischer Kalender mit Montag als Wochenbeginn in der gegebenen Zeitzone.
    public static func lichtung(timeZone: TimeZone) -> Calendar {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = timeZone
        cal.firstWeekday = 2
        cal.minimumDaysInFirstWeek = 4
        cal.locale = Locale(identifier: "de_DE")
        return cal
    }
}
