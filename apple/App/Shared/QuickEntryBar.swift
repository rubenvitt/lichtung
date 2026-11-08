// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Schnelleingabe per Satz mit Vorschau beim Tippen (Spec §5, LIC-17).
struct QuickEntryBar: View {
    @Environment(AppModel.self) private var model
    @State private var text = ""
    @FocusState private var focused: Bool
    /// Dauer, wenn der Satz keine nennt (LIC-53, `NewEventDuration`).
    @AppStorage(NewEventDuration.storageKey) private var storedMinutes = 0

    private var newEventMinutes: Int { NewEventDuration.minutes(stored: storedMinutes) }

    private var choices: [CalendarChoice] {
        model.writableCalendars.map { CalendarChoice(id: $0.id, name: $0.name) }
    }

    private func parse(now: Date) -> QuickEntry {
        QuickEntryParser(timeZone: model.timeZone, defaultDurationMinutes: newEventMinutes, calendars: choices).parse(text, now: now)
    }

    private var isEmpty: Bool { text.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        let now = Date()
        let entry = parse(now: now)
        let preview = QuickEntryPreview(entry, now: now, timeZone: model.timeZone, calendars: choices)
        VStack(alignment: .leading, spacing: 8) {
            if !text.isEmpty {
                previewCard(preview, calendarId: entry.calendarID ?? model.defaultCalendarId)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            HStack(spacing: 10) {
                HStack(spacing: 8) {
                    Image(systemName: "sparkles")
                        .font(.callout.weight(.semibold))
                        .foregroundStyle(focused ? Theme.accent : Color.secondary)
                        .accessibilityHidden(true)
                    TextField("Zahnarzt Di 14 Uhr …", text: $text)
                        .textFieldStyle(.plain)
                        .focused($focused)
                        .onSubmit(submit)
                        .accessibilityLabel("Neuer Termin als Satz")
                    if !text.isEmpty {
                        Button {
                            text = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundStyle(.tertiary)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Eingabe leeren")
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background {
                    Capsule().fill(.background.secondary)
                }
                .overlay {
                    Capsule().strokeBorder(focused ? Theme.accent.opacity(0.7) : Theme.hairline, lineWidth: focused ? 1.5 : 1)
                }
                // Leer: Editor öffnen. Mit Text: den Satz als Termin anlegen.
                Button(action: primaryAction) {
                    Image(systemName: isEmpty ? "plus" : "arrow.up")
                        .font(.body.weight(.bold))
                        .foregroundStyle(Theme.onAccent)
                        .frame(width: 38, height: 38)
                        .background(Circle().fill(Theme.accent))
                        .shadow(color: Theme.accent.opacity(0.35), radius: 6, y: 3)
                        .contentTransition(.symbolEffect(.replace))
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.return, modifiers: .command)
                .help(isEmpty ? "Neuer Termin" : "Termin anlegen (⌘↩)")
                .accessibilityLabel(isEmpty ? "Neuer Termin" : "Termin anlegen")
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .animation(.snappy(duration: 0.25), value: text.isEmpty)
    }

    /// Vorschau des Satzes: Titel groß, darunter Zeit, Wiederholung, Ort, Kalender.
    private func previewCard(_ preview: QuickEntryPreview, calendarId: String?) -> some View {
        let fallbackName: String? = calendarId.flatMap { model.state.calendars[$0]?.name }
        let calendarName: String = preview.calendarName ?? fallbackName ?? "Kein Kalender"
        let color: Color = calendarId.map { model.color(for: $0) } ?? Theme.accent
        return HStack(alignment: .top, spacing: 10) {
            CalendarBar(color: color)
            VStack(alignment: .leading, spacing: 4) {
                Text(preview.title.isEmpty ? "Neuer Termin" : preview.title)
                    .font(.headline)
                    .foregroundStyle(.primary)
                Label(preview.when, systemImage: "clock")
                if let r = preview.recurrence { Label(r, systemImage: "repeat") }
                if let l = preview.location { Label(l, systemImage: "mappin.and.ellipse") }
                Label(calendarName, systemImage: "calendar")
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(12)
        .background {
            RoundedRectangle(cornerRadius: Theme.cardRadius, style: .continuous)
                .fill(.background.secondary)
        }
        .overlay {
            RoundedRectangle(cornerRadius: Theme.cardRadius, style: .continuous)
                .strokeBorder(Theme.hairline, lineWidth: 1)
        }
        .accessibilityElement(children: .combine)
    }

    private func primaryAction() {
        if isEmpty {
            model.newEvent()
        } else {
            submit()
        }
    }

    private func submit() {
        let entry = parse(now: Date())
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        text = ""
        Task { await model.create(from: entry) }
    }
}
