import Foundation

// Objekte der Sync-API (docs/api/sync-api.md §3). Feldnamen in Swift sind
// camelCase; `APICoding` übersetzt nach snake_case.

public struct Account: Codable, Hashable, Sendable, Identifiable {
    public enum Kind: String, Codable, Sendable {
        case fastmail, icloud, ics, microsoft
        case unknown

        public init(from decoder: Decoder) throws {
            self = Kind(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
        }
    }

    public struct Status: Codable, Hashable, Sendable {
        public enum State: String, Codable, Sendable {
            case ok, syncing, error, authFailed = "auth_failed"
            case unknown

            public init(from decoder: Decoder) throws {
                self = State(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
            }
        }

        public var state: State
        public var message: String?
        public var lastSyncAt: Date?
    }

    public var id: String
    public var kind: Kind
    public var name: String
    public var username: String?
    public var status: Status?
}

public struct CalendarInfo: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var accountId: String
    public var name: String
    /// Farbe in Lichtung (`#RRGGBB`), überschreibt `providerColor`.
    public var color: String?
    public var providerColor: String?
    public var writable: Bool
    public var canInvite: Bool
    public var hidden: Bool
    public var mcpWrite: Bool
    public var openingsBusy: Bool
    public var allDayBlocks: Bool
    public var position: Int

    public init(
        id: String, accountId: String, name: String, color: String? = nil, providerColor: String? = nil,
        writable: Bool = true, canInvite: Bool = false, hidden: Bool = false, mcpWrite: Bool = false,
        openingsBusy: Bool = true, allDayBlocks: Bool = false, position: Int = 0
    ) {
        self.id = id
        self.accountId = accountId
        self.name = name
        self.color = color
        self.providerColor = providerColor
        self.writable = writable
        self.canInvite = canInvite
        self.hidden = hidden
        self.mcpWrite = mcpWrite
        self.openingsBusy = openingsBusy
        self.allDayBlocks = allDayBlocks
        self.position = position
    }

    public var displayColor: String? { color ?? providerColor }
}

public struct CalendarSet: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var calendarIds: [String]
    public var position: Int

    public init(id: String, name: String, calendarIds: [String], position: Int = 0) {
        self.id = id
        self.name = name
        self.calendarIds = calendarIds
        self.position = position
    }
}

public struct Person: Codable, Hashable, Sendable {
    public var email: String
    public var name: String?

    public init(email: String, name: String? = nil) {
        self.email = email
        self.name = name
    }
}

public enum ParticipationStatus: String, Codable, Sendable {
    case needsAction = "needs_action", accepted, declined, tentative, delegated
}

public struct Attendee: Codable, Hashable, Sendable {
    public enum Role: String, Codable, Sendable {
        case chair, required, optional, nonParticipant = "non_participant"
    }

    public var email: String
    public var name: String?
    public var role: Role?
    public var status: ParticipationStatus?
    public var isSelf: Bool?

    public init(email: String, name: String? = nil, role: Role? = .required, status: ParticipationStatus? = nil, isSelf: Bool? = nil) {
        self.email = email
        self.name = name
        self.role = role
        self.status = status
        self.isSelf = isSelf
    }
}

/// Erinnerung relativ zum Beginn oder zu einem festen Zeitpunkt.
public enum Alarm: Hashable, Sendable, Codable {
    case minutesBefore(Int)
    case at(Date)

    private enum CodingKeys: String, CodingKey { case minutesBefore, at }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        if let m = try c.decodeIfPresent(Int.self, forKey: .minutesBefore) {
            self = .minutesBefore(m)
        } else {
            self = .at(try c.decode(Date.self, forKey: .at))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .minutesBefore(m): try c.encode(m, forKey: .minutesBefore)
        case let .at(d): try c.encode(d, forKey: .at)
        }
    }
}

public enum EventStatus: String, Codable, Sendable {
    case confirmed, tentative, cancelled
}

public enum Availability: String, Codable, Sendable {
    case busy, free
}

public struct Recurrence: Codable, Hashable, Sendable {
    /// RRULE-Wert ohne `RRULE:`-Präfix.
    public var rrule: String
    public var rdates: [DateValue]
    public var exdates: [DateValue]
    /// Das Original hat mehrere RRULEs; nur die erste ist hier.
    public var incomplete: Bool?

    public init(rrule: String, rdates: [DateValue] = [], exdates: [DateValue] = [], incomplete: Bool? = nil) {
        self.rrule = rrule
        self.rdates = rdates
        self.exdates = exdates
        self.incomplete = incomplete
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        rrule = try c.decode(String.self, forKey: .rrule)
        rdates = try c.decodeIfPresent([DateValue].self, forKey: .rdates) ?? []
        exdates = try c.decodeIfPresent([DateValue].self, forKey: .exdates) ?? []
        incomplete = try c.decodeIfPresent(Bool.self, forKey: .incomplete)
    }
}

/// Felder eines Termins, die auch ein geändertes Vorkommen vollständig trägt.
public struct EventDetails: Codable, Hashable, Sendable {
    public var title: String
    public var location: String?
    public var notes: String?
    public var url: String?
    public var start: DateValue
    public var end: DateValue
    public var allDay: Bool
    public var status: EventStatus
    public var availability: Availability
    public var organizer: Person?
    public var attendees: [Attendee]
    public var selfStatus: ParticipationStatus?
    public var alarms: [Alarm]
    public var travelTimeMinutes: Int?

    public init(
        title: String, location: String? = nil, notes: String? = nil, url: String? = nil,
        start: DateValue, end: DateValue, status: EventStatus = .confirmed, availability: Availability = .busy,
        organizer: Person? = nil, attendees: [Attendee] = [], selfStatus: ParticipationStatus? = nil,
        alarms: [Alarm] = [], travelTimeMinutes: Int? = nil
    ) {
        self.title = title
        self.location = location
        self.notes = notes
        self.url = url
        self.start = start
        self.end = end
        self.allDay = start.isAllDay
        self.status = status
        self.availability = availability
        self.organizer = organizer
        self.attendees = attendees
        self.selfStatus = selfStatus
        self.alarms = alarms
        self.travelTimeMinutes = travelTimeMinutes
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        location = try c.decodeIfPresent(String.self, forKey: .location)
        notes = try c.decodeIfPresent(String.self, forKey: .notes)
        url = try c.decodeIfPresent(String.self, forKey: .url)
        start = try c.decode(DateValue.self, forKey: .start)
        end = try c.decode(DateValue.self, forKey: .end)
        allDay = try c.decodeIfPresent(Bool.self, forKey: .allDay) ?? start.isAllDay
        status = try c.decodeIfPresent(EventStatus.self, forKey: .status) ?? .confirmed
        availability = try c.decodeIfPresent(Availability.self, forKey: .availability) ?? .busy
        organizer = try c.decodeIfPresent(Person.self, forKey: .organizer)
        attendees = try c.decodeIfPresent([Attendee].self, forKey: .attendees) ?? []
        selfStatus = try c.decodeIfPresent(ParticipationStatus.self, forKey: .selfStatus)
        alarms = try c.decodeIfPresent([Alarm].self, forKey: .alarms) ?? []
        travelTimeMinutes = try c.decodeIfPresent(Int.self, forKey: .travelTimeMinutes)
    }
}

/// Geändertes Vorkommen einer Serie, vollständig (nicht als Differenz).
public struct EventOverride: Codable, Hashable, Sendable {
    /// Ursprünglicher Beginn des Vorkommens.
    public var recurrenceId: DateValue
    public var details: EventDetails

    public init(recurrenceId: DateValue, details: EventDetails) {
        self.recurrenceId = recurrenceId
        self.details = details
    }

    private enum CodingKeys: String, CodingKey { case recurrenceId }

    public init(from decoder: Decoder) throws {
        recurrenceId = try decoder.container(keyedBy: CodingKeys.self).decode(DateValue.self, forKey: .recurrenceId)
        details = try EventDetails(from: decoder)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(recurrenceId, forKey: .recurrenceId)
        try details.encode(to: encoder)
    }
}

/// Ein iCalendar-Objekt: Einzeltermin oder Serie mit geänderten Vorkommen.
public struct Event: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var calendarId: String
    public var uid: String?
    public var version: Int
    public var details: EventDetails
    public var recurrence: Recurrence?
    public var overrides: [EventOverride]
    public var createdAt: Date?
    public var updatedAt: Date?

    public init(
        id: String, calendarId: String, uid: String? = nil, version: Int = 1, details: EventDetails,
        recurrence: Recurrence? = nil, overrides: [EventOverride] = [], createdAt: Date? = nil, updatedAt: Date? = nil
    ) {
        self.id = id
        self.calendarId = calendarId
        self.uid = uid
        self.version = version
        self.details = details
        self.recurrence = recurrence
        self.overrides = overrides
        self.createdAt = createdAt
        self.updatedAt = updatedAt
    }

    private enum CodingKeys: String, CodingKey {
        case id, calendarId, uid, version, recurrence, overrides, createdAt, updatedAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        calendarId = try c.decode(String.self, forKey: .calendarId)
        uid = try c.decodeIfPresent(String.self, forKey: .uid)
        version = try c.decodeIfPresent(Int.self, forKey: .version) ?? 0
        recurrence = try c.decodeIfPresent(Recurrence.self, forKey: .recurrence)
        overrides = try c.decodeIfPresent([EventOverride].self, forKey: .overrides) ?? []
        createdAt = try c.decodeIfPresent(Date.self, forKey: .createdAt)
        updatedAt = try c.decodeIfPresent(Date.self, forKey: .updatedAt)
        details = try EventDetails(from: decoder)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(calendarId, forKey: .calendarId)
        try c.encodeIfPresent(uid, forKey: .uid)
        try c.encode(version, forKey: .version)
        try c.encode(recurrence, forKey: .recurrence)
        try c.encode(overrides, forKey: .overrides)
        try c.encodeIfPresent(createdAt, forKey: .createdAt)
        try c.encodeIfPresent(updatedAt, forKey: .updatedAt)
        try details.encode(to: encoder)
    }

    public var title: String { details.title }
    public var isRecurring: Bool { recurrence != nil }
    /// Offene Terminanfrage: Ich bin eingeladen und habe noch nicht geantwortet.
    public var isPendingInvitation: Bool { details.selfStatus == .needsAction }
}

/// Eine eigene Änderung, die der Server nicht übernehmen konnte (Vertrag §3).
public struct Conflict: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var eventId: String?
    public var calendarId: String?
    public var origin: String?
    public var createdAt: Date?
    public var message: String
    public var mutation: JSONValue?
}

/// Beliebiges JSON, für Felder, die der Client nur aufbewahrt.
public indirect enum JSONValue: Hashable, Sendable, Codable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([JSONValue].self) { self = .array(a) }
        else { self = .object(try c.decode([String: JSONValue].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case let .bool(b): try c.encode(b)
        case let .number(n): try c.encode(n)
        case let .string(s): try c.encode(s)
        case let .array(a): try c.encode(a)
        case let .object(o): try c.encode(o)
        }
    }
}
