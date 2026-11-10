import Foundation

// Blocker-Regeln (LIC-75, docs/api/blockers.md): Der Server spiegelt die
// Termine eines Quellkalenders als „Blockiert“-Termine in einen Zielkalender
// und hält sie aktuell. Verwaltet direkt über `/api/blocker-rules`.

/// Eine Regel „Termine aus Quelle blocken in Ziel“.
public struct BlockerRule: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var sourceCalendarId: String
    public var targetCalendarId: String
    public var title: String
    /// Auch ganztägige Termine blocken.
    public var allDay: Bool
    /// Blocker, die der Server für diese Regel angelegt hat.
    public var blockerCount: Int
    /// Blocker, deren letzter Schreibversuch scheiterte, und der Grund.
    public var failedCount: Int
    public var lastError: String?

    public init(
        id: String, sourceCalendarId: String, targetCalendarId: String, title: String = "Blockiert",
        allDay: Bool = false, blockerCount: Int = 0, failedCount: Int = 0, lastError: String? = nil
    ) {
        self.id = id
        self.sourceCalendarId = sourceCalendarId
        self.targetCalendarId = targetCalendarId
        self.title = title
        self.allDay = allDay
        self.blockerCount = blockerCount
        self.failedCount = failedCount
        self.lastError = lastError
    }

    private enum CodingKeys: String, CodingKey {
        case id, sourceCalendarId, targetCalendarId, title, allDay, blockerCount, failedCount, lastError
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        sourceCalendarId = try c.decode(String.self, forKey: .sourceCalendarId)
        targetCalendarId = try c.decode(String.self, forKey: .targetCalendarId)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? "Blockiert"
        allDay = try c.decodeIfPresent(Bool.self, forKey: .allDay) ?? false
        blockerCount = try c.decodeIfPresent(Int.self, forKey: .blockerCount) ?? 0
        failedCount = try c.decodeIfPresent(Int.self, forKey: .failedCount) ?? 0
        lastError = try c.decodeIfPresent(String.self, forKey: .lastError)
    }
}

/// `POST /api/blocker-rules`. Ohne Titel nimmt der Server „Blockiert“.
public struct NewBlockerRule: Encodable, Hashable, Sendable {
    public var sourceCalendarId: String
    public var targetCalendarId: String
    public var title: String?
    public var allDay: Bool

    public init(sourceCalendarId: String, targetCalendarId: String, title: String? = nil, allDay: Bool = false) {
        self.sourceCalendarId = sourceCalendarId
        self.targetCalendarId = targetCalendarId
        self.title = title
        self.allDay = allDay
    }
}

/// `PATCH /api/blocker-rules/{id}`: nur was sich ändert. Quelle und Ziel
/// stehen fest; dafür die Regel löschen und neu anlegen.
public struct BlockerRulePatch: Encodable, Hashable, Sendable {
    public var title: String?
    public var allDay: Bool?

    public init(title: String? = nil, allDay: Bool? = nil) {
        self.title = title
        self.allDay = allDay
    }

    public var isEmpty: Bool { title == nil && allDay == nil }
}

/// Welche Kalender als Quelle und Ziel einer Regel taugen.
public enum BlockerTargets {
    /// Ziel: Lichtung muss hineinschreiben können. Sortiert wie in den
    /// Einstellungen.
    public static func targets(_ calendars: [String: CalendarInfo]) -> [CalendarInfo] {
        sorted(calendars.values.filter(\.writable))
    }

    /// Quelle: jeder Kalender, auch nur lesbare und Abos.
    public static func sources(_ calendars: [String: CalendarInfo]) -> [CalendarInfo] {
        sorted(Array(calendars.values))
    }

    /// Ob eine neue Regel so angelegt werden kann (verschiedene Kalender,
    /// beschreibbares Ziel, noch keine Regel für dasselbe Paar).
    public static func canCreate(source: String?, target: String?, calendars: [String: CalendarInfo], existing: [BlockerRule]) -> Bool {
        guard let source, let target, source != target, calendars[target]?.writable == true, calendars[source] != nil else {
            return false
        }
        return !existing.contains { $0.sourceCalendarId == source && $0.targetCalendarId == target }
    }

    private static func sorted(_ list: [CalendarInfo]) -> [CalendarInfo] {
        list.sorted { a, b in
            if a.position != b.position { return a.position < b.position }
            return a.name.localizedStandardCompare(b.name) == .orderedAscending
        }
    }
}
