import Foundation

/// Abfragen auf einem Stand, wie die Ansichten sie brauchen.
public struct CalendarQuery: Sendable {
    public let events: [String: Event]
    public let calendars: [String: CalendarInfo]
    public let calendarSets: [String: CalendarSet]
    public let expander: SeriesExpander

    public init(state: SyncState, deviceTimeZone: TimeZone) {
        self.events = state.visibleEvents(deviceTimeZone: deviceTimeZone)
        self.calendars = state.calendars
        self.calendarSets = state.calendarSets
        self.expander = SeriesExpander(deviceTimeZone: deviceTimeZone)
    }

    /// Welche Kalender angezeigt werden.
    public enum Selection: Hashable, Sendable {
        /// Alle nicht ausgeblendeten.
        case visible
        /// Nur die Kalender eines Sets (ausgeblendete bleiben ausgeblendet).
        case set(String)
        case calendars(Set<String>)
    }

    public func calendarIds(for selection: Selection) -> Set<String> {
        let visible = Set(calendars.values.filter { !$0.hidden }.map(\.id))
        switch selection {
        case .visible: return visible
        case let .set(id): return visible.intersection(calendarSets[id]?.calendarIds ?? [])
        case let .calendars(ids): return ids
        }
    }

    /// Vorkommen im Zeitraum, sortiert: ganztägige zuerst, dann nach Beginn,
    /// längere vor kürzeren, dann nach Titel (stabile Reihenfolge).
    public func occurrences(in window: DateInterval, selection: Selection = .visible) -> [Occurrence] {
        let ids = calendarIds(for: selection)
        return events.values
            .filter { ids.contains($0.calendarId) }
            .flatMap { expander.occurrences(of: $0, in: window) }
            .sorted(by: Self.displayOrder)
    }

    static func displayOrder(_ a: Occurrence, _ b: Occurrence) -> Bool {
        if a.isAllDay != b.isAllDay { return a.isAllDay }
        if a.start != b.start { return a.start < b.start }
        if a.end != b.end { return a.end > b.end }
        if a.title != b.title { return a.title.localizedStandardCompare(b.title) == .orderedAscending }
        return a.id < b.id
    }

    /// Offene Terminanfragen (Spec §5), älteste Termine zuerst.
    public func pendingInvitations(from: Date, deviceTimeZone tz: TimeZone) -> [Event] {
        events.values
            .filter { e in
                e.isPendingInvitation || e.overrides.contains { $0.details.selfStatus == .needsAction }
            }
            .filter { e in
                // Vergangene Einzeltermine sind keine Anfrage mehr.
                e.isRecurring || e.details.end.instant(deviceTimeZone: tz) > from
            }
            .sorted { $0.details.start.instant(deviceTimeZone: tz) < $1.details.start.instant(deviceTimeZone: tz) }
    }
}
