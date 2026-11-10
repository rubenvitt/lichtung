import AppKit
import LichtungKit
import SwiftUI
import UniformTypeIdentifiers

/// Mac-Hauptfenster (Spec §5): links das Raster (Tag, Woche) bzw. Monat oder
/// Jahr, rechts die Spalte mit dem markierten Termin (`InspectorColumn`).
/// In der Toolbar Titel mit Mini-Monat, ein Feld für Eintragen und Suchen
/// (`CommandField`), Ansicht, Blättern, Kalender-Set, Anfragen und die Spalte.
///
/// Tastatur (LIC-24): Das Kalenderfeld nimmt den Fokus, sobald ein
/// Termin markiert oder leere Fläche angeklickt wird. Dann gelten ↑/↓ und
/// Tab/⇧Tab (Markierung wandern), ←/→ (Tag für Tag, LIC-50), Return/Leertaste
/// (öffnen), ⌫ (löschen), Esc (Markierung weg), ⌘C/⌘V. In Textfeldern bleiben
/// die Tasten beim Text. Belegung in `CalendarKeyCommand` (LichtungKit).
///
/// Trackpad (Spec §5): Waagerechtes Wischen mit zwei Fingern blättert einen
/// Schritt der Ansicht (`TrackpadPaging`), Aufziehen ändert im Raster die Höhe
/// der Stunde (40–120 pt, gemerkt).
struct MacRootView: View {
    @Environment(AppModel.self) private var model

    @AppStorage("inspectorVisible") private var inspectorVisible = true
    @AppStorage("hourHeight") private var hourHeight = Double(DayGridView.minimumHourHeight)
    /// Stundenhöhe zu Beginn des Aufziehens; die Vergrößerung zählt ab dort.
    @State private var pinchBase: Double?
    /// Stundenhöhe während des Aufziehens; gespeichert wird erst am Ende.
    @State private var pinchHeight: Double?
    /// Breite des Fensterinhalts; darunter blendet sich die rechte Spalte aus.
    @State private var contentWidth: CGFloat = .infinity
    @State private var commandMode = CommandField.Mode.create
    @State private var showsMiniMonth = false
    // Tastatur im Kalenderfeld (LIC-24)
    @FocusState private var calendarFocused: Bool

    static let hourHeightRange = 40.0...120.0
    /// Schmaler als das, blendet sich die rechte Spalte von selbst aus (Spec §8)
    /// und kommt wieder, wenn das Fenster breiter wird und sie eingeschaltet ist.
    static let inspectorMinWidth: CGFloat = 900
    static let minWidth: CGFloat = 720
    /// Darunter wird die Toolbar knapper (`compact`).
    static let compactToolbarWidth: CGFloat = 1100

    private var showsInspector: Bool { inspectorVisible && contentWidth >= Self.inspectorMinWidth }
    /// Schmales Fenster: Titel ohne Jahr, Feld schmaler, Umschalter als Menü, damit
    /// Blättern, „Heute“ und der Umschalter nicht ins Überlaufmenü rutschen.
    private var compact: Bool { contentWidth < Self.compactToolbarWidth }

    var body: some View {
        @Bindable var model = model
        HStack(spacing: 0) {
            keyboardDetail
                // Wischen nur über dem Kalender, nicht über Spalte und Toolbar.
                .background { TrackpadPagingMonitor { model.step($0) } }
            if showsInspector {
                InspectorColumn()
            }
        }
        .frame(minWidth: Self.minWidth)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { contentWidth = $0 }
        .background(Theme.shade)
        // Was weiter innen steht, landet in der Toolbar weiter rechts: Anfragen,
        // Sync-Probleme und ganz rechts der Knopf für die Spalte.
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { inspectorVisible.toggle() } label: {
                    Label("Rechte Spalte", systemImage: "sidebar.right")
                }
                .help(contentWidth < Self.inspectorMinWidth ? "Für die rechte Spalte ist das Fenster zu schmal"
                    : inspectorVisible ? "Rechte Spalte ausblenden (⌃⌘I)" : "Rechte Spalte einblenden (⌃⌘I)")
                .disabled(contentWidth < Self.inspectorMinWidth)
            }
        }
        .syncProblemsButton() // Konflikte, abgelehnte Änderungen (LIC-70)
        .invitationsButton() // Terminanfragen (LIC-30)
        .navigationTitle(windowTitle)
        // Der Titel steht als Knopf in der Toolbar (Mini-Monat).
        .toolbar(removing: .title)
        .toolbar {
            // Schmal und breit als getrennte Einträge: Die Toolbar misst einen
            // Eintrag nur einmal; ändert er nur seine Breite, überlappt er die
            // Nachbarn. Alles links (`.navigation`), damit Blättern, „Heute“ und
            // das Feld nicht ins Überlaufmenü rutschen; in der Mitte
            // (`.principal`) überlappte das Feld die Knöpfe zum Blättern.
            if compact {
                ToolbarItem(placement: .navigation) { titleButton(compact: true) }
                ToolbarItem(placement: .navigation) { pagingButtons }
                ToolbarItem(placement: .navigation) { CommandField(mode: $commandMode, compact: true) }
                // Umschalter als Aufklappmenü.
                ToolbarItem(placement: .primaryAction) { viewPicker.pickerStyle(.menu).labelsHidden().fixedSize() }
            } else {
                ToolbarItem(placement: .navigation) { titleButton(compact: false) }
                ToolbarItem(placement: .navigation) { pagingButtons }
                ToolbarItem(placement: .navigation) { CommandField(mode: $commandMode) }
                ToolbarItem(placement: .primaryAction) { viewPicker.pickerStyle(.segmented) }
            }
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Picker("Kalender", selection: $model.calendarSelection) {
                        Text("Alle Kalender").tag(CalendarQuery.Selection.visible)
                        ForEach(model.state.calendarSets.values.sorted { $0.position < $1.position }) { set in
                            Text(set.name).tag(CalendarQuery.Selection.set(set.id))
                        }
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()
                } label: {
                    Label("Kalender-Set", systemImage: "square.stack")
                }
                .help("Kalender-Set")
            }
        }
        .sheet(item: $model.selectedOccurrence, onDismiss: { model.detailDismissed() }) { occ in
            EventDetailView(occurrence: occ).frame(minWidth: 360, minHeight: 420)
        }
    }

    /// ⌘← und ⌘→ liegen im Menü „Darstellung“ (LIC-50): Als Kürzel der
    /// Toolbar fingen sie in Textfeldern den Sprung an Zeilenanfang und -ende ab.
    private var pagingButtons: some View {
        ControlGroup {
            Button { model.step(-1) } label: { Image(systemName: "chevron.left") }
                .help("\(model.viewMode.stepTitle(forward: false)) (⌘←)")
            Button("Heute") { model.goToToday() }
                .keyboardShortcut("t", modifiers: .command)
            Button { model.step(1) } label: { Image(systemName: "chevron.right") }
                .help("\(model.viewMode.stepTitle(forward: true)) (⌘→)")
        }
    }

    private var viewPicker: some View {
        @Bindable var model = model
        return Picker("Ansicht", selection: $model.viewMode) {
            ForEach(CalendarViewMode.allCases) { Text($0.label).tag($0) }
        }
        .help("Ansicht (⌘1–⌘4)")
    }

    /// „November 2026, KW 46“, schmal „November KW 46“; Klick zeigt den Mini-Monat.
    private func titleButton(compact: Bool) -> some View {
        let d = model.selectedDay
        return Button { showsMiniMonth.toggle() } label: {
            HStack(alignment: .firstTextBaseline, spacing: 5) {
                if model.viewMode == .year {
                    Text(verbatim: "\(d.year)").fontWeight(.semibold)
                } else {
                    Text(MonthTitle.monthName(d.month)).fontWeight(.semibold)
                    Text(verbatim: compact ? "KW \(MonthGrid.isoWeek(d))" : "\(d.year), KW \(MonthGrid.isoWeek(d))")
                        .foregroundStyle(Theme.text2)
                }
            }
            .font(.system(size: 15))
            .padding(.horizontal, 6)
        }
        .buttonStyle(.plain)
        .help("Monat zeigen")
        .accessibilityLabel(windowTitle)
        .popover(isPresented: $showsMiniMonth, arrowEdge: .bottom) {
            MiniMonthView(year: d.year, month: d.month)
                .padding(14)
                .frame(width: 240)
                .environment(model)
        }
    }

    /// Fenstertitel fürs Menü „Fenster“ und VoiceOver.
    private var windowTitle: String {
        let d = model.selectedDay
        return model.viewMode == .year ? "\(d.year)" : "\(MonthGrid.title(year: d.year, month: d.month)), KW \(MonthGrid.isoWeek(d))"
    }

    private var weekStart: LocalDate {
        model.selectedDay.adding(days: -(model.selectedDay.weekday.rawValue - 1))
    }

    @ViewBuilder
    private var detail: some View {
        let d = model.selectedDay
        switch model.viewMode {
        case .day: DayGridView(start: d, dayCount: 1, hourHeight: CGFloat(pinchHeight ?? hourHeight)).simultaneousGesture(pinch)
        case .week: DayGridView(start: weekStart, dayCount: 7, hourHeight: CGFloat(pinchHeight ?? hourHeight)).simultaneousGesture(pinch)
        case .month: MonthView(year: d.year, month: d.month)
        case .year: YearView(year: d.year)
        }
    }

    /// Aufziehen mit zwei Fingern: Höhe der Stunde, ab der Höhe zu Beginn der
    /// Geste. In die Einstellungen geschrieben wird erst am Ende.
    private var pinch: some Gesture {
        MagnifyGesture()
            .onChanged { value in
                let base = pinchBase ?? hourHeight
                pinchBase = base
                pinchHeight = min(max(base * value.magnification, Self.hourHeightRange.lowerBound), Self.hourHeightRange.upperBound)
            }
            .onEnded { _ in
                if let pinchHeight { hourHeight = pinchHeight }
                pinchBase = nil
                pinchHeight = nil
            }
    }

    // MARK: Tastatur (LIC-24)

    /// Ob gerade kein Blatt (Detail, Editor) offen ist.
    private var noSheetOpen: Bool {
        model.selectedOccurrence == nil && model.editorDraft == nil
    }

    /// Kalenderfeld mit Fokus, Tasten, Kopieren und Einfügen.
    private var keyboardDetail: some View {
        detail
            .focusable(interactions: .edit)
            .focusEffectDisabled()
            .focused($calendarFocused)
            // Für die Menüleiste: Fokus im Kalenderfeld des vordersten Fensters (LIC-50).
            .focusedValue(\.calendarKeyboardFocus, true)
            .onKeyPress(phases: [.down, .repeat]) { press in
                handleKey(press)
            }
            .onCopyCommand(perform: copyAction)
            .onPasteCommand(of: [UTType.plainText], perform: pasteAction)
            .onAppear { calendarFocused = true }
            .onChange(of: calendarFocused, initial: true) { _, focused in
                model.calendarHasKeyboardFocus = focused
            }
            .onChange(of: model.calendarFocusRequests) { calendarFocused = true }
            // Nach Detail oder Editor zurück ins Kalenderfeld.
            .onChange(of: noSheetOpen) { _, closed in
                if closed { calendarFocused = true }
            }
    }

    /// ⌘C im Kalenderfeld: Markierung als Text in die Zwischenablage und als
    /// Vorlage für ⌘V. `nil` lässt „Kopieren“ aus.
    private var copyAction: (() -> [NSItemProvider])? {
        guard let target = model.commandTarget else { return nil }
        let model = model
        return {
            let text: String = MainActor.assumeIsolated { model.copy(target) }
            return [NSItemProvider(object: text as NSString)]
        }
    }

    /// ⌘V im Kalenderfeld: die Kopie am ausgewählten Tag einfügen, solange die
    /// Zwischenablage noch ihren Text hält (`EventClipboard.matches`).
    private var pasteAction: ([NSItemProvider]) -> Void {
        let model = model
        return { _ in
            Task { @MainActor in
                await model.paste(pasteboardText: Pasteboard.string)
            }
        }
    }

    private func handleKey(_ press: KeyPress) -> KeyPress.Result {
        // Mit ⌘, ⌥ oder ⌃ gelten die Menüs (z. B. ⌘←).
        let menuModifiers: EventModifiers = [.command, .option, .control]
        guard press.modifiers.isDisjoint(with: menuModifiers) else { return .ignored }
        guard let key = Self.calendarKey(press.key),
              let command = CalendarKeyCommand(key: key, shift: press.modifiers.contains(.shift))
        else { return .ignored }
        // Gehaltene Taste: wandern und blättern ja, öffnen und löschen nur einmal.
        if press.phase == .repeat && (command == .open || command == .delete) {
            return .handled
        }
        return model.perform(command) ? .handled : .ignored
    }

    private static func calendarKey(_ key: KeyEquivalent) -> CalendarKey? {
        switch key {
        case .upArrow: return .up
        case .downArrow: return .down
        case .leftArrow: return .left
        case .rightArrow: return .right
        case .tab: return .tab
        case KeyEquivalent("\u{19}"): return .backTab // ⇧Tab kommt je nach Tastatur so an
        case .return: return .enter
        case .space: return .space
        case .escape: return .escape
        case .delete, .deleteForward: return .delete
        default: return nil
        }
    }
}

/// Jahresansicht: zwölf kleine Monate.
struct YearView: View {
    let year: Int

    var body: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), spacing: 24)], spacing: 24) {
                ForEach(1...12, id: \.self) { month in
                    MiniMonthView(year: year, month: month)
                        .padding(12)
                        // Flach auf dem Grund, nur mit feiner Linie (Spec §2: keine Karten).
                        .overlay {
                            RoundedRectangle(cornerRadius: Theme.cardRadius, style: .continuous)
                                .strokeBorder(Theme.line, lineWidth: 1)
                        }
                }
            }
            .padding(20)
        }
    }
}

/// Kalenderfeld mit Tastaturfokus im vordersten Fenster (LIC-50). Fehlt im
/// Einstellungsfenster, in Blättern und mit Fokus in einem Textfeld.
struct CalendarKeyboardFocusKey: FocusedValueKey {
    typealias Value = Bool
}

extension FocusedValues {
    var calendarKeyboardFocus: Bool? {
        get { self[CalendarKeyboardFocusKey.self] }
        set { self[CalendarKeyboardFocusKey.self] = newValue }
    }
}

/// Menüs und Tastenkürzel (LIC-24).
struct LichtungCommands: Commands {
    let model: AppModel
    /// Kürzel, die auch Textfelder kennen (⌘←, ⌘→, ⌘⌫ …), gelten nur mit
    /// Fokus im Kalenderfeld; sonst sind die Menüpunkte aus und die Taste
    /// bleibt beim Text (LIC-50).
    @FocusedValue(\.calendarKeyboardFocus) private var calendarKeyboardFocus: Bool?
    /// Rechte Spalte im Hauptfenster (`MacRootView`).
    @AppStorage("inspectorVisible") private var inspectorVisible = true

    var body: some Commands {
        // MARK: Editor (LIC-20)
        CommandGroup(replacing: .newItem) {
            Button("Neuer Termin") { Task { @MainActor in model.newEvent() } }
                .keyboardShortcut("n", modifiers: .command)
            // Feld in der Toolbar mit „Anlegen“ oben (Spec §5).
            Button("Termin eintragen …") { model.createFieldRequests += 1 }
                .keyboardShortcut("k", modifiers: .command)
        }
        CommandGroup(replacing: .undoRedo) {
            Button("Widerrufen") { Task { await model.undo() } }
                .keyboardShortcut("z", modifiers: .command)
            Button("Wiederholen") { Task { await model.redo() } }
                .keyboardShortcut("z", modifiers: [.command, .shift])
        }
        // ⌘F: Feld der Toolbar mit den Treffern oben. Ersetzt das Text-Suchen
        // im Menü „Bearbeiten“, sonst gäbe es ⌘F zweimal (LIC-42).
        CommandGroup(replacing: .textEditing) {
            Button("Termine suchen …") { model.searchFocusRequests += 1 }
                .keyboardShortcut("f", modifiers: .command)
        }
        CommandMenu("Kalender") {
            Button("Aktualisieren") { Task { await model.refresh() } }
                .keyboardShortcut("r", modifiers: .command)
        }
        // Ansicht wechseln: ⌘1 Tag, ⌘2 Woche, ⌘3 Monat, ⌘4 Jahr (Menü „Darstellung“).
        // Dazu ⌘← und ⌘→ (LIC-50).
        CommandGroup(before: .toolbar) {
            ViewModeMenuItems(model: model, calendarFocused: calendarKeyboardFocus == true)
            Button(inspectorVisible ? "Rechte Spalte ausblenden" : "Rechte Spalte einblenden") { inspectorVisible.toggle() }
                .keyboardShortcut("i", modifiers: [.command, .control])
            Divider()
        }
        // Aktionen am markierten Termin; ohne Markierung ausgegraut.
        CommandMenu("Termin") {
            EventMenuBarItems(model: model, calendarFocused: calendarKeyboardFocus == true)
        }
    }
}

/// Einträge für ⌘1–⌘4 sowie ⌘← und ⌘→ (LIC-50). Als eigene Ansicht, damit
/// sie dem Modell folgen.
struct ViewModeMenuItems: View {
    let model: AppModel
    /// Fokus im Kalenderfeld des vordersten Fensters (`calendarKeyboardFocus`).
    let calendarFocused: Bool

    var body: some View {
        ForEach(CalendarViewMode.allCases) { mode in
            Button(mode.label) { model.viewMode = mode }
                .keyboardShortcut(KeyEquivalent(mode.shortcutDigit), modifiers: .command)
        }
        Divider()
        // Ausgegraut, solange ein Textfeld (Schnelleingabe, Suche) oder ein
        // Blatt den Fokus hat: Dann springt ⌘←/⌘→ im Text an Zeilenanfang und -ende.
        let active = calendarFocused && model.calendarCommandsEnabled
        Button(model.viewMode.stepTitle(forward: false)) { model.step(-1) }
            .keyboardShortcut(.leftArrow, modifiers: .command)
            .disabled(!active)
        Button(model.viewMode.stepTitle(forward: true)) { model.step(1) }
            .keyboardShortcut(.rightArrow, modifiers: .command)
            .disabled(!active)
        Divider()
    }
}

/// Menü „Termin“: dieselben Einträge wie das Kontextmenü, mit Tastenkürzeln.
/// Ziel ist die Markierung, solange das Kalenderfeld im vordersten Fenster
/// den Fokus hat (im Einstellungsfenster löscht ⌘⌫ also weiter Text).
struct EventMenuBarItems: View {
    let model: AppModel
    let calendarFocused: Bool

    var body: some View {
        OccurrenceMenuItems(model: model, occurrence: calendarFocused ? model.commandTarget : nil, showsShortcuts: true)
        Divider()
        Button("Nächster Termin") { model.perform(.highlightNext) }
        Button("Vorheriger Termin") { model.perform(.highlightPrevious) }
        Button("Markierung aufheben") { model.perform(.clearHighlight) }
            .disabled(model.highlightedOccurrence == nil)
    }
}

/// Waagerechtes Wischen mit zwei Fingern blättert (Spec §5). Ein lokaler
/// Monitor sieht die Scroll-Ereignisse dieses Fensters, bevor das Raster sie
/// bekommt, und gibt sie unverändert weiter: Senkrechtes Scrollen bleibt also
/// unberührt. Es zählt nur, was über der Fläche dieses Views (dem Kalender)
/// geschieht; Blätter, Popover und andere Fenster zählen nicht mit.
private struct TrackpadPagingMonitor: NSViewRepresentable {
    let onStep: @MainActor (Int) -> Void

    func makeNSView(context: Context) -> MonitorView { MonitorView() }

    func updateNSView(_ view: MonitorView, context: Context) { view.onStep = onStep }

    static func dismantleNSView(_ view: MonitorView, coordinator: ()) { view.stop() }

    final class MonitorView: NSView {
        var onStep: @MainActor (Int) -> Void = { _ in }
        private var monitor: Any?
        private var paging = TrackpadPaging()

        func stop() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stop()
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
                MainActor.assumeIsolated { self?.handle(event) }
                return event
            }
        }

        private func handle(_ event: NSEvent) {
            // Nur Trackpad-Gesten über dem Kalender dieses Fensters, ohne offenes
            // Blatt; das Mausrad hat keine Phase.
            guard let window, event.window === window, window.attachedSheet == nil, event.hasPreciseScrollingDeltas,
                  bounds.contains(convert(event.locationInWindow, from: nil))
            else { return }
            let phase: TrackpadPaging.Phase
            if !event.momentumPhase.isEmpty {
                phase = .momentum
            } else if event.phase.contains(.began) {
                phase = .began
            } else if event.phase.contains(.changed) {
                phase = .changed
            } else if event.phase.contains(.ended) {
                phase = .ended
            } else if event.phase.contains(.cancelled) {
                phase = .cancelled
            } else {
                return
            }
            if let step = paging.feed(phase, dx: event.scrollingDeltaX, dy: event.scrollingDeltaY) {
                onStep(step)
            }
        }
    }
}
