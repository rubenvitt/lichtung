import Foundation

/// RDATEs und EXDATEs, wenn eine Serie zeitlich wandert (LIC-48).
///
/// Der Server verschiebt sie bei einer Zeitänderung selbst (`shift_rid` in
/// `lichtung-ical`), ersetzt sie aber durch die Liste, die mit einer neuen
/// Regel mitkommt. Wer Regel und Zeit zugleich ändert, muss sie deshalb
/// genauso verschieben, sonst taucht ein ausgelassenes Vorkommen wieder auf.
extension Recurrence {
    /// RDATEs, EXDATEs und `UNTIL`, wenn der Serienbeginn von `old` nach
    /// `new` wandert: gleicher Abstand in Wanduhrzeit, Form (Tag/Zeitpunkt)
    /// und Zone des neuen Beginns. Der Rest der Regel bleibt (LIC-52).
    public func movingDates(from old: DateValue, to new: DateValue, deviceTimeZone tz: TimeZone) -> Recurrence {
        guard old.normalized != new.normalized else { return self }
        var r = self
        r.rdates = rdates.map { SeriesShift.shift($0, from: old, to: new, deviceTimeZone: tz) }
        r.exdates = exdates.map { SeriesShift.shift($0, from: old, to: new, deviceTimeZone: tz) }
        if let text = rrule, let until = RRule(text)?.until {
            let moved = SeriesShift.shiftUntil(until, from: old, to: new, deviceTimeZone: tz)
            r.rrule = RRuleText.setting("UNTIL", to: moved, in: text)
        }
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

    /// `UNTIL` als RRULE-Text, wenn der Beginn von `old` nach `new` wandert.
    /// Wie `shift_until` im Server: Ein Tag bekommt die Uhrzeit des alten
    /// Beginns, UTC wird in dessen Zone gelesen. Ergebnis ist ein Tag bei
    /// ganztägigen Serien, eine Wanduhrzeit bei floating, sonst UTC (ein Tag
    /// gilt dann bis 23:59:59 in der Zone des neuen Beginns).
    static func shiftUntil(_ until: RRule.Until, from old: DateValue, to new: DateValue, deviceTimeZone tz: TimeZone) -> String {
        let oldWall = wall(old)
        let oldZone = old.timeZoneID.flatMap(TimeZone.init(identifier:)) ?? tz
        let start: LocalDateTime = switch until {
        case let .date(d): LocalDateTime(date: d, hour: oldWall.hour, minute: oldWall.minute, second: oldWall.second)
        case let .utc(t): LocalDateTime(t, calendar: .lichtung(timeZone: oldZone))
        case let .local(l): l
        }
        var moved = start.addingSeconds(wall(new).secondsSince(oldWall))
        switch new {
        case .date:
            return moved.date.icalString
        case let .dateTime(_, zoneID, _):
            let isUTC = if case .utc = until { true } else { false }
            if zoneID == nil, !isUTC { return moved.icalString }
            if case .date = until { moved = LocalDateTime(date: moved.date, hour: 23, minute: 59, second: 59) }
            let zone = zoneID.flatMap(TimeZone.init(identifier:)) ?? tz
            return RRuleText.utcStamp(moved.instant(in: .lichtung(timeZone: zone)))
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

extension LocalDateTime {
    /// `20261103T140000` (Wanduhrzeit ohne Zone, wie in `UNTIL`).
    var icalString: String {
        date.icalString + String(format: "T%02d%02d%02d", hour, minute, second)
    }
}
