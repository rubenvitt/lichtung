import Foundation
@testable import LichtungKit
import Testing

/// Erfundene Buchungslinks wie in docs/api/openings.md.
enum OpeningFixtures {
    static let tz = TimeZone(identifier: "Europe/Berlin")!
    static let cal = Calendar.lichtung(timeZone: tz)
    static let now = APICoding.parseTimestamp("2026-10-26T08:00:00Z")!
    static let today = LocalDate(now, calendar: cal)

    static let calendars: [String: CalendarInfo] = [
        "cal_privat": CalendarInfo(id: "cal_privat", accountId: "acc_1", name: "Fastmail Privat", writable: true, canInvite: true, position: 1),
        "cal_familie": CalendarInfo(id: "cal_familie", accountId: "acc_1", name: "Familie", writable: true, canInvite: true, hidden: true, position: 2),
        "cal_icloud": CalendarInfo(id: "cal_icloud", accountId: "acc_2", name: "iCloud", writable: true, canInvite: false, position: 0),
        "cal_abo": CalendarInfo(id: "cal_abo", accountId: "acc_3", name: "Feiertage", writable: false, canInvite: false, position: 3),
    ]

    static let templateJSON = """
    {"id": "opn_01", "kind": "template", "url": "https://lichtung.example.org/o/Zb3erfunden",
     "title": "30 Minuten Kennenlernen", "duration_minutes": 30, "calendar_id": "cal_privat",
     "time_zone": "Europe/Berlin",
     "windows": [{"weekday": "mo", "start": "09:00", "end": "17:00"}, {"weekday": "tu", "start": "09:00", "end": "12:00"}],
     "buffer_before_minutes": 10, "buffer_after_minutes": 10, "min_notice_minutes": 240, "horizon_days": 21,
     "video_url": "https://meet.example.org/kennenlernen", "suggestions_only": false, "suggestions": [],
     "booked_at": null, "booked_event_id": null,
     "created_at": "2026-10-26T08:00:00.123Z", "updated_at": "2026-10-26T08:00:00.123Z"}
    """

    static let onceJSON = """
    {"id": "opn_02", "kind": "once", "url": "https://lichtung.example.org/o/Yx9erfunden",
     "title": "Abstimmung", "duration_minutes": 30, "calendar_id": "cal_privat", "time_zone": "Europe/Berlin",
     "windows": [{"weekday": "mo", "start": "09:00", "end": "17:00"}], "buffer_before_minutes": 0, "buffer_after_minutes": 0, "min_notice_minutes": 240, "horizon_days": 21,
     "video_url": null, "suggestions_only": true,
     "suggestions": [{"id": "sug_1", "start": "2026-11-03T09:00:00Z", "end": "2026-11-03T09:30:00Z",
                      "placeholder_event_id": "ev_7", "placeholder_status": "reserved"},
                     {"id": "sug_2", "start": "2026-11-04T13:00:00Z", "end": "2026-11-04T13:30:00Z",
                      "placeholder_event_id": "ev_8", "placeholder_status": "missing"}],
     "booked_at": null, "booked_event_id": null,
     "created_at": "2026-10-26T09:00:00Z", "updated_at": "2026-10-26T09:00:00Z"}
    """

    static func decode(_ json: String) throws -> Opening {
        try APICoding.decoder.decode(Opening.self, from: Data(json.utf8))
    }

    static func once(_ id: String, created: String, booked: String? = nil, suggestionsOnly: Bool = false,
                     suggestions: [Opening.Suggestion] = []) -> Opening {
        Opening(id: id, kind: .once, url: "https://lichtung.example.org/o/\(id)", title: "Termin (30 Minuten)",
                durationMinutes: 30, calendarId: "cal_privat", suggestionsOnly: suggestionsOnly, suggestions: suggestions,
                bookedAt: booked.flatMap(APICoding.parseTimestamp), createdAt: APICoding.parseTimestamp(created))
    }

    static func template(_ id: String, title: String) -> Opening {
        Opening(id: id, kind: .template, url: "https://lichtung.example.org/o/\(id)", title: title,
                durationMinutes: 45, calendarId: "cal_privat")
    }
}

@Suite("Buchungslinks")
struct OpeningTests {
    typealias F = OpeningFixtures

    // MARK: Objekt und Pull

    @Test func decodesTemplateAndOnce() throws {
        let t = try F.decode(F.templateJSON)
        #expect(t.kind == .template)
        #expect(t.windows.count == 2)
        #expect(t.videoUrl == "https://meet.example.org/kennenlernen")
        #expect(t.createdAt != nil)
        let o = try F.decode(F.onceJSON)
        #expect(o.kind == .once)
        #expect(o.suggestionsOnly)
        #expect(o.suggestions.map(\.placeholderStatus) == [.reserved, .missing])
        #expect(o.suggestions[0].start == APICoding.parseTimestamp("2026-11-03T09:00:00Z"))
        #expect(!o.isBooked && !o.isExhausted)
    }

    @Test func toleratesMissingFieldsAndUnknownValues() throws {
        let o = try F.decode("""
        {"id": "opn_9", "kind": "sammeltermin", "url": "https://lichtung.example.org/o/x", "duration_minutes": 15, "calendar_id": "cal_privat",
         "suggestions": [{"id": "s", "start": "2026-11-03T09:00:00Z", "end": "2026-11-03T09:15:00Z", "placeholder_status": "neu"}]}
        """)
        #expect(o.kind == .unknown)
        #expect(o.minNoticeMinutes == 240 && o.horizonDays == 21)
        #expect(o.suggestionsOnly, "with suggestions")
        #expect(o.suggestions[0].placeholderStatus == .unknown)
    }

    @Test func pullUpsertsAndDeletesOpenings() throws {
        let json = ContractSamples.pull("""
        {"type": "opening", "op": "upsert", "data": \(F.templateJSON)},
        {"type": "opening", "op": "upsert", "data": \(F.onceJSON)},
        {"type": "opening", "op": "delete", "id": "opn_01"}
        """)
        let page = try APICoding.decoder.decode(PullPage.self, from: Data(json.utf8))
        var s = SyncState()
        s.apply(page)
        #expect(Array(s.openings.keys) == ["opn_02"])
        #expect(s.openings["opn_02"]?.title == "Abstimmung")
        s.resetMirror()
        #expect(s.openings.isEmpty)
    }

    @Test func cacheKeepsOpeningsAndReadsOlderFiles() throws {
        var s = SyncState()
        s.openings["opn_02"] = try F.decode(F.onceJSON)
        s.openings["opn_01"] = try F.decode(F.templateJSON)
        let data = try JSONEncoder().encode(s)
        #expect(try JSONDecoder().decode(SyncState.self, from: data) == s)

        // Cache-Datei von vorher: ohne `openings`, die Outbox bleibt.
        var old = SyncState()
        old.cursor = 17
        old.enqueue(Mutation(id: "m1", .deleteEvent(eventId: "ev_1", baseVersion: 1, scope: .all, occurrence: nil)), now: F.now)
        var object = try JSONSerialization.jsonObject(with: try JSONEncoder().encode(old)) as! [String: Any]
        object["openings"] = nil
        let legacy = try JSONSerialization.data(withJSONObject: object)
        let decoded = try JSONDecoder().decode(SyncState.self, from: legacy)
        #expect(decoded.cursor == 17)
        #expect(decoded.outbox.map(\.id) == ["m1"])
        #expect(decoded.openings.isEmpty)
    }

    @Test func olderCacheFormatIsPulledAgainKeepingTheOutbox() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("lichtung-test-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("cache.json")
        var old = SyncState()
        old.cursor = 99
        old.enqueue(Mutation(id: "m1", .deleteEvent(eventId: "ev_1", baseVersion: 1, scope: .all, occurrence: nil)), now: F.now)
        var state = try JSONSerialization.jsonObject(with: try JSONEncoder().encode(old)) as! [String: Any]
        state["openings"] = nil
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try JSONSerialization.data(withJSONObject: ["formatVersion": 1, "state": state]).write(to: url)
        let loaded = try #require(try FileCacheStore(url: url).load())
        #expect(loaded.cursor == 0, "pull everything again, incl. links skipped before")
        #expect(loaded.outbox.map(\.id) == ["m1"])
    }

    // MARK: API

    func api(_ t: ScriptedTransport) -> ManagementAPI {
        ManagementAPI(sync: SyncAPI(baseURL: EngineFixtures.base, transport: t, tokens: StaticToken("ld_test")))
    }

    @Test func createOnceWithSuggestions() async throws {
        let t = ScriptedTransport([ScriptedTransport.json(F.onceJSON, status: 201)])
        var draft = OneOffLinkDraft(durationMinutes: 30, calendarId: "cal_privat")
        draft.suggestions = [APICoding.parseTimestamp("2026-11-04T13:00:00Z")!, APICoding.parseTimestamp("2026-11-03T09:00:00Z")!]
        let body = try #require(draft.newOpening(now: F.now, calendars: F.calendars, timeZone: F.tz))
        let created = try await api(t).createOpening(body)
        #expect(created.url.hasPrefix("https://lichtung.example.org/o/"))
        let req = t.recorded[0]
        #expect(req.method == "POST")
        #expect(req.url.path == "/api/openings")
        let json = jsonObject(req.body ?? Data())
        #expect(json["kind"] as? String == "once")
        #expect(json["calendar_id"] as? String == "cal_privat")
        #expect(json["duration_minutes"] as? Int == 30)
        #expect(json["time_zone"] as? String == "Europe/Berlin")
        #expect(json["title"] == nil, "server default")
        #expect(json["windows"] == nil)
        let suggestions = json["suggestions"] as? [[String: Any]] ?? []
        #expect(suggestions.map { $0["start"] as? String } == ["2026-11-03T09:00:00Z", "2026-11-04T13:00:00Z"], "sorted")
        #expect(suggestions.allSatisfy { $0["end"] == nil })
    }

    @Test func patchListAndDelete() async throws {
        let ok: ScriptedTransport.Handler = { _ in HTTPResponse(status: 204) }
        let t = ScriptedTransport([
            ScriptedTransport.json(F.templateJSON),
            ScriptedTransport.json("[\(F.templateJSON), \(F.onceJSON)]"),
            ok,
        ])
        let m = api(t)
        _ = try await m.updateOpening("opn_01", OpeningPatch(title: "Kennenlernen", videoUrl: .clear))
        #expect(try await m.openings().map(\.id) == ["opn_01", "opn_02"])
        try await m.deleteOpening("opn_02")
        #expect(t.recorded.map { "\($0.method) \($0.url.path)" } == [
            "PATCH /api/openings/opn_01", "GET /api/openings", "DELETE /api/openings/opn_02",
        ])
        let text = String(decoding: t.recorded[0].body ?? Data(), as: UTF8.self)
        #expect(text == #"{"title":"Kennenlernen","video_url":null}"#)
    }

    @Test func refusedTargetSurfaces() async {
        let t = ScriptedTransport([ScriptedTransport.error(400, code: "calendar_cannot_invite")])
        await #expect(throws: APIError.server(status: 400, code: "calendar_cannot_invite", message: "Testfehler")) {
            _ = try await api(t).createOpening(NewOpening(kind: .once, durationMinutes: 30, calendarId: "cal_icloud"))
        }
    }

    // MARK: Zielkalender

    @Test func targetsAreWritableCalendarsWithInvitations() {
        #expect(OpeningTargets.allowed(F.calendars).map(\.id) == ["cal_privat", "cal_familie"])
        #expect(OpeningTargets.preferred(F.calendars, last: "cal_familie") == "cal_familie")
        #expect(OpeningTargets.preferred(F.calendars, last: "cal_icloud") == "cal_privat")
        #expect(OpeningTargets.preferred([:], last: nil) == nil)
    }

    // MARK: Texte

    @Test func summaries() throws {
        let o = try F.decode(F.onceJSON)
        #expect(o.summary(calendars: F.calendars) == "30 Min. · Fastmail Privat · 2 Vorschläge")
        let t = try F.decode(F.templateJSON)
        #expect(t.summary(calendars: F.calendars) == "30 Min. · Fastmail Privat · Mo 09:00–17:00 · Di 09:00–12:00")
        #expect(t.rulesText == "4 Std. Vorlauf · 21 Tage voraus · 10 Min. Puffer")
        let free = F.once("opn_3", created: "2026-10-26T07:00:00Z")
        #expect(free.summary(calendars: [:]) == "30 Min. · Unbekannter Kalender · freie Zeiten")
    }

    @Test func durationsAndRules() {
        #expect(OpeningText.duration(30) == "30 Min.")
        #expect(OpeningText.duration(60) == "1 Std.")
        #expect(OpeningText.duration(90) == "1 Std. 30 Min.")
        #expect(OpeningText.suggestionCount(1) == "1 Vorschlag")
        #expect(OpeningText.notice(0) == "ohne Vorlauf")
        #expect(OpeningText.notice(2880) == "2 Tage Vorlauf")
        #expect(OpeningText.notice(90) == "1 Std. 30 Min. Vorlauf")
        #expect(OpeningText.horizon(1) == "1 Tag voraus")
        #expect(OpeningText.buffers(before: 0, after: 0) == nil)
        #expect(OpeningText.buffers(before: 15, after: 0) == "15 Min. Puffer vorher")
        #expect(OpeningText.buffers(before: 10, after: 5) == "Puffer 10 Min. vorher, 5 Min. nachher")
        #expect(OpeningText.choices([15, 30], including: 25) == [15, 25, 30])
        #expect(OpeningText.minutes(clock: "09:30") == 570)
        #expect(OpeningText.minutes(clock: "24:00") == nil)
        #expect(OpeningText.clock(570) == "09:30")
    }

    @Test func windowTexts() {
        let w = { (d: String, s: String, e: String) in Opening.Window(weekday: d, start: s, end: e) }
        #expect(OpeningText.windows(Opening.defaultWindows) == "Mo–Fr 09:00–17:00")
        #expect(OpeningText.windows([w("we", "09:00", "12:00"), w("mo", "09:00", "12:00"), w("tu", "14:00", "16:00")])
            == "Mo, Mi 09:00–12:00 · Di 14:00–16:00")
        #expect(OpeningText.windows([w("sa", "10:00", "12:00"), w("su", "10:00", "12:00")]) == "Sa, So 10:00–12:00")
        #expect(OpeningText.windows([w("mo", "08:00", "10:00"), w("mo", "14:00", "16:00")]) == "Mo 08:00–10:00, 14:00–16:00")
        #expect(OpeningText.windows([]) == "keine Zeitfenster")
    }

    @Test func slotsInTheDeviceZone() throws {
        let o = try F.decode(F.onceJSON)
        let s = o.suggestions[0]
        // 09:00 UTC = 10:00 in Berlin (Winterzeit ab 25. Oktober).
        #expect(OpeningText.slot(start: s.start, end: s.end, today: F.today, calendar: F.cal) == "Di 3. Nov. · 10:00–10:30")
    }

    @Test func statusTexts() throws {
        let cal = F.cal
        var o = try F.decode(F.onceJSON)
        #expect(o.statusText(calendars: F.calendars, today: F.today, calendar: cal)?.contains("Platzhalter") == true)
        o.suggestions[1].placeholderStatus = .reserved
        #expect(o.statusText(calendars: F.calendars, today: F.today, calendar: cal) == nil)
        o.calendarId = "cal_icloud"
        #expect(o.statusText(calendars: F.calendars, today: F.today, calendar: cal)?.contains("Zielkalender") == true)
        o.suggestions = []
        #expect(o.isExhausted)
        #expect(o.statusText(calendars: F.calendars, today: F.today, calendar: cal) == "Alle Vorschläge sind vorbei, der Link bietet nichts mehr an.")
        o.bookedAt = F.now
        #expect(o.statusText(calendars: F.calendars, today: F.today, calendar: cal) == "Gebucht · Heute · Mo 26.")
    }

    // MARK: Liste

    @Test func listSplitsOpenTemplatesAndBooked() {
        let list = OpeningList([
            F.once("opn_a", created: "2026-10-20T08:00:00Z"),
            F.once("opn_b", created: "2026-10-25T08:00:00Z"),
            F.once("opn_c", created: "2026-10-01T08:00:00Z", booked: "2026-10-24T10:00:00Z"),
            F.once("opn_d", created: "2026-08-01T08:00:00Z", booked: "2026-08-02T10:00:00Z"),
            F.once("opn_e", created: "2026-10-02T08:00:00Z", booked: "2026-10-25T10:00:00Z"),
            F.template("opn_t2", title: "Sprechstunde"),
            F.template("opn_t1", title: "30 Minuten Kennenlernen"),
        ], now: F.now)
        #expect(list.open.map(\.id) == ["opn_b", "opn_a"], "newest first")
        #expect(list.templates.map(\.id) == ["opn_t1", "opn_t2"])
        #expect(list.booked.map(\.id) == ["opn_e", "opn_c"], "recent only, last booked first")
        #expect(!list.isEmpty)
        #expect(OpeningList([], now: F.now).isEmpty)
    }

    // MARK: Einmal-Link

    @Test func oneOffDraftChecks() {
        let later = F.now.addingTimeInterval(86_400)
        var d = OneOffLinkDraft(durationMinutes: 30, calendarId: nil)
        #expect(d.problem(now: F.now, calendars: F.calendars) == "Bitte einen Zielkalender wählen.")
        d.calendarId = "cal_icloud"
        #expect(d.problem(now: F.now, calendars: F.calendars)?.contains("Einladungen") == true)
        d.calendarId = "cal_privat"
        #expect(d.problem(now: F.now, calendars: F.calendars) == nil)
        #expect(d.summary(calendars: F.calendars) == "30 Min. · Fastmail Privat · freie Zeiten")
        d.durationMinutes = 4
        #expect(d.problem(now: F.now, calendars: F.calendars) != nil)
        d.durationMinutes = 30
        d.suggestions = [F.now.addingTimeInterval(-60)]
        #expect(d.problem(now: F.now, calendars: F.calendars) == "Ein Vorschlag liegt in der Vergangenheit.")
        d.suggestions = [later, later]
        #expect(d.problem(now: F.now, calendars: F.calendars) == "Ein Vorschlag steht doppelt in der Liste.")
        d.suggestions = [F.now.addingTimeInterval(366 * 86_400)]
        #expect(d.problem(now: F.now, calendars: F.calendars) != nil)
        d.suggestions = (1...21).map { F.now.addingTimeInterval(Double($0) * 3_600) }
        #expect(d.problem(now: F.now, calendars: F.calendars)?.contains("20") == true)
        d.suggestions = [later]
        d.title = "  Abstimmung "
        let body = d.newOpening(now: F.now, calendars: F.calendars, timeZone: F.tz)
        #expect(body?.title == "Abstimmung")
        #expect(body?.suggestions?.count == 1)
        #expect(d.summary(calendars: F.calendars) == "30 Min. · Fastmail Privat · 1 Vorschlag")
    }

    @Test func suggestionsAreProposedDayByDay() {
        var d = OneOffLinkDraft(calendarId: "cal_privat")
        d.addSuggestion(now: F.now, calendar: F.cal)
        // Morgen 10:00 in Berlin = 09:00 UTC.
        #expect(d.suggestions == [APICoding.parseTimestamp("2026-10-27T09:00:00Z")!])
        d.addSuggestion(now: F.now, calendar: F.cal)
        #expect(d.suggestions.last == APICoding.parseTimestamp("2026-10-28T09:00:00Z")!)
        #expect(d.suggestionsHint.contains("Reserviert"))
        #expect(OneOffLinkDraft().suggestionsHint.contains("Mo–Fr"))
    }

    // MARK: Vorlage

    @Test func newTemplateWithDefaults() throws {
        var d = OpeningTemplateDraft(calendarId: "cal_privat", timeZone: F.tz)
        #expect(d.problem(calendars: F.calendars)?.contains("Titel") == true)
        d.title = "Sprechstunde"
        d.videoURL = "meet.example.org"
        #expect(d.problem(calendars: F.calendars)?.contains("Videolink") == true)
        d.videoURL = " https://meet.example.org/sprechstunde "
        d.windows[0].end = d.windows[0].start
        #expect(d.problem(calendars: F.calendars)?.contains("Mo") == true)
        d.windows[0].end = 12 * 60
        d.addWindow()
        #expect(d.windows.last?.weekday == .saturday)
        let body = try #require(d.newOpening(calendars: F.calendars))
        let json = jsonObject(try APICoding.encoder.encode(body))
        #expect(json["kind"] as? String == "template")
        #expect(json["video_url"] as? String == "https://meet.example.org/sprechstunde")
        #expect(json["min_notice_minutes"] as? Int == 240)
        let windows = json["windows"] as? [[String: String]] ?? []
        #expect(windows.count == 6)
        #expect(windows.first == ["weekday": "mo", "start": "09:00", "end": "12:00"])
        #expect(windows.last?["weekday"] == "sa")
        #expect(d.patch(calendars: F.calendars) == nil, "new: no patch")
    }

    @Test func editingSendsOnlyChanges() throws {
        let t = try F.decode(F.templateJSON)
        var d = OpeningTemplateDraft(editing: t)
        #expect(d.patch(calendars: F.calendars)?.isEmpty == true)
        #expect(d.newOpening(calendars: F.calendars) == nil)
        // Reihenfolge der Fenster ändert nichts.
        d.windows.reverse()
        #expect(d.patch(calendars: F.calendars)?.isEmpty == true)
        d.horizonDays = 28
        d.videoURL = ""
        d.calendarId = "cal_familie"
        let p = try #require(d.patch(calendars: F.calendars))
        #expect(p == OpeningPatch(calendarId: "cal_familie", horizonDays: 28, videoUrl: .clear))
        d.calendarId = "cal_abo"
        #expect(d.patch(calendars: F.calendars) == nil)
    }

    @Test func onceWithSuggestionsKeepsCalendarAndDuration() throws {
        let o = try F.decode(F.onceJSON)
        #expect(!o.canChangeCalendarAndDuration)
        var d = OpeningTemplateDraft(editing: o)
        d.title = "Abstimmung Projekt"
        d.durationMinutes = 45
        let p = try #require(d.patch(calendars: F.calendars))
        #expect(p.title == "Abstimmung Projekt")
        #expect(p.durationMinutes == nil)
    }
}
