import Foundation

// Terminanfragen (Spec §5, §1 Kriterium 4; LIC-30): gesammelte Liste mit
// Badge, Antworten „Annehmen“, „Vielleicht“, „Ablehnen“ in Liste, Detail
// und Kontextmenü. Geschrieben wird über `respond` (Vertrag §6).

/// Eine offene Terminanfrage in der gesammelten Liste.
///
/// Wie `list_invitations` des Servers (MCP): Eine Serie erscheint einmal,
/// mit ihrem nächsten offenen Vorkommen und der Zahl offener Vorkommen; die
/// Antwort gilt dann für alle Vorkommen (`occurrence: null`). Ein einzeln
/// geändertes Vorkommen mit eigener offener Anfrage erscheint für sich und
/// wird nur für sich beantwortet.
public struct Invitation: Hashable, Sendable, Identifiable {
    /// Worauf sich eine Antwort aus der Liste bezieht.
    public enum Target: Hashable, Sendable {
        /// Einzeltermin oder alle Vorkommen einer Serie.
        case event
        /// Nur dieses geänderte Vorkommen (ursprünglicher Beginn).
        case occurrence(DateValue)
    }

    /// Einzeltermin, nächstes offenes Vorkommen der Serie oder das geänderte Vorkommen.
    public var occurrence: Occurrence
    public var target: Target
    /// Offene Vorkommen im Zeitraum der Liste (Einzeltermin: 1).
    public var pendingCount: Int
    /// Kalender beschreibbar. Sonst (z. B. ICS-Abo) steht die Anfrage nur zur Info da.
    public var canRespond: Bool
    public var calendarName: String?

    public init(occurrence: Occurrence, target: Target, pendingCount: Int = 1, canRespond: Bool, calendarName: String? = nil) {
        self.occurrence = occurrence
        self.target = target
        self.pendingCount = pendingCount
        self.canRespond = canRespond
        self.calendarName = calendarName
    }

    public var id: String {
        switch target {
        case .event: occurrence.eventId
        case .occurrence: occurrence.id
        }
    }

    /// Die Antwort als Änderung (über `SyncEngine.perform`, Rückgängig geht).
    public func intent(_ response: InvitationResponse) -> EditIntent {
        switch target {
        case .event: .respond(eventId: occurrence.eventId, status: response)
        case let .occurrence(recurrenceId): .respond(eventId: occurrence.eventId, occurrence: recurrenceId, status: response)
        }
    }

    /// „Von Anna Beispiel“ (sonst die Adresse), `nil` ohne Organisator.
    public var organizerText: String? {
        guard let o = occurrence.details.organizer else { return nil }
        let name = o.name?.trimmingCharacters(in: .whitespaces) ?? ""
        return "Von \(name.isEmpty ? o.email : name)"
    }

    /// „Von Anna Beispiel · Arbeit“: Organisator und Kalender, was davon bekannt ist.
    public var sourceText: String? {
        let parts = [organizerText, calendarName].compactMap { $0 }.filter { !$0.isEmpty }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// Hinweis zu Serien: „Serie · Antwort gilt für alle 4 Termine“,
    /// „Einzelner Termin einer Serie“; `nil` bei Einzelterminen.
    public var seriesText: String? {
        switch target {
        case .occurrence:
            return "Einzelner Termin einer Serie"
        case .event:
            guard occurrence.recurrenceId != nil else { return nil }
            return pendingCount > 1 ? "Serie · Antwort gilt für alle \(pendingCount) Termine" : "Serie · Antwort gilt für alle Termine"
        }
    }

    /// Zeile unter dem Titel wie in der Suche: „Heute · Fr 23. · 09:00–09:30“.
    public func dateText(today: LocalDate, calendar cal: Calendar) -> String {
        let day = LocalDate(occurrence.start, calendar: cal)
        let item = AgendaBuilder.item(occurrence, dayStart: day.startDate(in: cal), dayEnd: day.adding(days: 1).startDate(in: cal), cal: cal)
        return "\(AgendaBuilder.heading(day, today: today)) · \(item.timeText)"
    }

    /// Hinweis statt der Knöpfe, wenn der Kalender nur lesbar ist.
    public static let readOnlyHint = "Nur lesbarer Kalender: Antworten geht hier nicht."
}

/// Alle offenen Terminanfragen, sortiert nach Beginn.
public struct InvitationList: Hashable, Sendable {
    public struct Day: Hashable, Sendable, Identifiable {
        public var date: LocalDate
        /// „Heute · Fr 23.“, „Di 3. Nov.“ (wie Ansicht A).
        public var heading: String
        public var items: [Invitation]

        public var id: LocalDate { date }
    }

    public var items: [Invitation]

    public init(items: [Invitation]) {
        self.items = items
    }

    /// Zahl am Knopf: nur Anfragen, auf die sich antworten lässt (eine nur
    /// lesbare Anfrage ließe die Zahl sonst nie verschwinden).
    public var badgeCount: Int { items.filter(\.canRespond).count }

    public var isEmpty: Bool { items.isEmpty }

    /// Nach dem Tag des (nächsten offenen) Vorkommens gruppiert.
    public func days(today: LocalDate, calendar cal: Calendar) -> [Day] {
        var result: [Day] = []
        for item in items {
            let date = LocalDate(item.occurrence.start, calendar: cal)
            if result.last?.date == date {
                result[result.count - 1].items.append(item)
            } else {
                result.append(Day(date: date, heading: AgendaBuilder.heading(date, today: today), items: [item]))
            }
        }
        return result
    }
}

extension CalendarQuery {
    /// Wie weit die Liste nach vorn schaut (wie `list_invitations`: 180 Tage).
    public static let invitationHorizon: TimeInterval = 180 * 86_400

    /// Offene Terminanfragen ab `now`: eigene Antwort `needs_action`, nicht
    /// abgesagt, nicht vorbei, nicht in ausgeblendeten Kalendern. Eine Serie
    /// erscheint einmal, ein geändertes Vorkommen mit eigener Anfrage für sich.
    public func invitations(now: Date, horizon: TimeInterval = CalendarQuery.invitationHorizon) -> InvitationList {
        let window = DateInterval(start: now, duration: horizon)
        var groups: [String: Invitation] = [:]
        for e in events.values where calendars[e.calendarId]?.hidden != true {
            guard e.isPendingInvitation || e.overrides.contains(where: { $0.details.selfStatus == .needsAction }) else { continue }
            let calendar = calendars[e.calendarId]
            for o in expander.occurrences(of: e, in: window) where o.details.selfStatus == .needsAction && o.end > now {
                let own: DateValue? = o.isModified ? o.recurrenceId : nil
                let key = own == nil ? e.id : o.id
                if var g = groups[key] {
                    g.pendingCount += 1
                    if o.start < g.occurrence.start { g.occurrence = o }
                    groups[key] = g
                } else {
                    groups[key] = Invitation(
                        occurrence: o, target: own.map { .occurrence($0) } ?? .event,
                        canRespond: calendar?.writable == true, calendarName: calendar?.name
                    )
                }
            }
        }
        let items = groups.values.sorted { a, b in
            if a.occurrence.start != b.occurrence.start { return a.occurrence.start < b.occurrence.start }
            return CalendarQuery.displayOrder(a.occurrence, b.occurrence)
        }
        return InvitationList(items: items)
    }
}

/// Antworten an einem angezeigten Termin (Detail, Kontextmenü).
public enum InvitationActions {
    /// Ich bin eingeladen (und nicht selbst Organisator): Der Server setzt
    /// dann `self_status`.
    public static func isInvitation(_ occ: Occurrence) -> Bool {
        occ.details.selfStatus != nil
    }

    /// Antworten geht: eingeladen, nicht abgesagt, Kalender beschreibbar
    /// (ICS-Abos und andere nur lesbare Kalender lehnt der Server ab).
    public static func canRespond(_ occ: Occurrence, calendars: [String: CalendarInfo]) -> Bool {
        isInvitation(occ) && occ.details.status != .cancelled && calendars[occ.calendarId]?.writable == true
    }

    /// Geltungsbereiche, zwischen denen bei Serien gewählt wird; leer heißt:
    /// Einzeltermin, ohne Rückfrage. `respond` kennt kein „dieser und folgende“.
    public static func scopes(_ occ: Occurrence) -> [EditScope] {
        occ.recurrenceId != nil ? [.this, .all] : []
    }

    /// Antwort für dieses Vorkommen (`.this`) oder den ganzen Termin bzw.
    /// alle Vorkommen der Serie (`.all`, `occurrence: null`).
    public static func intent(_ occ: Occurrence, response: InvitationResponse, scope: EditScope = .all) -> EditIntent {
        guard let recurrenceId = occ.recurrenceId, scope == .this else {
            return .respond(eventId: occ.eventId, status: response)
        }
        return .respond(eventId: occ.eventId, occurrence: recurrenceId, status: response)
    }

    /// Aktueller eigener Status des Vorkommens im Stand `events` (sichtbarer
    /// Stand samt Outbox), damit ein offenes Detail-Blatt die Antwort sofort
    /// zeigt. Ohne Event der Status des Vorkommens selbst.
    public static func currentStatus(of occ: Occurrence, in events: [String: Event]) -> ParticipationStatus? {
        guard let e = events[occ.eventId] else { return occ.details.selfStatus }
        if let r = occ.recurrenceId, let o = e.overrides.first(where: { $0.recurrenceId.normalized == r.normalized }) {
            return o.details.selfStatus
        }
        return e.details.selfStatus
    }
}

extension InvitationResponse {
    /// Reihenfolge der Knöpfe.
    public static let ordered: [InvitationResponse] = [.accepted, .tentative, .declined]

    /// Knopf: „Annehmen“, „Vielleicht“, „Ablehnen“.
    public var actionTitle: String {
        switch self {
        case .accepted: "Annehmen"
        case .tentative: "Vielleicht"
        case .declined: "Ablehnen"
        }
    }

    /// Für Rückgängig: „Rückgängig: Terminanfrage annehmen“.
    public var undoLabel: String {
        switch self {
        case .accepted: "Terminanfrage annehmen"
        case .tentative: "Mit Vielleicht antworten"
        case .declined: "Terminanfrage ablehnen"
        }
    }

    /// Antwort, die ein eigener Status ausdrückt (`nil`: noch offen oder delegiert).
    public init?(_ status: ParticipationStatus?) {
        switch status {
        case .accepted?: self = .accepted
        case .tentative?: self = .tentative
        case .declined?: self = .declined
        default: return nil
        }
    }
}

extension ParticipationStatus {
    /// Eigene Antwort im Detail: „Angenommen“, „Noch nicht beantwortet“ …
    public var responseText: String {
        switch self {
        case .needsAction: "Noch nicht beantwortet"
        case .accepted: "Angenommen"
        case .tentative: "Vielleicht"
        case .declined: "Abgelehnt"
        case .delegated: "Weitergegeben"
        }
    }
}
