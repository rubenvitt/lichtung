import Foundation
@testable import LichtungKit
import Testing

/// Hilfen für den Termin-Editor (LIC-20). Termine liegen im November 2026,
/// Mo 2.11. ist ein Montag, Zeitzone Europe/Berlin (MEZ, UTC+1).
enum DraftFixtures {
    typealias F = SyncFixtures
    static let tz = SyncFixtures.tz

    static func day(_ d: Int, month: Int = 11) -> LocalDate { LocalDate(year: 2026, month: month, day: d) }

    static func time(_ d: Int, _ h: Int, _ m: Int = 0) -> LocalDateTime {
        LocalDateTime(date: day(d), hour: h, minute: m)
    }

    /// Einzeltermin Di 3.11. 9–10 Uhr als Vorkommen.
    static func single(_ configure: (inout Event) -> Void = { _ in }) -> (Occurrence, Event) {
        var e = F.event("ev_1", version: 2)
        configure(&e)
        return (occurrence(of: e).first!, e)
    }

    /// Wöchentliche Serie ab Mo 2.11. 9–10 Uhr.
    static func series(_ rrule: String = "FREQ=WEEKLY;BYDAY=MO") -> Event {
        F.event("ev_s", version: 4, day: 2, rrule: rrule)
    }

    static func occurrence(of e: Event) -> [Occurrence] {
        let window = DateInterval(start: day(1).startDate(in: .lichtung(timeZone: tz)), end: day(30).startDate(in: .lichtung(timeZone: tz)))
        return SeriesExpander(deviceTimeZone: tz).occurrences(of: e, in: window)
    }

    static func draft(_ occ: Occurrence, _ e: Event?) -> EventDraft {
        EventDraft(editing: occ, event: e, deviceTimeZone: tz, id: "draft")
    }

    /// Teiländerung aus einer `update`-Absicht.
    static func changes(_ intent: EditIntent?) -> EventChanges? {
        if case let .update(_, _, _, c)? = intent { return c }
        return nil
    }

    static func json(_ c: EventChanges) throws -> [String: Any] {
        let data = try APICoding.encoder.encode(c)
        return try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    }
}

@Suite("Termin-Editor: neuer Termin")
struct EventDraftCreateTests {
    typealias D = DraftFixtures
    typealias F = SyncFixtures

    @Test func startsAtNextFullHourTodayElseNine() {
        // F.now = 23.10. 10:00 in Berlin.
        let today = EventDraft.new(on: D.day(23, month: 10), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        #expect(today.start == LocalDateTime(date: D.day(23, month: 10), hour: 11, minute: 0))
        #expect(today.end == LocalDateTime(date: D.day(23, month: 10), hour: 12, minute: 0))
        #expect(today.timeZoneID == "Europe/Berlin")

        let later = EventDraft.new(on: D.day(3), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        #expect(later.start == D.time(3, 9))
        #expect(later.end == D.time(3, 10))
        #expect(later.isNew)
        #expect(later.hasChanges)
        #expect(later.saveScopes.isEmpty)
        #expect(later.deleteIntent() == nil)
    }

    @Test func lateEveningEndsAtMidnight() {
        let now = APICoding.parseTimestamp("2026-11-03T22:30:00Z")! // 23:30 in Berlin
        let d = EventDraft.new(on: D.day(3), now: now, calendarId: "cal_1", deviceTimeZone: D.tz)
        #expect(d.start == D.time(3, 23))
        #expect(d.end == D.time(4, 0))
        #expect(d.canSave)
    }

    @Test func emptyTitleBecomesNewEvent() {
        var d = EventDraft.new(on: D.day(3), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        d.title = "   "
        d.location = "  Raum 2 "
        d.notes = ""
        d.addAlarm(10)
        guard case let .create(cal, input)? = d.saveIntent() else {
            Issue.record("kein create")
            return
        }
        #expect(cal == "cal_1")
        #expect(input.title == "Neuer Termin")
        #expect(input.location == "Raum 2")
        #expect(input.notes == nil)
        #expect(input.start == F.at(3, 9))
        #expect(input.end == F.at(3, 10))
        #expect(input.alarms == [.minutesBefore(10)])
        #expect(input.recurrence == nil)
    }

    @Test func allDayCreatesDatesWithExclusiveEnd() {
        var d = EventDraft.new(on: D.day(3), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        d.title = "Urlaub"
        d.setAllDay(true)
        d.end = LocalDateTime(date: D.day(7), hour: 10, minute: 0)
        guard case let .create(_, input)? = d.saveIntent() else {
            Issue.record("kein create")
            return
        }
        #expect(input.start == .date(D.day(3)))
        #expect(input.end == .date(D.day(8)))
    }

    @Test func withoutCalendarNothingIsSaved() {
        let d = EventDraft.new(on: D.day(3), now: F.now, calendarId: nil, deviceTimeZone: D.tz)
        #expect(d.validationMessage == "Kein beschreibbarer Kalender ausgewählt.")
        #expect(d.saveIntent() == nil)
    }

    @Test func weeklyRepetitionBecomesRRule() {
        var d = EventDraft.new(on: D.day(2), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        d.repeatFrequency = .weekly
        #expect(d.effectiveWeekdays == [.monday])
        d.toggleWeekday(.wednesday)
        #expect(d.effectiveWeekdays == [.monday, .wednesday])
        d.repeatEnd = .until
        d.repeatUntil = D.day(31, month: 12)
        guard case let .create(_, input)? = d.saveIntent() else {
            Issue.record("kein create")
            return
        }
        // Ende des 31.12. in Berlin (MEZ) als UTC.
        #expect(input.recurrence?.rrule == "FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20261231T225959Z")
        #expect(d.repeatSummary == "jeden Montag und Mittwoch bis 31. Dez. 2026")
    }

    @Test func otherFrequencies() {
        var d = EventDraft.new(on: D.day(2), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        func rrule() -> String? {
            if case let .create(_, input)? = d.saveIntent() { return input.recurrence?.rrule }
            return nil
        }
        d.repeatFrequency = .daily
        d.repeatEnd = .count
        #expect(rrule() == "FREQ=DAILY;COUNT=10")
        d.repeatCount = 3
        #expect(rrule() == "FREQ=DAILY;COUNT=3")
        // Frequenz wechseln: Ende bleibt, Abstand fängt bei 1 an.
        d.repeatFrequency = .monthly
        d.repeatInterval = 2
        #expect(rrule() == "FREQ=MONTHLY;INTERVAL=2;COUNT=3")
        d.repeatFrequency = .yearly
        d.repeatEnd = .never
        #expect(rrule() == "FREQ=YEARLY")
        d.setAllDay(true)
        d.repeatEnd = .until
        d.repeatUntil = D.day(1, month: 6).adding(months: 12)
        #expect(rrule() == "FREQ=YEARLY;UNTIL=20270601")
        d.repeatFrequency = .none
        #expect(rrule() == nil)
        #expect(!d.frequencyChoices.contains(.custom))
    }

    @Test func weekdayToggleKeepsLastDay() {
        var d = EventDraft.new(on: D.day(2), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        d.toggleWeekday(.friday) // ohne wöchentliche Regel: nichts
        #expect(d.repetition == .none)
        d.repeatFrequency = .weekly
        d.toggleWeekday(.monday)
        #expect(d.effectiveWeekdays == [.monday])
        d.toggleWeekday(.tuesday)
        d.toggleWeekday(.monday)
        #expect(d.effectiveWeekdays == [.tuesday])
        // Beginn am Montag passt nicht zu „jeden Dienstag“.
        #expect(d.validationMessage == "Der Beginn muss auf einen der gewählten Wochentage fallen.")
        d.moveStart(to: D.time(3, 9))
        #expect(d.canSave)
    }

    @Test func untilBeforeStartIsInvalid() {
        var d = EventDraft.new(on: D.day(10), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        d.repeatFrequency = .daily
        d.repeatUntil = D.day(9)
        #expect(d.repeatEnd == .until)
        #expect(d.validationMessage == "Die Wiederholung endet vor dem Beginn.")
    }
}

@Suite("Termin-Editor: Beginn, Ende, ganztägig")
struct EventDraftTimeTests {
    typealias D = DraftFixtures
    typealias F = SyncFixtures

    func draft() -> EventDraft {
        EventDraft.new(on: D.day(3), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
    }

    @Test func movingStartKeepsDuration() {
        var d = draft()
        d.end = D.time(3, 10, 30)
        d.moveStart(to: D.time(4, 23))
        #expect(d.start == D.time(4, 23))
        #expect(d.end == D.time(5, 0, 30))
    }

    @Test func endBeforeStartIsInvalid() {
        var d = draft()
        d.end = D.time(3, 8)
        #expect(d.validationMessage == "Das Ende muss nach dem Beginn liegen.")
        #expect(!d.canSave)
        #expect(d.saveIntent() == nil)
        d.end = D.time(3, 9)
        #expect(!d.canSave, "Dauer null zählt nicht als „nach dem Beginn“")
    }

    @Test func allDayEndBeforeStartIsInvalid() {
        var d = draft()
        d.setAllDay(true)
        d.end = LocalDateTime(date: D.day(2), hour: 10, minute: 0)
        #expect(d.validationMessage == "Das Ende liegt vor dem Beginn.")
        d.end = LocalDateTime(date: D.day(3), hour: 0, minute: 0)
        #expect(d.canSave, "ganztägig: nur die Tage zählen")
    }

    @Test func allDayRoundTripKeepsTimes() {
        var d = draft()
        d.setAllDay(true)
        #expect(d.startValue == .date(D.day(3)))
        #expect(d.endValue == .date(D.day(4)))
        d.setAllDay(false)
        #expect(d.startValue == F.at(3, 9))
        #expect(d.endValue == F.at(3, 10))
    }

    @Test func eveningUntilMidnightStaysOneDay() {
        var d = draft()
        d.moveStart(to: D.time(3, 23))
        #expect(d.end == D.time(4, 0))
        d.setAllDay(true)
        #expect(d.endValue == .date(D.day(4)))
        d.setAllDay(false)
        // Ende 3.11. 0:00 läge vor dem Beginn: Beginn plus eine Stunde.
        #expect(d.end == D.time(4, 0))
    }

    @Test func timeZoneKeepsWallClock() {
        var d = draft()
        d.setTimeZone("America/New_York")
        #expect(d.startValue == .dateTime(D.time(3, 9), timeZone: "America/New_York"))
        #expect(d.endValue == .dateTime(D.time(3, 10), timeZone: "America/New_York"))
        #expect(APICoding.formatTimestamp(d.startInstant) == "2026-11-03T14:00:00Z")
        #expect(d.timeZoneChoices.first == "America/New_York")
        #expect(d.timeZoneChoices.contains("Europe/Berlin"))
        #expect(EventDraft.timeZoneLabel("America/New_York") == "New York")
        #expect(EventDraft.timeZoneLabel(nil) == "Ohne Zeitzone")
    }

    @Test func pickerDatesShowWallClockInDeviceZone() {
        var d = draft()
        d.timeZoneSelection = "America/New_York"
        #expect(d.timeZoneID == "America/New_York")
        // Die Auswahl zeigt 9:00 Gerätezeit, obwohl der Termin in New York liegt.
        #expect(APICoding.formatTimestamp(d.startPickerDate) == "2026-11-03T08:00:00Z")
        d.startPickerDate = APICoding.parseTimestamp("2026-11-04T10:30:00Z")!
        #expect(d.start == D.time(4, 11, 30))
        #expect(d.end == D.time(4, 12, 30))
        d.endPickerDate = APICoding.parseTimestamp("2026-11-04T13:00:00Z")!
        #expect(d.end == D.time(4, 14))
        d.allDay = true
        #expect(d.isAllDay)
        d.timeZoneSelection = ""
        #expect(d.timeZoneID == nil)
        d.repeatFrequency = .daily
        d.repeatUntilPickerDate = APICoding.parseTimestamp("2026-11-20T12:00:00Z")!
        #expect(d.repeatUntil == D.day(20))
        #expect(EventDraft.RepeatFrequency.weekly.intervalText(1) == "Jede Woche")
        #expect(EventDraft.RepeatFrequency.yearly.intervalText(3) == "Alle 3 Jahre")
    }
}

@Suite("Termin-Editor: bestehender Termin")
struct EventDraftEditTests {
    typealias D = DraftFixtures
    typealias F = SyncFixtures

    @Test func readsOccurrence() {
        let (occ, e) = D.single { e in
            e.details.location = "Raum 2"
            e.details.notes = "Agenda"
            e.details.alarms = [.minutesBefore(10), .at(F.now), .minutesBefore(0)]
        }
        let d = D.draft(occ, e)
        #expect(d.title == "Termin")
        #expect(d.start == D.time(3, 9))
        #expect(d.end == D.time(3, 10))
        #expect(d.timeZoneID == "Europe/Berlin")
        #expect(d.location == "Raum 2")
        #expect(d.notes == "Agenda")
        #expect(d.alarms == [0, 10])
        #expect(d.fixedAlarms == [F.now])
        #expect(d.calendarId == "cal_1")
        #expect(!d.isNew)
        #expect(!d.isSeriesOccurrence)
        #expect(!d.hasChanges)
        #expect(d.saveIntent() == nil, "ohne Änderung nichts zu tun")
    }

    @Test func onlyChangedFieldsAreSent() throws {
        let (occ, e) = D.single { $0.details.notes = "Agenda" }
        var d = D.draft(occ, e)
        d.location = "Raum 3"
        d.notes = "  "
        let intent = d.saveIntent()
        guard case let .update(id, scope, occurrence, c)? = intent else {
            Issue.record("kein update")
            return
        }
        #expect(id == "ev_1")
        #expect(scope == .all)
        #expect(occurrence == nil)
        #expect(c == EventChanges(location: .set("Raum 3"), notes: .clear))
        let json = try D.json(c)
        #expect(Set(json.keys) == ["location", "notes"])
        #expect(json["notes"] is NSNull)
    }

    @Test func titleWhitespaceIsNoChange() {
        let (occ, e) = D.single()
        var d = D.draft(occ, e)
        d.title = " Termin "
        #expect(!d.hasChanges)
        d.title = ""
        #expect(D.changes(d.saveIntent()) == EventChanges(title: "Neuer Termin"))
    }

    @Test func timesAreSentTogether() {
        let (occ, e) = D.single()
        var d = D.draft(occ, e)
        d.moveStart(to: D.time(3, 11))
        #expect(D.changes(d.saveIntent()) == EventChanges(start: F.at(3, 11), end: F.at(3, 12)))
        d.setAllDay(true)
        #expect(D.changes(d.saveIntent()) == EventChanges(start: .date(D.day(3)), end: .date(D.day(4))))
    }

    @Test func allDayEventToTimed() {
        let (occ, e) = D.single { e in
            e.details = EventDetails(title: "Urlaub", start: .date(D.day(3)), end: .date(D.day(6)))
        }
        var d = D.draft(occ, e)
        #expect(d.isAllDay)
        #expect(d.start.date == D.day(3))
        #expect(d.end.date == D.day(5), "der Editor zeigt den letzten Tag")
        #expect(!d.hasChanges)
        d.setAllDay(false)
        // Uhrzeiten wie bei einem neuen Termin, Ende am letzten Tag.
        #expect(D.changes(d.saveIntent()) == EventChanges(start: F.at(3, 9), end: F.at(5, 10)))
    }

    @Test func alarmChangesKeepFixedAlarms() {
        let (occ, e) = D.single { $0.details.alarms = [.at(F.now), .minutesBefore(10)] }
        var d = D.draft(occ, e)
        #expect(d.alarmChoices.first == 0)
        #expect(!d.alarmChoices.contains(10))
        d.removeAlarm(10)
        d.addAlarm(60)
        d.addAlarm(60)
        #expect(d.alarms == [60])
        #expect(D.changes(d.saveIntent()) == EventChanges(alarms: [.at(F.now), .minutesBefore(60)]))
    }

    @Test func movingToAnotherCalendar() {
        let (occ, e) = D.single()
        var d = D.draft(occ, e)
        d.calendarId = "cal_2"
        #expect(d.saveScopes.isEmpty, "Einzeltermin: keine Frage")
        #expect(D.changes(d.saveIntent()) == EventChanges(calendarId: "cal_2"))
    }

    @Test func addingRepetitionToSingleEvent() {
        let (occ, e) = D.single()
        var d = D.draft(occ, e)
        d.repeatFrequency = .weekly
        d.toggleWeekday(.thursday)
        let c = D.changes(d.saveIntent())
        #expect(c?.recurrence == .set(Recurrence(rrule: "FREQ=WEEKLY;BYDAY=TU,TH")))
    }

    @Test func untranslatableZoneUsesDeviceZone() {
        let utc = APICoding.parseTimestamp("2026-11-03T08:00:00Z")!
        let (occ, e) = D.single { e in
            e.details = EventDetails(
                title: "Fremd",
                start: .dateTime(D.time(3, 8), timeZone: nil, utc: utc),
                end: .dateTime(D.time(3, 9), timeZone: nil, utc: utc.addingTimeInterval(3600))
            )
        }
        var d = D.draft(occ, e)
        #expect(d.start == D.time(3, 9))
        #expect(d.timeZoneID == "Europe/Berlin")
        #expect(!d.hasChanges)
        d.title = "Fremd (neu)"
        #expect(D.changes(d.saveIntent()) == EventChanges(title: "Fremd (neu)"), "Zeiten bleiben unangetastet")
    }

    @Test func deleteSingleEvent() {
        let (occ, e) = D.single()
        let d = D.draft(occ, e)
        #expect(d.deleteScopes.isEmpty)
        #expect(d.deleteIntent() == .delete(eventId: "ev_1"))
        #expect(d.deleteIntent(scope: .this) == .delete(eventId: "ev_1"))
    }
}

@Suite("Termin-Editor: Serien")
struct EventDraftSeriesTests {
    typealias D = DraftFixtures
    typealias F = SyncFixtures

    /// Vorkommen Mo 9.11. der Serie.
    func secondMonday(_ e: Event = D.series()) -> (Occurrence, EventDraft) {
        let occ = D.occurrence(of: e)[1]
        return (occ, D.draft(occ, e))
    }

    @Test func readsRule() {
        let (occ, d) = secondMonday()
        #expect(occ.recurrenceId == F.at(9, 9))
        #expect(d.isSeriesOccurrence)
        #expect(d.start == D.time(9, 9))
        #expect(d.repetition == .rule(.weekly(.monday)))
        #expect(d.repeatFrequency == .weekly)
        #expect(d.repeatEnd == .never)
        #expect(d.saveScopes.isEmpty, "ohne Änderung keine Frage")
        #expect(d.deleteScopes == [.this, .thisAndFollowing, .all])
    }

    @Test func scopesForFieldChanges() {
        var (_, d) = secondMonday()
        d.title = "Team-Sync"
        #expect(d.saveScopes == [.this, .thisAndFollowing, .all])
        #expect(d.saveIntent(scope: .this) == .update(eventId: "ev_s", scope: .this, occurrence: F.at(9, 9), changes: EventChanges(title: "Team-Sync")))
        #expect(d.saveIntent(scope: .thisAndFollowing) == .update(eventId: "ev_s", scope: .thisAndFollowing, occurrence: F.at(9, 9), changes: EventChanges(title: "Team-Sync")))
        #expect(d.saveIntent(scope: .all) == .update(eventId: "ev_s", scope: .all, changes: EventChanges(title: "Team-Sync")))
    }

    @Test func thisOccurrenceMovesItself() {
        var (_, d) = secondMonday()
        d.moveStart(to: D.time(10, 14))
        #expect(D.changes(d.saveIntent(scope: .this)) == EventChanges(start: F.at(10, 14), end: F.at(10, 15)))
        #expect(D.changes(d.saveIntent(scope: .thisAndFollowing)) == EventChanges(start: F.at(10, 14), end: F.at(10, 15)))
    }

    @Test func allMovesSeriesByTheSameDays() {
        var (_, d) = secondMonday()
        d.moveStart(to: D.time(10, 14)) // Di 10.11. statt Mo 9.11.
        let intent = d.saveIntent(scope: .all)
        // Serie beginnt dann Di 3.11. 14 Uhr, ohne `occurrence`.
        #expect(intent == .update(eventId: "ev_s", scope: .all, changes: EventChanges(start: F.at(3, 14), end: F.at(3, 15))))
    }

    @Test func allWithAllDaySeries() {
        let e = Event(id: "ev_s", calendarId: "cal_1", version: 1, details: EventDetails(
            title: "Müll", start: .date(D.day(2)), end: .date(D.day(3))
        ), recurrence: Recurrence(rrule: "FREQ=WEEKLY"))
        var (_, d) = secondMonday(e)
        #expect(d.start.date == D.day(9))
        d.end = LocalDateTime(date: D.day(10), hour: 10, minute: 0) // zwei Tage
        #expect(D.changes(d.saveIntent(scope: .all)) == EventChanges(start: .date(D.day(2)), end: .date(D.day(4))))
    }

    @Test func optimisticPreviewOfAllMatchesServerFrame() async {
        var s = SyncState()
        s.events["ev_s"] = D.series()
        let engine = EngineFixtures.engine(ScriptedTransport([]), store: InMemoryCacheStore(s))
        var (_, d) = secondMonday()
        d.moveStart(to: D.time(9, 11))
        await engine.perform(d.saveIntent(scope: .all)!, label: d.saveLabel)
        let e = await engine.visibleEvents()["ev_s"]
        #expect(e?.details.start == F.at(2, 11), "Serienbeginn bleibt am 2.11.")
        #expect(e?.details.end == F.at(2, 12))
    }

    @Test func changedRepetitionExcludesThis() {
        var (_, d) = secondMonday()
        d.repeatFrequency = .daily
        #expect(d.saveScopes == [.thisAndFollowing, .all])
        #expect(d.saveIntent(scope: .this) == nil)
        let c = D.changes(d.saveIntent(scope: .thisAndFollowing))
        #expect(c?.recurrence == .set(Recurrence(rrule: "FREQ=DAILY")))
        d.repeatFrequency = .none
        #expect(D.changes(d.saveIntent(scope: .all))?.recurrence == .clear)
    }

    @Test func repetitionKeepsExdates() {
        var e = D.series()
        e.recurrence?.exdates = [F.at(16, 9)]
        var (_, d) = secondMonday(e)
        d.repeatEnd = .count
        d.repeatCount = 5
        let c = D.changes(d.saveIntent(scope: .all))
        #expect(c?.recurrence == .set(Recurrence(rrule: "FREQ=WEEKLY;BYDAY=MO;COUNT=5", exdates: [F.at(16, 9)])))
    }

    @Test func ruleAndTimeTogetherMoveExdates() async {
        // LIC-48: Der Server ersetzt seine verschobenen EXDATEs durch die
        // mitgeschickten. Sie müssen also schon verschoben ankommen.
        var e = D.series()
        e.recurrence?.exdates = [F.at(16, 9)]
        var (_, d) = secondMonday(e)
        d.repeatEnd = .count
        d.repeatCount = 5
        d.moveStart(to: D.time(9, 10, 30))
        let c = D.changes(d.saveIntent(scope: .all))
        #expect(c?.start == F.at(2, 10, 30))
        #expect(c?.recurrence == .set(Recurrence(rrule: "FREQ=WEEKLY;BYDAY=MO;COUNT=5", exdates: [F.at(16, 10, 30)])))

        // Das ausgelassene Vorkommen bleibt ausgelassen, auch in der Vorschau.
        var s = SyncState()
        s.events["ev_s"] = e
        let engine = EngineFixtures.engine(ScriptedTransport([]), store: InMemoryCacheStore(s))
        await engine.perform(d.saveIntent(scope: .all)!, label: d.saveLabel)
        let moved = await engine.visibleEvents()["ev_s"]!
        let days = D.occurrence(of: moved).map { LocalDate($0.start, calendar: .lichtung(timeZone: D.tz)).day }
        #expect(days == [2, 9, 23], "16.11. fehlt weiter")
    }

    @Test func ruleAndDayTogetherMoveExdatesAcrossDays() {
        var e = D.series()
        e.recurrence?.exdates = [F.at(16, 9), .date(D.day(23))]
        var (_, d) = secondMonday(e)
        d.repeatFrequency = .daily
        d.moveStart(to: D.time(10, 8)) // Di statt Mo, 8 statt 9 Uhr
        let c = D.changes(d.saveIntent(scope: .all))
        #expect(c?.start == F.at(3, 8))
        // Ein reiner Tag bekommt die Uhrzeit der Serie und wandert mit.
        #expect(c?.recurrence == .set(Recurrence(rrule: "FREQ=DAILY", exdates: [F.at(17, 8), F.at(24, 8)])))
    }

    @Test func thisAndFollowingKeepsOnlyLaterExdatesMoved() {
        var e = D.series()
        e.recurrence?.exdates = [F.at(2, 9), F.at(16, 9)]
        e.recurrence?.rdates = [F.at(4, 9), F.at(18, 9)]
        var (_, d) = secondMonday(e)
        d.repeatEnd = .count
        d.repeatCount = 3
        d.moveStart(to: D.time(9, 11))
        let c = D.changes(d.saveIntent(scope: .thisAndFollowing))
        #expect(c?.start == F.at(9, 11))
        #expect(c?.recurrence == .set(Recurrence(rrule: "FREQ=WEEKLY;BYDAY=MO;COUNT=3", rdates: [F.at(18, 11)], exdates: [F.at(16, 11)])))
    }

    @Test func ruleChangeAloneKeepsExdates() {
        var e = D.series()
        e.recurrence?.exdates = [F.at(16, 9)]
        var (_, d) = secondMonday(e)
        d.repeatInterval = 2
        #expect(D.changes(d.saveIntent(scope: .all))?.recurrence == .set(Recurrence(rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO", exdates: [F.at(16, 9)])))
    }

    @Test func allDayToggleMovesExdatesToDays() {
        var e = D.series()
        e.recurrence?.exdates = [F.at(16, 9)]
        var (_, d) = secondMonday(e)
        d.allDay = true
        let c = D.changes(d.saveIntent(scope: .all))
        #expect(c?.start == .date(D.day(2)))
        #expect(c?.recurrence?.value?.exdates == [.date(D.day(16))])
    }

    @Test func utcExdateStaysUtc() {
        let berlin = D.tz
        let utc = DateValue.dateTime(LocalDateTime(date: D.day(16), hour: 8, minute: 0), timeZone: "UTC")
        let moved = SeriesShift.shift(utc, from: F.at(2, 9), to: F.at(2, 10), deviceTimeZone: berlin)
        #expect(moved == .dateTime(LocalDateTime(date: D.day(16), hour: 9, minute: 0), timeZone: "UTC"))
    }

    @Test func seriesOnlyFromRdatesOpensAsCustom() {
        var e = D.series()
        e.recurrence = Recurrence(rrule: nil, rdates: [F.at(9, 9)])
        let occ = D.occurrence(of: e)[0]
        var d = D.draft(occ, e)
        #expect(d.repetition == .custom(nil))
        #expect(d.repeatFrequency == .custom)
        d.title = "Kurs"
        #expect(D.changes(d.saveIntent(scope: .all)) == EventChanges(title: "Kurs"))
    }

    @Test func calendarChangeOnlyForAll() {
        var (_, d) = secondMonday()
        d.calendarId = "cal_2"
        d.title = "Neu"
        #expect(d.saveScopes == [.all])
        #expect(d.saveIntent(scope: .this) == nil)
        #expect(d.saveIntent(scope: .all) == .update(eventId: "ev_s", scope: .all, changes: EventChanges(title: "Neu", calendarId: "cal_2")))
    }

    @Test func customRuleStaysUntouched() {
        let e = D.series("FREQ=MONTHLY;BYDAY=2TU")
        let occ = D.occurrence(of: e)[0]
        var d = D.draft(occ, e)
        #expect(d.repetition == .custom("FREQ=MONTHLY;BYDAY=2TU"))
        #expect(d.repeatFrequency == .custom)
        #expect(d.frequencyChoices.contains(.custom))
        d.title = "Stammtisch"
        #expect(D.changes(d.saveIntent(scope: .all)) == EventChanges(title: "Stammtisch"))
        #expect(d.saveScopes == [.this, .thisAndFollowing, .all])
        d.repeatFrequency = .monthly
        #expect(D.changes(d.saveIntent(scope: .all))?.recurrence == .set(Recurrence(rrule: "FREQ=MONTHLY")))
        d.repeatFrequency = .custom
        #expect(d.repetition == .custom("FREQ=MONTHLY;BYDAY=2TU"))
        #expect(d.saveScopes == [.this, .thisAndFollowing, .all])
    }

    @Test func weekdayCheckOnlyForNewSeries() {
        // Regel vom Anbieter: jeden Dienstag, Beginn trotzdem Montag.
        let e = D.series("FREQ=WEEKLY;BYDAY=TU")
        let occ = D.occurrence(of: e)[0]
        var d = D.draft(occ, e)
        d.title = "Geht trotzdem"
        #expect(d.canSave)
    }

    @Test func deleteScopes() {
        let (_, d) = secondMonday()
        #expect(d.deleteIntent(scope: .this) == .delete(eventId: "ev_s", scope: .this, occurrence: F.at(9, 9)))
        #expect(d.deleteIntent(scope: .thisAndFollowing) == .delete(eventId: "ev_s", scope: .thisAndFollowing, occurrence: F.at(9, 9)))
        #expect(d.deleteIntent(scope: .all) == .delete(eventId: "ev_s"))
        #expect(EditScope.thisAndFollowing.editorLabel == "Dieser und alle folgenden")
    }
}

@Suite("Termin-Editor: Erinnerungen und Regeln")
struct EventDraftTextTests {
    @Test func alarmLabels() {
        #expect(EventDraft.alarmLabel(0, allDay: false) == "Zur Startzeit")
        #expect(EventDraft.alarmLabel(1, allDay: false) == "1 Minute vorher")
        #expect(EventDraft.alarmLabel(15, allDay: false) == "15 Minuten vorher")
        #expect(EventDraft.alarmLabel(60, allDay: false) == "1 Stunde vorher")
        #expect(EventDraft.alarmLabel(120, allDay: false) == "2 Stunden vorher")
        #expect(EventDraft.alarmLabel(1440, allDay: false) == "1 Tag vorher")
        #expect(EventDraft.alarmLabel(-5, allDay: false) == "5 Minuten nach Beginn")
        #expect(EventDraft.alarmLabel(-540, allDay: true) == "Am Tag des Termins um 9:00")
        #expect(EventDraft.alarmLabel(900, allDay: true) == "1 Tag vorher um 9:00")
        #expect(EventDraft.alarmLabel(2340, allDay: true) == "2 Tage vorher um 9:00")
        #expect(EventDraft.alarmLabel(9540, allDay: true) == "1 Woche vorher um 9:00")
        #expect(EventDraft.alarmPresets(allDay: false).prefix(7) == [0, 5, 10, 15, 30, 60, 120])
    }

    @Test(arguments: [
        ("FREQ=WEEKLY;BYDAY=MO,WE", RecurrenceRule(frequency: .weekly, weekdays: [.monday, .wednesday])),
        ("RRULE:FREQ=DAILY;INTERVAL=2;COUNT=4", RecurrenceRule(frequency: .daily, interval: 2, count: 4)),
        ("FREQ=MONTHLY;UNTIL=20270101", RecurrenceRule(frequency: .monthly, until: LocalDate(year: 2027, month: 1, day: 1))),
        // 31.12. 22:59:59Z ist in Berlin noch der 31.12.
        ("FREQ=YEARLY;UNTIL=20261231T225959Z", RecurrenceRule(frequency: .yearly, until: LocalDate(year: 2026, month: 12, day: 31))),
        ("FREQ=WEEKLY;WKST=SU;BYDAY=MO", RecurrenceRule.weekly(.monday)),
    ])
    func readsRulesItCanWrite(text: String, rule: RecurrenceRule) {
        #expect(RecurrenceRule(rrule: text, timeZone: SyncFixtures.tz) == rule)
    }

    @Test(arguments: [
        "FREQ=MONTHLY;BYDAY=2TU", "FREQ=MONTHLY;BYMONTHDAY=15", "FREQ=YEARLY;BYMONTH=3",
        "FREQ=HOURLY", "FREQ=DAILY;COUNT=3;UNTIL=20270101", "FREQ=WEEKLY;INTERVAL=2;WKST=SU;BYDAY=MO,TH",
        "kaputt",
    ])
    func leavesOtherRulesAlone(text: String) {
        #expect(RecurrenceRule(rrule: text, timeZone: SyncFixtures.tz) == nil)
    }

    @Test func countIsWrittenAndRoundTrips() {
        let r = RecurrenceRule(frequency: .weekly, weekdays: [.friday], count: 6)
        let text = r.rruleString(allDay: false, timeZone: SyncFixtures.tz)
        #expect(text == "FREQ=WEEKLY;BYDAY=FR;COUNT=6")
        #expect(RecurrenceRule(rrule: text, timeZone: SyncFixtures.tz) == r)
        // `until` hat Vorrang, beides zusammen gibt es nicht.
        #expect(RecurrenceRule(frequency: .daily, until: LocalDate(year: 2027, month: 1, day: 1), count: 3).count == nil)
    }
}
