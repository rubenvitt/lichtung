// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Ansicht A: Tagesleiste oben, Termine als Liste darunter (Spec §5).
struct AgendaView: View {
    @Environment(AppModel.self) private var model
    /// Wie viele Tage die Liste ab dem ausgewählten Tag zeigt.
    var days = 60
    var showsTicker = true

    var body: some View {
        let start = model.selectedDay
        let occurrences = model.occurrences(from: start, days: days)
        let sections = AgendaBuilder.days(occurrences, from: start, days: days, today: model.today, calendar: model.calendar)
        VStack(spacing: 0) {
            if showsTicker {
                DayTickerView()
                Divider()
            }
            List {
                ForEach(sections) { section in
                    Section {
                        if section.items.isEmpty {
                            Text("Keine Termine").foregroundStyle(.secondary)
                        }
                        ForEach(section.items) { item in
                            AgendaRow(item: item)
                                .contentShape(Rectangle())
                                .onTapGesture { model.selectedOccurrence = item.occurrence }
                                .contextMenu { OccurrenceMenu(occurrence: item.occurrence) }
                        }
                    } header: {
                        Text(section.heading)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(section.isToday ? Color.accentColor : Color.primary)
                    }
                }
            }
            .listStyle(.plain)
        }
    }
}

struct AgendaRow: View {
    @Environment(AppModel.self) private var model
    let item: AgendaItem

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            CalendarDot(color: model.color(for: item.occurrence.calendarId))
            Text(item.timeText)
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: 96, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.occurrence.title)
                    .strikethrough(item.occurrence.details.status == .cancelled)
                if let location = item.occurrence.details.location, !location.isEmpty {
                    Text(location).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
            if model.state.isPending(item.occurrence.eventId) {
                Image(systemName: "arrow.triangle.2.circlepath")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Noch nicht synchronisiert")
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
    }
}

/// Fantasticals DayTicker: Tage nebeneinander, Farbpunkte je Kalender.
struct DayTickerView: View {
    @Environment(AppModel.self) private var model
    var count = 21

    var body: some View {
        let start = model.today.adding(days: -3)
        let occurrences = model.occurrences(from: start, days: count)
        let days = DayTicker.days(occurrences, from: start, count: count, today: model.today, calendar: model.calendar)
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(days) { day in
                        TickerCell(day: day, isSelected: day.date == model.selectedDay)
                            .id(day.date)
                            .onTapGesture { model.selectedDay = day.date }
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
            }
            .onAppear { proxy.scrollTo(model.selectedDay, anchor: .center) }
        }
    }
}

struct TickerCell: View {
    @Environment(AppModel.self) private var model
    let day: TickerDay
    let isSelected: Bool

    var body: some View {
        VStack(spacing: 2) {
            Text(day.weekdayShort).font(.caption2)
            Text("\(day.dayNumber)").font(.callout.weight(day.isToday ? .bold : .regular))
            HStack(spacing: 2) {
                ForEach(day.calendarIds, id: \.self) { id in
                    CalendarDot(color: model.color(for: id), size: 4)
                }
            }
            .frame(height: 4)
            Capsule()
                .fill(Color.secondary.opacity(0.4))
                .frame(width: 28 * max(0.08, day.busyFraction), height: 2)
                .opacity(day.busyFraction > 0 ? 1 : 0)
        }
        .frame(width: 40, height: 58)
        .foregroundStyle(isSelected ? Color.white : (day.isWeekend ? Color.secondary : Color.primary))
        .background(RoundedRectangle(cornerRadius: 8).fill(isSelected ? Color.accentColor : Color.clear))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(day.weekdayShort) \(day.dayNumber)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

/// Kontextmenü eines Termins (Mac: Rechtsklick, iPhone: lange drücken).
struct OccurrenceMenu: View {
    @Environment(AppModel.self) private var model
    let occurrence: Occurrence

    var body: some View {
        Button("Duplizieren") { Task { await model.duplicate(occurrence) } }
        Button("Auf morgen duplizieren") { Task { await model.duplicate(occurrence, days: 1) } }
        Divider()
        Button("Löschen", role: .destructive) { Task { await model.delete(occurrence) } }
    }
}
