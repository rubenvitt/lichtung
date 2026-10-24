import Foundation

/// Wanduhrzeit ohne Zeitzone, z. B. `2026-11-03T14:00:00`.
public struct LocalDateTime: Hashable, Comparable, Sendable, CustomStringConvertible {
    public var date: LocalDate
    public var hour: Int
    public var minute: Int
    public var second: Int

    public init(date: LocalDate, hour: Int, minute: Int, second: Int = 0) {
        self.date = date
        self.hour = hour
        self.minute = minute
        self.second = second
    }

    public init(_ instant: Date, calendar: Calendar) {
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: instant)
        self.init(date: LocalDate(year: c.year!, month: c.month!, day: c.day!), hour: c.hour!, minute: c.minute!, second: c.second!)
    }

    /// `2026-11-03T14:00:00` (auch mit Bruchteilen von Sekunden, die verworfen werden).
    public init?(iso text: String) {
        let parts = text.split(separator: "T", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }
        let d = parts[0].split(separator: "-").compactMap { Int($0) }
        let t = parts[1].split(separator: ":").map(String.init)
        guard d.count == 3, t.count >= 2, let h = Int(t[0]), let m = Int(t[1]) else { return nil }
        let s = t.count > 2 ? Int(Double(t[2]) ?? -1) : 0
        guard LocalDate.isValid(year: d[0], month: d[1], day: d[2]),
              (0...23).contains(h), (0...59).contains(m), (0...60).contains(s) else { return nil }
        self.init(date: LocalDate(year: d[0], month: d[1], day: d[2]), hour: h, minute: m, second: s)
    }

    /// Zeitpunkt dieser Wanduhrzeit in `calendar`s Zeitzone. Liegt sie in der
    /// Lücke der Sommerzeit-Umstellung, gilt der nächste gültige Zeitpunkt;
    /// liegt sie doppelt vor (Herbst), der erste.
    public func instant(in calendar: Calendar) -> Date {
        calendar.date(from: DateComponents(
            year: date.year, month: date.month, day: date.day,
            hour: hour, minute: minute, second: second
        ))!
    }

    public func adding(days: Int) -> LocalDateTime {
        LocalDateTime(date: date.adding(days: days), hour: hour, minute: minute, second: second)
    }

    public var secondsOfDay: Int { hour * 3600 + minute * 60 + second }

    public static func < (lhs: LocalDateTime, rhs: LocalDateTime) -> Bool {
        (lhs.date, lhs.secondsOfDay) < (rhs.date, rhs.secondsOfDay)
    }

    public var description: String {
        "\(date)T" + String(format: "%02d:%02d:%02d", hour, minute, second)
    }
}

/// Beginn oder Ende eines Termins (Vertrag §1 „Datumswerte“).
///
/// Entweder ein Tag (ganztägig) oder eine Wanduhrzeit in einer IANA-Zeitzone.
/// `timeZone == nil` heißt floating: gilt in der Zeitzone des Geräts.
public enum DateValue: Hashable, Sendable {
    case date(LocalDate)
    case dateTime(LocalDateTime, timeZone: String?, utc: Date? = nil)

    public var isAllDay: Bool {
        if case .date = self { return true }
        return false
    }

    public var timeZoneID: String? {
        if case let .dateTime(_, tz, _) = self { return tz }
        return nil
    }

    /// Zeitpunkt für Sortierung und Anzeige. Ganztägige und floating Werte
    /// gelten in `deviceTimeZone`.
    public func instant(deviceTimeZone: TimeZone) -> Date {
        switch self {
        case let .date(d):
            return d.startDate(in: .lichtung(timeZone: deviceTimeZone))
        case let .dateTime(local, tzID, _):
            let tz = tzID.flatMap(TimeZone.init(identifier:)) ?? deviceTimeZone
            return local.instant(in: .lichtung(timeZone: tz))
        }
    }

    /// Gleicher Wert ohne den vom Server gelieferten `utc`-Hinweis.
    /// Zwei Werte mit gleicher Wanduhrzeit und Zone sind damit vergleichbar.
    public var normalized: DateValue {
        if case let .dateTime(l, tz, _) = self { return .dateTime(l, timeZone: tz, utc: nil) }
        return self
    }
}

extension DateValue: Codable {
    private enum CodingKeys: String, CodingKey {
        case date, dateTime, timeZone, utc
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        if let text = try c.decodeIfPresent(String.self, forKey: .date) {
            guard let d = LocalDate(iso: text) else {
                throw DecodingError.dataCorruptedError(forKey: .date, in: c, debugDescription: "Kein ISO-Datum: \(text)")
            }
            self = .date(d)
            return
        }
        let text = try c.decode(String.self, forKey: .dateTime)
        guard let local = LocalDateTime(iso: text) else {
            throw DecodingError.dataCorruptedError(forKey: .dateTime, in: c, debugDescription: "Keine Wanduhrzeit: \(text)")
        }
        let tz = try c.decodeIfPresent(String.self, forKey: .timeZone)
        let utc = try c.decodeIfPresent(Date.self, forKey: .utc)
        self = .dateTime(local, timeZone: tz, utc: utc)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .date(d):
            try c.encode(d.description, forKey: .date)
        case let .dateTime(local, tz, utc):
            try c.encode(local.description, forKey: .dateTime)
            // `null` heißt floating und muss mitgeschickt werden.
            try c.encode(tz, forKey: .timeZone)
            try c.encodeIfPresent(utc, forKey: .utc)
        }
    }
}

extension LocalDate {
    /// `2026-11-03`.
    public init?(iso text: String) {
        let d = text.split(separator: "-").compactMap { Int($0) }
        guard d.count == 3, LocalDate.isValid(year: d[0], month: d[1], day: d[2]) else { return nil }
        self.init(year: d[0], month: d[1], day: d[2])
    }
}
