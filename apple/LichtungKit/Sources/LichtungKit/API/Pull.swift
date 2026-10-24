import Foundation

// Lesen über `GET /api/sync/pull` (Vertrag §4).

public enum SyncChange: Hashable, Sendable {
    case upsertAccount(Account)
    case upsertCalendar(CalendarInfo)
    case upsertCalendarSet(CalendarSet)
    case upsertEvent(Event)
    case upsertConflict(Conflict)
    case delete(type: String, id: String)
    /// Objektart, die dieser Client noch nicht kennt; wird übersprungen.
    case unknown(type: String)
}

extension SyncChange: Decodable {
    private enum CodingKeys: String, CodingKey { case type, op, data, id }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decode(String.self, forKey: .type)
        let op = try c.decode(String.self, forKey: .op)
        if op == "delete" {
            self = .delete(type: type, id: try c.decode(String.self, forKey: .id))
            return
        }
        guard op == "upsert" else { self = .unknown(type: type); return }
        switch type {
        case "account": self = .upsertAccount(try c.decode(Account.self, forKey: .data))
        case "calendar": self = .upsertCalendar(try c.decode(CalendarInfo.self, forKey: .data))
        case "calendar_set": self = .upsertCalendarSet(try c.decode(CalendarSet.self, forKey: .data))
        case "event": self = .upsertEvent(try c.decode(Event.self, forKey: .data))
        case "conflict": self = .upsertConflict(try c.decode(Conflict.self, forKey: .data))
        default: self = .unknown(type: type)
        }
    }
}

public struct PullPage: Decodable, Sendable {
    public var cursor: Int
    public var hasMore: Bool
    public var changes: [SyncChange]
}

/// JSON-Einstellungen der API: snake_case, RFC-3339-Zeitpunkte.
public enum APICoding {
    public static var decoder: JSONDecoder {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        d.dateDecodingStrategy = .custom { decoder in
            let c = try decoder.singleValueContainer()
            let text = try c.decode(String.self)
            guard let date = parseTimestamp(text) else {
                throw DecodingError.dataCorruptedError(in: c, debugDescription: "Kein RFC-3339-Zeitpunkt: \(text)")
            }
            return date
        }
        return d
    }

    public static var encoder: JSONEncoder {
        let e = JSONEncoder()
        e.keyEncodingStrategy = .convertToSnakeCase
        e.dateEncodingStrategy = .custom { date, encoder in
            var c = encoder.singleValueContainer()
            try c.encode(formatTimestamp(date))
        }
        e.outputFormatting = [.sortedKeys]
        return e
    }

    /// `2026-11-03T13:00:00Z`, auch mit Bruchteilen oder Offset.
    public static func parseTimestamp(_ text: String) -> Date? {
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        if let d = plain.date(from: text) { return d }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: text)
    }

    public static func formatTimestamp(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f.string(from: date)
    }
}
