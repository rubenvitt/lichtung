import Foundation
@testable import LichtungKit
import Testing

@Suite("Konflikte und abgelehnte Änderungen (LIC-70)")
struct SyncProblemTests {
    typealias F = SyncFixtures

    func state() -> SyncState {
        var s = SyncState()
        s.calendars["cal_1"] = F.calendar("cal_1")
        s.events["ev_1"] = F.event("ev_1", title: "Zahnarzt")
        s.conflicts["cf_1"] = Conflict(
            id: "cf_1", eventId: "ev_1", calendarId: "cal_1", origin: "app:iPhone",
            createdAt: berlinDate(2026, 11, 3, 9, 1), message: "Der Termin wurde inzwischen bei Fastmail geändert.",
            mutation: nil
        )
        s.conflicts["cf_2"] = Conflict(
            id: "cf_2", eventId: nil, calendarId: "cal_1", origin: "mcp:Planung",
            createdAt: berlinDate(2026, 11, 4, 8), message: "Der Termin wurde inzwischen gelöscht.",
            mutation: .object(["op": .string("update_event"), "changes": .object(["title": .string("Review")])])
        )
        s.notices = [SyncNotice(id: "m_1", message: "Kalender ist schreibgeschützt.", code: "forbidden",
                                createdAt: berlinDate(2026, 11, 2, 12))]
        return s
    }

    @Test func newestFirstWithTitlesAndOrigin() {
        let p = state().problems(deviceTimeZone: F.tz)
        #expect(p.map(\.id) == ["cf_2", "cf_1", "m_1"])
        #expect(p[0].title == "Nicht übernommen: Review")
        #expect(p[0].eventId == nil)
        #expect(p[0].detail == "Geändert über Claude (Planung), 4.11., 8:00")
        #expect(p[1].title == "Nicht übernommen: Zahnarzt")
        #expect(p[1].eventId == "ev_1")
        #expect(p[1].detail == "Geändert auf iPhone, 3.11., 9:01")
        #expect(p[1].kind == .conflict)
        #expect(p[2].kind == .rejected)
        #expect(p[2].title == "Vom Server abgelehnt")
    }

    @Test func deletedConflictDisappears() {
        var s = state()
        s.apply(PullPage(cursor: 9, hasMore: false, changes: [.delete(type: "conflict", id: "cf_1")]))
        #expect(s.problems(deviceTimeZone: F.tz).map(\.id) == ["cf_2", "m_1"])
        s.dismissNotice("m_1")
        #expect(s.problems(deviceTimeZone: F.tz).map(\.id) == ["cf_2"])
    }

    @Test func originTexts() {
        #expect(SyncState.originText("web") == "Geändert im Browser")
        #expect(SyncState.originText("api:Skript") == "Geändert über API-Token Skript")
        #expect(SyncState.originText("server") == nil)
        #expect(SyncState.originText(nil) == nil)
    }
}
