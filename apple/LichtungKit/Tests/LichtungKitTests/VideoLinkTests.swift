import Foundation
import LichtungKit
import Testing

/// Alle Meeting-IDs, Kenncodes, Räume und Firmen sind erfunden.
private struct VideoCase: CustomTestStringConvertible, Sendable {
    let note: String
    let input: String
    /// `nil`: kein Videolink erwartet.
    let provider: VideoLink.Provider?
    var url: String? = nil
    var id: String? = nil
    var passcode: String? = nil

    var testDescription: String { note }
}

private let teamsLink = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_QWxwaGFCZXRh%40thread.v2/0?context=%7b%22Tid%22%3a%2200000000-1111-2222-3333-444444444444%22%7d"

private let videoCases: [VideoCase] = [
    // Zoom
    .init(note: "Zoom schlicht", input: "https://zoom.us/j/1234567890",
          provider: .zoom, url: "https://zoom.us/j/1234567890", id: "1234567890"),
    .init(note: "Zoom mit Kenncode", input: "Einwahl: https://zoom.us/j/9876543210?pwd=QWxwaGExMjM",
          provider: .zoom, url: "https://zoom.us/j/9876543210?pwd=QWxwaGExMjM", id: "9876543210", passcode: "QWxwaGExMjM"),
    .init(note: "Zoom Firmen-Subdomain", input: "https://beispielfirma.zoom.us/j/1112223334?pwd=abc.1",
          provider: .zoom, url: "https://beispielfirma.zoom.us/j/1112223334?pwd=abc.1", id: "1112223334", passcode: "abc.1"),
    .init(note: "Zoom persönlicher Raum", input: "Mein Raum: zoom.us/my/max.muster",
          provider: .zoom, url: "https://zoom.us/my/max.muster", id: "max.muster"),
    .init(note: "Zoom ohne Schema", input: "zoom.us/j/123",
          provider: .zoom, url: "https://zoom.us/j/123", id: "123"),
    .init(note: "Zoom mit Punkt am Satzende", input: "Bitte über https://us02web.zoom.us/j/5550001112 beitreten.",
          provider: .zoom, url: "https://us02web.zoom.us/j/5550001112", id: "5550001112"),
    .init(note: "Zoom in Klammern", input: "Call (https://zoom.us/j/4440001112).",
          provider: .zoom, url: "https://zoom.us/j/4440001112", id: "4440001112"),
    .init(note: "Zoom-Startseite ist kein Meeting", input: "Infos auf https://zoom.us/pricing", provider: nil),

    // Teams
    .init(note: "Teams meetup-join mit Escapes", input: teamsLink, provider: .teams, url: teamsLink),
    .init(note: "Teams in spitzen Klammern (Outlook)",
          input: "Microsoft Teams-Besprechung\nJetzt an der Besprechung teilnehmen<\(teamsLink)>\nBesprechungs-ID: 000 000 000",
          provider: .teams, url: teamsLink),
    .init(note: "Teams neues Kurzformat", input: "https://teams.microsoft.com/meet/31415926535?p=AbCdEfGh",
          provider: .teams, url: "https://teams.microsoft.com/meet/31415926535?p=AbCdEfGh", id: "31415926535", passcode: "AbCdEfGh"),
    .init(note: "Teams privat", input: "https://teams.live.com/meet/9990001112223",
          provider: .teams, url: "https://teams.live.com/meet/9990001112223", id: "9990001112223"),
    .init(note: "Teams hinter Safelinks",
          input: "https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2Fl%2Fmeetup-join%2F19%253ameeting_QWxwaGFCZXRh%2540thread.v2%2F0&data=05%7C01%7Cx&reserved=0",
          provider: .teams, url: "https://teams.microsoft.com/l/meetup-join/19%3ameeting_QWxwaGFCZXRh%40thread.v2/0"),
    .init(note: "Teams in HTML mit &amp;",
          input: "<p><a href=\"https://teams.microsoft.com/l/meetup-join/19%3ameeting_ZGVsdGE%40thread.v2/0?context=%7b%7d&amp;anon=true\">Teilnehmen</a></p>",
          provider: .teams, url: "https://teams.microsoft.com/l/meetup-join/19%3ameeting_ZGVsdGE%40thread.v2/0?context=%7b%7d&anon=true"),

    // Google Meet
    .init(note: "Meet", input: "https://meet.google.com/abc-defg-hij",
          provider: .meet, url: "https://meet.google.com/abc-defg-hij", id: "abc-defg-hij"),
    .init(note: "Meet ohne Schema mit Komma", input: "Meet: meet.google.com/xyz-abcd-efg, Tel. folgt",
          provider: .meet, url: "https://meet.google.com/xyz-abcd-efg", id: "xyz-abcd-efg"),
    .init(note: "Meet mit Query", input: "https://meet.google.com/qrs-tuvw-xyz?authuser=1",
          provider: .meet, url: "https://meet.google.com/qrs-tuvw-xyz?authuser=1", id: "qrs-tuvw-xyz"),

    // Übrige Anbieter
    .init(note: "Webex mit MTID", input: "https://beispiel.webex.com/beispiel/j.php?MTID=m0123456789abcdef",
          provider: .webex, url: "https://beispiel.webex.com/beispiel/j.php?MTID=m0123456789abcdef", id: "m0123456789abcdef"),
    .init(note: "Webex persönlicher Raum", input: "https://beispiel.webex.com/meet/erika.muster",
          provider: .webex, url: "https://beispiel.webex.com/meet/erika.muster", id: "erika.muster"),
    .init(note: "Jitsi", input: "Treffpunkt: https://meet.jit.si/LichtungTestraum42!",
          provider: .jitsi, url: "https://meet.jit.si/LichtungTestraum42", id: "LichtungTestraum42"),
    .init(note: "FaceTime", input: "https://facetime.apple.com/join#v=1&p=AbCdEf&k=GhIjKl",
          provider: .facetime, url: "https://facetime.apple.com/join#v=1&p=AbCdEf&k=GhIjKl"),
    .init(note: "Whereby", input: "https://whereby.com/lichtung-demo",
          provider: .whereby, url: "https://whereby.com/lichtung-demo", id: "lichtung-demo"),
    .init(note: "GoTo Meeting", input: "https://global.gotomeeting.com/join/123456789",
          provider: .gotomeeting, url: "https://global.gotomeeting.com/join/123456789", id: "123456789"),
    .init(note: "Skype", input: "https://join.skype.com/AbCdEfGhIjKl",
          provider: .skype, url: "https://join.skype.com/AbCdEfGhIjKl", id: "AbCdEfGhIjKl"),
    .init(note: "Markdown-Link", input: "[Beitreten](https://zoom.us/j/7770001112)",
          provider: .zoom, url: "https://zoom.us/j/7770001112", id: "7770001112"),

    // Kein Videolink
    .init(note: "Normale Webseite", input: "Agenda: https://example.org/agenda", provider: nil),
    .init(note: "E-Mail-Adresse bei Zoom", input: "Fragen an support@zoom.us", provider: nil),
    .init(note: "Leerer Text", input: "", provider: nil),
    .init(note: "Webex-Startseite", input: "https://www.webex.com/", provider: nil),
]

private func details(url: String? = nil, location: String? = nil, notes: String? = nil) -> EventDetails {
    let day = LocalDate(year: 2026, month: 10, day: 27)
    return EventDetails(
        title: "Abstimmung", location: location, notes: notes, url: url,
        start: .dateTime(LocalDateTime(date: day, hour: 14, minute: 0), timeZone: "Europe/Berlin"),
        end: .dateTime(LocalDateTime(date: day, hour: 15, minute: 0), timeZone: "Europe/Berlin")
    )
}

@Suite("VideoLink")
struct VideoLinkTests {
    @Test(arguments: videoCases)
    fileprivate func table(_ c: VideoCase) {
        let link = VideoLinkDetector.detectAll(in: c.input).first
        #expect(link?.provider == c.provider)
        guard let link, c.provider != nil else { return }
        #expect(link.url.absoluteString == c.url)
        #expect(link.meetingID == c.id)
        #expect(link.passcode == c.passcode)
    }

    @Test("Reihenfolge: url vor location vor notes")
    func fieldOrder() {
        let d = details(url: "https://meet.google.com/aaa-bbbb-ccc", location: "https://zoom.us/j/1", notes: teamsLink)
        #expect(VideoLinkDetector.detect(in: d)?.provider == .meet)
        #expect(VideoLinkDetector.detect(in: details(location: "https://zoom.us/j/1", notes: teamsLink))?.provider == .zoom)
        #expect(VideoLinkDetector.detect(in: details(notes: teamsLink))?.provider == .teams)
    }

    @Test("Bekannter Anbieter in den Notizen schlägt unbekannte url")
    func knownBeforeOther() {
        let d = details(url: "https://intranet.example.org/raum", notes: "https://zoom.us/j/42")
        #expect(VideoLinkDetector.detect(in: d)?.provider == .zoom)
    }

    @Test("other nur aus details.url, nie aus Freitext")
    func other() {
        let d = details(url: "https://video.example.org/r/1")
        let link = VideoLinkDetector.detect(in: d)
        #expect(link?.provider == .other)
        #expect(link?.url.absoluteString == "https://video.example.org/r/1")
        #expect(VideoLinkDetector.detect(in: details(notes: "https://video.example.org/r/1")) == nil)
        #expect(VideoLinkDetector.detect(in: details(url: "mailto:a@example.org")) == nil)
        #expect(VideoLinkDetector.detect(in: details()) == nil)
    }

    @Test("Mehrere Links: alle, in Reihenfolge, ohne Doppelte")
    func all() {
        let text = "Zoom https://zoom.us/j/1 oder <https://zoom.us/j/1> oder https://meet.google.com/abc-defg-hij"
        #expect(VideoLinkDetector.detectAll(in: text).map(\.provider) == [.zoom, .meet])
    }

    @Test("App-Links für Zoom und Teams")
    func appURLs() {
        let zoom = VideoLinkDetector.detectAll(in: "https://firma.zoom.us/j/123?pwd=xY9").first
        #expect(zoom?.appURL?.absoluteString == "zoommtg://firma.zoom.us/join?action=join&confno=123&pwd=xY9")
        #expect(zoom?.joinURL.absoluteString == "https://firma.zoom.us/j/123?pwd=xY9")
        let teams = VideoLinkDetector.detectAll(in: teamsLink).first
        #expect(teams?.appURL?.absoluteString == "msteams:" + teamsLink.dropFirst("https://teams.microsoft.com".count))
        #expect(VideoLinkDetector.detectAll(in: "https://meet.google.com/abc-defg-hij").first?.appURL == nil)
        #expect(VideoLinkDetector.detectAll(in: "zoom.us/my/max").first?.appURL == nil)
    }

    @Test("Anzeigenamen sind deutsch")
    func names() {
        #expect(VideoLink.Provider.teams.displayName == "Microsoft Teams")
        #expect(VideoLink.Provider.meet.displayName == "Google Meet")
        #expect(VideoLink.Provider.allCases.allSatisfy { !$0.displayName.isEmpty })
    }

    @Test("Beitreten ab zehn Minuten vorher bis zum Ende")
    func joinable() {
        let tz = TimeZone(identifier: "Europe/Berlin")!
        let cal = Calendar.lichtung(timeZone: tz)
        let d = details(url: "https://zoom.us/j/1")
        let start = d.start.instant(deviceTimeZone: tz)
        let end = d.end.instant(deviceTimeZone: tz)
        let occ = Occurrence(eventId: "e1", calendarId: "c1", recurrenceId: nil, details: d, start: start, end: end, isModified: false)
        func at(_ h: Int, _ m: Int) -> Date { LocalDate(year: 2026, month: 10, day: 27).date(hour: h, minute: m, in: cal) }
        #expect(!VideoLinkDetector.isJoinable(occ, now: at(13, 49)))
        #expect(VideoLinkDetector.isJoinable(occ, now: at(13, 50)))
        #expect(VideoLinkDetector.isJoinable(occ, now: at(14, 30)))
        #expect(!VideoLinkDetector.isJoinable(occ, now: at(15, 0)))
        #expect(VideoLinkDetector.isJoinable(occ, now: at(13, 45), lead: 15 * 60))
    }
}
