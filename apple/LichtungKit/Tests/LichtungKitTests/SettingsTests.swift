import Foundation
@testable import LichtungKit
import Testing

@Suite("Einstellungen: Reihenfolge")
struct CalendarOrderTests {
    func cal(_ id: String, _ account: String, _ position: Int, _ name: String? = nil) -> CalendarInfo {
        CalendarInfo(id: id, accountId: account, name: name ?? id, position: position)
    }

    @Test func movedMatchesSwiftUI() {
        let items = ["a", "b", "c", "d"]
        #expect(CalendarOrder.moved(items, fromOffsets: [0], toOffset: 2) == ["b", "a", "c", "d"])
        #expect(CalendarOrder.moved(items, fromOffsets: [3], toOffset: 0) == ["d", "a", "b", "c"])
        #expect(CalendarOrder.moved(items, fromOffsets: [0, 2], toOffset: 4) == ["b", "d", "a", "c"])
        #expect(CalendarOrder.moved(items, fromOffsets: [1], toOffset: 1) == items)
        #expect(CalendarOrder.moved(items, fromOffsets: [9], toOffset: 0) == items)
    }

    @Test func sortedIsStable() {
        let cals = [cal("c3", "a", 0, "Zeta"), cal("c1", "a", 0, "Alpha"), cal("c2", "a", -1, "Mitte")]
        #expect(CalendarOrder.sorted(cals).map(\.id) == ["c2", "c1", "c3"])
    }

    @Test func sectionsFollowFirstCalendar() {
        let accounts: [String: Account] = [
            "acc_f": Account(id: "acc_f", kind: .fastmail, name: "Fastmail"),
            "acc_i": Account(id: "acc_i", kind: .icloud, name: "iCloud"),
            "acc_leer": Account(id: "acc_leer", kind: .ics, name: "Feiertage"),
        ]
        let cals = [cal("i1", "acc_i", 0), cal("f1", "acc_f", 1), cal("f2", "acc_f", 2), cal("i2", "acc_i", 3), cal("x", "acc_weg", 4)]
        let sections = CalendarOrder.sections(calendars: Dictionary(uniqueKeysWithValues: cals.map { ($0.id, $0) }), accounts: accounts)
        #expect(sections.map(\.accountId) == ["acc_i", "acc_f", "acc_weg", "acc_leer"])
        #expect(sections[0].calendars.map(\.id) == ["i1", "i2"])
        #expect(sections[2].title == "Weitere Kalender")
        #expect(sections[3].calendars.isEmpty)
    }

    @Test func moveWithinSectionKeepsOtherSlots() {
        // Global: i1(0) f1(1) f2(2) i2(3). iCloud-Abschnitt: [i1, i2] → [i2, i1].
        let cals = [cal("i1", "acc_i", 0), cal("f1", "acc_f", 1), cal("f2", "acc_f", 2), cal("i2", "acc_i", 3)]
        let changes = CalendarOrder.positionsAfterMove(section: ["i1", "i2"], fromOffsets: [1], toOffset: 0, all: cals)
        #expect(changes == ["i2": 0, "i1": 3])
    }

    @Test func moveRenumbersDuplicatePositions() {
        let cals = [cal("a", "x", 0, "A"), cal("b", "x", 0, "B"), cal("c", "x", 0, "C")]
        let changes = CalendarOrder.positionsAfterMove(section: ["a", "b", "c"], fromOffsets: [2], toOffset: 0, all: cals)
        #expect(changes == ["a": 1, "b": 2])
    }

    @Test func setPositions() {
        let sets = [CalendarSet(id: "s1", name: "Arbeit", calendarIds: [], position: 0),
                    CalendarSet(id: "s2", name: "Privat", calendarIds: [], position: 1),
                    CalendarSet(id: "s3", name: "Reisen", calendarIds: [], position: 2)]
        #expect(CalendarOrder.setPositionsAfterMove(sets, fromOffsets: [0], toOffset: 3) == ["s2": 0, "s3": 1, "s1": 2])
        #expect(CalendarOrder.nextSetPosition(sets) == 3)
        #expect(CalendarOrder.nextSetPosition([]) == 0)
    }
}

@Suite("Einstellungen: Texte und Regeln")
struct SettingsTextTests {
    let now = Date(timeIntervalSince1970: 1_793_000_000)

    @Test func accountStatus() {
        typealias S = Account.Status
        #expect(AccountStatusSummary(S(state: .ok, lastSyncAt: now.addingTimeInterval(-300)), now: now).text == "Aktuell · vor 5 Min.")
        #expect(AccountStatusSummary(S(state: .ok), now: now).text == "Aktuell")
        #expect(AccountStatusSummary(S(state: .syncing), now: now).level == .busy)
        let failed = AccountStatusSummary(S(state: .authFailed), now: now)
        #expect(failed.needsCredentials)
        #expect(failed.level == .error)
        #expect(AccountStatusSummary(S(state: .error, message: "Zeitüberschreitung"), now: now).text == "Abgleich gestört: Zeitüberschreitung")
        #expect(AccountStatusSummary(nil, now: now).text == "Noch nicht abgeglichen")
    }

    @Test func relativeTimes() {
        #expect(AccountStatusSummary.relative(now, now: now) == "gerade eben")
        #expect(AccountStatusSummary.relative(now.addingTimeInterval(-3 * 3600), now: now) == "vor 3 Std.")
        #expect(AccountStatusSummary.relative(now.addingTimeInterval(-26 * 3600), now: now) == "vor einem Tag")
        #expect(AccountStatusSummary.relative(now.addingTimeInterval(-50 * 3600), now: now) == "vor 2 Tagen")
    }

    @Test func caldavKindDecodes() throws {
        let a = try APICoding.decoder.decode(Account.self, from: Data(#"{"id": "acc_1", "kind": "caldav", "name": "Nextcloud"}"#.utf8))
        #expect(a.kind == .caldav)
        #expect(a.kind.displayName == "CalDAV")
        #expect(Account.Kind.fastmail.passwordHint?.contains("App-Passwort") == true)
    }

    @Test func workCalendarNeedsConfirmation() {
        let work = CalendarInfo(id: "c1", accountId: "a1", name: "Arbeit")
        let privat = CalendarInfo(id: "c2", accountId: "a1", name: "Privat")
        let graph = Account(id: "a2", kind: .microsoft, name: "Firma GmbH")
        #expect(MCPWriteGuard.isWorkCalendar(work, account: nil))
        #expect(!MCPWriteGuard.isWorkCalendar(privat, account: Account(id: "a1", kind: .fastmail, name: "Fastmail")))
        #expect(MCPWriteGuard.isWorkCalendar(privat, account: graph))
        #expect(MCPWriteGuard.confirmation(enabling: true, calendar: work, account: nil)?.contains("„Arbeit“") == true)
        #expect(MCPWriteGuard.confirmation(enabling: false, calendar: work, account: nil) == nil)
        #expect(MCPWriteGuard.confirmation(enabling: true, calendar: privat, account: nil) == nil)
    }

    @Test func palette() {
        #expect(CalendarPalette.swatches.allSatisfy { CalendarPalette.normalizedHex($0.hex) == $0.hex })
        #expect(Set(CalendarPalette.swatches.map(\.hex)).count == CalendarPalette.swatches.count)
        #expect(CalendarPalette.normalizedHex(" #a1b2c3 ") == "#A1B2C3")
        #expect(CalendarPalette.normalizedHex("a1b2c3") == "#A1B2C3")
        #expect(CalendarPalette.normalizedHex("#12345") == nil)
        #expect(CalendarPalette.normalizedHex("#GGGGGG") == nil)
    }

    @Test func errorTexts() {
        #expect(APIError.server(status: 422, code: "invalid_credentials", message: "x").userMessage.contains("App-Passwort"))
        #expect(APIError.server(status: 400, code: "invalid_request", message: "name fehlt").userMessage == "name fehlt")
        #expect(APIError.server(status: 503, code: nil, message: nil).userMessage == "Fehler auf dem Server (503).")
        #expect(APIError.transport("offline").localizedDescription == "Keine Verbindung zum Server.")
        #expect(APIError.tooManyRequests(retryAfter: 2.5).userMessage.contains("3 s"))
    }

    @Test func newAccountBodies() {
        let caldav = NewAccount.caldav(url: URL(string: "https://dav.example.org/")!, username: "u", password: "p", name: " ").body
        #expect(caldav["kind"] == "caldav")
        #expect(caldav["url"] == "https://dav.example.org/")
        #expect(caldav["name"] == nil)
        let ics = NewAccount.icsWithLogin(url: URL(string: "https://feeds.example.org/a.ics")!, name: "Abo", username: "u", password: "p").body
        #expect(ics["kind"] == "ics")
        #expect(ics["username"] == "u")
    }

    @Test func deviceListUsesServerFields() throws {
        let json = #"[{"id": "d1", "name": "iPhone", "created_at": "2026-10-20T07:00:00.123Z", "last_used_at": "2026-10-25T08:00:00Z", "current": true}]"#
        let devices = try APICoding.decoder.decode([Device].self, from: Data(json.utf8))
        #expect(devices[0].lastActivity != nil)
        #expect(devices[0].createdAt != nil)
        #expect(devices[0].current == true)
    }

    @Test func meDecodes() throws {
        let json = #"{"user_id": "u1", "name": "Ruben", "email": "ruben@example.org", "auth": "device", "device": {"id": "d1", "name": "Mac"}}"#
        let me = try APICoding.decoder.decode(ServerInfo.self, from: Data(json.utf8))
        #expect(me.device?.id == "d1")
        #expect(me.serverVersion == nil)
    }
}

@Suite("Einstellungen: zweite Zeitzone")
struct TimeZoneCatalogTests {
    let home = TimeZone(identifier: "Europe/Berlin")!
    let now = Date(timeIntervalSince1970: 1_793_000_000)

    @Test func choicesAndSearch() {
        let ids = ["Europe/Berlin", "Europe/Vienna", "America/New_York", "Etc/GMT+5", "UTC", "Asia/Kolkata"]
        let choices = TimeZoneCatalog.choices(home: home, at: now, identifiers: ids)
        #expect(choices.map(\.id) == ["Europe/Berlin", "Asia/Kolkata", "America/New_York", "Europe/Vienna"])
        #expect(choices.first { $0.id == "Europe/Vienna" }?.city == "Wien")
        #expect(choices.first { $0.id == "Europe/Berlin" }?.offset == "±0")
        #expect(TimeZoneCatalog.filter(choices, query: "york").map(\.id) == ["America/New_York"])
        #expect(TimeZoneCatalog.filter(choices, query: "WIEN").map(\.id) == ["Europe/Vienna"])
        #expect(TimeZoneCatalog.filter(choices, query: "new york").map(\.id) == ["America/New_York"])
        #expect(TimeZoneCatalog.filter(choices, query: " ").count == choices.count)
    }

    @Test func storedValue() {
        #expect(TimeZoneCatalog.zone(fromStored: "") == nil)
        #expect(TimeZoneCatalog.zone(fromStored: "America/New_York")?.identifier == "America/New_York")
        #expect(TimeZoneCatalog.zone(fromStored: "Mars/Olympus") == nil)
    }
}

@Suite("Einstellungen: Konto anlegen")
struct NewAccountFormTests {
    @Test func fastmailNeedsCredentials() {
        var f = NewAccountForm(kind: .fastmail)
        #expect(f.missing == "Benutzername fehlt.")
        f.username = " person@example.org "
        #expect(f.missing == "Passwort fehlt.")
        f.password = "erfunden-123"
        #expect(f.build() == .fastmail(username: "person@example.org", appPassword: "erfunden-123", name: "Fastmail"))
    }

    @Test func icsWithoutLogin() {
        var f = NewAccountForm(kind: .ics)
        #expect(f.missing == "Adresse fehlt.")
        f.url = "webcal://feeds.example.org/feiertage.ics"
        #expect(f.build() == .ics(url: URL(string: "webcal://feeds.example.org/feiertage.ics")!, name: ""))
        f.password = "geheim"
        #expect(f.missing == "Zum Passwort gehört ein Benutzername.")
        f.username = "u"
        #expect(f.build() == .icsWithLogin(url: URL(string: "webcal://feeds.example.org/feiertage.ics")!, name: "", username: "u", password: "geheim"))
    }

    @Test func caldavURL() {
        var f = NewAccountForm(kind: .caldav)
        f.url = "dav.example.org/remote.php/dav"
        f.username = "u"
        f.password = "p"
        f.name = "Nextcloud"
        #expect(f.build() == .caldav(url: URL(string: "https://dav.example.org/remote.php/dav")!, username: "u", password: "p", name: "Nextcloud"))
        f.url = "webcal://dav.example.org"
        #expect(f.missing == "Die Adresse ist ungültig.")
    }

    @Test func settingsApplyLocally() {
        let c = CalendarInfo(id: "c1", accountId: "a1", name: "Privat", color: "#111111", providerColor: "#222222")
        let changed = CalendarSettings(color: .clear, hidden: true, mcpWrite: true, position: 4).applied(to: c)
        #expect(changed.displayColor == "#222222")
        #expect(changed.hidden && changed.mcpWrite)
        #expect(changed.position == 4)
        #expect(changed.openingsBusy == c.openingsBusy)
        #expect(CalendarSettings(color: .set("#333333")).applied(to: c).color == "#333333")
    }
}
