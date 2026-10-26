import Foundation
@testable import LichtungKit
import Testing

/// LIC-30: Terminanfragen gesammelt, beantworten in Liste, Detail und
/// Kontextmenü. Jetzt ist Fr 23.10.2026, 10:00 in Berlin (`SyncFixtures.now`).
@Suite("Terminanfragen")
struct InvitationTests {
    typealias F = SyncFixtures
    let cal = Calendar.lichtung(timeZone: SyncFixtures.tz)
    static let anna = Person(email: "anna@example.org", name: "Anna Beispiel")

    /// Einladung mit eigenem Status; `day`/`month` wie `SyncFixtures.at`.
    static func invite(_ id: String, title: String = "Termin", day: Int = 3, hour: Int = 9, month: Int = 11,
                       status: ParticipationStatus = .needsAction, rrule: String? = nil, calendar: String = "cal_1") -> Event {
        Event(
            id: id, calendarId: calendar,
            details: EventDetails(
                title: title, start: F.at(day, hour, month: month), end: F.at(day, hour + 1, month: month),
                organizer: anna,
                attendees: [Attendee(email: "anna@example.org", name: "Anna Beispiel", role: .chair, status: .accepted),
                            Attendee(email: "ich@example.org", status: status, isSelf: true)],
                selfStatus: status
            ),
            recurrence: rrule.map { Recurrence(rrule: $0) }
        )
    }

    func state() -> SyncState {
        var s = SyncState()
        s.calendars["cal_1"] = CalendarInfo(id: "cal_1", accountId: "acc_1", name: "Arbeit")
        s.calendars["cal_abo"] = CalendarInfo(id: "cal_abo", accountId: "acc_2", name: "Verein (Abo)", writable: false)
        s.calendars["cal_aus"] = F.calendar("cal_aus", hidden: true)
        s.events["einzel"] = Self.invite("einzel", title: "Kickoff", day: 3)
        // Montags ab 2.11., vier Mal.
        s.events["serie"] = Self.invite("serie", title: "Jour fixe", day: 2, rrule: "FREQ=WEEKLY;COUNT=4")
        s.events["zugesagt"] = Self.invite("zugesagt", day: 4, status: .accepted)
        s.events["vorbei"] = Self.invite("vorbei", day: 20, month: 10)
        s.events["versteckt"] = Self.invite("versteckt", day: 5, calendar: "cal_aus")
        s.events["eigener"] = F.event("eigener", day: 6)
        return s
    }

    func query(_ s: SyncState) -> CalendarQuery { CalendarQuery(state: s, deviceTimeZone: F.tz) }

    @Test func listsOpenRequestsSeriesOnce() {
        let list = query(state()).invitations(now: F.now)
        #expect(list.items.map(\.id) == ["serie", "einzel"])
        let serie = list.items[0]
        #expect(serie.target == .event)
        #expect(serie.pendingCount == 4)
        #expect(serie.occurrence.start == F.at(2, 9).instant(deviceTimeZone: F.tz))
        #expect(list.items[1].pendingCount == 1)
        #expect(list.badgeCount == 2)
    }

    @Test func seriesStartsAtNextOpenOccurrence() {
        var s = state()
        // Begann Mo 19.10.: das Vorkommen vom 19.10. ist vorbei, das nächste ist am 26.10.
        s.events["serie"] = Self.invite("serie", day: 19, month: 10, rrule: "FREQ=WEEKLY;COUNT=3")
        let serie = query(s).invitations(now: F.now).items.first { $0.id == "serie" }
        #expect(serie?.pendingCount == 2)
        #expect(serie?.occurrence.start == F.at(26, 9, month: 10).instant(deviceTimeZone: F.tz))
    }

    @Test func changedOccurrenceWithOwnRequestStandsAlone() throws {
        var s = SyncState()
        s.calendars["cal_1"] = F.calendar("cal_1")
        var e = Self.invite("serie", day: 2, status: .accepted, rrule: "FREQ=WEEKLY;COUNT=4")
        var moved = e.details
        moved.start = F.at(9, 14)
        moved.end = F.at(9, 15)
        moved.selfStatus = .needsAction
        e.overrides = [EventOverride(recurrenceId: F.at(9, 9), details: moved)]
        s.events["serie"] = e

        let list = query(s).invitations(now: F.now)
        let item = try #require(list.items.first)
        #expect(list.items.count == 1)
        #expect(item.id == "serie/2026-11-09T09:00:00/Europe/Berlin")
        guard case let .occurrence(rid) = item.target else {
            Issue.record("Vorkommen erwartet")
            return
        }
        #expect(rid.normalized == F.at(9, 9))
        #expect(item.seriesText == "Einzelner Termin einer Serie")
        guard case let .respond(id, occ, status) = item.intent(.declined) else {
            Issue.record("respond erwartet")
            return
        }
        #expect(id == "serie" && occ?.normalized == F.at(9, 9) && status == .declined)
    }

    @Test func readOnlyCalendarIsInfoOnly() throws {
        var s = state()
        s.events["abo"] = Self.invite("abo", title: "Vereinsfest", day: 7, calendar: "cal_abo")
        let list = query(s).invitations(now: F.now)
        let abo = try #require(list.items.first { $0.id == "abo" })
        #expect(!abo.canRespond)
        #expect(list.items.count == 3)
        #expect(list.badgeCount == 2)
    }

    @Test func answeredRequestsLeaveTheList() {
        var s = state()
        s.enqueue(Mutation(.respond(eventId: "einzel", occurrence: nil, status: .tentative, comment: nil)), now: F.now)
        // Ein Vorkommen der Serie beantwortet: Die Serie bleibt mit drei offenen.
        s.enqueue(Mutation(.respond(eventId: "serie", occurrence: F.at(2, 9), status: .declined, comment: nil)), now: F.now)
        let list = query(s).invitations(now: F.now)
        #expect(list.items.map(\.id) == ["serie"])
        #expect(list.items[0].pendingCount == 3)
        #expect(list.items[0].occurrence.start == F.at(9, 9).instant(deviceTimeZone: F.tz))
    }

    @Test func intentsMirrorServerSemantics() {
        let list = query(state()).invitations(now: F.now)
        // Serie in der Liste: alle Vorkommen (`occurrence: null`).
        #expect(list.items[0].intent(.accepted) == .respond(eventId: "serie", occurrence: nil, status: .accepted))
        #expect(list.items[1].intent(.declined) == .respond(eventId: "einzel", occurrence: nil, status: .declined))
    }

    @Test func groupedByDay() {
        var s = state()
        s.events["heute"] = Self.invite("heute", title: "Rückruf", day: 23, hour: 14, month: 10)
        s.events["morgen"] = Self.invite("morgen", title: "Brunch", day: 24, hour: 11, month: 10)
        let days = query(s).invitations(now: F.now).days(today: LocalDate(year: 2026, month: 10, day: 23), calendar: cal)
        #expect(days.map(\.heading) == ["Heute · Fr 23.", "Morgen · Sa 24.", "Mo 2. Nov.", "Di 3. Nov."])
        #expect(days.map { $0.items.map(\.id) } == [["heute"], ["morgen"], ["serie"], ["einzel"]])
    }

    @Test func rowTexts() {
        let list = query(state()).invitations(now: F.now)
        let today = LocalDate(year: 2026, month: 10, day: 23)
        #expect(list.items[0].dateText(today: today, calendar: cal) == "Mo 2. Nov. · 09:00–10:00")
        #expect(list.items[0].sourceText == "Von Anna Beispiel · Arbeit")
        #expect(list.items[0].seriesText == "Serie · Antwort gilt für alle 4 Termine")
        #expect(list.items[1].seriesText == nil)
        #expect(InvitationResponse.ordered.map(\.actionTitle) == ["Annehmen", "Vielleicht", "Ablehnen"])
        #expect(ParticipationStatus.needsAction.responseText == "Noch nicht beantwortet")
        #expect(InvitationResponse(.tentative) == .tentative)
        #expect(InvitationResponse(.needsAction) == nil)
    }

    @Test func organizerFallsBackToAddress() {
        var e = Self.invite("x")
        e.details.organizer = Person(email: "bert@example.org", name: " ")
        let o = SeriesExpander(deviceTimeZone: F.tz).occurrences(of: e, in: DateInterval(start: F.now, duration: 86_400 * 30))[0]
        let item = Invitation(occurrence: o, target: .event, canRespond: true)
        #expect(item.organizerText == "Von bert@example.org")
        #expect(item.sourceText == "Von bert@example.org")
    }
}

@Suite("Terminanfragen: Antworten am Termin")
struct InvitationActionTests {
    typealias F = SyncFixtures
    typealias I = InvitationTests

    func occurrences(_ e: Event) -> [Occurrence] {
        SeriesExpander(deviceTimeZone: F.tz).occurrences(of: e, in: DateInterval(start: F.now, duration: 86_400 * 60))
    }

    @Test func whoCanRespond() {
        let calendars = [
            "cal_1": F.calendar("cal_1"),
            "cal_abo": CalendarInfo(id: "cal_abo", accountId: "acc_2", name: "Abo", writable: false),
        ]
        let invited = occurrences(I.invite("a"))[0]
        let own = occurrences(F.event("b"))[0]
        let abo = occurrences(I.invite("c", calendar: "cal_abo"))[0]
        #expect(InvitationActions.canRespond(invited, calendars: calendars))
        #expect(!InvitationActions.canRespond(own, calendars: calendars))
        #expect(!InvitationActions.isInvitation(own))
        #expect(InvitationActions.isInvitation(abo))
        #expect(!InvitationActions.canRespond(abo, calendars: calendars))
    }

    @Test func seriesAskThisOrAll() throws {
        let occ = try #require(occurrences(I.invite("s", day: 2, rrule: "FREQ=WEEKLY;COUNT=4")).dropFirst().first)
        #expect(InvitationActions.scopes(occ) == [.this, .all])
        #expect(InvitationActions.intent(occ, response: .accepted, scope: .all) == .respond(eventId: "s", status: .accepted))
        guard case let .respond(_, rid, _) = InvitationActions.intent(occ, response: .accepted, scope: .this) else {
            Issue.record("respond erwartet")
            return
        }
        #expect(rid?.normalized == F.at(9, 9))
        let single = occurrences(I.invite("e"))[0]
        #expect(InvitationActions.scopes(single).isEmpty)
        #expect(InvitationActions.intent(single, response: .declined, scope: .this) == .respond(eventId: "e", status: .declined))
    }

    @Test func currentStatusFollowsOutbox() throws {
        var s = SyncState()
        s.events["s"] = I.invite("s", day: 2, rrule: "FREQ=WEEKLY;COUNT=4")
        let list = occurrences(s.events["s"]!)
        let second = list[1]
        s.enqueue(Mutation(.respond(eventId: "s", occurrence: second.recurrenceId, status: .tentative, comment: nil)), now: F.now)
        let v = s.visibleEvents(deviceTimeZone: F.tz)
        #expect(InvitationActions.currentStatus(of: second, in: v) == .tentative)
        #expect(InvitationActions.currentStatus(of: list[0], in: v) == .needsAction)
        #expect(InvitationActions.currentStatus(of: second, in: [:]) == .needsAction)
        // Eigener Eintrag unter den Teilnehmenden folgt mit.
        let override = try #require(v["s"]?.overrides.first)
        #expect(override.details.attendees.first { $0.isSelf == true }?.status == .tentative)
    }

    @Test func answeringAllAlsoAnswersChangedOccurrences() {
        var s = SyncState()
        var e = I.invite("s", day: 2, rrule: "FREQ=WEEKLY;COUNT=4")
        var moved = e.details
        moved.start = F.at(9, 14)
        moved.end = F.at(9, 15)
        e.overrides = [EventOverride(recurrenceId: F.at(9, 9), details: moved)]
        s.events["s"] = e
        s.enqueue(Mutation(.respond(eventId: "s", occurrence: nil, status: .accepted, comment: nil)), now: F.now)
        let v = s.visibleEvents(deviceTimeZone: F.tz)["s"]!
        #expect(v.details.selfStatus == .accepted)
        #expect(v.overrides.map(\.details.selfStatus) == [.accepted])
        #expect(CalendarQuery(state: s, deviceTimeZone: F.tz).invitations(now: F.now).isEmpty)
    }

    @Test func undoRestoresPreviousAnswer() {
        var s = SyncState()
        s.events["e"] = I.invite("e", status: .declined)
        let intent = InvitationActions.intent(occurrences(s.events["e"]!)[0], response: .accepted)
        #expect(intent.inverse(before: s.events, createdId: nil, deviceTimeZone: F.tz) == .respond(eventId: "e", status: .declined))
    }
}
