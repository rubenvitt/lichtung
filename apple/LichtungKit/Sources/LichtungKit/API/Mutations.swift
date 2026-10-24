import Foundation

// Schreiben über `POST /api/sync/push` (Vertrag §6).

/// Ein Feld einer Teiländerung: neu setzen oder leeren (`null`).
/// Fehlt das Feld (`nil` in `EventChanges`), bleibt es, wie es ist.
public enum Patch<Value: Hashable & Sendable & Codable>: Hashable, Sendable {
    case set(Value)
    case clear

    public var value: Value? {
        if case let .set(v) = self { return v }
        return nil
    }
}

public struct AttendeeInput: Codable, Hashable, Sendable {
    public var email: String
    public var name: String?
    public var role: Attendee.Role?

    public init(email: String, name: String? = nil, role: Attendee.Role? = .required) {
        self.email = email
        self.name = name
        self.role = role
    }
}

/// Schreibbare Felder beim Anlegen.
public struct EventInput: Codable, Hashable, Sendable {
    public var uid: String?
    public var title: String
    public var location: String?
    public var notes: String?
    public var url: String?
    public var start: DateValue
    public var end: DateValue
    public var status: EventStatus?
    public var availability: Availability?
    public var recurrence: Recurrence?
    public var attendees: [AttendeeInput]?
    public var alarms: [Alarm]?
    public var travelTimeMinutes: Int?

    public init(
        uid: String? = nil, title: String, location: String? = nil, notes: String? = nil, url: String? = nil,
        start: DateValue, end: DateValue, status: EventStatus? = nil, availability: Availability? = nil,
        recurrence: Recurrence? = nil, attendees: [AttendeeInput]? = nil, alarms: [Alarm]? = nil,
        travelTimeMinutes: Int? = nil
    ) {
        self.uid = uid
        self.title = title
        self.location = location
        self.notes = notes
        self.url = url
        self.start = start
        self.end = end
        self.status = status
        self.availability = availability
        self.recurrence = recurrence
        self.attendees = attendees
        self.alarms = alarms
        self.travelTimeMinutes = travelTimeMinutes
    }

    /// Eingabe, die einen bestehenden Termin nachbildet (z. B. zum Duplizieren).
    public init(copying d: EventDetails, recurrence: Recurrence? = nil) {
        self.init(
            title: d.title, location: d.location, notes: d.notes, url: d.url, start: d.start, end: d.end,
            status: d.status, availability: d.availability, recurrence: recurrence,
            attendees: d.attendees.isEmpty ? nil : d.attendees.map { AttendeeInput(email: $0.email, name: $0.name, role: $0.role) },
            alarms: d.alarms, travelTimeMinutes: d.travelTimeMinutes
        )
    }
}

/// Teiländerung für `update_event`: nur gesetzte Felder werden geschickt.
public struct EventChanges: Hashable, Sendable, Encodable {
    public var title: String?
    public var location: Patch<String>?
    public var notes: Patch<String>?
    public var url: Patch<String>?
    public var start: DateValue?
    public var end: DateValue?
    public var status: EventStatus?
    public var availability: Availability?
    public var recurrence: Patch<Recurrence>?
    public var attendees: [AttendeeInput]?
    public var alarms: [Alarm]?
    public var travelTimeMinutes: Patch<Int>?
    /// In einen anderen Kalender verschieben (nur mit Geltungsbereich `all`).
    public var calendarId: String?

    public init(
        title: String? = nil, location: Patch<String>? = nil, notes: Patch<String>? = nil, url: Patch<String>? = nil,
        start: DateValue? = nil, end: DateValue? = nil, status: EventStatus? = nil, availability: Availability? = nil,
        recurrence: Patch<Recurrence>? = nil, attendees: [AttendeeInput]? = nil, alarms: [Alarm]? = nil,
        travelTimeMinutes: Patch<Int>? = nil, calendarId: String? = nil
    ) {
        self.title = title
        self.location = location
        self.notes = notes
        self.url = url
        self.start = start
        self.end = end
        self.status = status
        self.availability = availability
        self.recurrence = recurrence
        self.attendees = attendees
        self.alarms = alarms
        self.travelTimeMinutes = travelTimeMinutes
        self.calendarId = calendarId
    }

    private enum CodingKeys: String, CodingKey {
        case title, location, notes, url, start, end, status, availability, recurrence, attendees, alarms
        case travelTimeMinutes, calendarId
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        func patch<T>(_ p: Patch<T>?, _ key: CodingKeys) throws {
            switch p {
            case nil: break
            case .clear?: try c.encodeNil(forKey: key)
            case let .set(v)?: try c.encode(v, forKey: key)
            }
        }
        try c.encodeIfPresent(title, forKey: .title)
        try patch(location, .location)
        try patch(notes, .notes)
        try patch(url, .url)
        try c.encodeIfPresent(start, forKey: .start)
        try c.encodeIfPresent(end, forKey: .end)
        try c.encodeIfPresent(status, forKey: .status)
        try c.encodeIfPresent(availability, forKey: .availability)
        try patch(recurrence, .recurrence)
        try c.encodeIfPresent(attendees, forKey: .attendees)
        try c.encodeIfPresent(alarms, forKey: .alarms)
        try patch(travelTimeMinutes, .travelTimeMinutes)
        try c.encodeIfPresent(calendarId, forKey: .calendarId)
    }

    public var isEmpty: Bool { self == EventChanges() }
}

extension EventChanges: Decodable {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func patch<T: Decodable>(_ key: CodingKeys) throws -> Patch<T>? {
            guard c.contains(key) else { return nil }
            if try c.decodeNil(forKey: key) { return .clear }
            return .set(try c.decode(T.self, forKey: key))
        }
        self.init(
            title: try c.decodeIfPresent(String.self, forKey: .title),
            location: try patch(.location),
            notes: try patch(.notes),
            url: try patch(.url),
            start: try c.decodeIfPresent(DateValue.self, forKey: .start),
            end: try c.decodeIfPresent(DateValue.self, forKey: .end),
            status: try c.decodeIfPresent(EventStatus.self, forKey: .status),
            availability: try c.decodeIfPresent(Availability.self, forKey: .availability),
            recurrence: try patch(.recurrence),
            attendees: try c.decodeIfPresent([AttendeeInput].self, forKey: .attendees),
            alarms: try c.decodeIfPresent([Alarm].self, forKey: .alarms),
            travelTimeMinutes: try patch(.travelTimeMinutes),
            calendarId: try c.decodeIfPresent(String.self, forKey: .calendarId)
        )
    }
}

/// Geltungsbereich bei Serien.
public enum EditScope: String, Codable, Hashable, Sendable {
    case all
    case this
    case thisAndFollowing = "this_and_following"
}

public enum InvitationResponse: String, Codable, Hashable, Sendable {
    case accepted, tentative, declined
}

/// Eine Änderung in der Outbox. `id` macht sie beim Server idempotent.
public struct Mutation: Hashable, Sendable, Identifiable, Codable {
    public enum Operation: Hashable, Sendable {
        case createEvent(calendarId: String, event: EventInput)
        case updateEvent(eventId: String, baseVersion: Int, scope: EditScope, occurrence: DateValue?, changes: EventChanges)
        case deleteEvent(eventId: String, baseVersion: Int, scope: EditScope, occurrence: DateValue?)
        case respond(eventId: String, occurrence: DateValue?, status: InvitationResponse, comment: String?)
    }

    public var id: String
    public var operation: Operation

    public init(id: String = UUID().uuidString.lowercased(), _ operation: Operation) {
        self.id = id
        self.operation = operation
    }

    /// Event, auf das sich die Mutation bezieht (bei `createEvent`: keins).
    public var eventId: String? {
        switch operation {
        case .createEvent: nil
        case let .updateEvent(id, _, _, _, _), let .deleteEvent(id, _, _, _), let .respond(id, _, _, _): id
        }
    }

    private enum CodingKeys: String, CodingKey {
        case id, op, calendarId, event, eventId, baseVersion, scope, occurrence, changes, status, comment
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        switch operation {
        case let .createEvent(cal, event):
            try c.encode("create_event", forKey: .op)
            try c.encode(cal, forKey: .calendarId)
            try c.encode(event, forKey: .event)
        case let .updateEvent(eventId, base, scope, occ, changes):
            try c.encode("update_event", forKey: .op)
            try c.encode(eventId, forKey: .eventId)
            try c.encode(base, forKey: .baseVersion)
            try c.encode(scope, forKey: .scope)
            try c.encodeIfPresent(occ, forKey: .occurrence)
            try c.encode(changes, forKey: .changes)
        case let .deleteEvent(eventId, base, scope, occ):
            try c.encode("delete_event", forKey: .op)
            try c.encode(eventId, forKey: .eventId)
            try c.encode(base, forKey: .baseVersion)
            try c.encode(scope, forKey: .scope)
            try c.encodeIfPresent(occ, forKey: .occurrence)
        case let .respond(eventId, occ, status, comment):
            try c.encode("respond", forKey: .op)
            try c.encode(eventId, forKey: .eventId)
            try c.encode(occ, forKey: .occurrence)
            try c.encode(status, forKey: .status)
            try c.encode(comment, forKey: .comment)
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        let op = try c.decode(String.self, forKey: .op)
        switch op {
        case "create_event":
            operation = .createEvent(
                calendarId: try c.decode(String.self, forKey: .calendarId),
                event: try c.decode(EventInput.self, forKey: .event)
            )
        case "update_event":
            operation = .updateEvent(
                eventId: try c.decode(String.self, forKey: .eventId),
                baseVersion: try c.decode(Int.self, forKey: .baseVersion),
                scope: try c.decodeIfPresent(EditScope.self, forKey: .scope) ?? .all,
                occurrence: try c.decodeIfPresent(DateValue.self, forKey: .occurrence),
                changes: try c.decode(EventChanges.self, forKey: .changes)
            )
        case "delete_event":
            operation = .deleteEvent(
                eventId: try c.decode(String.self, forKey: .eventId),
                baseVersion: try c.decode(Int.self, forKey: .baseVersion),
                scope: try c.decodeIfPresent(EditScope.self, forKey: .scope) ?? .all,
                occurrence: try c.decodeIfPresent(DateValue.self, forKey: .occurrence)
            )
        case "respond":
            operation = .respond(
                eventId: try c.decode(String.self, forKey: .eventId),
                occurrence: try c.decodeIfPresent(DateValue.self, forKey: .occurrence),
                status: try c.decode(InvitationResponse.self, forKey: .status),
                comment: try c.decodeIfPresent(String.self, forKey: .comment)
            )
        default:
            throw DecodingError.dataCorruptedError(forKey: .op, in: c, debugDescription: "Unbekannte Operation \(op)")
        }
    }
}

public struct APIErrorBody: Codable, Hashable, Sendable {
    public var code: String
    public var message: String
}

/// Antwort des Servers auf eine Mutation.
public struct MutationResult: Codable, Hashable, Sendable {
    public enum Status: String, Codable, Sendable {
        case applied, conflict, rejected, retry
    }

    public var id: String
    public var status: Status
    public var events: [Event]?
    public var deletedEventIds: [String]?
    public var conflict: Conflict?
    public var error: APIErrorBody?
    public var retryAfterSeconds: Int?
}

struct PushRequest: Encodable {
    var mutations: [Mutation]
}

struct PushResponse: Decodable {
    var results: [MutationResult]
}
