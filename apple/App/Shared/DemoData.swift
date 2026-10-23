// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import Foundation
import LichtungKit

/// Erfundene Kalender und Termine für den Demo-Modus und Vorschauen
/// (öffentliches Repo, Spec §8: keine echten Daten).
enum DemoData {
    static func state(around now: Date, timeZone tz: TimeZone) -> SyncState {
        let cal = Calendar.lichtung(timeZone: tz)
        let today = LocalDate(now, calendar: cal)
        let monday = today.adding(days: -(today.weekday.rawValue - 1))
        let zone = tz.identifier

        func at(_ day: LocalDate, _ h: Int, _ m: Int = 0) -> DateValue {
            .dateTime(LocalDateTime(date: day, hour: h, minute: m), timeZone: zone)
        }
        func event(_ id: String, _ cal: String, _ title: String, _ day: LocalDate, _ h: Int, _ m: Int, minutes: Int,
                   location: String? = nil, rrule: String? = nil, notes: String? = nil) -> Event {
            let end = (h * 60 + m + minutes)
            return Event(
                id: id, calendarId: cal,
                details: EventDetails(title: title, location: location, notes: notes,
                                      start: at(day, h, m), end: at(day, end / 60, end % 60)),
                recurrence: rrule.map { Recurrence(rrule: $0) }
            )
        }
        func allDay(_ id: String, _ cal: String, _ title: String, _ first: LocalDate, days: Int = 1) -> Event {
            Event(id: id, calendarId: cal, details: EventDetails(title: title, start: .date(first), end: .date(first.adding(days: days))))
        }

        var s = SyncState()
        s.calendars = [
            "demo_arbeit": CalendarInfo(id: "demo_arbeit", accountId: "demo", name: "Arbeit", color: "#3B82F6", position: 0),
            "demo_privat": CalendarInfo(id: "demo_privat", accountId: "demo", name: "Privat", color: "#D98A0B", position: 1),
            "demo_sport": CalendarInfo(id: "demo_sport", accountId: "demo", name: "Sport", color: "#22A05A", position: 2),
            "demo_ehrenamt": CalendarInfo(id: "demo_ehrenamt", accountId: "demo", name: "Ehrenamt", color: "#E5484D", position: 3),
            "demo_fokus": CalendarInfo(id: "demo_fokus", accountId: "demo", name: "Fokus", color: "#8B5CF6", position: 4),
        ]
        s.calendarSets = [
            "demo_set_arbeit": CalendarSet(id: "demo_set_arbeit", name: "Arbeit", calendarIds: ["demo_arbeit", "demo_fokus"], position: 0),
            "demo_set_privat": CalendarSet(id: "demo_set_privat", name: "Privat", calendarIds: ["demo_privat", "demo_sport", "demo_ehrenamt"], position: 1),
        ]
        let events = [
            event("d1", "demo_arbeit", "Team-Sync", monday, 9, 0, minutes: 30, location: "Raum 2", rrule: "FREQ=WEEKLY;BYDAY=MO,WE",
                  notes: "Einwahl: https://meet.google.com/abc-defg-hij"),
            event("d2", "demo_fokus", "Fokuszeit", monday, 10, 0, minutes: 120, rrule: "FREQ=WEEKLY;BYDAY=MO,FR"),
            event("d3", "demo_sport", "Laufen mit Hund", today, 7, 0, minutes: 45, rrule: "FREQ=DAILY"),
            event("d4", "demo_privat", "Zahnarzt", today.adding(days: 2), 14, 0, minutes: 60, location: "Praxis am Markt"),
            event("d5", "demo_arbeit", "Workshop", today.adding(days: 1), 10, 0, minutes: 90, location: "Raum 4"),
            event("d6", "demo_arbeit", "Abstimmung", today, 11, 0, minutes: 60),
            event("d7", "demo_privat", "Mittagessen", today, 11, 30, minutes: 60, location: "Café Sonne"),
            event("d8", "demo_ehrenamt", "Dienstabend", today.adding(days: 3), 19, 30, minutes: 120, rrule: "FREQ=WEEKLY;INTERVAL=2"),
            allDay("d9", "demo_ehrenamt", "Bereitschaft", today.adding(days: 1)),
            allDay("d10", "demo_privat", "Urlaub", today.adding(days: 10), days: 5),
        ]
        for e in events { s.events[e.id] = e }
        return s
    }
}
