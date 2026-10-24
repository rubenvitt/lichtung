import Foundation
import LichtungKit
import Testing

/// Umstellungstage: Europa 25.10.2026 (Ende Sommerzeit) und 28.3.2027
/// (Beginn), USA 1.11.2026 (Ende). Dazwischen liegt Berlin–New York bei −5.
private enum Zones {
    static let newYork = TimeZone(identifier: "America/New_York")!
    static let kolkata = TimeZone(identifier: "Asia/Kolkata")!
    static let london = TimeZone(identifier: "Europe/London")!
    static let kathmandu = TimeZone(identifier: "Asia/Kathmandu")!
    static let paris = TimeZone(identifier: "Europe/Paris")!

    static func hours(from first: Int, count: Int) -> [String] {
        (0..<count).map { String(format: "%02d", (first + $0) % 24) }
    }

    static func details(start: DateValue) -> EventDetails {
        EventDetails(title: "Call", start: start, end: start)
    }
}

@Suite("TimeZoneDisplay")
struct TimeZoneDisplayTests {
    @Test("Kurznamen")
    func labels() {
        #expect(ZoneClock(timeZone: Zones.newYork).label == "New York")
        #expect(ZoneClock(timeZone: Zones.newYork, label: "NYC").label == "NYC")
        #expect(ZoneClock(timeZone: Zones.newYork, label: "  ").label == "New York")
        #expect(ZoneClock(timeZone: Zones.kolkata).label == "Kolkata")
        #expect(ZoneClock(timeZone: TimeZone(identifier: "Europe/Vienna")!).label == "Wien")
        #expect(ZoneClock(timeZone: TimeZone(identifier: "America/Argentina/Buenos_Aires")!).label == "Buenos Aires")
        #expect(ZoneClock(timeZone: TimeZone(identifier: "UTC")!).label == "UTC")
    }

    @Test("Abstand mit Sommerzeit beider Seiten")
    func offsets() {
        #expect(TimeZoneDisplay.offsetLabel(Zones.newYork, relativeTo: berlin, at: berlinDate(2026, 10, 23, 12)) == "−6 Std.")
        // Europa schon Winterzeit, USA noch Sommerzeit
        #expect(TimeZoneDisplay.offsetLabel(Zones.newYork, relativeTo: berlin, at: berlinDate(2026, 10, 28, 12)) == "−5 Std.")
        #expect(TimeZoneDisplay.offsetLabel(Zones.newYork, relativeTo: berlin, at: berlinDate(2026, 11, 2, 12)) == "−6 Std.")
        #expect(TimeZoneDisplay.offsetLabel(berlin, relativeTo: Zones.newYork, at: berlinDate(2026, 11, 2, 12)) == "+6 Std.")
        #expect(TimeZoneDisplay.offsetLabel(Zones.kolkata, relativeTo: berlin, at: berlinDate(2026, 10, 23, 12)) == "+3,5 Std.")
        #expect(TimeZoneDisplay.offsetLabel(Zones.kolkata, relativeTo: berlin, at: berlinDate(2026, 10, 26, 12)) == "+4,5 Std.")
        #expect(TimeZoneDisplay.offsetLabel(Zones.kolkata, relativeTo: Zones.london, at: berlinDate(2026, 12, 1, 12)) == "+5,5 Std.")
        #expect(TimeZoneDisplay.offsetLabel(Zones.kathmandu, relativeTo: Zones.london, at: berlinDate(2026, 12, 1, 12)) == "+5,75 Std.")
        #expect(TimeZoneDisplay.offsetLabel(Zones.paris, relativeTo: berlin, at: berlinDate(2026, 10, 23, 12)) == "±0")
    }

    @Test("Stundenleiste an einem normalen Tag")
    func hoursNormalDay() {
        let day = LocalDate(year: 2026, month: 10, day: 23)
        #expect(TimeZoneDisplay.hourLabels(for: Zones.newYork, home: berlin, day: day) == Zones.hours(from: 18, count: 24))
        let kolkata = TimeZoneDisplay.hourLabels(for: Zones.kolkata, home: berlin, day: day)
        #expect(kolkata.count == 24)
        #expect(kolkata.first == "03:30")
        #expect(kolkata.last == "02:30")
        #expect(TimeZoneDisplay.hourLabels(for: berlin, home: berlin, day: day) == Zones.hours(from: 0, count: 24))
    }

    @Test("Heimat-Tag mit 25 Stunden (Europa, 25.10.2026)")
    func hoursLongDay() {
        let day = LocalDate(year: 2026, month: 10, day: 25)
        let labels = TimeZoneDisplay.hourLabels(for: Zones.newYork, home: berlin, day: day)
        // 18 Uhr New York bis 18 Uhr: 25 echte Stunden, New York stellt erst eine Woche später um.
        #expect(labels == Zones.hours(from: 18, count: 25))
        // Berlin selbst: die 2 doppelt.
        #expect(TimeZoneDisplay.hourLabels(for: berlin, home: berlin, day: day)
            == ["00", "01", "02", "02"] + Zones.hours(from: 3, count: 21))
    }

    @Test("Heimat-Tag mit 23 Stunden (Europa, 28.3.2027)")
    func hoursShortDay() {
        let day = LocalDate(year: 2027, month: 3, day: 28)
        let labels = TimeZoneDisplay.hourLabels(for: Zones.newYork, home: berlin, day: day)
        // USA ist seit dem 14.3. in Sommerzeit: 0 Uhr Berlin (MEZ) = 19 Uhr New York.
        #expect(labels == Zones.hours(from: 19, count: 23))
        #expect(TimeZoneDisplay.hourLabels(for: berlin, home: berlin, day: day)
            == ["00", "01"] + Zones.hours(from: 3, count: 21))
    }

    @Test("Heimat New York am 1.11.2026 (25 Stunden)")
    func hoursUSChange() {
        let day = LocalDate(year: 2026, month: 11, day: 1)
        let labels = TimeZoneDisplay.hourLabels(for: berlin, home: Zones.newYork, day: day)
        #expect(labels == Zones.hours(from: 5, count: 25))
        let kolkata = TimeZoneDisplay.hourLabels(for: Zones.kolkata, home: Zones.newYork, day: day)
        #expect(kolkata.count == 25)
        #expect(kolkata.first == "09:30")
        #expect(kolkata.last == "09:30")
    }

    @Test("Uhrzeit in zweiter Zone mit Tageswechsel")
    func describe() {
        #expect(TimeZoneDisplay.describe(berlinDate(2026, 10, 23, 14), in: Zones.newYork, home: berlin) == "14:00 (8:00 New York)")
        #expect(TimeZoneDisplay.describe(berlinDate(2026, 10, 23, 3), in: Zones.newYork, home: berlin) == "3:00 (21:00 New York, Vortag)")
        #expect(TimeZoneDisplay.describe(berlinDate(2026, 10, 23, 22, 15), in: Zones.kolkata, home: berlin) == "22:15 (1:45 Kolkata, Folgetag)")
        #expect(TimeZoneDisplay.describe(berlinDate(2026, 10, 28, 14), in: Zones.newYork, home: berlin) == "14:00 (9:00 New York)")
        let nyc = ZoneClock(timeZone: Zones.newYork, label: "NYC")
        #expect(TimeZoneDisplay.describe(berlinDate(2026, 11, 2, 9, 5), in: nyc, home: berlin) == "9:05 (3:05 NYC)")
        #expect(TimeZoneDisplay.describe(berlinDate(2026, 10, 23, 14), in: Zones.paris, home: berlin) == "14:00")
    }

    @Test("Hinweis auf die Original-Zone eines Termins")
    func originalZone() {
        let day = LocalDate(year: 2026, month: 10, day: 27)
        let nine = LocalDateTime(date: day, hour: 9, minute: 0)
        let la = Zones.details(start: .dateTime(nine, timeZone: "America/Los_Angeles"))
        #expect(TimeZoneDisplay.originalZoneHint(for: la, home: berlin) == "9:00 in Los Angeles")
        let kolkata = Zones.details(start: .dateTime(LocalDateTime(date: day, hour: 16, minute: 30), timeZone: "Asia/Kolkata"))
        #expect(TimeZoneDisplay.originalZoneHint(for: kolkata, home: berlin) == "16:30 in Kolkata")
        #expect(TimeZoneDisplay.originalZoneHint(for: Zones.details(start: .dateTime(nine, timeZone: "Europe/Paris")), home: berlin) == nil)
        #expect(TimeZoneDisplay.originalZoneHint(for: Zones.details(start: .dateTime(nine, timeZone: "Europe/Berlin")), home: berlin) == nil)
        #expect(TimeZoneDisplay.originalZoneHint(for: Zones.details(start: .dateTime(nine, timeZone: nil)), home: berlin) == nil)
        #expect(TimeZoneDisplay.originalZoneHint(for: Zones.details(start: .date(day)), home: berlin) == nil)
        // London und Berlin: im Sommer wie im Winter eine Stunde Abstand.
        let london = Zones.details(start: .dateTime(nine, timeZone: "Europe/London"))
        #expect(TimeZoneDisplay.originalZoneHint(for: london, home: berlin) == "9:00 in London")
    }
}
