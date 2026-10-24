import Foundation
@testable import LichtungKit
import Testing

@Suite("Terminsuche")
struct EventSearchTests {
    typealias F = SyncFixtures
    let search = EventSearch(deviceTimeZone: SyncFixtures.tz)

    func events() -> [Event] {
        var cafe = F.event("cafe", title: "Kaffee im Café Sonne", day: 5)
        cafe.details.location = "Hauptstraße 1"
        var team = F.event("team", title: "Team-Sync", day: 2, rrule: "FREQ=WEEKLY;BYDAY=MO")
        team.details.attendees = [Attendee(email: "mila@example.org", name: "Mila Beispiel")]
        let alt = F.event("alt", title: "Team-Workshop", day: 1).with(month: 9)
        let notiz = Event(id: "notiz", calendarId: "cal_1", details: EventDetails(
            title: "Zahnarzt", notes: "Bonusheft mitnehmen", start: F.at(10, 8), end: F.at(10, 9)
        ))
        return [cafe, team, alt, notiz]
    }

    @Test func ignoresCaseAndAccents() {
        #expect(search.search("CAFE", in: events(), now: F.now).map(\.id) == ["cafe"])
        #expect(search.search("strasse", in: events(), now: F.now).map(\.id) == ["cafe"])
    }

    @Test func allTermsMustMatch() {
        #expect(search.search("team sync", in: events(), now: F.now).map(\.id) == ["team"])
        #expect(search.search("team kaffee", in: events(), now: F.now).isEmpty)
        #expect(search.search("   ", in: events(), now: F.now).isEmpty)
    }

    @Test func searchesNotesAndAttendees() {
        #expect(search.search("bonusheft", in: events(), now: F.now).map(\.id) == ["notiz"])
        #expect(search.search("mila", in: events(), now: F.now).map(\.id) == ["team"])
    }

    @Test func upcomingFirstThenPast() {
        let hits = search.search("team", in: events(), now: F.now)
        // Serie beginnt Mo 2.11.; der Workshop lag im September.
        #expect(hits.map(\.id) == ["team", "alt"])
        let next = hits[0].occurrence!
        #expect(APICoding.formatTimestamp(next.start) == "2026-11-02T08:00:00Z")
    }
}

@Suite("Rückgängig und Wiederholen")
struct UndoTests {
    typealias F = SyncFixtures

    func engine(_ seeded: SyncState = SyncState()) -> SyncEngine {
        EngineFixtures.engine(ScriptedTransport([]), store: InMemoryCacheStore(seeded))
    }

    @Test func undoMoveRestoresTimes() async {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1", version: 3, hour: 9)
        let e = engine(s)
        await e.perform(.update(eventId: "ev_1", changes: EventChanges(start: F.at(3, 11), end: F.at(3, 12))), label: "Termin verschieben")
        #expect(await e.visibleEvents()["ev_1"]?.details.start == F.at(3, 11))
        #expect(await e.undoHistory.undoLabel == "Termin verschieben")

        #expect(await e.undo())
        #expect(await e.visibleEvents()["ev_1"]?.details.start == F.at(3, 9))
        #expect(await e.visibleEvents()["ev_1"]?.details.end == F.at(3, 10))
        #expect(await e.undoHistory.canRedo)

        #expect(await e.redo())
        #expect(await e.visibleEvents()["ev_1"]?.details.start == F.at(3, 11))
        // Alle Schritte gehen als Mutationen in die Outbox.
        #expect(await e.state.outbox.count == 3)
    }

    @Test func undoCreateDeletesPlaceholder() async {
        let e = engine()
        let id = await e.perform(.create(calendarId: "cal_1", event: EventInput(title: "Neu", start: F.at(4, 9), end: F.at(4, 10))), label: "Termin anlegen")
        #expect(await e.visibleEvents()[id!] != nil)
        await e.undo()
        #expect(await e.visibleEvents()[id!] == nil)
    }

    @Test func undoDeleteRecreatesWithSameUID() async {
        var s = SyncState()
        var ev = F.event("ev_1", title: "Zahnarzt")
        ev.uid = "uid-1@example.org"
        ev.details.location = "Praxis"
        s.events["ev_1"] = ev
        let e = engine(s)
        await e.perform(.delete(eventId: "ev_1"), label: "Termin löschen")
        #expect(await e.visibleEvents()["ev_1"] == nil)
        await e.undo()
        let recreated = await e.visibleEvents().values.first { $0.title == "Zahnarzt" }
        #expect(recreated?.details.location == "Praxis")
        #expect(recreated?.uid == "uid-1@example.org")
    }

    @Test func undoClearedFieldSetsItAgain() async {
        var s = SyncState()
        var ev = F.event("ev_1")
        ev.details.location = "Raum 2"
        s.events["ev_1"] = ev
        let e = engine(s)
        await e.perform(.update(eventId: "ev_1", changes: EventChanges(location: .clear)), label: "Ort entfernen")
        #expect(await e.visibleEvents()["ev_1"]?.details.location == nil)
        await e.undo()
        #expect(await e.visibleEvents()["ev_1"]?.details.location == "Raum 2")
    }

    @Test func undoDeleteOfOneOccurrence() async {
        var s = SyncState()
        s.events["ev_s"] = F.event("ev_s", day: 2, rrule: "FREQ=WEEKLY;BYDAY=MO")
        let e = engine(s)
        await e.perform(.delete(eventId: "ev_s", scope: .this, occurrence: F.at(9, 9)), label: "Vorkommen löschen")
        #expect(await e.visibleEvents()["ev_s"]?.recurrence?.exdates == [F.at(9, 9)])
        await e.undo()
        #expect(await e.visibleEvents()["ev_s"]?.recurrence?.exdates == [])
    }

    @Test func notUndoableClearsHistory() async {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1")
        s.events["ev_s"] = F.event("ev_s", day: 2, rrule: "FREQ=WEEKLY;BYDAY=MO")
        let e = engine(s)
        await e.perform(.update(eventId: "ev_1", changes: EventChanges(title: "A")), label: "Umbenennen")
        await e.perform(.update(eventId: "ev_s", scope: .thisAndFollowing, occurrence: F.at(9, 9), changes: EventChanges(title: "B")), label: "Serie ändern")
        #expect(await e.undoHistory.canUndo == false)
    }

    @Test func newActionClearsRedo() async {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1", title: "Alt")
        let e = engine(s)
        await e.perform(.update(eventId: "ev_1", changes: EventChanges(title: "Eins")), label: "Umbenennen")
        await e.undo()
        await e.perform(.update(eventId: "ev_1", changes: EventChanges(title: "Zwei")), label: "Umbenennen")
        #expect(await e.undoHistory.canRedo == false)
        #expect(await e.undo() == true)
        #expect(await e.visibleEvents()["ev_1"]?.title == "Alt")
        #expect(await e.undo() == false)
    }

    @Test func undoOfFollowUpUsesCreatedServerId() async throws {
        // Anlegen, gesendet, dann Rückgängig: Die Löschung muss die Server-ID treffen.
        let t = ScriptedTransport([
            { req in
                let id = ((jsonObject(req.body ?? Data())["mutations"] as? [[String: Any]])?.first?["id"] as? String) ?? "?"
                return HTTPResponse(status: 200, body: Data(#"{"results": [{"id": "\#(id)", "status": "applied", "events": [\#(EngineFixtures.eventJSON("ev_77", version: 2, title: "Neu"))]}]}"#.utf8))
            },
            ScriptedTransport.json(ContractSamples.pull("", cursor: 1)),
        ])
        let e = EngineFixtures.engine(t)
        await e.perform(.create(calendarId: "cal_1", event: EventInput(title: "Neu", start: F.at(4, 9), end: F.at(4, 10))), label: "Anlegen")
        try await e.sync()
        await e.undo()
        guard case let .deleteEvent(id, base, _, _)? = await e.state.outbox.last?.mutation.operation else {
            Issue.record("keine Löschung"); return
        }
        #expect(id == "ev_77")
        #expect(base == 2)
    }

    @Test func historyIsLimited() {
        var h = UndoHistory()
        h.limit = 3
        for i in 0..<5 { h.recordNew(label: "\(i)", inverse: .delete(eventId: "e\(i)")) }
        #expect(h.undoStack.map(\.label) == ["2", "3", "4"])
    }
}

@Suite("Duplizieren")
struct DuplicateTests {
    typealias F = SyncFixtures

    func occurrence() -> Occurrence {
        var e = F.event("ev_1", title: "Workshop", day: 3, hour: 10)
        e.uid = "uid-x@example.org"
        e.details.location = "Raum 4"
        e.details.attendees = [Attendee(email: "gast@example.org")]
        e.details.alarms = [.minutesBefore(15)]
        return SeriesExpander(deviceTimeZone: F.tz).occurrences(of: e, in: DateInterval(start: .distantPast, end: .distantFuture))[0]
    }

    @Test func copiesWithoutUIDAndAttendees() {
        let input = EventDuplicate.input(from: occurrence())
        #expect(input.title == "Workshop")
        #expect(input.location == "Raum 4")
        #expect(input.alarms == [.minutesBefore(15)])
        #expect(input.uid == nil)
        #expect(input.attendees == nil)
        #expect(input.recurrence == nil)
        #expect(input.start == F.at(3, 10))
    }

    @Test func shiftKeepsWallClock() {
        // Über das Sommerzeit-Ende hinweg bleibt 10:00 10:00.
        let input = EventDuplicate.input(from: occurrence(), shiftedByDays: -10)
        #expect(input.start == F.at(24, 10, month: 10))
        #expect(input.end == F.at(24, 11, month: 10))
    }

    @Test func attendeesOnlyOnRequest() {
        #expect(EventDuplicate.input(from: occurrence(), includeAttendees: true).attendees?.map(\.email) == ["gast@example.org"])
    }

    @Test func moveToNewStartKeepsDuration() {
        let input = EventDuplicate.input(from: occurrence(), movedTo: F.at(5, 14, 30), deviceTimeZone: F.tz)
        #expect(input.start == F.at(5, 14, 30))
        #expect(input.end.normalized == F.at(5, 15, 30))
    }

    @Test func duplicateOfSeriesOccurrenceIsSingle() {
        let series = F.event("ev_s", title: "Review", day: 2, hour: 9, rrule: "FREQ=WEEKLY;BYDAY=MO")
        let window = DateInterval(start: APICoding.parseTimestamp("2026-11-08T23:00:00Z")!, duration: 86_400)
        let occ = SeriesExpander(deviceTimeZone: F.tz).occurrences(of: series, in: window)[0]
        let input = EventDuplicate.input(from: occ)
        #expect(input.recurrence == nil)
        #expect(input.start.normalized == F.at(9, 9))
    }
}
