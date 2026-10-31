// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Kontextmenü eines Termins (Mac: Rechtsklick, iPhone: lange drücken).
/// Überall dasselbe: Liste, Raster, ganztägige Zeile, Monat (LIC-24). Ziehbare
/// Blöcke im iPhone-Raster zeigen die Einträge in `OccurrenceMenuPopover`.
struct OccurrenceMenu: View {
    @Environment(AppModel.self) private var model
    let occurrence: Occurrence

    var body: some View {
        OccurrenceMenuItems(model: model, occurrence: occurrence)
    }
}

/// Einträge zu einem Termin, für Kontextmenü und Menü „Termin“ der
/// Menüleiste. Ohne Termin (`nil`) sind sie ausgegraut. Das Modell kommt
/// ausdrücklich herein, weil die Menüleiste keine Umgebung der Fenster hat.
struct OccurrenceMenuItems: View {
    let model: AppModel
    let occurrence: Occurrence?
    /// Menüleiste: Tastenkürzel zeigen und belegen. Im Kontextmenü nicht,
    /// dort stünden sie doppelt.
    var showsShortcuts = false
    /// Führt die gewählte Aktion aus. Das Popover (iPhone, LIC-54) schließt
    /// sich damit erst und lässt sie danach laufen, weil SwiftUI kein
    /// Blatt öffnet, solange das Popover noch steht.
    var run: @MainActor (@escaping @MainActor () -> Void) -> Void = { $0() }

    @Environment(\.openURL) private var openURL

    var body: some View {
        let occ = occurrence
        let editable: Bool = occ.map { model.canEdit($0) } ?? false
        let link: VideoLink? = occ.flatMap { VideoLinkDetector.detect(in: $0.details) }
        Button("Öffnen") { if let occ { run { model.open(occ) } } }
            .shortcut("o", enabled: showsShortcuts)
            .disabled(occ == nil)
        Button("Bearbeiten …") { if let occ { run { model.edit(occ) } } }
            .shortcut("e", enabled: showsShortcuts)
            .disabled(!editable)
        if let link {
            let joinTitle: String = "\(link.provider.displayName) beitreten"
            Button(joinTitle) { run { JoinButton.join(link, with: openURL) } }
                .shortcut("j", enabled: showsShortcuts)
        } else if showsShortcuts {
            // Menüleiste: Eintrag bleibt stehen, damit das Menü nicht springt.
            Button("Beitreten") {}
                .shortcut("j", enabled: true)
                .disabled(true)
        }
        // Terminanfrage beantworten (LIC-30)
        if let occ, model.canRespond(occ) {
            InvitationMenu(model: model, occurrence: occ)
        }
        Divider()
        Button("Duplizieren") { duplicate(occ, days: 0) }
            .shortcut("d", enabled: showsShortcuts)
            .disabled(occ == nil)
        Button("Auf morgen duplizieren") { duplicate(occ, days: 1) }
            .disabled(occ == nil)
        Button("Auf nächste Woche duplizieren") { duplicate(occ, days: 7) }
            .disabled(occ == nil)
        if !showsShortcuts, let occ {
            // Mac: In der Menüleiste kopiert „Bearbeiten > Kopieren“ (⌘C,
            // `onCopyCommand`). iPhone: Einfügen über die Toolbar oder
            // langes Drücken auf einen Monatstag (LIC-50).
            Button("Kopieren") { run { Pasteboard.write(model.copy(occ)) } }
        }
        calendarMenu(occ)
        Divider()
        Button(deleteTitle(occ), role: .destructive) { if let occ { run { model.requestDelete(occ) } } }
            .shortcut(.delete, enabled: showsShortcuts)
            .disabled(!editable)
    }

    private func deleteTitle(_ occ: Occurrence?) -> String {
        guard let occ, OccurrenceActions.isSeries(occ) else { return "Löschen" }
        return "Löschen …"
    }

    private func duplicate(_ occ: Occurrence?, days: Int) {
        guard let occ else { return }
        run { Task { await model.duplicate(occ, days: days) } }
    }

    /// Untermenü der beschreibbaren Kalender. Bei Serien wandert die ganze
    /// Serie (der Server erlaubt den Wechsel nur für alle Termine).
    @ViewBuilder
    private func calendarMenu(_ occ: Occurrence?) -> some View {
        let targets: [CalendarInfo] = occ.map { model.moveTargets(for: $0) } ?? []
        let isSeries: Bool = occ.map { OccurrenceActions.isSeries($0) } ?? false
        let title: String = isSeries ? "Kalender wechseln (ganze Serie)" : "Kalender wechseln"
        Menu(title) {
            ForEach(targets) { cal in
                Button(cal.name) {
                    if let occ { run { Task { await model.move(occ, toCalendar: cal.id) } } }
                }
            }
        }
        .disabled(targets.isEmpty)
    }
}

#if os(iOS)
/// iPhone: Menü eines ziehbaren Blocks im Raster (LIC-54). Dort gehört langes
/// Drücken dem Ziehen; wer den aufgenommenen Block ohne Bewegung loslässt,
/// bekommt dieselben Einträge wie im Kontextmenü als Popover am Block
/// (`GridInteraction.opensMenuOnRelease`).
struct OccurrenceMenuPopover: View {
    let model: AppModel
    let occurrence: Occurrence
    /// Merkt die gewählte Aktion und schließt das Popover; die Aktion läuft
    /// danach (siehe `OccurrenceMenuItems.run`).
    let choose: @MainActor (@escaping @MainActor () -> Void) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(occurrence.title)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
            Divider()
            OccurrenceMenuItems(model: model, occurrence: occurrence, run: choose)
        }
        .buttonStyle(MenuRowButtonStyle())
        .menuStyle(.button)
        .fixedSize(horizontal: false, vertical: true)
        .frame(width: 260)
        .presentationCompactAdaptation(.popover)
    }
}

/// iPhone: Menü beim Halten auf leerer Fläche im Raster (LIC-58). Erscheint
/// nur, wenn eine Kopie vorliegt und der Finger ohne Bewegung loslässt
/// (`GridInteraction.slotHoldRelease`); wer zieht, legt wie bisher an.
/// „Neuer Termin hier“ steht dabei, damit das Anlegen an der Stelle auch
/// ohne Ziehen erreichbar bleibt.
struct SlotMenuPopover: View {
    let model: AppModel
    /// Gehaltene Uhrzeit, gerundet wie beim Anlegen (`GridInteraction.pasteStart`).
    let start: Date
    /// Editor für einen neuen Termin an der Stelle.
    let newEvent: @MainActor () -> Void
    /// Wie bei `OccurrenceMenuPopover.choose`: schließt das Popover, die
    /// Aktion läuft danach.
    let choose: @MainActor (@escaping @MainActor () -> Void) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let clip = model.clipboard {
                Text(clip.pasteTitle(at: start, calendar: model.calendar))
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                Divider()
            }
            Button("Hier einfügen") {
                let start = start
                choose {
                    let text = Pasteboard.string
                    Task { await model.paste(at: start, pasteboardText: text) }
                }
            }
            .disabled(model.clipboard == nil)
            Button("Neuer Termin hier") { choose(newEvent) }
        }
        .buttonStyle(MenuRowButtonStyle())
        .fixedSize(horizontal: false, vertical: true)
        .frame(width: 260)
        .presentationCompactAdaptation(.popover)
    }
}

/// Zeile im Popover: ganze Breite, wie ein Eintrag im Kontextmenü.
private struct MenuRowButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        Row(configuration: configuration)
    }

    private struct Row: View {
        let configuration: ButtonStyleConfiguration
        @Environment(\.isEnabled) private var isEnabled

        var body: some View {
            configuration.label
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.vertical, 11)
                .foregroundStyle(configuration.role == .destructive ? Color.red : Color.primary)
                .opacity(isEnabled ? 1 : 0.4)
                .background(configuration.isPressed ? Color.secondary.opacity(0.15) : Color.clear)
                .contentShape(Rectangle())
        }
    }
}
#endif

private extension View {
    /// Tastenkürzel nur, wenn `enabled` (Menüleiste).
    @ViewBuilder
    func shortcut(_ key: KeyEquivalent, modifiers: EventModifiers = .command, enabled: Bool) -> some View {
        if enabled {
            keyboardShortcut(key, modifiers: modifiers)
        } else {
            self
        }
    }
}

/// Klick auf einen Termin. Mac wie Fantastical: einmal markiert, doppelt
/// öffnet das Detail. iPhone: Tippen öffnet wie bisher.
struct OccurrenceTap: ViewModifier {
    @Environment(AppModel.self) private var model
    let occurrence: Occurrence
    /// iPhone: `false`, wenn der Tap das Detail nicht öffnen soll.
    var opensDetail: @MainActor () -> Bool = { true }

    func body(content: Content) -> some View {
        #if os(macOS)
        content
            .onTapGesture(count: 2) { model.open(occurrence) }
            .simultaneousGesture(TapGesture().onEnded { model.highlight(occurrence) })
        #else
        content
            .onTapGesture { if opensDetail() { model.open(occurrence) } }
        #endif
    }
}

/// Frage nach dem Geltungsbereich beim Löschen eines Serientermins aus
/// Kontextmenü, Menü „Termin“ oder ⌫ (wie im Editor).
struct OccurrenceDeleteDialog: ViewModifier {
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        @Bindable var bindable = model
        content.modifier(SeriesDeleteDialog(
            model: model,
            isPresented: $bindable.isConfirmingDelete,
            occurrence: model.pendingDelete,
            onDelete: {}
        ))
    }
}

/// Die Frage selbst: ein Knopf je Geltungsbereich (`OccurrenceActions.deleteScopes`).
/// `onDelete` läuft nach der Wahl, z. B. schließt das Detail-Blatt (LIC-50).
struct SeriesDeleteDialog: ViewModifier {
    let model: AppModel
    @Binding var isPresented: Bool
    let occurrence: Occurrence?
    let onDelete: @MainActor () -> Void

    func body(content: Content) -> some View {
        content.confirmationDialog(
            "Serientermin löschen",
            isPresented: $isPresented,
            titleVisibility: .visible,
            presenting: occurrence
        ) { occ in
            ForEach(OccurrenceActions.deleteScopes(occ), id: \.self) { scope in
                Button(scope.editorLabel, role: .destructive) {
                    Task { await model.delete(occ, scope: scope) }
                    onDelete()
                }
            }
            Button("Abbrechen", role: .cancel) {}
        } message: { occ in
            Text("„\(occ.title)“ gehört zu einer Serie. Welche Termine sollen gelöscht werden?")
        }
    }
}

extension View {
    func occurrenceDeleteDialog() -> some View {
        modifier(OccurrenceDeleteDialog())
    }
}

/// Kontextmenü auf leerer Fläche im Raster (Mac) oder an einem Monatstag.
/// Am iPhone ist langes Drücken im Raster das Anlegen per Ziehen, dort gibt
/// es das Menü nur am Monatstag (LIC-50); im Raster öffnet Halten ohne
/// Bewegung mit einer Kopie `SlotMenuPopover` (LIC-58).
struct DaySlotMenu: View {
    @Environment(AppModel.self) private var model
    let day: LocalDate
    /// Beginn an der Stelle des Rechtsklicks; `nil` nimmt die Vorgabe des Editors.
    var start: () -> LocalDateTime? = { nil }

    var body: some View {
        Button("Neuer Termin hier") { model.newEvent(on: day, at: start()) }
        if model.clipboard != nil {
            Button("Einfügen") {
                let text = Pasteboard.string
                Task { await model.paste(on: day, pasteboardText: text) }
            }
        }
    }
}

extension AppModel {
    /// Einfügen mit Beginn `start` (iPhone-Raster, „Hier einfügen“, LIC-58),
    /// sonst wie `paste(on:pasteboardText:)`: Hält die Zwischenablage etwas
    /// anderes, gilt die Kopie nicht mehr.
    func paste(at start: Date, pasteboardText: String?) async {
        guard let clip = clipboard else { return }
        guard clip.matches(pasteboardText) else {
            clipboard = nil
            return
        }
        guard let calendarId = duplicateCalendarId(for: clip.occurrence) else { return }
        let intent = clip.intent(pastingAt: start, calendarId: calendarId, deviceTimeZone: timeZone)
        await engine.perform(intent, label: "Termin einfügen")
    }
}

/// Zwischenablage des Systems (nur Text).
@MainActor
enum Pasteboard {
    #if os(macOS)
    static var string: String? {
        NSPasteboard.general.string(forType: .string)
    }

    static func write(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
    #else
    /// Zuletzt hier geschriebener Text und der Stand der Zwischenablage danach.
    private static var written: PasteboardMark?

    /// Inhalt der Zwischenablage, solange er noch von Lichtung stammt, sonst
    /// `nil` (LIC-50). Liest den Inhalt nicht, sondern vergleicht nur
    /// `changeCount`: So fragt iOS nicht nach der Erlaubnis zum Einfügen, und
    /// `EventClipboard.matches` wirkt wie am Mac (wer danach etwas anderes
    /// kopiert, fügt keinen alten Termin ein).
    static var string: String? {
        written?.text(atChangeCount: UIPasteboard.general.changeCount)
    }

    static func write(_ text: String) {
        UIPasteboard.general.string = text
        Self.written = PasteboardMark(text: text, changeCount: UIPasteboard.general.changeCount)
    }
    #endif
}

/// Kontextmenü „Neuer Termin hier“ und „Einfügen“ an einem Monatstag; am
/// iPhone mit langem Drücken (LIC-50).
struct DaySlotContextMenu: ViewModifier {
    let day: LocalDate

    func body(content: Content) -> some View {
        content.contextMenu { DaySlotMenu(day: day) }
    }
}

/// Letzte Mausposition in einer Rasterspalte, für „Neuer Termin hier“. Eine
/// Klasse ohne Beobachtung, damit Mausbewegungen nichts neu zeichnen.
final class HoverPoint {
    var y: CGFloat?
}
