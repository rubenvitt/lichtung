import AppKit
import LichtungKit
import SwiftUI

/// Ein Feld für Eintragen und Suchen in der Mitte der Mac-Toolbar (Spec §5).
///
/// Beim Tippen zeigt ein Popover darunter die Zeile „Anlegen: …“ mit der
/// Vorschau der Schnelleingabe und die Suchtreffer. ⌘K (`.create`) stellt
/// „Anlegen“ nach oben und wählt es vor, ⌘F (`.search`) die Treffer. Return
/// führt die vorgewählte Zeile aus, ↑/↓ wählen, Esc leert das Feld und gibt
/// den Fokus ans Raster zurück. Ohne Text zeigt das Feld nichts.
struct CommandField: View {
    enum Mode { case create, search }

    @Binding var mode: Mode
    @Environment(AppModel.self) private var model
    /// Dauer, wenn der Satz keine nennt (LIC-53, `NewEventDuration`).
    @AppStorage(NewEventDuration.storageKey) private var storedMinutes = 0
    @State private var text = ""
    /// Vorgewählte Zeile.
    @State private var selection = 0
    @FocusState private var focused: Bool

    /// Eine Zeile der Liste.
    private enum Row: Identifiable {
        case create(QuickEntry)
        case hit(Occurrence)

        var id: String {
            switch self {
            case .create: "create"
            case let .hit(occ): occ.id
            }
        }
    }

    var body: some View {
        let rows = self.rows(now: Date())
        HStack(spacing: 6) {
            TextField("Termin eintragen oder suchen", text: $text)
                .textFieldStyle(.plain)
                .focused($focused)
                .onSubmit { run(rows.indices.contains(selection) ? rows[selection] : nil) }
                .onKeyPress(.downArrow) { move(1, count: rows.count) }
                .onKeyPress(.upArrow) { move(-1, count: rows.count) }
                .onExitCommand(perform: close)
                .accessibilityLabel("Termin eintragen oder suchen")
            Text(mode == .search ? "⌘F" : "⌘K")
                .font(.callout)
                .foregroundStyle(Theme.text2)
                .accessibilityHidden(true)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .frame(minWidth: 160, idealWidth: 240, maxWidth: 380)
        .background(RoundedRectangle(cornerRadius: Theme.buttonRadius, style: .continuous).fill(Theme.line))
        .background {
            DropdownPanel(content: showsList ? AnyView(list(rows).environment(model)) : nil)
        }
        .onChange(of: text) { selection = 0 }
        .onChange(of: model.createFieldRequests) { request(.create) }
        .onChange(of: model.searchFocusRequests) { request(.search) }
    }

    /// Liste nur mit Text und Fokus im Feld.
    private var showsList: Bool { focused && !isEmpty }

    private var isEmpty: Bool { text.trimmingCharacters(in: .whitespaces).isEmpty }

    private func request(_ newMode: Mode) {
        mode = newMode
        selection = 0
        focused = true
    }

    private func move(_ delta: Int, count: Int) -> KeyPress.Result {
        guard count > 0 else { return .ignored }
        selection = min(max(selection + delta, 0), count - 1)
        return .handled
    }

    /// Esc: Feld leeren, Fokus zurück ans Raster.
    private func close() {
        text = ""
        model.calendarFocusRequests += 1
    }

    // MARK: Zeilen

    private var choices: [CalendarChoice] {
        model.writableCalendars.map { CalendarChoice(id: $0.id, name: $0.name) }
    }

    private func parse(now: Date) -> QuickEntry {
        QuickEntryParser(timeZone: model.timeZone, defaultDurationMinutes: NewEventDuration.minutes(stored: storedMinutes),
                         calendars: choices).parse(text, now: now)
    }

    private func rows(now: Date) -> [Row] {
        guard !isEmpty else { return [] }
        let hits: [Row] = model.search(text).prefix(8).compactMap { $0.occurrence.map(Row.hit) }
        let create = Row.create(parse(now: now))
        return mode == .search ? hits + [create] : [create] + hits
    }

    private func run(_ row: Row?) {
        guard let row else { return }
        switch row {
        case .create:
            // Neu parsen: „morgen“ gilt ab jetzt, nicht ab dem letzten Zeichnen.
            let entry = parse(now: Date())
            Task { await model.create(from: entry) }
            close()
        case let .hit(occ):
            text = ""
            model.focus(LocalDate(occ.start, calendar: model.calendar))
            model.highlight(occ) // markiert und holt den Fokus ins Raster
        }
    }

    // MARK: Liste

    private func list(_ rows: [Row]) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                Button { run(row) } label: {
                    rowLabel(row)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background {
                            if index == selection {
                                RoundedRectangle(cornerRadius: Theme.buttonRadius, style: .continuous)
                                    .fill(Theme.light.opacity(0.12))
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .accessibilityAddTraits(index == selection ? .isSelected : [])
            }
            if !rows.contains(where: { if case .hit = $0 { true } else { false } }) {
                Text("Keine Treffer")
                    .font(.callout)
                    .foregroundStyle(Theme.text2)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
            }
        }
        .padding(6)
        .frame(width: 380)
        .background(Theme.shade, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Theme.line, lineWidth: 1)
        }
    }

    @ViewBuilder
    private func rowLabel(_ row: Row) -> some View {
        switch row {
        case let .create(entry):
            let preview = QuickEntryPreview(entry, now: Date(), timeZone: model.timeZone, calendars: choices)
            let calendarId = entry.calendarID ?? model.defaultCalendarId
            let calendarName = preview.calendarName ?? calendarId.flatMap { model.state.calendars[$0]?.name } ?? "Kein Kalender"
            HStack(alignment: .top, spacing: 8) {
                CalendarBar(color: calendarId.map { model.color(for: $0) } ?? Theme.text2, width: 3)
                VStack(alignment: .leading, spacing: 1) {
                    (Text("Anlegen: ").fontWeight(.semibold) + Text(preview.title))
                        .foregroundStyle(Theme.text1)
                    Text("\(when(entry)) in \(calendarName)")
                        .font(.callout)
                        .foregroundStyle(Theme.text2)
                    let extras = [preview.recurrence, preview.location].compactMap { $0 }
                    if !extras.isEmpty {
                        Text(extras.joined(separator: ", "))
                            .font(.callout)
                            .foregroundStyle(Theme.text2)
                    }
                }
            }
        case let .hit(occ):
            HStack(alignment: .top, spacing: 8) {
                CalendarBar(color: model.color(for: occ.calendarId), width: 3)
                VStack(alignment: .leading, spacing: 1) {
                    Text(occ.title)
                        .foregroundStyle(Theme.text1)
                        .strikethrough(occ.details.status == .cancelled)
                    Text(AgendaBuilder.sentence(occ, today: model.today, calendar: model.calendar))
                        .font(Theme.digits(.callout))
                        .foregroundStyle(Theme.text2)
                }
                .lineLimit(1)
            }
        }
    }

    /// Zeit der Vorschau als Satz. Ohne Zeitangabe legt `AppModel.create`
    /// ganztägig am ausgewählten Tag an.
    private func when(_ entry: QuickEntry) -> String {
        let cal = model.calendar
        let today = model.today
        switch entry.timing {
        case .unspecified:
            let day = model.selectedDay
            return AgendaBuilder.sentence(from: day.startDate(in: cal), to: day.adding(days: 1).startDate(in: cal),
                                          allDay: true, today: today, calendar: cal)
        case let .allDay(first, last):
            return AgendaBuilder.sentence(from: first.startDate(in: cal), to: last.adding(days: 1).startDate(in: cal),
                                          allDay: true, today: today, calendar: cal)
        case let .timed(start, end):
            return AgendaBuilder.sentence(from: start, to: end, allDay: false, today: today, calendar: cal)
        }
    }
}

/// Liste unter dem Feld als eigenes Fenster (Spec §8), das nie key wird: So
/// bleibt der Tastaturfokus beim Tippen im Feld, auch während die Liste
/// offen ist. Es hängt als Kind am Fenster, wandert also mit und verschwindet,
/// wenn die App in den Hintergrund geht (`hidesOnDeactivate`).
private struct DropdownPanel: NSViewRepresentable {
    /// `nil`: Liste zu.
    let content: AnyView?

    func makeNSView(context: Context) -> AnchorView { AnchorView() }

    func updateNSView(_ view: AnchorView, context: Context) { view.show(content) }

    static func dismantleNSView(_ view: AnchorView, coordinator: ()) { view.show(nil) }

    /// Unsichtbar hinter dem Feld; gibt die Lage für die Liste vor.
    final class AnchorView: NSView {
        private var panel: NonKeyPanel?

        func show(_ content: AnyView?) {
            guard let content, let window else {
                panel?.parent?.removeChildWindow(panel!)
                panel?.orderOut(nil)
                return
            }
            let panel = self.panel ?? NonKeyPanel()
            self.panel = panel
            panel.appearance = window.effectiveAppearance
            let hosting = panel.contentView as? NSHostingView<AnyView> ?? NSHostingView(rootView: content)
            hosting.rootView = content
            panel.contentView = hosting
            let size = hosting.fittingSize
            let anchor = window.convertToScreen(convert(bounds, to: nil))
            panel.setFrame(NSRect(x: anchor.midX - size.width / 2, y: anchor.minY - 6 - size.height,
                                  width: size.width, height: size.height), display: true)
            if panel.parent == nil { window.addChildWindow(panel, ordered: .above) }
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if window == nil { show(nil) }
        }
    }

    final class NonKeyPanel: NSPanel {
        init() {
            super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
            isOpaque = false
            backgroundColor = .clear
            hasShadow = true
            hidesOnDeactivate = true
        }

        override var canBecomeKey: Bool { false }
        override var canBecomeMain: Bool { false }
    }
}
