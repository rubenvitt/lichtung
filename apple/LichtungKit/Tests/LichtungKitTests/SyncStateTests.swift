import Foundation
@testable import LichtungKit
import Testing

/// Bausteine für Sync-Tests (erfundene Daten).
enum SyncFixtures {
    static let tz = TimeZone(identifier: "Europe/Berlin")!
    static let now = APICoding.parseTimestamp("2026-10-23T08:00:00Z")!

    static func at(_ d: Int, _ h: Int, _ m: Int = 0, month: Int = 11) -> DateValue {
        .dateTime(LocalDateTime(date: LocalDate(year: 2026, month: month, day: d), hour: h, minute: m), timeZone: "Europe/Berlin")
    }

    static func event(_ id: String, version: Int = 1, title: String = "Termin", day: Int = 3, hour: Int = 9,
                      rrule: String? = nil, calendar: String = "cal_1") -> Event {
        Event(
            id: id, calendarId: calendar, version: version,
            details: EventDetails(title: title, start: at(day, hour), end: at(day, hour + 1)),
            recurrence: rrule.map { Recurrence(rrule: $0) }
        )
    }

    static func calendar(_ id: String, hidden: Bool = false) -> CalendarInfo {
        CalendarInfo(id: id, accountId: "acc_1", name: id, hidden: hidden)
    }

    static func applied(_ id: String, _ events: [Event] = [], deleted: [String] = []) -> MutationResult {
        MutationResult(id: id, status: .applied, events: events, deletedEventIds: deleted)
    }

    static func page(_ changes: [SyncChange], cursor: Int, hasMore: Bool = false) -> PullPage {
        PullPage(cursor: cursor, hasMore: hasMore, changes: changes)
    }
}

@Suite("SyncState: Pull")
struct SyncStatePullTests {
    typealias F = SyncFixtures

    @Test func appliesUpsertsAndDeletes() {
        var s = SyncState()
        s.apply(F.page([.upsertCalendar(F.calendar("cal_1")), .upsertEvent(F.event("ev_1")), .upsertEvent(F.event("ev_2"))], cursor: 5))
        s.apply(F.page([.delete(type: "event", id: "ev_1"), .delete(type: "event", id: "ev_unbekannt")], cursor: 7))
        #expect(s.cursor == 7)
        #expect(Set(s.events.keys) == ["ev_2"])
        #expect(s.calendars["cal_1"] != nil)
    }

    @Test func olderVersionDoesNotReplaceNewer() {
        var s = SyncState()
        s.apply(F.page([.upsertEvent(F.event("ev_1", version: 5, title: "neu"))], cursor: 2))
        s.apply(F.page([.upsertEvent(F.event("ev_1", version: 4, title: "alt"))], cursor: 3))
        #expect(s.events["ev_1"]?.title == "neu")
    }

    @Test func resetKeepsOutbox() {
        var s = SyncState()
        s.apply(F.page([.upsertEvent(F.event("ev_1"))], cursor: 9))
        s.enqueue(Mutation(.deleteEvent(eventId: "ev_1", baseVersion: 1, scope: .all, occurrence: nil)), now: F.now)
        s.resetMirror()
        #expect(s.cursor == 0)
        #expect(s.events.isEmpty)
        #expect(s.outbox.count == 1)
    }

    @Test func pruneKeepsSeriesAndPinned() {
        var s = SyncState()
        s.events = [
            "alt": F.event("alt", day: 1, hour: 9).with(month: 1),
            "serie": F.event("serie", day: 1, rrule: "FREQ=DAILY").with(month: 1),
            "drin": F.event("drin", day: 3),
            "angeheftet": F.event("angeheftet", day: 2).with(month: 1),
        ]
        s.enqueue(Mutation(.updateEvent(eventId: "angeheftet", baseVersion: 1, scope: .all, occurrence: nil, changes: EventChanges(title: "x"))), now: F.now)
        let window = DateInterval(start: APICoding.parseTimestamp("2026-07-01T00:00:00Z")!, end: APICoding.parseTimestamp("2027-07-01T00:00:00Z")!)
        s.prune(to: window, deviceTimeZone: F.tz)
        #expect(Set(s.events.keys) == ["serie", "drin", "angeheftet"])
    }
}

@Suite("SyncState: Outbox")
struct SyncStateOutboxTests {
    typealias F = SyncFixtures

    @Test func createShowsPlaceholderUntilApplied() {
        var s = SyncState()
        let m = Mutation(id: "m1", .createEvent(calendarId: "cal_1", event: EventInput(title: "Zahnarzt", start: F.at(3, 14), end: F.at(3, 15))))
        let localId = s.enqueue(m, now: F.now)
        #expect(localId == "local:m1")
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["local:m1"]?.title == "Zahnarzt")
        #expect(s.isPending("local:m1"))

        s.handle([F.applied("m1", [F.event("ev_9", title: "Zahnarzt")])], now: F.now)
        let visible = s.visibleEvents(deviceTimeZone: F.tz)
        #expect(visible["local:m1"] == nil)
        #expect(visible["ev_9"]?.title == "Zahnarzt")
        #expect(s.currentId(for: "local:m1") == "ev_9")
        #expect(s.outbox.isEmpty)
    }

    @Test func followUpOnPlaceholderWaitsAndIsRewritten() {
        var s = SyncState()
        s.enqueue(Mutation(id: "m1", .createEvent(calendarId: "cal_1", event: EventInput(title: "A", start: F.at(3, 9), end: F.at(3, 10)))), now: F.now)
        s.enqueue(Mutation(id: "m2", .updateEvent(eventId: "local:m1", baseVersion: 0, scope: .all, occurrence: nil, changes: EventChanges(title: "B"))), now: F.now)
        // Die Änderung am lokalen Event wird angezeigt …
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["local:m1"]?.title == "B")
        // … aber erst nach dem Anlegen gesendet.
        #expect(s.nextBatch(now: F.now).map(\.id) == ["m1"])

        s.handle([F.applied("m1", [F.event("ev_9", version: 3, title: "A")])], now: F.now)
        #expect(s.nextBatch(now: F.now).map(\.id) == ["m2"])
        guard case let .updateEvent(eid, base, _, _, _) = s.outbox[0].mutation.operation else { Issue.record("keine Änderung"); return }
        #expect(eid == "ev_9")
        #expect(base == 3)
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["ev_9"]?.title == "B")
    }

    @Test func secondChangeToSameEventGoesInNextBatch() {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1", version: 2)
        s.enqueue(Mutation(id: "a", .updateEvent(eventId: "ev_1", baseVersion: 2, scope: .all, occurrence: nil, changes: EventChanges(title: "1"))), now: F.now)
        s.enqueue(Mutation(id: "b", .updateEvent(eventId: "ev_1", baseVersion: 2, scope: .all, occurrence: nil, changes: EventChanges(title: "2"))), now: F.now)
        #expect(s.nextBatch(now: F.now).map(\.id) == ["a"])
        s.handle([F.applied("a", [F.event("ev_1", version: 3, title: "1")])], now: F.now)
        guard case let .updateEvent(_, base, _, _, _) = s.outbox[0].mutation.operation else { Issue.record("weg"); return }
        #expect(base == 3)
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["ev_1"]?.title == "2")
    }

    @Test func batchRespectsLimitAndOrder() {
        var s = SyncState()
        for i in 0..<5 {
            s.enqueue(Mutation(id: "m\(i)", .deleteEvent(eventId: "ev_\(i)", baseVersion: 1, scope: .all, occurrence: nil)), now: F.now)
        }
        #expect(s.nextBatch(now: F.now, limit: 3).map(\.id) == ["m0", "m1", "m2"])
    }

    @Test func rejectedRollsBackAndNotifies() {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1", title: "Original")
        s.enqueue(Mutation(id: "a", .updateEvent(eventId: "ev_1", baseVersion: 1, scope: .all, occurrence: nil, changes: EventChanges(title: "Geändert"))), now: F.now)
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["ev_1"]?.title == "Geändert")
        s.handle([MutationResult(id: "a", status: .rejected, error: APIErrorBody(code: "calendar_read_only", message: "Der Kalender ist nur lesbar."))], now: F.now)
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["ev_1"]?.title == "Original")
        #expect(s.notices.map(\.message) == ["Der Kalender ist nur lesbar."])
        s.dismissNotice("a")
        #expect(s.notices.isEmpty)
    }

    @Test func rejectedCreateDropsItsFollowUps() {
        var s = SyncState()
        s.enqueue(Mutation(id: "m1", .createEvent(calendarId: "cal_ro", event: EventInput(title: "A", start: F.at(3, 9), end: F.at(3, 10)))), now: F.now)
        s.enqueue(Mutation(id: "m2", .deleteEvent(eventId: "local:m1", baseVersion: 0, scope: .all, occurrence: nil)), now: F.now)
        s.handle([MutationResult(id: "m1", status: .rejected, error: APIErrorBody(code: "calendar_read_only", message: "Nur lesbar."))], now: F.now)
        #expect(s.outbox.isEmpty)
    }

    @Test func conflictTakesProviderVersion() {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1", version: 4, title: "Alt")
        s.enqueue(Mutation(id: "a", .updateEvent(eventId: "ev_1", baseVersion: 4, scope: .all, occurrence: nil, changes: EventChanges(title: "Meins"))), now: F.now)
        let conflict = Conflict(id: "cf_1", eventId: "ev_1", calendarId: "cal_1", origin: "app:iPhone", createdAt: F.now, message: "Der Termin wurde inzwischen bei Fastmail geändert.", mutation: nil)
        s.handle([MutationResult(id: "a", status: .conflict, events: [F.event("ev_1", version: 6, title: "Anbieter")], conflict: conflict)], now: F.now)
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["ev_1"]?.title == "Anbieter")
        #expect(s.conflicts["cf_1"]?.message.contains("Fastmail") == true)
        #expect(s.outbox.isEmpty)
    }

    @Test func conflictOnDeletedEventRemovesIt() {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1", version: 4)
        s.enqueue(Mutation(id: "a", .updateEvent(eventId: "ev_1", baseVersion: 4, scope: .all, occurrence: nil, changes: EventChanges(title: "x"))), now: F.now)
        s.handle([MutationResult(id: "a", status: .conflict, events: [], conflict: nil)], now: F.now)
        #expect(s.events["ev_1"] == nil)
    }

    @Test func retryWaitsAndKeepsOrder() {
        var s = SyncState()
        s.enqueue(Mutation(id: "a", .deleteEvent(eventId: "ev_1", baseVersion: 1, scope: .all, occurrence: nil)), now: F.now)
        s.enqueue(Mutation(id: "b", .deleteEvent(eventId: "ev_2", baseVersion: 1, scope: .all, occurrence: nil)), now: F.now)
        s.handle([
            MutationResult(id: "a", status: .retry, error: APIErrorBody(code: "provider_unavailable", message: "weg"), retryAfterSeconds: 60),
            MutationResult(id: "b", status: .retry, error: APIErrorBody(code: "not_attempted", message: "später")),
        ], now: F.now)
        #expect(s.outbox.map(\.id) == ["a", "b"])
        #expect(s.nextBatch(now: F.now.addingTimeInterval(30)).isEmpty)
        #expect(s.nextBatch(now: F.now.addingTimeInterval(61)).map(\.id) == ["a", "b"])
    }

    @Test func backoffGrowsAndCaps() {
        #expect(SyncState.backoff(attempt: 1) == 5)
        #expect(SyncState.backoff(attempt: 2) == 10)
        #expect(SyncState.backoff(attempt: 20) == 600)
    }

    @Test func resultForUnknownMutationIsIgnored() {
        var s = SyncState()
        s.handle([F.applied("nie-gesendet", [F.event("ev_x")])], now: F.now)
        #expect(s.events.isEmpty)
    }
}

@Suite("Optimistische Anzeige bei Serien")
struct OptimisticSeriesTests {
    typealias F = SyncFixtures

    func seriesState() -> SyncState {
        var s = SyncState()
        s.events["ev_s"] = F.event("ev_s", version: 2, title: "Review", day: 2, hour: 9, rrule: "FREQ=WEEKLY;BYDAY=MO;COUNT=10")
        return s
    }

    @Test func updateThisCreatesOverride() {
        var s = seriesState()
        s.enqueue(Mutation(.updateEvent(eventId: "ev_s", baseVersion: 2, scope: .this, occurrence: F.at(9, 9),
                                        changes: EventChanges(title: "Review (verschoben)", start: F.at(9, 11), end: F.at(9, 12)))), now: F.now)
        let e = s.visibleEvents(deviceTimeZone: F.tz)["ev_s"]!
        #expect(e.title == "Review")
        #expect(e.overrides.count == 1)
        #expect(e.overrides[0].recurrenceId == F.at(9, 9))
        #expect(e.overrides[0].details.title == "Review (verschoben)")
        #expect(e.overrides[0].details.start == F.at(9, 11))
    }

    @Test func overrideKeepsDurationWhenOnlyTitleChanges() {
        var s = seriesState()
        s.enqueue(Mutation(.updateEvent(eventId: "ev_s", baseVersion: 2, scope: .this, occurrence: F.at(9, 9), changes: EventChanges(title: "Anders"))), now: F.now)
        let o = s.visibleEvents(deviceTimeZone: F.tz)["ev_s"]!.overrides[0]
        #expect(o.details.start == F.at(9, 9))
        #expect(o.details.end.normalized == F.at(9, 10))
    }

    @Test func deleteThisAddsExdate() {
        var s = seriesState()
        s.enqueue(Mutation(.deleteEvent(eventId: "ev_s", baseVersion: 2, scope: .this, occurrence: F.at(16, 9))), now: F.now)
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["ev_s"]?.recurrence?.exdates == [F.at(16, 9)])
    }

    @Test func thisAndFollowingSplitsSeries() {
        var s = seriesState()
        s.enqueue(Mutation(id: "m", .updateEvent(eventId: "ev_s", baseVersion: 2, scope: .thisAndFollowing, occurrence: F.at(16, 9),
                                                 changes: EventChanges(title: "Review neu"))), now: F.now)
        let v = s.visibleEvents(deviceTimeZone: F.tz)
        // 16.11. 9:00 Berlin = 8:00 UTC, UNTIL eine Sekunde davor.
        #expect(v["ev_s"]?.recurrence?.rrule == "FREQ=WEEKLY;BYDAY=MO;UNTIL=20261116T075959Z")
        #expect(v["local:m"]?.title == "Review neu")
        #expect(v["local:m"]?.details.start == F.at(16, 9))
        #expect(v["local:m"]?.recurrence?.rrule == "FREQ=WEEKLY;BYDAY=MO")
    }

    @Test func deleteFollowingOfAllDaySeries() {
        var s = SyncState()
        s.events["ev_a"] = Event(id: "ev_a", calendarId: "cal_1", details: EventDetails(
            title: "Müll", start: .date(LocalDate(year: 2026, month: 11, day: 3)), end: .date(LocalDate(year: 2026, month: 11, day: 4))
        ), recurrence: Recurrence(rrule: "FREQ=WEEKLY;INTERVAL=2"))
        s.enqueue(Mutation(.deleteEvent(eventId: "ev_a", baseVersion: 1, scope: .thisAndFollowing, occurrence: .date(LocalDate(year: 2026, month: 12, day: 1)))), now: F.now)
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["ev_a"]?.recurrence?.rrule == "FREQ=WEEKLY;INTERVAL=2;UNTIL=20261130")
    }

    @Test func respondSetsOwnStatus() {
        var s = SyncState()
        var e = F.event("ev_i")
        e.details.selfStatus = .needsAction
        e.details.attendees = [Attendee(email: "ich@example.org", status: .needsAction, isSelf: true)]
        s.events["ev_i"] = e
        #expect(CalendarQuery(state: s, deviceTimeZone: F.tz).pendingInvitations(from: F.now, deviceTimeZone: F.tz).map(\.id) == ["ev_i"])
        s.enqueue(Mutation(.respond(eventId: "ev_i", occurrence: nil, status: .accepted, comment: nil)), now: F.now)
        let v = s.visibleEvents(deviceTimeZone: F.tz)["ev_i"]!
        #expect(v.details.selfStatus == .accepted)
        #expect(v.details.attendees[0].status == .accepted)
        #expect(CalendarQuery(state: s, deviceTimeZone: F.tz).pendingInvitations(from: F.now, deviceTimeZone: F.tz).isEmpty)
    }

    @Test func moveToOtherCalendar() {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1")
        s.enqueue(Mutation(.updateEvent(eventId: "ev_1", baseVersion: 1, scope: .all, occurrence: nil, changes: EventChanges(calendarId: "cal_2"))), now: F.now)
        #expect(s.visibleEvents(deviceTimeZone: F.tz)["ev_1"]?.calendarId == "cal_2")
    }
}

@Suite("CalendarQuery")
struct CalendarQueryTests {
    typealias F = SyncFixtures

    @Test func filtersHiddenAndSets() {
        var s = SyncState()
        s.calendars = ["cal_1": F.calendar("cal_1"), "cal_2": F.calendar("cal_2"), "cal_x": F.calendar("cal_x", hidden: true)]
        s.calendarSets = ["set_a": CalendarSet(id: "set_a", name: "Arbeit", calendarIds: ["cal_2", "cal_x"])]
        s.events = [
            "a": F.event("a", title: "Eins", calendar: "cal_1"),
            "b": F.event("b", title: "Zwei", calendar: "cal_2"),
            "c": F.event("c", title: "Versteckt", calendar: "cal_x"),
        ]
        let q = CalendarQuery(state: s, deviceTimeZone: F.tz)
        let day = DateInterval(start: APICoding.parseTimestamp("2026-11-02T23:00:00Z")!, duration: 86_400)
        #expect(q.occurrences(in: day).map(\.title) == ["Eins", "Zwei"])
        #expect(q.occurrences(in: day, selection: .set("set_a")).map(\.title) == ["Zwei"])
    }

    @Test func allDayFirstThenByStart() {
        var s = SyncState()
        s.calendars = ["cal_1": F.calendar("cal_1")]
        s.events = [
            "spaet": F.event("spaet", title: "Spät", hour: 15),
            "frueh": F.event("frueh", title: "Früh", hour: 8),
            "ganz": Event(id: "ganz", calendarId: "cal_1", details: EventDetails(
                title: "Ganztags", start: .date(LocalDate(year: 2026, month: 11, day: 3)), end: .date(LocalDate(year: 2026, month: 11, day: 4))
            )),
        ]
        let q = CalendarQuery(state: s, deviceTimeZone: F.tz)
        let day = DateInterval(start: APICoding.parseTimestamp("2026-11-02T23:00:00Z")!, duration: 86_400)
        #expect(q.occurrences(in: day).map(\.title) == ["Ganztags", "Früh", "Spät"])
    }
}

extension Event {
    func with(month: Int) -> Event {
        var e = self
        func move(_ v: DateValue) -> DateValue {
            guard case let .dateTime(l, tz, _) = v else { return v }
            var l2 = l
            l2.date.month = month
            return .dateTime(l2, timeZone: tz)
        }
        e.details.start = move(e.details.start)
        e.details.end = move(e.details.end)
        return e
    }
}
