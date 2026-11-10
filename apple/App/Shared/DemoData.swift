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
            "demo_privat": CalendarInfo(id: "demo_privat", accountId: "demo", name: "Privat", color: "#D98A0B", canInvite: true, position: 1),
            "demo_sport": CalendarInfo(id: "demo_sport", accountId: "demo", name: "Sport", color: "#22A05A", position: 2),
            "demo_ehrenamt": CalendarInfo(id: "demo_ehrenamt", accountId: "demo", name: "Ehrenamt", color: "#E5484D", position: 3),
            "demo_fokus": CalendarInfo(id: "demo_fokus", accountId: "demo", name: "Fokus", color: "#8B5CF6", position: 4),
        ]
        s.calendarSets = [
            "demo_set_arbeit": CalendarSet(id: "demo_set_arbeit", name: "Arbeit", calendarIds: ["demo_arbeit", "demo_fokus"], position: 0),
            "demo_set_privat": CalendarSet(id: "demo_set_privat", name: "Privat", calendarIds: ["demo_privat", "demo_sport", "demo_ehrenamt"], position: 1),
        ]
        // Laufende Woche nach dem Muster KW 46 der Vorlage (Spec §5), Werte erfunden.
        let tue = monday.adding(days: 1), wed = monday.adding(days: 2), thu = monday.adding(days: 3)
        let fri = monday.adding(days: 4), sat = monday.adding(days: 5)
        let teams = "Einwahl: https://teams.microsoft.com/l/meetup-join/demo"
        let events = [
            event("d1", "demo_arbeit", "Daily Stand-up", monday, 8, 45, minutes: 15, rrule: "FREQ=WEEKLY;BYDAY=MO,TU,WE,FR", notes: teams),
            // Drei Termine à 15 Minuten hintereinander.
            event("d2", "demo_arbeit", "Kurzabstimmung Design", monday, 9, 0, minutes: 15),
            event("d2b", "demo_arbeit", "Kurzabstimmung Technik", monday, 9, 15, minutes: 15),
            event("d2c", "demo_arbeit", "Kurzabstimmung Planung", monday, 9, 30, minutes: 15),
            event("d3", "demo_fokus", "Fokuszeit", monday, 10, 15, minutes: 45),
            event("d3b", "demo_privat", "Mittag", monday, 12, 20, minutes: 55),
            event("d4", "demo_arbeit", "Statusrunde", tue, 9, 0, minutes: 60, location: "Raum 2"),
            // Sport privat, im Arbeitskalender als „Blockiert“ gespiegelt.
            event("d5", "demo_privat", "Sport", tue, 11, 0, minutes: 45),
            event("d5b", "demo_arbeit", "Blockiert", tue, 11, 0, minutes: 45),
            event("d6", "demo_arbeit", "Vorbereitung Workshop", tue, 13, 0, minutes: 120, notes: teams),
            event("d7", "demo_arbeit", "Retro November", wed, 10, 0, minutes: 90, notes: "Einwahl: https://meet.google.com/abc-defg-hij"),
            // Ein Termin, gleich in drei Kalendern.
            event("d8a", "demo_arbeit", "Jahresplanung", wed, 14, 0, minutes: 60),
            event("d8b", "demo_privat", "Jahresplanung", wed, 14, 0, minutes: 60),
            event("d8c", "demo_ehrenamt", "Jahresplanung", wed, 14, 0, minutes: 60),
            // Ein langer Termin mit drei kurzen: wird zum Rahmen.
            event("d14", "demo_arbeit", "Werkstatt-Tag", thu, 8, 0, minutes: 9 * 60, location: "Köln"),
            event("d15", "demo_arbeit", "Austausch Plattform", thu, 11, 0, minutes: 60),
            event("d16", "demo_arbeit", "Abschluss Werkstatt", thu, 14, 0, minutes: 60),
            event("d17", "demo_arbeit", "Kurzer Anruf", thu, 15, 30, minutes: 30),
            event("d18", "demo_arbeit", "Gespräch mit Lea", fri, 10, 0, minutes: 45),
            event("d19", "demo_privat", "Zug nach Hause", fri, 17, 45, minutes: 195),
            event("d20", "demo_ehrenamt", "Lehrgang", sat, 10, 0, minutes: 8 * 60, location: "Bereitschaftsheim"),
            event("d21", "demo_sport", "Laufen mit Hund", today, 7, 0, minutes: 45, rrule: "FREQ=DAILY"),
            allDay("d9", "demo_ehrenamt", "Bereitschaft", monday, days: 7),
            allDay("d9b", "demo_privat", "Urlaub Marie", fri),
            allDay("d10", "demo_privat", "Urlaub", today.adding(days: 10), days: 5),
        ]
        for e in events { s.events[e.id] = e }

        // Terminanfragen (LIC-30): zwei offene, eine angenommen.
        func invitation(_ id: String, _ cal: String, _ title: String, _ day: LocalDate, _ h: Int, minutes: Int,
                        from organizer: Person, status: ParticipationStatus, rrule: String? = nil, location: String? = nil) -> Event {
            var e = event(id, cal, title, day, h, 0, minutes: minutes, location: location, rrule: rrule)
            e.details.organizer = organizer
            e.details.attendees = [
                Attendee(email: organizer.email, name: organizer.name, role: .chair, status: .accepted),
                Attendee(email: "ich@example.org", name: "Ich", status: status, isSelf: true),
            ]
            e.details.selfStatus = status
            return e
        }
        let anna = Person(email: "anna@example.org", name: "Anna Beispiel")
        let verein = Person(email: "vorstand@verein.example.org", name: "Vorstand Musterverein")
        for e in [
            invitation("d11", "demo_arbeit", "Projekt-Kickoff", today.adding(days: 1), 15, minutes: 60,
                       from: anna, status: .needsAction, location: "Raum 1"),
            invitation("d12", "demo_ehrenamt", "Vorstandssitzung", monday.adding(days: 3), 19, minutes: 90,
                       from: verein, status: .needsAction, rrule: "FREQ=WEEKLY;INTERVAL=2"),
            invitation("d13", "demo_arbeit", "Quartalsrückblick", today.adding(days: 4), 13, minutes: 60,
                       from: anna, status: .accepted),
        ] { s.events[e.id] = e }

        // Buchungslinks (LIC-35): ein Einmal-Link mit zwei Vorschlägen (samt
        // Platzhaltern im Kalender), einer ohne, eine Vorlage, ein gebuchter.
        let calendar = Calendar.lichtung(timeZone: tz)
        func instant(_ day: LocalDate, _ h: Int) -> Date {
            LocalDateTime(date: day, hour: h, minute: 0).instant(in: calendar)
        }
        let first = today.adding(days: 2), second = today.adding(days: 3)
        for (id, day, h) in [("dp1", first, 10), ("dp2", second, 16)] {
            s.events[id] = event(id, "demo_privat", "Reserviert: Abstimmung Umzug", day, h, 0, minutes: 30,
                                 notes: "Platzhalter für einen Vorschlag des Buchungslinks „Abstimmung Umzug“.")
        }
        let base = "https://lichtung.invalid/o/"
        let openings = [
            Opening(id: "demo_o1", kind: .once, url: base + "demo-umzug", title: "Abstimmung Umzug", durationMinutes: 30,
                    calendarId: "demo_privat", timeZone: zone, suggestionsOnly: true,
                    suggestions: [
                        Opening.Suggestion(id: "demo_s1", start: instant(first, 10), end: instant(first, 10).addingTimeInterval(1_800), placeholderEventId: "dp1"),
                        Opening.Suggestion(id: "demo_s2", start: instant(second, 16), end: instant(second, 16).addingTimeInterval(1_800), placeholderEventId: "dp2"),
                    ],
                    createdAt: now.addingTimeInterval(-3_600)),
            Opening(id: "demo_o2", kind: .once, url: base + "demo-telefonat", title: "Termin (45 Minuten)", durationMinutes: 45,
                    calendarId: "demo_privat", timeZone: zone, createdAt: now.addingTimeInterval(-86_400)),
            Opening(id: "demo_o3", kind: .template, url: base + "demo-kennenlernen", title: "30 Minuten Kennenlernen", durationMinutes: 30,
                    calendarId: "demo_privat", timeZone: zone, bufferBeforeMinutes: 10, bufferAfterMinutes: 10,
                    videoUrl: "https://meet.example.org/kennenlernen", createdAt: now.addingTimeInterval(-7 * 86_400)),
            Opening(id: "demo_o4", kind: .once, url: base + "demo-fahrrad", title: "Fahrrad abholen", durationMinutes: 15,
                    calendarId: "demo_privat", timeZone: zone, bookedAt: now.addingTimeInterval(-26 * 3_600),
                    bookedEventId: "demo_fahrrad",
                    bookings: [
                        Opening.Booking(id: "demo_b1", eventId: "demo_fahrrad", start: instant(first, 18),
                                        end: instant(first, 18).addingTimeInterval(900), guestName: "Bert Beispiel",
                                        guestEmail: "bert@example.org", note: "Ich komme mit dem Lastenrad.",
                                        createdAt: now.addingTimeInterval(-26 * 3_600)),
                    ],
                    createdAt: now.addingTimeInterval(-3 * 86_400)),
        ]
        for o in openings { s.openings[o.id] = o }
        return s
    }
}
