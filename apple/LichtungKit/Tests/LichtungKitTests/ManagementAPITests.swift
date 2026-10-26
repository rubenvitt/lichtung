import Foundation
@testable import LichtungKit
import Testing

@Suite("Verwaltungs-API")
struct ManagementAPITests {
    func api(_ t: ScriptedTransport) -> ManagementAPI {
        ManagementAPI(sync: SyncAPI(baseURL: EngineFixtures.base, transport: t, tokens: StaticToken("ld_test")))
    }

    static let account = #"{"id": "acc_9", "kind": "fastmail", "name": "Fastmail", "username": "person@example.org", "status": {"state": "syncing"}}"#

    @Test func addAccountSendsCredentialsOnce() async throws {
        let t = ScriptedTransport([ScriptedTransport.json(Self.account, status: 201)])
        let a = try await api(t).addAccount(.fastmail(username: "person@example.org", appPassword: "erfunden-123"))
        #expect(a.id == "acc_9")
        #expect(a.status?.state == .syncing)
        let req = t.recorded[0]
        #expect(req.method == "POST")
        #expect(req.url.path == "/api/accounts")
        let body = jsonObject(req.body ?? Data())
        #expect(body["kind"] as? String == "fastmail")
        #expect(body["password"] as? String == "erfunden-123")
    }

    @Test func icsAccount() async throws {
        let t = ScriptedTransport([ScriptedTransport.json(#"{"id": "acc_3", "kind": "ics", "name": "Feiertage"}"#)])
        _ = try await api(t).addAccount(.ics(url: URL(string: "https://feeds.example.org/feiertage.ics")!, name: "Feiertage"))
        let body = jsonObject(t.recorded[0].body ?? Data())
        #expect(body["url"] as? String == "https://feeds.example.org/feiertage.ics")
        #expect(body["password"] == nil)
    }

    @Test func invalidCredentialsSurface() async {
        let t = ScriptedTransport([ScriptedTransport.error(422, code: "invalid_credentials")])
        await #expect(throws: APIError.server(status: 422, code: "invalid_credentials", message: "Testfehler")) {
            _ = try await api(t).addAccount(.icloud(username: "x@example.org", appPassword: "falsch"))
        }
    }

    @Test func calendarSettingsPatch() async throws {
        let cal = ##"{"id": "cal_2", "account_id": "acc_1", "name": "Arbeit", "color": null, "provider_color": "#123456", "writable": true, "can_invite": false, "hidden": false, "mcp_write": false, "openings_busy": true, "all_day_blocks": false, "position": 1}"##
        let t = ScriptedTransport([ScriptedTransport.json(cal)])
        let c = try await api(t).updateCalendar("cal_2", CalendarSettings(color: .clear, mcpWrite: false))
        #expect(c.displayColor == "#123456")
        #expect(t.recorded[0].method == "PATCH")
        let text = String(decoding: t.recorded[0].body ?? Data(), as: UTF8.self)
        #expect(text == #"{"color":null,"mcp_write":false}"#)
    }

    @Test func workFlagIsSent() throws {
        let text = String(decoding: try APICoding.encoder.encode(CalendarSettings(isWork: true)), as: UTF8.self)
        #expect(text == #"{"is_work":true}"#)
    }

    @Test func renameDevice() async throws {
        let t = ScriptedTransport([ScriptedTransport.json(#"{"id": "d1", "name": "Rubens iPhone", "created_at": "2026-10-20T07:00:00Z", "last_used_at": null, "current": true}"#)])
        let d = try await api(t).renameDevice("d1", name: "Rubens iPhone")
        #expect(d.name == "Rubens iPhone")
        #expect(d.current == true)
        #expect(t.recorded[0].method == "PATCH")
        #expect(t.recorded[0].url.path == "/api/devices/d1")
        #expect(jsonObject(t.recorded[0].body ?? Data())["name"] as? String == "Rubens iPhone")
    }

    @Test func listsBareOrWrapped() async throws {
        let t = ScriptedTransport([
            ScriptedTransport.json(#"[{"id": "tok_1", "name": "Claude.ai", "created_at": "2026-10-20T07:00:00Z"}]"#),
            ScriptedTransport.json(#"{"tokens": [{"id": "tok_2", "name": "Routine"}]}"#),
        ])
        let m = api(t)
        #expect(try await m.tokens().map(\.id) == ["tok_1"])
        #expect(try await m.tokens().map(\.id) == ["tok_2"])
    }

    @Test func createTokenReturnsSecretOnce() async throws {
        let t = ScriptedTransport([ScriptedTransport.json(#"{"id": "tok_5", "name": "Planung", "token": "lt_erfunden"}"#, status: 201)])
        let tok = try await api(t).createToken(name: "Planung")
        #expect(tok.token == "lt_erfunden")
    }

    @Test func deleteAndSyncCalls() async throws {
        let ok: ScriptedTransport.Handler = { _ in HTTPResponse(status: 204) }
        let t = ScriptedTransport([ok, ok, ok, ok])
        let m = api(t)
        try await m.removeAccount("acc_1")
        try await m.syncAccountNow("acc_1")
        try await m.revokeToken("tok_1")
        try await m.dismissConflict("cf_1")
        #expect(t.recorded.map { "\($0.method) \($0.url.path)" } == [
            "DELETE /api/accounts/acc_1", "POST /api/accounts/acc_1/sync", "DELETE /api/tokens/tok_1", "POST /api/conflicts/cf_1/dismiss",
        ])
    }

    @Test func calendarSetBodies() async throws {
        let set = #"{"id": "set_1", "name": "Arbeit", "calendar_ids": ["cal_2"], "position": 0}"#
        let t = ScriptedTransport([ScriptedTransport.json(set), ScriptedTransport.json(set)])
        let m = api(t)
        _ = try await m.createCalendarSet(name: "Arbeit", calendarIds: ["cal_2"])
        _ = try await m.updateCalendarSet("set_1", calendarIds: ["cal_2", "cal_5"])
        #expect(jsonObject(t.recorded[0].body ?? Data())["calendar_ids"] as? [String] == ["cal_2"])
        let patch = jsonObject(t.recorded[1].body ?? Data())
        #expect(patch["calendar_ids"] as? [String] == ["cal_2", "cal_5"])
        #expect(patch["name"] == nil)
    }
}
