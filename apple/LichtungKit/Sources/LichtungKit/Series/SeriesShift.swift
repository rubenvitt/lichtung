import Foundation

/// RDATEs und EXDATEs, wenn eine Serie zeitlich wandert (LIC-48).
///
/// Der Server verschiebt sie bei einer Zeitänderung selbst (`shift_rid` in
/// `lichtung-ical`), ersetzt sie aber durch die Liste, die mit einer neuen
/// Regel mitkommt. Wer Regel und Zeit zugleich ändert, muss sie deshalb
/// genauso verschieben, sonst taucht ein ausgelassenes Vorkommen wieder auf.
extension Recurrence {
    /// RDATEs und EXDATEs, wenn der Serienbeginn von `old` nach `new` wandert:
    /// gleicher Abstand in Wanduhrzeit, Form (Tag/Zeitpunkt) und Zone des
    /// neuen Beginns. Die Regel bleibt, wie sie ist.
    public func movingDates(from old: DateValue, to new: DateValue, deviceTimeZone tz: TimeZone) -> Recurrence {
        guard old.normalized != new.normalized else { return self }
        var r = self
        r.rdates = rdates.map { SeriesShift.shift($0, from: old, to: new, deviceTimeZone: tz) }
        r.exdates = exdates.map { SeriesShift.shift($0, from: old, to: new, deviceTimeZone: tz) }
        return r
    }

    /// Nur die RDATEs und EXDATEs ab `occurrence`: was die neue Serie bei
    /// „dieser und folgende“ übernimmt. Tage zählen ganz.
    public func keepingDates(from occurrence: DateValue, deviceTimeZone tz: TimeZone) -> Recurrence {
        var r = self
        let atOrAfter = { (v: DateValue) in SeriesShift.isAtOrAfter(v, occurrence, deviceTimeZone: tz) }
        r.rdates = rdates.filter(atOrAfter)
        r.exdates = exdates.filter(atOrAfter)
        return r
    }
}

enum SeriesShift {
    /// Ein Datum der Serie (RDATE, EXDATE, Vorkommen), wenn der Beginn von
    /// `old` nach `new` wandert. Wie im Server: Ein Tag bekommt die Uhrzeit
    /// des alten Beginns, ein Zeitpunkt wird in dessen Zone gelesen; ein
    /// UTC-Wert bleibt UTC.
    static func shift(_ v: DateValue, from old: DateValue, to new: DateValue, deviceTimeZone tz: TimeZone) -> DateValue {
        let oldWall = wall(old)
        let oldZone = old.timeZoneID.flatMap(TimeZone.init(identifier:)) ?? tz
        let start: LocalDateTime
        switch v {
        case let .date(d):
            start = LocalDateTime(date: d, hour: oldWall.hour, minute: oldWall.minute, second: oldWall.second)
        case let .dateTime(local, zone, utc):
            if zone == old.timeZoneID, zone != nil || utc == nil {
                start = local
            } else {
                start = LocalDateTime(utc ?? v.instant(deviceTimeZone: tz), calendar: .lichtung(timeZone: oldZone))
            }
        }
        let moved = start.addingSeconds(wall(new).secondsSince(oldWall))
        switch new {
        case .date:
            return .date(moved.date)
        case let .dateTime(_, newZoneID, _):
            if v.timeZoneID == "UTC", newZoneID != "UTC" {
                let newZone = newZoneID.flatMap(TimeZone.init(identifier:)) ?? tz
                let instant = moved.instant(in: .lichtung(timeZone: newZone))
                return .dateTime(LocalDateTime(instant, calendar: .gregorianUTC), timeZone: "UTC")
            }
            return .dateTime(moved, timeZone: newZoneID)
        }
    }

    /// Liegt `v` am oder nach `occurrence`? Bei Tagen zählt der Tag.
    static func isAtOrAfter(_ v: DateValue, _ occurrence: DateValue, deviceTimeZone tz: TimeZone) -> Bool {
        if v.isAllDay || occurrence.isAllDay { return wall(v).date >= wall(occurrence).date }
        let instant = { (x: DateValue) -> Date in
            if case let .dateTime(_, zone, utc?) = x, zone == nil { return utc }
            return x.instant(deviceTimeZone: tz)
        }
        return instant(v) >= instant(occurrence)
    }

    /// Wanduhrzeit eines Datumswerts; ein Tag beginnt um 0 Uhr.
    static func wall(_ v: DateValue) -> LocalDateTime {
        switch v {
        case let .date(d): LocalDateTime(date: d, hour: 0, minute: 0)
        case let .dateTime(l, _, _): l
        }
    }
}
