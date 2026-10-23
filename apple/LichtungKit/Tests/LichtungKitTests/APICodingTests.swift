import Foundation
import LichtungKit
import Testing

/// Beispiele aus docs/api/sync-api.md (erfundene Daten).
enum ContractSamples {
    static let event = """
    {
      "id": "ev_01",
      "calendar_id": "cal_01",
      "uid": "7d1f0c9e-2b1a-4c55-9d0e-000000000001@example.org",
      "version": 4,
      "title": "Team-Sync",
      "location": "Raum 2",
      "notes": "Agenda im Dokument",
      "url": null,
      "start": {"date_time": "2026-11-02T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-02T08:00:00Z"},
      "end": {"date_time": "2026-11-02T09:30:00", "time_zone": "Europe/Berlin", "utc": "2026-11-02T08:30:00Z"},
      "all_day": false,
      "status": "confirmed",
      "availability": "busy",
      "recurrence": {
        "rrule": "FREQ=WEEKLY;BYDAY=MO;UNTIL=20270201T075959Z",
        "rdates": [],
        "exdates": [{"date_time": "2026-11-16T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-16T08:00:00Z"}]
      },
      "overrides": [
        {
          "recurrence_id": {"date_time": "2026-11-09T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T08:00:00Z"},
          "title": "Team-Sync (verschoben)",
          "location": "Raum 2",
          "notes": "Agenda im Dokument",
          "url": null,
          "start": {"date_time": "2026-11-09T10:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T09:00:00Z"},
          "end": {"date_time": "2026-11-09T10:30:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T09:30:00Z"},
          "all_day": false,
          "status": "confirmed",
          "availability": "busy",
          "organizer": null,
          "attendees": [],
          "self_status": null,
          "alarms": [{"minutes_before": 10}],
          "travel_time_minutes": null
        }
      ],
      "organizer": null,
      "attendees": [{"email": "gast@example.org", "name": "Gast", "role": "required", "status": "needs_action", "is_self": false}],
      "self_status": null,
      "alarms": [{"minutes_before": 10}],
      "travel_time_minutes": null,
      "created_at": "2026-10-20T07:12:00Z",
      "updated_at": "2026-10-28T16:40:00.123Z",
      "some_future_field": {"nested": true}
    }
    """

    static func pull(_ changes: String, cursor: Int = 10, hasMore: Bool = false) -> String {
        "{\"cursor\": \(cursor), \"has_more\": \(hasMore), \"changes\": [\(changes)]}"
    }
}

func jsonObject(_ data: Data) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
}

@Suite("API-Codierung")
struct APICodingTests {
    @Test func decodesContractEvent() throws {
        let e = try APICoding.decoder.decode(Event.self, from: Data(ContractSamples.event.utf8))
        #expect(e.id == "ev_01")
        #expect(e.calendarId == "cal_01")
        #expect(e.version == 4)
        #expect(e.title == "Team-Sync")
        #expect(e.details.location == "Raum 2")
        #expect(e.details.allDay == false)
        let start = LocalDateTime(date: LocalDate(year: 2026, month: 11, day: 2), hour: 9, minute: 0)
        #expect(e.details.start.normalized == .dateTime(start, timeZone: "Europe/Berlin"))
        #expect(e.recurrence?.rrule == "FREQ=WEEKLY;BYDAY=MO;UNTIL=20270201T075959Z")
        #expect(e.recurrence?.exdates.count == 1)
        #expect(e.overrides.first?.details.title == "Team-Sync (verschoben)")
        #expect(e.overrides.first?.details.alarms == [.minutesBefore(10)])
        #expect(e.details.attendees.first?.status == .needsAction)
        #expect(e.details.attendees.first?.isSelf == false)
        #expect(e.updatedAt != nil)
    }

    @Test func decodesAllDayAndFloating() throws {
        let json = """
        {"id": "ev_2", "calendar_id": "cal_1", "version": 1, "title": "Urlaub",
         "start": {"date": "2026-11-03"}, "end": {"date": "2026-11-08"}, "all_day": true,
         "status": "confirmed", "availability": "free", "recurrence": null, "overrides": []}
        """
        let e = try APICoding.decoder.decode(Event.self, from: Data(json.utf8))
        #expect(e.details.start == .date(LocalDate(year: 2026, month: 11, day: 3)))
        #expect(e.details.availability == .free)

        let floating = try APICoding.decoder.decode(DateValue.self, from: Data(#"{"date_time": "2026-11-03T08:00:00", "time_zone": null, "utc": null}"#.utf8))
        #expect(floating == .dateTime(LocalDateTime(date: LocalDate(year: 2026, month: 11, day: 3), hour: 8, minute: 0), timeZone: nil))
    }

    @Test func decodesPullPage() throws {
        let json = ContractSamples.pull("""
        {"type": "account", "op": "upsert", "data": {"id": "acc_01", "kind": "fastmail", "name": "Fastmail", "username": "person@example.org",
          "status": {"state": "auth_failed", "message": "Passwort erneuern", "last_sync_at": "2026-11-03T06:00:12Z"}}},
        {"type": "calendar", "op": "upsert", "data": {"id": "cal_01", "account_id": "acc_01", "name": "Privat", "color": "#3B82F6",
          "provider_color": "#3B82F6", "writable": true, "can_invite": true, "hidden": false, "mcp_write": true,
          "openings_busy": true, "all_day_blocks": false, "position": 0}},
        {"type": "calendar_set", "op": "upsert", "data": {"id": "set_01", "name": "Arbeit", "calendar_ids": ["cal_02"], "position": 1}},
        {"type": "event", "op": "delete", "id": "ev_07"},
        {"type": "opening", "op": "upsert", "data": {"id": "op_1"}},
        {"type": "account", "op": "upsert", "data": {"id": "acc_02", "kind": "microsoft", "name": "Arbeit"}}
        """, cursor: 1834, hasMore: true)
        let page = try APICoding.decoder.decode(PullPage.self, from: Data(json.utf8))
        #expect(page.cursor == 1834)
        #expect(page.hasMore)
        #expect(page.changes.count == 6)
        guard case let .upsertAccount(a) = page.changes[0] else { Issue.record("kein Konto"); return }
        #expect(a.status?.state == .authFailed)
        guard case let .upsertCalendar(c) = page.changes[1] else { Issue.record("kein Kalender"); return }
        #expect(c.mcpWrite && c.canInvite && c.accountId == "acc_01")
        #expect(page.changes[3] == .delete(type: "event", id: "ev_07"))
        #expect(page.changes[4] == .unknown(type: "opening"))
    }

    @Test func encodesCreateLikeContract() throws {
        let start = LocalDateTime(date: LocalDate(year: 2026, month: 11, day: 3), hour: 14, minute: 0)
        let end = LocalDateTime(date: LocalDate(year: 2026, month: 11, day: 3), hour: 15, minute: 0)
        let m = Mutation(id: "m-1", .createEvent(calendarId: "cal_01", event: EventInput(
            title: "Zahnarzt", start: .dateTime(start, timeZone: "Europe/Berlin"),
            end: .dateTime(end, timeZone: "Europe/Berlin"), alarms: [.minutesBefore(60)]
        )))
        let o = jsonObject(try APICoding.encoder.encode(m))
        #expect(o["op"] as? String == "create_event")
        #expect(o["calendar_id"] as? String == "cal_01")
        let event = o["event"] as? [String: Any] ?? [:]
        let s = event["start"] as? [String: Any] ?? [:]
        #expect(s["date_time"] as? String == "2026-11-03T14:00:00")
        #expect(s["time_zone"] as? String == "Europe/Berlin")
        #expect((event["alarms"] as? [[String: Any]])?.first?["minutes_before"] as? Int == 60)
    }

    @Test func encodesUpdateWithClearedField() throws {
        let m = Mutation(id: "m-2", .updateEvent(
            eventId: "ev_01", baseVersion: 4, scope: .thisAndFollowing,
            occurrence: .date(LocalDate(year: 2026, month: 11, day: 9)),
            changes: EventChanges(title: "Neu", location: .clear)
        ))
        let data = try APICoding.encoder.encode(m)
        let text = String(decoding: data, as: UTF8.self)
        #expect(text.contains(#""location":null"#))
        #expect(!text.contains("notes"))
        let o = jsonObject(data)
        #expect(o["scope"] as? String == "this_and_following")
        #expect(o["base_version"] as? Int == 4)
        #expect((o["occurrence"] as? [String: Any])?["date"] as? String == "2026-11-09")
        // Rundweg für die Outbox
        let back = try APICoding.decoder.decode(Mutation.self, from: data)
        #expect(back == m)
    }

    @Test func encodesFloatingWithExplicitNull() throws {
        let v = DateValue.dateTime(LocalDateTime(date: LocalDate(year: 2026, month: 1, day: 1), hour: 8, minute: 0), timeZone: nil)
        let text = String(decoding: try APICoding.encoder.encode(v), as: UTF8.self)
        #expect(text == #"{"date_time":"2026-01-01T08:00:00","time_zone":null}"#)
    }

    @Test func decodesMutationResults() throws {
        let json = """
        {"results": [
          {"id": "a", "status": "applied", "events": [], "deleted_event_ids": ["ev_3"]},
          {"id": "b", "status": "conflict", "conflict": {"id": "cf_03", "event_id": "ev_01", "message": "Anbieter gewinnt", "mutation": {"op": "update_event", "base_version": 3}}, "events": []},
          {"id": "c", "status": "rejected", "error": {"code": "calendar_read_only", "message": "Nur lesbar."}},
          {"id": "d", "status": "retry", "error": {"code": "provider_unavailable", "message": "Weg."}, "retry_after_seconds": 60}
        ]}
        """
        struct R: Decodable { var results: [MutationResult] }
        let r = try APICoding.decoder.decode(R.self, from: Data(json.utf8)).results
        #expect(r.map(\.status) == [.applied, .conflict, .rejected, .retry])
        #expect(r[0].deletedEventIds == ["ev_3"])
        #expect(r[1].conflict?.eventId == "ev_01")
        #expect(r[2].error?.code == "calendar_read_only")
        #expect(r[3].retryAfterSeconds == 60)
    }

    @Test func timestampsWithAndWithoutFraction() {
        #expect(APICoding.parseTimestamp("2026-11-03T13:00:00Z") != nil)
        #expect(APICoding.parseTimestamp("2026-11-03T13:00:00.250Z") != nil)
        #expect(APICoding.parseTimestamp("2026-11-03T14:00:00+01:00") == APICoding.parseTimestamp("2026-11-03T13:00:00Z"))
        #expect(APICoding.parseTimestamp("gestern") == nil)
    }

    @Test func wallClockInstantAcrossDST() {
        // 9:00 Berlin ist im Sommer 7:00 UTC, im Winter 8:00 UTC.
        let tz = TimeZone(identifier: "Europe/Berlin")!
        let summer = DateValue.dateTime(LocalDateTime(date: LocalDate(year: 2026, month: 10, day: 19), hour: 9, minute: 0), timeZone: "Europe/Berlin")
        let winter = DateValue.dateTime(LocalDateTime(date: LocalDate(year: 2026, month: 10, day: 26), hour: 9, minute: 0), timeZone: "Europe/Berlin")
        #expect(APICoding.formatTimestamp(summer.instant(deviceTimeZone: tz)) == "2026-10-19T07:00:00Z")
        #expect(APICoding.formatTimestamp(winter.instant(deviceTimeZone: tz)) == "2026-10-26T08:00:00Z")
    }
}
