import Foundation

/// Terminsuche, wie die Apps sie aufrufen (LIC-42): Suchfeld am iPhone und in
/// der Mac-Toolbar, ein Treffer springt zum Tag und öffnet das Detail.
extension CalendarQuery {
    /// Treffer in den Kalendern von `selection`.
    ///
    /// Durchsucht wird der lokale Stand, also nur, was im Cache liegt
    /// (Einzeltermine im Cache-Fenster, Serien immer). Serien liefern ihr
    /// nächstes Vorkommen in `EventSearch.horizon`, sonst das letzte davor.
    /// Treffer ohne Vorkommen in dieser Reichweite (lange beendete Serien)
    /// fallen weg: Es gibt keinen Tag, zu dem die Liste springen könnte.
    public func search(_ text: String, selection: Selection = .visible, now: Date, limit: Int = 50) -> [EventSearch.Hit] {
        let ids = calendarIds(for: selection)
        var search = EventSearch(deviceTimeZone: expander.deviceTimeZone)
        search.expander = expander
        return search.search(text, in: events.values.filter { ids.contains($0.calendarId) }, now: now, limit: limit)
            .filter { $0.occurrence != nil }
    }
}

extension EventSearch.Hit {
    /// Tag, zu dem ein Treffer springt: Beginn des Vorkommens.
    public func day(calendar cal: Calendar) -> LocalDate? {
        occurrence.map { LocalDate($0.start, calendar: cal) }
    }

    /// Zeile unter dem Titel: „Heute · Mo 26. · 09:00–09:30“,
    /// „Di 3. Nov. · ganztägig“, „Fr 6. Nov. · ab 22:00“ (über Mitternacht).
    public func dateText(today: LocalDate, calendar cal: Calendar) -> String? {
        guard let o = occurrence else { return nil }
        let day = LocalDate(o.start, calendar: cal)
        let item = AgendaBuilder.item(o, dayStart: day.startDate(in: cal), dayEnd: day.adding(days: 1).startDate(in: cal), cal: cal)
        return "\(AgendaBuilder.heading(day, today: today)) · \(item.timeText)"
    }
}
