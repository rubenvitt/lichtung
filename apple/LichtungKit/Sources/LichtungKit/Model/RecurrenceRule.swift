import Foundation

/// Wiederholungsregel, die Teilmenge von RFC 5545 `RRULE`, die Lichtung selbst
/// erzeugt (Satz-Parser, Termin-Editor).
///
/// Serien, die von den Anbietern kommen, behält der Server als iCalendar-Original;
/// dieses Modell muss nicht jede denkbare RRULE abbilden.
public struct RecurrenceRule: Hashable, Sendable, Codable {
    public enum Frequency: String, Sendable, Codable {
        case daily = "DAILY"
        case weekly = "WEEKLY"
        case monthly = "MONTHLY"
        case yearly = "YEARLY"
    }

    public var frequency: Frequency
    public var interval: Int
    /// Nur bei `weekly`: Wochentage. Leer heißt: Wochentag des Serienbeginns.
    public var weekdays: [Weekday]
    /// Letzter Tag, an dem noch ein Vorkommen liegen darf (inklusive).
    public var until: LocalDate?

    public init(frequency: Frequency, interval: Int = 1, weekdays: [Weekday] = [], until: LocalDate? = nil) {
        self.frequency = frequency
        self.interval = max(1, interval)
        self.weekdays = weekdays.sortedUnique()
        self.until = until
    }

    public static let daily = RecurrenceRule(frequency: .daily)
    public static let workdays = RecurrenceRule(frequency: .weekly, weekdays: Weekday.workdays)

    public static func weekly(_ days: Weekday..., interval: Int = 1) -> RecurrenceRule {
        RecurrenceRule(frequency: .weekly, interval: interval, weekdays: days)
    }

    /// RRULE-Wert ohne `RRULE:`-Präfix, z. B. `FREQ=WEEKLY;BYDAY=MO`.
    ///
    /// `UNTIL` muss nach RFC 5545 denselben Typ haben wie `DTSTART`: Bei
    /// ganztägigen Serien ein Datum, sonst ein UTC-Zeitpunkt (Ende des Tages in
    /// `timeZone`).
    public func rruleString(allDay: Bool, timeZone: TimeZone) -> String {
        var parts = ["FREQ=\(frequency.rawValue)"]
        if interval > 1 { parts.append("INTERVAL=\(interval)") }
        if frequency == .weekly, !weekdays.isEmpty {
            parts.append("BYDAY=" + weekdays.map(\.icalCode).joined(separator: ","))
        }
        if let until {
            if allDay {
                parts.append("UNTIL=\(until.icalString)")
            } else {
                let cal = Calendar.lichtung(timeZone: timeZone)
                let end = until.date(hour: 23, minute: 59, in: cal).addingTimeInterval(59)
                parts.append("UNTIL=\(Self.utcStamp(end))")
            }
        }
        return parts.joined(separator: ";")
    }

    private static func utcStamp(_ date: Date) -> String {
        let c = Calendar.gregorianUTC.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        return String(format: "%04d%02d%02dT%02d%02d%02dZ", c.year!, c.month!, c.day!, c.hour!, c.minute!, c.second!)
    }
}

extension Array where Element == Weekday {
    func sortedUnique() -> [Weekday] { Array(Set(self)).sorted() }
}
