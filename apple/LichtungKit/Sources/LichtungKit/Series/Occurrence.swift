import Foundation

/// Ein angezeigtes Vorkommen eines Termins: ein Einzeltermin oder ein
/// Vorkommen einer Serie (ggf. mit seinen geänderten Feldern).
public struct Occurrence: Hashable, Sendable, Identifiable {
    public var eventId: String
    public var calendarId: String
    /// Ursprünglicher Beginn bei Serien, `nil` bei Einzelterminen.
    public var recurrenceId: DateValue?
    public var details: EventDetails
    /// Beginn und Ende als Zeitpunkte (ganztägig: Mitternacht in der Gerätezone,
    /// Ende exklusiv).
    public var start: Date
    public var end: Date
    /// Das Vorkommen wurde gegenüber der Serie geändert.
    public var isModified: Bool

    public init(
        eventId: String, calendarId: String, recurrenceId: DateValue?, details: EventDetails,
        start: Date, end: Date, isModified: Bool
    ) {
        self.eventId = eventId
        self.calendarId = calendarId
        self.recurrenceId = recurrenceId
        self.details = details
        self.start = start
        self.end = end
        self.isModified = isModified
    }

    /// Stabil über Neuberechnungen: Event plus ursprünglicher Beginn.
    public var id: String {
        guard let r = recurrenceId else { return eventId }
        switch r {
        case let .date(d): return "\(eventId)/\(d)"
        case let .dateTime(l, tz, _): return "\(eventId)/\(l)/\(tz ?? "floating")"
        }
    }

    public var isAllDay: Bool { details.allDay }
    public var title: String { details.title }
}
