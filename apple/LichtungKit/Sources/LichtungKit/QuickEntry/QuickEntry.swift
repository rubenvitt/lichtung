import Foundation

/// Ergebnis des Satz-Parsers: ein Termin-Entwurf aus einem deutschen Satz
/// wie „Zahnarzt Di 14 Uhr“ (Spec §5, Schnelleingabe).
public struct QuickEntry: Equatable, Sendable {
    public enum Timing: Equatable, Sendable {
        /// Der Satz nennt weder Tag noch Uhrzeit. Der Editor wählt den Tag.
        case unspecified
        /// Ganztägig von `first` bis `last` (beide inklusive).
        case allDay(first: LocalDate, last: LocalDate)
        /// Mit Uhrzeit; `end` liegt immer nach `start`.
        case timed(start: Date, end: Date)
    }

    public var title: String
    public var timing: Timing
    public var recurrence: RecurrenceRule?
    public var location: String?
    /// Was nach `/` stand, z. B. „arbeit“ aus „/arbeit“.
    public var calendarQuery: String?
    /// Kalender, auf den `calendarQuery` eindeutig passt.
    public var calendarID: String?
    /// Erkannte Stellen im Eingabetext, für die Hervorhebung beim Tippen.
    public var highlights: [Highlight]

    public init(
        title: String,
        timing: Timing = .unspecified,
        recurrence: RecurrenceRule? = nil,
        location: String? = nil,
        calendarQuery: String? = nil,
        calendarID: String? = nil,
        highlights: [Highlight] = []
    ) {
        self.title = title
        self.timing = timing
        self.recurrence = recurrence
        self.location = location
        self.calendarQuery = calendarQuery
        self.calendarID = calendarID
        self.highlights = highlights
    }

    public var isAllDay: Bool {
        if case .allDay = timing { return true }
        return false
    }
}

/// Eine erkannte Stelle im Eingabetext.
public struct Highlight: Equatable, Sendable {
    public enum Kind: String, Sendable {
        case date, time, duration, recurrence, location, calendar
    }

    public var kind: Kind
    /// Position in Zeichen (`Character`), nicht in UTF-16.
    public var range: Range<Int>

    public init(kind: Kind, range: Range<Int>) {
        self.kind = kind
        self.range = range
    }
}

/// Ein Kalender, den die Schnelleingabe per `/name` auswählen kann.
public struct CalendarChoice: Equatable, Sendable {
    public var id: String
    public var name: String

    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }
}
