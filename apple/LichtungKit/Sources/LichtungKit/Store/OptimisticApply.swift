import Foundation

/// Wendet eine ausstehende Mutation lokal an, damit die Anzeige sofort stimmt.
///
/// Nur eine Vorschau: Der Server bleibt die Autorität und schickt nach dem
/// Push den echten Stand. Bei „dieser und folgende“ teilt der Server die
/// Serie selbst; lokal wird das angenähert (alte Serie endet, neue beginnt).
enum OptimisticApply {
    static func apply(_ m: Mutation, to events: inout [String: Event], deviceTimeZone tz: TimeZone) {
        switch m.operation {
        case let .createEvent(calendarId, input):
            let id = SyncState.placeholderPrefix + m.id
            events[id] = Event(
                id: id, calendarId: calendarId, uid: input.uid, version: 0,
                details: EventDetails(input: input), recurrence: input.recurrence
            )

        case let .updateEvent(eventId, _, scope, occurrence, changes):
            guard var e = events[eventId] else { return }
            switch (scope, occurrence) {
            case (.this, let occ?) where e.isRecurring:
                var details = overrideDetails(of: e, at: occ, tz: tz)
                details.apply(changes)
                setOverride(&e, at: occ, details: details)
                events[eventId] = e
            case (.thisAndFollowing, let occ?) where e.isRecurring:
                var details = overrideDetails(of: e, at: occ, tz: tz)
                details.apply(changes)
                var tail = Event(id: SyncState.placeholderPrefix + m.id, calendarId: e.calendarId, version: 0, details: details)
                switch changes.recurrence {
                case let .set(r)?:
                    // Der Server übernimmt eine mitgeschickte Regel wörtlich.
                    tail.recurrence = r
                case .clear?:
                    tail.recurrence = nil
                case nil:
                    tail.recurrence = e.recurrence.map {
                        remainder($0, seriesStart: e.details.start, from: occ, movedTo: changes.start.map { _ in details.start }, tz: tz)
                    }
                }
                truncate(&e, before: occ, tz: tz)
                events[eventId] = e
                events[tail.id] = tail
            default:
                let oldStart = e.details.start
                e.details.apply(changes)
                if let r = changes.recurrence {
                    e.recurrence = r.value
                } else if changes.start != nil {
                    // Wie der Server: RDATEs und EXDATEs wandern mit der Serie (LIC-48).
                    e.recurrence = e.recurrence?.movingDates(from: oldStart, to: e.details.start, deviceTimeZone: tz)
                }
                if let cal = changes.calendarId { e.calendarId = cal }
                events[eventId] = e
            }

        case let .deleteEvent(eventId, _, scope, occurrence):
            guard var e = events[eventId] else { return }
            switch (scope, occurrence) {
            case (.this, let occ?) where e.isRecurring:
                e.recurrence?.exdates.append(occ)
                e.overrides.removeAll { $0.recurrenceId.normalized == occ.normalized }
                events[eventId] = e
            case (.thisAndFollowing, let occ?) where e.isRecurring:
                truncate(&e, before: occ, tz: tz)
                events[eventId] = e
            default:
                events[eventId] = nil
            }

        case let .respond(eventId, occurrence, status, _):
            guard var e = events[eventId] else { return }
            let s = ParticipationStatus(response: status)
            if let occ = occurrence, e.isRecurring {
                var details = overrideDetails(of: e, at: occ, tz: tz)
                details.selfStatus = s
                setOverride(&e, at: occ, details: details)
            } else {
                e.details.selfStatus = s
                for i in e.details.attendees.indices where e.details.attendees[i].isSelf == true {
                    e.details.attendees[i].status = s
                }
            }
            events[eventId] = e
        }
    }

    /// Details des Vorkommens `occ`: das vorhandene Override oder der Master,
    /// auf den Beginn des Vorkommens verschoben.
    static func overrideDetails(of e: Event, at occ: DateValue, tz: TimeZone) -> EventDetails {
        if let o = e.overrides.first(where: { $0.recurrenceId.normalized == occ.normalized }) {
            return o.details
        }
        var d = e.details
        d.end = shift(e.details.end, by: e.details.start, to: occ, tz: tz)
        d.start = occ
        return d
    }

    static func setOverride(_ e: inout Event, at occ: DateValue, details: EventDetails) {
        e.overrides.removeAll { $0.recurrenceId.normalized == occ.normalized }
        e.overrides.append(EventOverride(recurrenceId: occ, details: details))
    }

    /// Ende so verschieben, dass die Dauer gleich bleibt, wenn der Beginn von
    /// `oldStart` auf `newStart` wandert.
    static func shift(_ end: DateValue, by oldStart: DateValue, to newStart: DateValue, tz: TimeZone) -> DateValue {
        switch (end, oldStart, newStart) {
        case let (.date(e), .date(s), .date(n)):
            return .date(n.adding(days: s.days(until: e)))
        default:
            let duration = end.instant(deviceTimeZone: tz).timeIntervalSince(oldStart.instant(deviceTimeZone: tz))
            let newEnd = newStart.instant(deviceTimeZone: tz).addingTimeInterval(duration)
            let zone = end.timeZoneID ?? newStart.timeZoneID
            let cal = Calendar.lichtung(timeZone: zone.flatMap(TimeZone.init(identifier:)) ?? tz)
            return .dateTime(LocalDateTime(newEnd, calendar: cal), timeZone: zone)
        }
    }

    /// Was die neue Serie bei „dieser und folgende“ von der alten übernimmt,
    /// wie `split_series` im Server: RDATEs und EXDATEs ab `occ`, `UNTIL`
    /// bleibt, `COUNT` wird zur Restzahl. Wandert der neue Beginn (`movedTo`),
    /// wandern Daten und `UNTIL` mit.
    static func remainder(_ r: Recurrence, seriesStart: DateValue, from occ: DateValue, movedTo: DateValue?, tz: TimeZone) -> Recurrence {
        var out = r.keepingDates(from: occ, deviceTimeZone: tz)
        if let text = r.rrule, let total = RRule(text)?.count {
            let before = SeriesExpander(deviceTimeZone: tz).ruleInstances(of: text, seriesStart: seriesStart, before: occ)
            out.rrule = RRuleText.setting("COUNT", to: String(max(1, total - before)), in: text)
        }
        guard let movedTo else { return out }
        return out.movingDates(from: occ, to: movedTo, deviceTimeZone: tz)
    }

    /// Serie endet vor `occ`; Overrides ab `occ` fallen weg.
    static func truncate(_ e: inout Event, before occ: DateValue, tz: TimeZone) {
        guard var r = e.recurrence else { return }
        let cut = occ.instant(deviceTimeZone: tz)
        r.rrule = r.rrule.map { RRuleText.until($0, before: occ, deviceTimeZone: tz) }
        r.rdates.removeAll { $0.instant(deviceTimeZone: tz) >= cut }
        e.recurrence = r
        e.overrides.removeAll { $0.recurrenceId.instant(deviceTimeZone: tz) >= cut }
    }
}

/// Kleine Textoperationen auf RRULE-Werten.
enum RRuleText {
    static func removing(_ keys: Set<String>, from rrule: String) -> String {
        rrule.split(separator: ";")
            .filter { part in !keys.contains(part.split(separator: "=").first.map { $0.uppercased() } ?? "") }
            .joined(separator: ";")
    }

    /// Ersetzt den Teil `key` (ans Ende gestellt).
    static func setting(_ key: String, to value: String, in rrule: String) -> String {
        removing([key], from: rrule) + ";\(key)=" + value
    }

    /// Setzt `UNTIL` auf den Moment vor `occ` (ganztägig: den Tag davor) und entfernt `COUNT`.
    static func until(_ rrule: String, before occ: DateValue, deviceTimeZone tz: TimeZone) -> String {
        let base = removing(["COUNT", "UNTIL"], from: rrule)
        let until: String
        switch occ {
        case let .date(d):
            until = d.adding(days: -1).icalString
        case .dateTime:
            until = utcStamp(occ.instant(deviceTimeZone: tz).addingTimeInterval(-1))
        }
        return base + ";UNTIL=" + until
    }

    /// `20261116T075959Z`.
    static func utcStamp(_ instant: Date) -> String {
        let c = Calendar.gregorianUTC.dateComponents([.year, .month, .day, .hour, .minute, .second], from: instant)
        return String(format: "%04d%02d%02dT%02d%02d%02dZ", c.year!, c.month!, c.day!, c.hour!, c.minute!, c.second!)
    }
}

extension ParticipationStatus {
    init(response: InvitationResponse) {
        switch response {
        case .accepted: self = .accepted
        case .tentative: self = .tentative
        case .declined: self = .declined
        }
    }
}

extension EventDetails {
    init(input: EventInput) {
        self.init(
            title: input.title, location: input.location, notes: input.notes, url: input.url,
            start: input.start, end: input.end, status: input.status ?? .confirmed,
            availability: input.availability ?? .busy,
            attendees: (input.attendees ?? []).map { Attendee(email: $0.email, name: $0.name, role: $0.role, status: .needsAction) },
            alarms: input.alarms ?? [], travelTimeMinutes: input.travelTimeMinutes
        )
    }

    /// Übernimmt eine Teiländerung.
    public mutating func apply(_ c: EventChanges) {
        if let v = c.title { title = v }
        if let p = c.location { location = p.value }
        if let p = c.notes { notes = p.value }
        if let p = c.url { url = p.value }
        if let v = c.start { start = v; allDay = v.isAllDay }
        if let v = c.end { end = v }
        if let v = c.status { status = v }
        if let v = c.availability { availability = v }
        if let v = c.attendees {
            attendees = v.map { a in
                attendees.first { $0.email.lowercased() == a.email.lowercased() }
                    ?? Attendee(email: a.email, name: a.name, role: a.role, status: .needsAction)
            }
        }
        if let v = c.alarms { alarms = v }
        if let p = c.travelTimeMinutes { travelTimeMinutes = p.value }
    }
}
