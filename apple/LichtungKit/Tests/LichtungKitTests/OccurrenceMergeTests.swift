import Foundation
@testable import LichtungKit
import Testing

private enum M {
    static let tz = TimeZone(identifier: "Europe/Berlin")!
    static let calendars: [String: CalendarInfo] = [
        "work": CalendarInfo(id: "work", accountId: "a", name: "Arbeit", writable: false, position: 0),
        "private": CalendarInfo(id: "private", accountId: "a", name: "Privat", writable: true, position: 1),
        "drk": CalendarInfo(id: "drk", accountId: "a", name: "DRK", writable: true, position: 2),
    ]

    static func occ(
        _ id: String, _ title: String, in calendar: String, from sh: Int = 9, to eh: Int = 10,
        selfStatus: ParticipationStatus? = nil
    ) -> Occurrence {
        func value(_ h: Int) -> DateValue {
            .dateTime(LocalDateTime(date: LocalDate(year: 2026, month: 11, day: 10), hour: h, minute: 0), timeZone: "Europe/Berlin")
        }
        let details = EventDetails(title: title, start: value(sh), end: value(eh), selfStatus: selfStatus)
        return Occurrence(eventId: id, calendarId: calendar, recurrenceId: nil, details: details,
                          start: value(sh).instant(deviceTimeZone: tz), end: value(eh).instant(deviceTimeZone: tz), isModified: false)
    }

    static func merge(_ occ: [Occurrence], _ rules: MergeRules = .default) -> [MergedOccurrence] {
        OccurrenceMerge.merge(occ, calendars: calendars, rules: rules)
    }
}

@Suite("Termine zusammenfassen")
struct OccurrenceMergeTests {
    @Test func identicalInThreeCalendarsBecomeOne() {
        let r = M.merge([M.occ("a", "Lehrgang", in: "work"), M.occ("b", "Lehrgang", in: "private"), M.occ("c", "Lehrgang", in: "drk")])
        #expect(r.count == 1)
        #expect(r[0].shown.calendarId == "private")
        #expect(r[0].alsoInCalendarIds == ["work", "drk"])
        #expect(r[0].others.map(\.calendarId) == ["work", "drk"])
    }

    @Test func titleCompareIgnoresCaseAndWhitespace() {
        #expect(M.merge([M.occ("a", " lehrgang ", in: "work"), M.occ("b", "Lehrgang", in: "private")]).count == 1)
    }

    @Test func differentEndIsNotMerged() {
        #expect(M.merge([M.occ("a", "Lehrgang", in: "work"), M.occ("b", "Lehrgang", in: "private", to: 11)]).count == 2)
    }

    @Test func sameCalendarIsNotMerged() {
        #expect(M.merge([M.occ("a", "Lehrgang", in: "work"), M.occ("b", "Lehrgang", in: "work")]).count == 2)
    }

    @Test func blockerWithPartnerIsHidden() {
        let r = M.merge([M.occ("a", "Sport", in: "private"), M.occ("b", "Blockiert", in: "work")])
        #expect(r.count == 1)
        #expect(r[0].shown.title == "Sport")
        #expect(r[0].alsoInCalendarIds == ["work"])
    }

    @Test func blockerWithoutPartnerStays() {
        let r = M.merge([M.occ("a", "Sport", in: "private", from: 11, to: 12), M.occ("b", "Blockiert", in: "work")])
        #expect(r.count == 2)
    }

    @Test func blockerWithTwoPartnersGoesToOne() {
        let r = M.merge([M.occ("a", "Sport", in: "private"), M.occ("b", "Dienst", in: "drk"), M.occ("c", "Blockiert", in: "work")])
        #expect(r.count == 2)
        #expect(r.filter { $0.others.contains { $0.title == "Blockiert" } }.count == 1)
    }

    @Test func twoBlockersSameTimeWithoutPartnerStayMergedAsIdentical() {
        let r = M.merge([M.occ("a", "Blockiert", in: "work"), M.occ("b", "Blockiert", in: "private")])
        #expect(r.count == 1)
        #expect(r[0].shown.title == "Blockiert")
    }

    @Test func pendingInvitationWinsAsShown() {
        let r = M.merge([M.occ("a", "Lehrgang", in: "work", selfStatus: .needsAction), M.occ("b", "Lehrgang", in: "private")])
        #expect(r.count == 1)
        #expect(r[0].shown.calendarId == "work")
        #expect(r[0].alsoInCalendarIds == ["private"])
    }

    @Test func pendingBlockerIsNotHidden() {
        let r = M.merge([M.occ("a", "Sport", in: "private"), M.occ("b", "Blockiert", in: "work", selfStatus: .needsAction)])
        #expect(r.count == 2)
    }

    @Test func emptyBlockerTitleHidesNothing() {
        let rules = MergeRules(mergeIdentical: false, hideBlockers: true, blockerTitles: ["", "  "])
        let r = M.merge([M.occ("a", "Sport", in: "private"), M.occ("b", "", in: "work")], rules)
        #expect(r.count == 2)
    }

    @Test func rulesOffReturnsEverything() {
        let r = M.merge([M.occ("a", "Sport", in: "private"), M.occ("b", "Sport", in: "work"), M.occ("c", "Blockiert", in: "drk")], .off)
        #expect(r.count == 3)
        #expect(r.allSatisfy { $0.others.isEmpty })
    }

    @Test func orderOfInputIsKept() {
        let r = M.merge([
            M.occ("a", "Früh", in: "work", from: 8, to: 9),
            M.occ("b", "Lehrgang", in: "work"), M.occ("c", "Spät", in: "drk", from: 12, to: 13),
            M.occ("d", "Lehrgang", in: "private"),
        ])
        // shown ist "d" (privat, schreibbar): Position 4, also hinter „Spät“.
        #expect(r.map(\.shown.eventId) == ["a", "c", "d"])
    }
}
