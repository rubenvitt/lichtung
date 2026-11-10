import Foundation
@testable import LichtungKit
import Testing

/// Erfundene Termine, festes „Jetzt“: Mo 9.11.2026, 8:00 in Berlin.
private enum R {
    static let tz = TimeZone(identifier: "Europe/Berlin")!
    static let cal = Calendar.lichtung(timeZone: tz)
    static let now = berlinDate(2026, 11, 9, 8)

    static func timed(_ d: Int, _ h: Int, _ min: Int = 0, month: Int = 11) -> DateValue {
        .dateTime(LocalDateTime(date: LocalDate(year: 2026, month: month, day: d), hour: h, minute: min), timeZone: "Europe/Berlin")
    }

    static func event(
        _ id: String, title: String = "Zahnarzt", start: DateValue, end: DateValue, alarms: [Alarm],
        location: String? = nil, calendar: String = "cal_1", rrule: String? = nil,
        status: EventStatus = .confirmed, selfStatus: ParticipationStatus? = nil
    ) -> Event {
        Event(
            id: id, calendarId: calendar,
            details: EventDetails(title: title, location: location, start: start, end: end, status: status,
                                  selfStatus: selfStatus, alarms: alarms),
            recurrence: rrule.map { Recurrence(rrule: $0) }
        )
    }

    static func plan(_ events: [Event], limit: Int = ReminderPlanner.defaultLimit, merge: MergeRules = .off) -> [PlannedReminder] {
        var s = SyncState()
        s.calendars["cal_1"] = CalendarInfo(id: "cal_1", accountId: "acc", name: "Privat")
        s.calendars["cal_2"] = CalendarInfo(id: "cal_2", accountId: "acc", name: "Arbeit", writable: false, position: 1)
        s.calendars["cal_aus"] = CalendarInfo(id: "cal_aus", accountId: "acc", name: "Aus", hidden: true)
        for e in events { s.events[e.id] = e }
        return ReminderPlanner.plan(CalendarQuery(state: s, deviceTimeZone: tz), now: now, limit: limit, calendar: cal, merge: merge)
    }
}

@Suite("Erinnerungen (LIC-68)")
struct ReminderTests {
    @Test("Relative Erinnerung vor dem Beginn, Text mit Uhrzeit und Ort")
    func relative() {
        let e = R.event("e1", start: R.timed(9, 14), end: R.timed(9, 15), alarms: [.minutesBefore(15)], location: "Praxis")
        let plan = R.plan([e])
        #expect(plan.count == 1)
        #expect(plan[0].fireDate == berlinDate(2026, 11, 9, 13, 45))
        #expect(plan[0].title == "Zahnarzt")
        #expect(plan[0].body == "14:00–15:00, Praxis")
        #expect(plan[0].id == "e1#15")
    }

    @Test("Vergangene Erinnerungen und solche hinter dem Horizont fallen weg")
    func window() {
        let past = R.event("past", start: R.timed(9, 8, 10), end: R.timed(9, 9), alarms: [.minutesBefore(30)])
        let far = R.event("far", start: R.timed(20, 9), end: R.timed(20, 10), alarms: [.minutesBefore(10)])
        let soon = R.event("soon", start: R.timed(9, 8, 10), end: R.timed(9, 9), alarms: [.minutesBefore(5)])
        #expect(R.plan([past, far, soon]).map(\.id) == ["soon#5"])
    }

    @Test("Serie: jedes Vorkommen im Horizont, eigene Kennungen")
    func series() {
        let e = R.event("daily", title: "Standup", start: R.timed(9, 9), end: R.timed(9, 9, 15),
                        alarms: [.minutesBefore(10)], rrule: "FREQ=DAILY;COUNT=10")
        let plan = R.plan([e])
        // 9.–15.11. liegen im Horizont von 7 Tagen ab Mo 8:00 (16.11. 8:50 nicht mehr).
        #expect(plan.count == 7)
        #expect(Set(plan.map(\.id)).count == 7)
        #expect(plan[0].body == "9:00–9:15")
        #expect(plan.last?.fireDate == berlinDate(2026, 11, 15, 8, 50))
    }

    @Test("Ganztägig: Minuten zählen ab Mitternacht, Text nennt den Tag")
    func allDay() {
        // 15 Stunden vorher = Vortag 9:00 (übliche „am Vortag um 9“-Erinnerung).
        let e = R.event("urlaub", title: "Urlaub", start: .date(LocalDate(year: 2026, month: 11, day: 11)),
                        end: .date(LocalDate(year: 2026, month: 11, day: 12)), alarms: [.minutesBefore(900)])
        let plan = R.plan([e])
        #expect(plan.map(\.fireDate) == [berlinDate(2026, 11, 10, 9)])
        #expect(plan[0].body == "Morgen, ganztägig")
    }

    @Test("Fester Zeitpunkt gilt einmal, auch bei Serien")
    func absolute() {
        let fire = berlinDate(2026, 11, 10, 7)
        let e = R.event("weekly", start: R.timed(9, 18), end: R.timed(9, 19), alarms: [.at(fire)], rrule: "FREQ=DAILY")
        let plan = R.plan([e])
        #expect(plan.map(\.fireDate) == [fire])
        // Beschrieben wird das Vorkommen, das um 7:00 am 10.11. als nächstes kommt.
        #expect(plan[0].body == "18:00–19:00")
        #expect(plan[0].id == "weekly#at\(Int(fire.timeIntervalSince1970))")
    }

    @Test("Abgesagt, abgelehnt und ausgeblendet erinnern nicht; vielleicht schon")
    func skipped() {
        let a = R.event("abgesagt", start: R.timed(9, 12), end: R.timed(9, 13), alarms: [.minutesBefore(5)], status: .cancelled)
        let b = R.event("abgelehnt", start: R.timed(9, 12), end: R.timed(9, 13), alarms: [.minutesBefore(5)], selfStatus: .declined)
        let c = R.event("aus", start: R.timed(9, 12), end: R.timed(9, 13), alarms: [.minutesBefore(5)], calendar: "cal_aus")
        let d = R.event("vielleicht", start: R.timed(9, 12), end: R.timed(9, 13), alarms: [.minutesBefore(5)], selfStatus: .tentative)
        #expect(R.plan([a, b, c, d]).map(\.eventId) == ["vielleicht"])
    }

    @Test("Limit: früheste zuerst, höchstens so viele wie erlaubt")
    func limit() {
        // 70 Termine am 9. und 10.11., je einer pro Viertelstunde ab 9:00.
        let events = (0..<70).map { i -> Event in
            let minutes = 9 * 60 + i * 15
            let day = 9 + minutes / (24 * 60), m = minutes % (24 * 60)
            return R.event("e\(i)", start: R.timed(day, m / 60, m % 60), end: R.timed(day, m / 60, m % 60 + 10), alarms: [.minutesBefore(0)])
        }
        let plan = R.plan(events, limit: 64)
        #expect(plan.count == 64)
        #expect(plan.first?.fireDate == berlinDate(2026, 11, 9, 9))
        #expect(zip(plan, plan.dropFirst()).allSatisfy { $0.fireDate <= $1.fireDate })
    }

    @Test("Doppelte Erinnerungen am selben Termin nur einmal")
    func duplicates() {
        let e = R.event("e1", start: R.timed(9, 14), end: R.timed(9, 15), alarms: [.minutesBefore(15), .minutesBefore(15), .minutesBefore(60)])
        #expect(R.plan([e]).map(\.id) == ["e1#60", "e1#15"])
    }

    @Test("Text: übermorgen mit Wochentag, anderes Jahr mit Jahreszahl")
    func bodyText() {
        let e = R.event("e1", start: R.timed(12, 10), end: R.timed(12, 11), alarms: [.minutesBefore(2 * 24 * 60)])
        #expect(R.plan([e]).first?.body == "Do 12. Nov., 10:00–11:00")
        let occ = Occurrence(eventId: "x", calendarId: "cal_1", recurrenceId: nil,
                             details: EventDetails(title: "Silvester", start: .date(LocalDate(year: 2027, month: 1, day: 1)),
                                                   end: .date(LocalDate(year: 2027, month: 1, day: 2))),
                             start: berlinDate(2027, 1, 1), end: berlinDate(2027, 1, 2), isModified: false)
        let text = ReminderPlanner.body(for: occ, firingOn: LocalDate(year: 2026, month: 12, day: 29),
                                        today: LocalDate(year: 2026, month: 12, day: 29), calendar: R.cal)
        #expect(text == "Fr 1. Jan. 2027, ganztägig")
    }

    @Test("Differenz: nur Geändertes neu einplanen")
    func diff() {
        let a = PlannedReminder(id: "a", fireDate: R.now, title: "A", body: "", eventId: "a", occurrenceId: "a")
        let b = PlannedReminder(id: "b", fireDate: R.now, title: "B", body: "", eventId: "b", occurrenceId: "b")
        var b2 = b
        b2.fireDate = R.now.addingTimeInterval(60)
        let c = PlannedReminder(id: "c", fireDate: R.now, title: "C", body: "", eventId: "c", occurrenceId: "c")
        let d = ReminderPlanner.diff(scheduled: [a, b], wanted: [b2, c])
        #expect(d.remove == ["a", "b"])
        #expect(d.add.map(\.id) == ["b", "c"])
        let same = ReminderPlanner.diff(scheduled: [a, b], wanted: [a, b])
        #expect(same.remove.isEmpty && same.add.isEmpty)
    }

    @Test("Zusammengefasste Termine erinnern einmal pro Zeitpunkt")
    func mergedGroupRemindsOncePerFireDate() {
        let a = R.event("a", start: R.timed(9, 14), end: R.timed(9, 15), alarms: [.minutesBefore(10)], calendar: "cal_1")
        let b = R.event("b", start: R.timed(9, 14), end: R.timed(9, 15), alarms: [.minutesBefore(10)], calendar: "cal_2")
        #expect(R.plan([a, b]).count == 2)
        let merged = R.plan([a, b], merge: .default)
        #expect(merged.count == 1)
        #expect(merged[0].occurrenceId == "a")
    }

    @Test("Nur der Blocker hat einen Alarm: Erinnerung bleibt, mit Titel des Sichtbaren")
    func blockerAlarmSurvivesWithShownTitle() {
        let sport = R.event("s", title: "Sport", start: R.timed(9, 14), end: R.timed(9, 15), alarms: [], calendar: "cal_1")
        let blocker = R.event("b", title: "Blockiert", start: R.timed(9, 14), end: R.timed(9, 15), alarms: [.minutesBefore(10)], location: "Raum 9", calendar: "cal_2")
        let plan = R.plan([sport, blocker], merge: .default)
        #expect(plan.count == 1)
        #expect(plan[0].title == "Sport")
        #expect(plan[0].occurrenceId == "s")
        #expect(plan[0].eventId == "s")
        #expect(plan[0].body == "14:00–15:00")
    }
}
