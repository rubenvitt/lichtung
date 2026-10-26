import Foundation

/// Lage eines Zeitraums in einer Tagesspalte, als Anteil der Spaltenhöhe.
public struct GridFrame: Hashable, Sendable {
    public var top: Double
    public var height: Double

    public init(top: Double, height: Double) {
        self.top = top
        self.height = height
    }
}

/// Rechenschicht für das Ziehen im Zeitraster (LIC-22): anlegen, verschieben,
/// Länge ändern. Die Oberfläche liefert Punkte in der Spalte, hier wird daraus
/// Zeit, eingerastet auf 15 Minuten.
///
/// Alles gilt auf der Wanduhr-Achse (`WallClockAxis`), damit Vorschau und
/// Ergebnis an Umstellungstagen dort liegen, wo der Block gezeichnet wird.
/// Verschoben wird um ganze 15-Minuten-Schritte; ein Termin um 9:05 landet
/// also auf 9:20, nicht auf 9:15. Die Dauer bleibt beim Verschieben in echter
/// Zeit erhalten (eine Stunde bleibt eine Stunde, auch über die Umstellung).
public enum GridInteraction {
    /// Raster beim Ziehen in Minuten.
    public static let snapMinutes = 15
    /// Kürzester Termin beim Anlegen und Länge-Ändern.
    public static let minimumMinutes = 15

    // MARK: - Punkte und Minuten

    /// Lage `y` in einer Spalte der Höhe `columnHeight` als Minuten der Achse, 0…1440.
    public static func minutes(atY y: Double, columnHeight: Double) -> Double {
        guard columnHeight > 0 else { return 0 }
        return min(WallClockAxis.minutesPerDay, max(0, y / columnHeight * WallClockAxis.minutesPerDay))
    }

    /// Senkrechte Strecke in Minuten der Achse (für Verschieben und Länge-Ändern).
    public static func minutes(forDistance dy: Double, columnHeight: Double) -> Double {
        guard columnHeight > 0 else { return 0 }
        return dy / columnHeight * WallClockAxis.minutesPerDay
    }

    /// Auf das Raster gerundet.
    public static func snapped(_ minutes: Double, rule: FloatingPointRoundingRule = .toNearestOrAwayFromZero) -> Double {
        let step = Double(snapMinutes)
        return (minutes / step).rounded(rule) * step
    }

    /// Wie viele Spalten weit ein waagrechter Zug reicht, begrenzt auf die
    /// sichtbaren Tage (`column` ist die Spalte, in der der Zug beginnt).
    public static func dayOffset(forDistance dx: Double, columnWidth: Double, column: Int, columns: Int) -> Int {
        guard columnWidth > 0, columns > 0 else { return 0 }
        let raw = Int((dx / columnWidth).rounded())
        return min(columns - 1 - column, max(-column, raw))
    }

    // MARK: - Was sich ziehen lässt

    /// Ganztägige Termine und Termine in schreibgeschützten (oder unbekannten)
    /// Kalendern lassen sich nicht ziehen.
    public static func isDraggable(_ occ: Occurrence, calendar: CalendarInfo?) -> Bool {
        !occ.isAllDay && (calendar?.writable ?? false)
    }

    // MARK: - Anlegen

    /// Zug in leerer Fläche von `fromY` nach `toY` (beliebige Richtung):
    /// Beginn auf das Raster abgerundet, Ende aufgerundet, mindestens
    /// `minimumMinutes`. Bleibt im Tag der Spalte.
    public static func creation(day: LocalDate, fromY: Double, toY: Double, columnHeight: Double, calendar: Calendar) -> DateInterval {
        let a = minutes(atY: min(fromY, toY), columnHeight: columnHeight)
        let b = minutes(atY: max(fromY, toY), columnHeight: columnHeight)
        var start = snapped(a, rule: .down)
        var end = max(snapped(b, rule: .up), start + Double(minimumMinutes))
        if end > WallClockAxis.minutesPerDay {
            end = WallClockAxis.minutesPerDay
            start = end - Double(minimumMinutes)
        }
        let axis = WallClockAxis(day: day, calendar: calendar)
        let s = axis.instant(atMinutes: start)
        // In der Lücke beim Vorstellen fallen Beginn und Ende sonst zusammen.
        let e = max(axis.instant(atMinutes: end), s.addingTimeInterval(Double(minimumMinutes) * 60))
        return interval(s, e)
    }

    /// Beginn für „Neuer Termin hier“ (Kontextmenü, LIC-24) an der Stelle `y`
    /// der Spalte: Wanduhrzeit auf das Raster abgerundet, spätestens 23:00,
    /// damit eine Stunde in den Tag passt. Ohne `y` 9 Uhr.
    public static func slotStart(day: LocalDate, atY y: Double?, columnHeight: Double) -> LocalDateTime {
        guard let y else { return LocalDateTime(date: day, hour: 9, minute: 0) }
        let raw = snapped(minutes(atY: y, columnHeight: columnHeight), rule: .down)
        let m = Int(min(raw, WallClockAxis.minutesPerDay - 60))
        return LocalDateTime(date: day, hour: m / 60, minute: m % 60)
    }

    // MARK: - Verschieben

    /// Termin um `deltaMinutes` (Achse, wird auf ganze Raster-Schritte
    /// gerundet) und `dayOffset` Tage verschieben. Über Mitternacht hinaus
    /// wandert er in den Nachbartag. Die Dauer bleibt.
    public static func moved(start: Date, end: Date, deltaMinutes: Double, dayOffset: Int, calendar: Calendar) -> DateInterval {
        let step = snapped(deltaMinutes)
        let duration = max(0, end.timeIntervalSince(start))
        guard step != 0 || dayOffset != 0 else { return interval(start, end) }
        let startDay = LocalDate(start, calendar: calendar)
        var m = WallClockAxis(day: startDay, calendar: calendar).minutes(at: start) + step
        var carry = 0
        while m < 0 { m += WallClockAxis.minutesPerDay; carry -= 1 }
        while m >= WallClockAxis.minutesPerDay { m -= WallClockAxis.minutesPerDay; carry += 1 }
        let target = WallClockAxis(day: startDay.adding(days: dayOffset + carry), calendar: calendar)
        let newStart = target.instant(atMinutes: m)
        return interval(newStart, newStart.addingTimeInterval(duration))
    }

    // MARK: - Länge ändern

    /// Unteren Rand um `deltaMinutes` (gerundet) ziehen. Das Ende bleibt im
    /// Tag `day` (der Spalte mit dem Griff), mindestens `minimumMinutes` nach
    /// dem Beginn.
    public static func resized(start: Date, end: Date, day: LocalDate, deltaMinutes: Double, calendar: Calendar) -> DateInterval {
        let step = snapped(deltaMinutes)
        guard step != 0 else { return interval(start, end) }
        let axis = WallClockAxis(day: day, calendar: calendar)
        let floor = axis.minutes(at: start) + Double(minimumMinutes)
        let m = min(WallClockAxis.minutesPerDay, max(floor, axis.minutes(at: end) + step))
        var newEnd = axis.instant(atMinutes: m)
        // In der Lücke beim Vorstellen kann das Ende auf den Beginn fallen.
        newEnd = max(newEnd, start.addingTimeInterval(Double(minimumMinutes) * 60))
        return interval(start, newEnd)
    }

    // MARK: - Vorschau

    /// Lage von `interval` in der Spalte `day`, abgeschnitten an den
    /// Tagesgrenzen; `nil`, wenn nichts davon im Tag liegt.
    public static func frame(of interval: DateInterval, day: LocalDate, calendar: Calendar) -> GridFrame? {
        let axis = WallClockAxis(day: day, calendar: calendar)
        guard interval.start < axis.dayEnd, interval.end > axis.dayStart || (interval.duration == 0 && interval.start >= axis.dayStart)
        else { return nil }
        let top = axis.fraction(at: interval.start)
        return GridFrame(top: top, height: max(0, axis.fraction(at: interval.end) - top))
    }

    /// „9:15–10:30“ für die Vorschau beim Ziehen.
    public static func timeText(_ interval: DateInterval, calendar: Calendar) -> String {
        func text(_ d: Date) -> String {
            let c = calendar.dateComponents([.hour, .minute], from: d)
            return "\(c.hour!):" + String(format: "%02d", c.minute!)
        }
        return text(interval.start) + "–" + text(interval.end)
    }

    // MARK: - Änderungen

    /// Geltungsbereiche, nach denen vor dem Verschieben gefragt wird (LIC-47):
    /// bei Serien „Nur dieser“ und „Dieser und alle folgenden“, sonst keine.
    public static func moveScopes(_ occ: Occurrence) -> [EditScope] {
        occ.recurrenceId == nil ? [] : [.this, .thisAndFollowing]
    }

    /// Verschieben oder Länge ändern als Absicht für `SyncEngine.perform`.
    ///
    /// Bei Serien gilt die Änderung für dieses Vorkommen (`scope: .this`) oder
    /// ab hier (`.thisAndFollowing`), jeweils mit der ursprünglichen
    /// `recurrenceId`. Die Zeitzone des Termins bleibt; floating bleibt
    /// floating. `nil` bei ganztägigen Terminen und bei einem Bereich, den
    /// `moveScopes` nicht anbietet; Einzeltermine gelten immer ganz.
    public static func moveIntent(_ occ: Occurrence, to new: DateInterval, scope: EditScope = .this, deviceTimeZone: TimeZone) -> EditIntent? {
        guard !occ.isAllDay else { return nil }
        let changes = EventChanges(
            start: dateValue(new.start, like: occ.details.start, deviceTimeZone: deviceTimeZone),
            end: dateValue(new.end, like: occ.details.end, deviceTimeZone: deviceTimeZone)
        )
        if let recurrenceId = occ.recurrenceId {
            guard moveScopes(occ).contains(scope) else { return nil }
            return .update(eventId: occ.eventId, scope: scope, occurrence: recurrenceId, changes: changes)
        }
        return .update(eventId: occ.eventId, scope: .all, changes: changes)
    }

    // MARK: - Bedienhilfen

    /// Schritt der Bedienhilfen-Aktionen „15 Minuten früher/später“ (LIC-47).
    public static let nudgeMinutes = snapMinutes

    /// Aktionen für VoiceOver am Block: Name und Minuten.
    public static let nudgeActions: [(name: String, minutes: Int)] = [
        ("\(nudgeMinutes) Minuten früher", -nudgeMinutes),
        ("\(nudgeMinutes) Minuten später", nudgeMinutes),
    ]

    /// Termin um `minutes` verschoben (wie Ziehen, Dauer bleibt).
    public static func nudged(start: Date, end: Date, minutes: Int, calendar: Calendar) -> DateInterval {
        moved(start: start, end: end, deltaMinutes: Double(minutes), dayOffset: 0, calendar: calendar)
    }

    // MARK: - Automatisch scrollen

    /// Randzone oben und unten im sichtbaren Ausschnitt, in der beim Ziehen
    /// gescrollt wird (Punkte).
    public static let autoScrollEdge: Double = 44
    /// Größter Schritt pro Takt (Punkte, etwa 60 Takte pro Sekunde).
    public static let autoScrollMaxStep: Double = 16

    /// Schritt pro Takt, wenn der Zeiger beim Ziehen bei `pointerY` steht
    /// (im sichtbaren Ausschnitt der Höhe `viewportHeight`, oben = 0):
    /// negativ nach oben, positiv nach unten, 0 außerhalb der Randzonen. Je
    /// tiefer im Rand (oder darüber hinaus), desto schneller. Bei kleinen
    /// Ausschnitten wird die Zone schmaler (höchstens ein Viertel).
    public static func autoScrollStep(pointerY: Double, viewportHeight: Double) -> Double {
        guard viewportHeight > 0 else { return 0 }
        let edge = min(autoScrollEdge, viewportHeight / 4)
        let depth: Double
        if pointerY < edge {
            depth = -(edge - pointerY) / edge
        } else if pointerY > viewportHeight - edge {
            depth = (pointerY - (viewportHeight - edge)) / edge
        } else {
            return 0
        }
        return autoScrollMaxStep * min(1, max(-1, depth))
    }

    /// Versatz nach einem Schritt, begrenzt auf den Inhalt
    /// (0 … `contentHeight − viewportHeight`).
    public static func autoScrolledOffset(_ offset: Double, step: Double, contentHeight: Double, viewportHeight: Double) -> Double {
        min(max(0, contentHeight - viewportHeight), max(0, offset + step))
    }

    /// Neuer Termin aus einem Zug in leerer Fläche, in der Zone des Geräts.
    public static func createIntent(calendarId: String, interval: DateInterval, timeZone: TimeZone, title: String = "Neuer Termin") -> EditIntent {
        let cal = Calendar.lichtung(timeZone: timeZone)
        let input = EventInput(
            title: title,
            start: .dateTime(LocalDateTime(interval.start, calendar: cal), timeZone: timeZone.identifier),
            end: .dateTime(LocalDateTime(interval.end, calendar: cal), timeZone: timeZone.identifier)
        )
        return .create(calendarId: calendarId, event: input)
    }

    /// `instant` als Wanduhrzeit in der Zone von `original`.
    static func dateValue(_ instant: Date, like original: DateValue, deviceTimeZone: TimeZone) -> DateValue {
        switch original {
        case let .dateTime(_, tzID, _):
            let zone = tzID.flatMap(TimeZone.init(identifier:)) ?? deviceTimeZone
            return .dateTime(LocalDateTime(instant, calendar: .lichtung(timeZone: zone)), timeZone: tzID)
        case .date:
            return .dateTime(LocalDateTime(instant, calendar: .lichtung(timeZone: deviceTimeZone)), timeZone: deviceTimeZone.identifier)
        }
    }

    private static func interval(_ a: Date, _ b: Date) -> DateInterval {
        DateInterval(start: a, end: max(a, b))
    }
}
