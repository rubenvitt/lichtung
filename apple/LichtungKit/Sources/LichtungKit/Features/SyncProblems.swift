import Foundation

/// Eigene Änderungen, die nicht angekommen sind (LIC-70): Konflikte (der
/// Anbieter hatte eine neuere Fassung und hat gewonnen, Vertrag §3) und vom
/// Server abgelehnte Änderungen (`SyncNotice`). Spec §4: „Nichts verschwindet
/// still.“ Die App zeigt sie als Liste mit Zahl, bis sie weggeklickt sind.
public struct SyncProblem: Hashable, Sendable, Identifiable {
    public enum Kind: Hashable, Sendable {
        /// Weg damit über `POST /api/conflicts/{id}/dismiss` (alle Geräte).
        case conflict
        /// Nur auf diesem Gerät (`SyncEngine.dismissNotice`).
        case rejected
    }

    public var id: String
    public var kind: Kind
    /// „Nicht übernommen: Zahnarzt“, „Abgelehnt: Neuer Termin“.
    public var title: String
    public var message: String
    /// „Geändert auf iPhone · 3.11., 9:01“
    public var detail: String?
    /// Event, das sich öffnen lässt (fehlt, wenn es gelöscht ist).
    public var eventId: String?
    public var date: Date?

    public init(id: String, kind: Kind, title: String, message: String, detail: String?, eventId: String?, date: Date?) {
        self.id = id
        self.kind = kind
        self.title = title
        self.message = message
        self.detail = detail
        self.eventId = eventId
        self.date = date
    }
}

extension SyncState {
    /// Konflikte und abgelehnte Änderungen, neueste zuerst.
    public func problems(deviceTimeZone tz: TimeZone) -> [SyncProblem] {
        let events = visibleEvents(deviceTimeZone: tz)
        let cal = Calendar.lichtung(timeZone: tz)
        let fromConflicts = conflicts.values.map { c -> SyncProblem in
            let eventId = c.eventId.map(currentId(for:)).flatMap { events[$0] == nil ? nil : $0 }
            let name = eventId.flatMap { events[$0]?.title } ?? Self.mutationTitle(c.mutation)
            var parts: [String] = []
            if let origin = Self.originText(c.origin) { parts.append(origin) }
            if let at = c.createdAt { parts.append(Self.timestamp(at, cal)) }
            return SyncProblem(
                id: c.id, kind: .conflict,
                title: name.map { "Nicht übernommen: \($0)" } ?? "Änderung nicht übernommen",
                message: c.message,
                detail: parts.isEmpty ? nil : parts.joined(separator: " · "),
                eventId: eventId, date: c.createdAt
            )
        }
        let fromNotices = notices.map { n in
            SyncProblem(
                id: n.id, kind: .rejected, title: "Vom Server abgelehnt", message: n.message,
                detail: Self.timestamp(n.createdAt, cal), eventId: nil, date: n.createdAt
            )
        }
        return (fromConflicts + fromNotices).sorted {
            switch ($0.date, $1.date) {
            case let (a?, b?) where a != b: a > b
            case (nil, _?): false
            case (_?, nil): true
            default: $0.id < $1.id
            }
        }
    }

    /// „auf iPhone“, „über Claude (Token)“, „im Browser“; `server` und
    /// Unbekanntes ohne Text.
    static func originText(_ origin: String?) -> String? {
        guard let origin, let colon = origin.firstIndex(of: ":") else {
            return origin == "web" ? "Geändert im Browser" : nil
        }
        let kind = origin[..<colon], name = origin[origin.index(after: colon)...]
        switch kind {
        case "app": return "Geändert auf \(name)"
        case "mcp": return "Geändert über Claude (\(name))"
        case "api": return "Geändert über API-Token \(name)"
        default: return nil
        }
    }

    /// Titel aus der verworfenen Mutation, falls das Event weg ist.
    static func mutationTitle(_ m: JSONValue?) -> String? {
        guard case let .object(o)? = m else { return nil }
        for key in ["changes", "event"] {
            if case let .object(inner)? = o[key], case let .string(t)? = inner["title"], !t.isEmpty { return t }
        }
        return nil
    }

    static func timestamp(_ d: Date, _ cal: Calendar) -> String {
        let c = cal.dateComponents([.day, .month, .hour, .minute], from: d)
        return "\(c.day ?? 0).\(c.month ?? 0)., \(c.hour ?? 0):" + String(format: "%02d", c.minute ?? 0)
    }
}
