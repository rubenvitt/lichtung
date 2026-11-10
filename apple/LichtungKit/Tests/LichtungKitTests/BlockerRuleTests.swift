import Foundation
@testable import LichtungKit
import Testing

@Suite("Blocker-Regeln (LIC-75)")
struct BlockerRuleTests {
    func api(_ t: ScriptedTransport) -> ManagementAPI {
        ManagementAPI(sync: SyncAPI(baseURL: EngineFixtures.base, transport: t, tokens: StaticToken("ld_test")))
    }

    static let rule = #"{"id": "blr_1", "source_calendar_id": "cal_privat", "target_calendar_id": "cal_arbeit", "title": "Blockiert", "all_day": false, "blocker_count": 3, "failed_count": 1, "last_error": "Fastmail ist gerade nicht erreichbar.", "created_at": "2026-11-10T20:00:00Z", "updated_at": "2026-11-10T20:00:00Z"}"#

    static let calendars: [String: CalendarInfo] = [
        "cal_privat": CalendarInfo(id: "cal_privat", accountId: "a", name: "Privat", writable: true, position: 0),
        "cal_arbeit": CalendarInfo(id: "cal_arbeit", accountId: "a", name: "Arbeit", writable: true, position: 1),
        "cal_abo": CalendarInfo(id: "cal_abo", accountId: "b", name: "Feiertage", writable: false, position: 2),
    ]

    @Test func listCreateUpdateDelete() async throws {
        let t = ScriptedTransport([
            ScriptedTransport.json("[\(Self.rule)]"),
            ScriptedTransport.json(Self.rule, status: 201),
            ScriptedTransport.json(Self.rule),
            ScriptedTransport.json("", status: 204),
        ])
        let api = api(t)
        let rules = try await api.blockerRules()
        #expect(rules.count == 1)
        #expect(rules[0].blockerCount == 3)
        #expect(rules[0].failedCount == 1)
        #expect(rules[0].lastError?.contains("nicht erreichbar") == true)

        _ = try await api.createBlockerRule(NewBlockerRule(sourceCalendarId: "cal_privat", targetCalendarId: "cal_arbeit"))
        #expect(t.recorded[1].method == "POST")
        #expect(t.recorded[1].url.path == "/api/blocker-rules")
        let body = jsonObject(t.recorded[1].body ?? Data())
        #expect(body["source_calendar_id"] as? String == "cal_privat")
        #expect(body["target_calendar_id"] as? String == "cal_arbeit")
        #expect(body["all_day"] as? Bool == false)
        #expect(body["title"] == nil)

        _ = try await api.updateBlockerRule("blr_1", BlockerRulePatch(title: "Belegt"))
        #expect(t.recorded[2].method == "PATCH")
        #expect(String(decoding: t.recorded[2].body ?? Data(), as: UTF8.self) == #"{"title":"Belegt"}"#)

        try await api.deleteBlockerRule("blr_1")
        #expect(t.recorded[3].method == "DELETE")
        #expect(t.recorded[3].url.path == "/api/blocker-rules/blr_1")
    }

    @Test func targetsAndSources() {
        #expect(BlockerTargets.targets(Self.calendars).map(\.id) == ["cal_privat", "cal_arbeit"])
        #expect(BlockerTargets.sources(Self.calendars).map(\.id) == ["cal_privat", "cal_arbeit", "cal_abo"])
    }

    @Test func canCreate() throws {
        let existing = [try APICoding.decoder.decode(BlockerRule.self, from: Data(Self.rule.utf8))]
        func can(_ s: String?, _ t: String?) -> Bool {
            BlockerTargets.canCreate(source: s, target: t, calendars: Self.calendars, existing: existing)
        }
        #expect(can("cal_abo", "cal_arbeit"))
        #expect(can("cal_arbeit", "cal_privat"), "die Gegenrichtung ist erlaubt")
        #expect(!can("cal_privat", "cal_arbeit"), "gibt es schon")
        #expect(!can("cal_privat", "cal_privat"))
        #expect(!can("cal_privat", "cal_abo"), "Abo ist nur lesbar")
        #expect(!can(nil, "cal_arbeit"))
        #expect(!can("cal_weg", "cal_arbeit"))
    }
}
