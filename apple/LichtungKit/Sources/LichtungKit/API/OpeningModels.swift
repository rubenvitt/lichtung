import Foundation

// Openings (Buchungslinks, Spec §6, LIC-35): Objekt `opening` aus dem Pull
// und Eingaben für `POST`/`PATCH /api/openings` (docs/api/openings.md).

/// Ein Buchungslink: Einmal-Link (`once`) oder Vorlage (`template`).
///
/// Felder, die ein älterer Server nicht schickt, bekommen die Standardwerte
/// des Vertrags (§3), damit der Pull nicht an einer Kleinigkeit scheitert.
public struct Opening: Codable, Hashable, Sendable, Identifiable {
    public enum Kind: String, Codable, Sendable {
        case once, template
        case unknown

        public init(from decoder: Decoder) throws {
            self = Kind(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
        }
    }

    /// Wochenfenster in `timeZone`: `weekday` `mo` … `su`, Zeiten `hh:mm`.
    public struct Window: Codable, Hashable, Sendable {
        public var weekday: String
        public var start: String
        public var end: String

        public init(weekday: String, start: String, end: String) {
            self.weekday = weekday
            self.start = start
            self.end = end
        }
    }

    /// Vorschlag eines Einmal-Links mit dem Zustand seines Platzhalters.
    public struct Suggestion: Codable, Hashable, Sendable, Identifiable {
        public enum PlaceholderStatus: String, Codable, Sendable {
            /// Der Platzhalter liegt im Zielkalender.
            case reserved
            /// Außerhalb von Lichtung gelöscht; der Vorschlag bleibt buchbar.
            case missing
            /// Wird gerade angelegt.
            case pending
            case unknown

            public init(from decoder: Decoder) throws {
                self = PlaceholderStatus(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
            }
        }

        public var id: String
        public var start: Date
        public var end: Date
        public var placeholderEventId: String?
        public var placeholderStatus: PlaceholderStatus

        public init(id: String, start: Date, end: Date, placeholderEventId: String? = nil, placeholderStatus: PlaceholderStatus = .reserved) {
            self.id = id
            self.start = start
            self.end = end
            self.placeholderEventId = placeholderEventId
            self.placeholderStatus = placeholderStatus
        }
    }

    /// Eine Buchung über die Buchungsseite mit dem, was der Gast eingegeben
    /// hat (LIC-33). Steht nur hier, nicht im Termin: Die Einladung geht von
    /// der eigenen Adresse an den Gast und trägt nichts, was er eingetippt hat.
    public struct Booking: Codable, Hashable, Sendable, Identifiable {
        public var id: String
        /// Der angelegte Termin.
        public var eventId: String
        public var start: Date
        public var end: Date
        public var guestName: String
        public var guestEmail: String
        public var note: String?
        /// Zeitpunkt der Buchung.
        public var createdAt: Date?

        public init(
            id: String, eventId: String, start: Date, end: Date, guestName: String, guestEmail: String,
            note: String? = nil, createdAt: Date? = nil
        ) {
            self.id = id
            self.eventId = eventId
            self.start = start
            self.end = end
            self.guestName = guestName
            self.guestEmail = guestEmail
            self.note = note
            self.createdAt = createdAt
        }
    }

    public var id: String
    public var kind: Kind
    /// Öffentliche Buchungsseite `<öffentliche URL>/o/<token>`.
    public var url: String
    public var title: String
    public var durationMinutes: Int
    public var calendarId: String
    public var timeZone: String
    public var windows: [Window]
    public var bufferBeforeMinutes: Int
    public var bufferAfterMinutes: Int
    public var minNoticeMinutes: Int
    public var horizonDays: Int
    public var videoUrl: String?
    /// Einmal-Link mit Vorschlägen: bietet nur diese an, auch wenn alle vorbei sind.
    public var suggestionsOnly: Bool
    /// Vorschläge, die noch nicht begonnen haben, frühester zuerst.
    public var suggestions: [Suggestion]
    public var bookedAt: Date?
    public var bookedEventId: String?
    /// Buchungen, neueste zuerst (höchstens 50; ältere Server: leer).
    public var bookings: [Booking]
    public var createdAt: Date?
    public var updatedAt: Date?

    public init(
        id: String, kind: Kind, url: String, title: String, durationMinutes: Int, calendarId: String,
        timeZone: String = "Europe/Berlin", windows: [Window] = Opening.defaultWindows,
        bufferBeforeMinutes: Int = 0, bufferAfterMinutes: Int = 0,
        minNoticeMinutes: Int = Opening.defaultMinNoticeMinutes, horizonDays: Int = Opening.defaultHorizonDays,
        videoUrl: String? = nil, suggestionsOnly: Bool = false, suggestions: [Suggestion] = [],
        bookedAt: Date? = nil, bookedEventId: String? = nil, bookings: [Booking] = [],
        createdAt: Date? = nil, updatedAt: Date? = nil
    ) {
        self.id = id
        self.kind = kind
        self.url = url
        self.title = title
        self.durationMinutes = durationMinutes
        self.calendarId = calendarId
        self.timeZone = timeZone
        self.windows = windows
        self.bufferBeforeMinutes = bufferBeforeMinutes
        self.bufferAfterMinutes = bufferAfterMinutes
        self.minNoticeMinutes = minNoticeMinutes
        self.horizonDays = horizonDays
        self.videoUrl = videoUrl
        self.suggestionsOnly = suggestionsOnly
        self.suggestions = suggestions
        self.bookedAt = bookedAt
        self.bookedEventId = bookedEventId
        self.bookings = bookings
        self.createdAt = createdAt
        self.updatedAt = updatedAt
    }

    private enum CodingKeys: String, CodingKey {
        case id, kind, url, title, durationMinutes, calendarId, timeZone, windows, bufferBeforeMinutes, bufferAfterMinutes
        case minNoticeMinutes, horizonDays, videoUrl, suggestionsOnly, suggestions, bookedAt, bookedEventId, bookings
        case createdAt, updatedAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.decode(Kind.self, forKey: .kind)
        url = try c.decode(String.self, forKey: .url)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        durationMinutes = try c.decode(Int.self, forKey: .durationMinutes)
        calendarId = try c.decode(String.self, forKey: .calendarId)
        timeZone = try c.decodeIfPresent(String.self, forKey: .timeZone) ?? "UTC"
        windows = try c.decodeIfPresent([Window].self, forKey: .windows) ?? []
        bufferBeforeMinutes = try c.decodeIfPresent(Int.self, forKey: .bufferBeforeMinutes) ?? 0
        bufferAfterMinutes = try c.decodeIfPresent(Int.self, forKey: .bufferAfterMinutes) ?? 0
        minNoticeMinutes = try c.decodeIfPresent(Int.self, forKey: .minNoticeMinutes) ?? Opening.defaultMinNoticeMinutes
        horizonDays = try c.decodeIfPresent(Int.self, forKey: .horizonDays) ?? Opening.defaultHorizonDays
        videoUrl = try c.decodeIfPresent(String.self, forKey: .videoUrl)
        suggestions = try c.decodeIfPresent([Suggestion].self, forKey: .suggestions) ?? []
        suggestionsOnly = try c.decodeIfPresent(Bool.self, forKey: .suggestionsOnly) ?? !suggestions.isEmpty
        bookedAt = try c.decodeIfPresent(Date.self, forKey: .bookedAt)
        bookedEventId = try c.decodeIfPresent(String.self, forKey: .bookedEventId)
        bookings = try c.decodeIfPresent([Booking].self, forKey: .bookings) ?? []
        createdAt = try c.decodeIfPresent(Date.self, forKey: .createdAt)
        updatedAt = try c.decodeIfPresent(Date.self, forKey: .updatedAt)
    }

    // MARK: Regeln des Vertrags (§2, §5)

    public static let minDurationMinutes = 5
    public static let maxDurationMinutes = 12 * 60
    public static let maxSuggestions = 20
    public static let maxWindows = 50
    public static let maxTitleLength = 200
    public static let maxBufferMinutes = 12 * 60
    public static let maxNoticeMinutes = 30 * 24 * 60
    public static let maxHorizonDays = 365
    public static let maxVideoURLLength = 2_000
    public static let defaultMinNoticeMinutes = 4 * 60
    public static let defaultHorizonDays = 21
    /// Montag bis Freitag 09:00–17:00.
    public static let defaultWindows: [Window] = ["mo", "tu", "we", "th", "fr"].map { Window(weekday: $0, start: "09:00", end: "17:00") }

    /// Einmal-Link, der gebucht wurde (verbraucht).
    public var isBooked: Bool { bookedAt != nil }

    /// Einmal-Link mit Vorschlägen, deren Zeit ganz vorbei ist: bietet nichts mehr an.
    public var isExhausted: Bool { kind == .once && !isBooked && suggestionsOnly && suggestions.isEmpty }
}

/// Neuer Link (`POST /api/openings`). Nicht gesetzte Felder lässt der
/// Server auf seinen Standardwerten.
public struct NewOpening: Encodable, Hashable, Sendable {
    public struct SuggestionInput: Encodable, Hashable, Sendable {
        public var start: Date
        public var end: Date?

        public init(start: Date, end: Date? = nil) {
            self.start = start
            self.end = end
        }
    }

    public var kind: Opening.Kind
    public var title: String?
    public var durationMinutes: Int
    public var calendarId: String
    public var timeZone: String?
    public var windows: [Opening.Window]?
    public var bufferBeforeMinutes: Int?
    public var bufferAfterMinutes: Int?
    public var minNoticeMinutes: Int?
    public var horizonDays: Int?
    public var videoUrl: String?
    public var suggestions: [SuggestionInput]?

    public init(
        kind: Opening.Kind, title: String? = nil, durationMinutes: Int, calendarId: String, timeZone: String? = nil,
        windows: [Opening.Window]? = nil, bufferBeforeMinutes: Int? = nil, bufferAfterMinutes: Int? = nil,
        minNoticeMinutes: Int? = nil, horizonDays: Int? = nil, videoUrl: String? = nil, suggestions: [SuggestionInput]? = nil
    ) {
        self.kind = kind
        self.title = title
        self.durationMinutes = durationMinutes
        self.calendarId = calendarId
        self.timeZone = timeZone
        self.windows = windows
        self.bufferBeforeMinutes = bufferBeforeMinutes
        self.bufferAfterMinutes = bufferAfterMinutes
        self.minNoticeMinutes = minNoticeMinutes
        self.horizonDays = horizonDays
        self.videoUrl = videoUrl
        self.suggestions = suggestions
    }
}

/// Änderung eines Links (`PATCH /api/openings/{id}`): nur gesetzte Felder;
/// `videoUrl: .clear` entfernt den Videolink. `kind` und Vorschläge lassen
/// sich nicht ändern.
public struct OpeningPatch: Encodable, Hashable, Sendable {
    public var title: String?
    public var durationMinutes: Int?
    public var calendarId: String?
    public var timeZone: String?
    public var windows: [Opening.Window]?
    public var bufferBeforeMinutes: Int?
    public var bufferAfterMinutes: Int?
    public var minNoticeMinutes: Int?
    public var horizonDays: Int?
    public var videoUrl: Patch<String>?

    public init(
        title: String? = nil, durationMinutes: Int? = nil, calendarId: String? = nil, timeZone: String? = nil,
        windows: [Opening.Window]? = nil, bufferBeforeMinutes: Int? = nil, bufferAfterMinutes: Int? = nil,
        minNoticeMinutes: Int? = nil, horizonDays: Int? = nil, videoUrl: Patch<String>? = nil
    ) {
        self.title = title
        self.durationMinutes = durationMinutes
        self.calendarId = calendarId
        self.timeZone = timeZone
        self.windows = windows
        self.bufferBeforeMinutes = bufferBeforeMinutes
        self.bufferAfterMinutes = bufferAfterMinutes
        self.minNoticeMinutes = minNoticeMinutes
        self.horizonDays = horizonDays
        self.videoUrl = videoUrl
    }

    /// Nichts zu ändern.
    public var isEmpty: Bool { self == OpeningPatch() }

    private enum CodingKeys: String, CodingKey {
        case title, durationMinutes, calendarId, timeZone, windows, bufferBeforeMinutes, bufferAfterMinutes, minNoticeMinutes, horizonDays, videoUrl
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encodeIfPresent(title, forKey: .title)
        try c.encodeIfPresent(durationMinutes, forKey: .durationMinutes)
        try c.encodeIfPresent(calendarId, forKey: .calendarId)
        try c.encodeIfPresent(timeZone, forKey: .timeZone)
        try c.encodeIfPresent(windows, forKey: .windows)
        try c.encodeIfPresent(bufferBeforeMinutes, forKey: .bufferBeforeMinutes)
        try c.encodeIfPresent(bufferAfterMinutes, forKey: .bufferAfterMinutes)
        try c.encodeIfPresent(minNoticeMinutes, forKey: .minNoticeMinutes)
        try c.encodeIfPresent(horizonDays, forKey: .horizonDays)
        switch videoUrl {
        case nil: break
        case .clear?: try c.encodeNil(forKey: .videoUrl)
        case let .set(v)?: try c.encode(v, forKey: .videoUrl)
        }
    }
}
