import Foundation
import LichtungKit
import Testing

private enum Trips {
    static func occurrence(_ start: Date, _ end: Date, travel: Int?, allDay: Bool = false) -> Occurrence {
        let day = LocalDate(start, calendar: .lichtung(timeZone: berlin))
        let value: DateValue = allDay ? .date(day) : .dateTime(LocalDateTime(start, calendar: .lichtung(timeZone: berlin)), timeZone: "Europe/Berlin")
        let details = EventDetails(title: "Termin", start: value, end: value, travelTimeMinutes: travel)
        return Occurrence(eventId: "e1", calendarId: "c1", recurrenceId: nil, details: details, start: start, end: end, isModified: false)
    }
}

@Suite("TravelTime")
struct TravelTimeTests {
    @Test("Reisezeit liegt direkt vor dem Beginn")
    func block() {
        let occ = Trips.occurrence(berlinDate(2026, 10, 27, 14), berlinDate(2026, 10, 27, 15), travel: 30)
        #expect(TravelTime.travelBlock(for: occ) == DateInterval(start: berlinDate(2026, 10, 27, 13, 30), end: berlinDate(2026, 10, 27, 14)))
        #expect(TravelTime.busyInterval(for: occ) == DateInterval(start: berlinDate(2026, 10, 27, 13, 30), end: berlinDate(2026, 10, 27, 15)))
    }

    @Test("Ohne Reisezeit, mit 0 oder ganztägig: kein Block")
    func none() {
        let start = berlinDate(2026, 10, 27, 14), end = berlinDate(2026, 10, 27, 15)
        for travel in [nil, 0, -5] {
            let occ = Trips.occurrence(start, end, travel: travel)
            #expect(TravelTime.travelBlock(for: occ) == nil)
            #expect(TravelTime.busyInterval(for: occ) == DateInterval(start: start, end: end))
        }
        let allDay = Trips.occurrence(berlinDate(2026, 10, 27), berlinDate(2026, 10, 28), travel: 60, allDay: true)
        #expect(TravelTime.travelBlock(for: allDay) == nil)
        #expect(TravelTime.busyInterval(for: allDay).duration == 24 * 3600)
    }

    @Test("Über die Zeitumstellung zählt echte Zeit (25.10.2026)")
    func acrossDSTChange() {
        // 3:00 MEZ am Umstellungstag; 90 Minuten davor ist 2:30 Sommerzeit (MESZ).
        let start = berlinDate(2026, 10, 25, 3)
        let occ = Trips.occurrence(start, start.addingTimeInterval(3600), travel: 90)
        let block = try! #require(TravelTime.travelBlock(for: occ))
        #expect(block.duration == 90 * 60)
        #expect(berlin.secondsFromGMT(for: block.start) == 7200)
        let wall = LocalDateTime(block.start, calendar: .lichtung(timeZone: berlin))
        #expect((wall.hour, wall.minute) == (2, 30))
    }

    @Test("Auswahl und Beschriftung")
    func labels() {
        #expect(TravelTime.presets == [5, 15, 30, 60, 90, 120])
        #expect(TravelTime.presets.map(TravelTime.label(minutes:))
            == ["5 Min.", "15 Min.", "30 Min.", "1 Std.", "1,5 Std.", "2 Std."])
        #expect(TravelTime.label(minutes: 45) == "45 Min.")
        #expect(TravelTime.label(minutes: 75) == "1 Std. 15 Min.")
        #expect(TravelTime.label(minutes: 150) == "2,5 Std.")
    }
}
