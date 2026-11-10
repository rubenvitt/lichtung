import AppKit
import LichtungKit
import SwiftUI

/// Ein Feld für Eintragen und Suchen in der Mitte der Mac-Toolbar (Spec §5).
///
/// Beim Tippen zeigt eine Liste darunter die Zeile „Anlegen: …“ mit der
/// Vorschau der Schnelleingabe und die Suchtreffer. ⌘K (`.create`) stellt
/// „Anlegen“ nach oben und wählt es vor, ⌘F (`.search`) die Treffer. Return
/// führt die vorgewählte Zeile aus, ↑/↓ wählen, Esc leert das Feld und gibt
/// den Fokus ans Raster zurück. Ohne Text zeigt das Feld nichts.
///
/// Die Liste liegt in einem Fenster, das nie key wird; VoiceOver bleibt also
/// im Feld. Deshalb trägt das Feld die vorgewählte Zeile als Wert und sagt
/// sie an, wenn sie wechselt.
struct CommandField: View {
    enum Mode { case create, search }

    @Binding var mode: Mode
    /// Schmales Fenster: Feld auf Mindestbreite.
    var compact = false
    @Environment(AppModel.self) private var model
    /// Dauer, wenn der Satz keine nennt (LIC-53, `NewEventDuration`).
    @AppStorage(NewEventDuration.storageKey) private var storedMinutes = 0
    @State private var text = ""
    /// Vorgewählte Zeile (wird beim Zeichnen auf die vorhandenen Zeilen begrenzt).
    @State private var selection = 0
    /// Ob das Feld gerade bearbeitet wird (meldet `CommandTextField`).
    @State private var focused = false
    /// Zähler: Fokus ins Feld holen (⌘K, ⌘F).
    @State private var focusRequest = 0

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

    /// Was sich ändern muss, damit VoiceOver die vorgewählte Zeile neu ansagt:
    /// andere Zeile gewählt oder andere Zeilen da, nicht jeder Tastendruck.
    private struct Announcement: Equatable {
        var selected: Int?
        var rows: [String]
    }

    var body: some View {
        let rows = self.rows(now: Date())
        let current: Int? = rows.isEmpty ? nil : min(selection, rows.count - 1)
        let spoken: String? = current.map { accessibilityText(rows[$0]) }
        HStack(spacing: 6) {
            CommandTextField(
                text: $text,
                isFocused: $focused,
                focusRequest: focusRequest,
                placeholder: compact ? "Eintragen oder suchen" : "Termin eintragen oder suchen",
                valueDescription: spoken,
                onMove: { delta in
                    guard !rows.isEmpty else { return }
                    selection = min(max((current ?? 0) + delta, 0), rows.count - 1)
                },
                onSubmit: { run(current.map { rows[$0] }) },
                onCancel: close
            )
            Text(mode == .search ? "⌘F" : "⌘K")
                .font(.callout)
                .foregroundStyle(Theme.text2)
                .accessibilityHidden(true)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        // Feste Breite: Das `NSTextField` bringt keine sinnvolle Wunschbreite mit.
        .frame(width: compact ? 190 : 280)
        .background(RoundedRectangle(cornerRadius: Theme.buttonRadius, style: .continuous).fill(Theme.line))
        .background {
            DropdownPanel(content: showsList ? AnyView(list(rows, selected: current).environment(model)) : nil)
        }
        .onChange(of: text) { selection = 0 }
        .onChange(of: Announcement(selected: current, rows: rows.map(\.id))) {
            if focused, let spoken { AccessibilityNotification.Announcement(spoken).post() }
        }
        .onChange(of: model.createFieldRequests) { request(.create) }
        .onChange(of: model.searchFocusRequests) { request(.search) }
    }

    /// Liste nur mit Text und Fokus im Feld.
    private var showsList: Bool { focused && !isEmpty }

    private var isEmpty: Bool { text.trimmingCharacters(in: .whitespaces).isEmpty }

    private func request(_ newMode: Mode) {
        mode = newMode
        selection = 0
        focusRequest += 1
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

    /// Texte der Zeile „Anlegen“: Titel, Zeit mit Kalender, Wiederholung und Ort.
    private func createTexts(_ entry: QuickEntry) -> (title: String, when: String, extras: String?, calendarId: String?) {
        let preview = QuickEntryPreview(entry, now: Date(), timeZone: model.timeZone, calendars: choices)
        let calendarId = entry.calendarID ?? model.defaultCalendarId
        let calendarName = preview.calendarName ?? calendarId.flatMap { model.state.calendars[$0]?.name } ?? "Kein Kalender"
        let title = preview.title.trimmingCharacters(in: .whitespaces)
        let extras = [preview.recurrence, preview.location].compactMap { $0 }
        return (title.isEmpty ? "Neuer Termin" : title, "\(when(entry)) in \(calendarName)",
                extras.isEmpty ? nil : extras.joined(separator: ", "), calendarId)
    }

    /// Die Zeile als ein Satz für VoiceOver.
    private func accessibilityText(_ row: Row) -> String {
        switch row {
        case let .create(entry):
            let t = createTexts(entry)
            return "Anlegen: \(t.title), \(t.when)"
        case let .hit(occ):
            return "\(occ.title), \(AgendaBuilder.sentence(occ, today: model.today, calendar: model.calendar))"
        }
    }

    // MARK: Liste

    private func list(_ rows: [Row], selected: Int?) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                Button { run(row) } label: {
                    rowLabel(row)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background {
                            if index == selected {
                                RoundedRectangle(cornerRadius: Theme.buttonRadius, style: .continuous)
                                    .fill(Theme.light.opacity(0.12))
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .accessibilityAddTraits(index == selected ? .isSelected : [])
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
            let t = createTexts(entry)
            HStack(alignment: .top, spacing: 8) {
                CalendarBar(color: t.calendarId.map { model.color(for: $0) } ?? Theme.text2, width: 3)
                VStack(alignment: .leading, spacing: 1) {
                    (Text("Anlegen: ").fontWeight(.semibold) + Text(t.title))
                        .foregroundStyle(Theme.text1)
                    Text(t.when)
                        .font(.callout)
                        .foregroundStyle(Theme.text2)
                    if let extras = t.extras {
                        Text(extras)
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

/// Das Textfeld selbst als `NSTextField`: ↑/↓, Return und Esc kommen über
/// `control(_:textView:doCommandBy:)` sicher an, auch während der Feld-Editor
/// den Fokus hat. Bei `TextField` mit `onKeyPress` ist das am Mac nicht verlässlich.
private struct CommandTextField: NSViewRepresentable {
    @Binding var text: String
    @Binding var isFocused: Bool
    /// Ändert sich der Zähler, holt sich das Feld den Fokus.
    let focusRequest: Int
    let placeholder: String
    /// Vorgewählte Zeile für VoiceOver.
    let valueDescription: String?
    let onMove: (Int) -> Void
    let onSubmit: () -> Void
    let onCancel: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> FocusTextField {
        let field = FocusTextField()
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: NSFont.systemFontSize)
        field.lineBreakMode = .byTruncatingTail
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        field.setAccessibilityLabel("Termin eintragen oder suchen")
        field.delegate = context.coordinator
        field.onFocus = { [weak coordinator = context.coordinator] in coordinator?.parent.isFocused = true }
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return field
    }

    func updateNSView(_ field: FocusTextField, context: Context) {
        context.coordinator.parent = self
        if field.stringValue != text { field.stringValue = text }
        field.placeholderString = placeholder
        field.setAccessibilityValueDescription(valueDescription)
        if context.coordinator.focusRequest != focusRequest {
            context.coordinator.focusRequest = focusRequest
            DispatchQueue.main.async { field.window?.makeFirstResponder(field) }
        }
    }

    final class FocusTextField: NSTextField {
        var onFocus: () -> Void = {}

        override func becomeFirstResponder() -> Bool {
            let became = super.becomeFirstResponder()
            if became { onFocus() }
            return became
        }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: CommandTextField
        var focusRequest: Int

        init(_ parent: CommandTextField) {
            self.parent = parent
            focusRequest = parent.focusRequest
        }

        func controlTextDidChange(_ note: Notification) {
            guard let field = note.object as? NSTextField else { return }
            parent.text = field.stringValue
        }

        func controlTextDidEndEditing(_ note: Notification) {
            parent.isFocused = false
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            switch selector {
            case #selector(NSResponder.moveUp(_:)): parent.onMove(-1)
            case #selector(NSResponder.moveDown(_:)): parent.onMove(1)
            case #selector(NSResponder.insertNewline(_:)): parent.onSubmit()
            case #selector(NSResponder.cancelOperation(_:)): parent.onCancel()
            default: return false
            }
            return true
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
