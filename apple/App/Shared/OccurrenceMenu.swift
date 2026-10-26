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
/// Überall dasselbe: Liste, Raster, ganztägige Zeile, Monat (LIC-24).
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

    @Environment(\.openURL) private var openURL

    var body: some View {
        let occ = occurrence
        let editable: Bool = occ.map { model.canEdit($0) } ?? false
        let link: VideoLink? = occ.flatMap { VideoLinkDetector.detect(in: $0.details) }
        Button("Öffnen") { if let occ { model.open(occ) } }
            .shortcut("o", enabled: showsShortcuts)
            .disabled(occ == nil)
        Button("Bearbeiten …") { if let occ { model.edit(occ) } }
            .shortcut("e", enabled: showsShortcuts)
            .disabled(!editable)
        if let link {
            let joinTitle: String = "\(link.provider.displayName) beitreten"
            Button(joinTitle) { JoinButton.join(link, with: openURL) }
                .shortcut("j", enabled: showsShortcuts)
        } else if showsShortcuts {
            // Menüleiste: Eintrag bleibt stehen, damit das Menü nicht springt.
            Button("Beitreten") {}
                .shortcut("j", enabled: true)
                .disabled(true)
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
            Button("Kopieren") { Pasteboard.write(model.copy(occ)) }
        }
        calendarMenu(occ)
        Divider()
        Button(deleteTitle(occ), role: .destructive) { if let occ { model.requestDelete(occ) } }
            .shortcut(.delete, enabled: showsShortcuts)
            .disabled(!editable)
    }

    private func deleteTitle(_ occ: Occurrence?) -> String {
        guard let occ, OccurrenceActions.isSeries(occ) else { return "Löschen" }
        return "Löschen …"
    }

    private func duplicate(_ occ: Occurrence?, days: Int) {
        guard let occ else { return }
        Task { await model.duplicate(occ, days: days) }
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
                    if let occ { Task { await model.move(occ, toCalendar: cal.id) } }
                }
            }
        }
        .disabled(targets.isEmpty)
    }
}

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

    func body(content: Content) -> some View {
        #if os(macOS)
        content
            .onTapGesture(count: 2) { model.open(occurrence) }
            .simultaneousGesture(TapGesture().onEnded { model.highlight(occurrence) })
        #else
        content
            .onTapGesture { model.open(occurrence) }
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
/// es das Menü nur am Monatstag (LIC-50).
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
    private static var written: (text: String, changeCount: Int)?

    /// Inhalt der Zwischenablage, solange er noch von Lichtung stammt, sonst
    /// `nil` (LIC-50). Liest den Inhalt nicht, sondern vergleicht nur
    /// `changeCount`: So fragt iOS nicht nach der Erlaubnis zum Einfügen, und
    /// `EventClipboard.matches` wirkt wie am Mac (wer danach etwas anderes
    /// kopiert, fügt keinen alten Termin ein).
    static var string: String? {
        guard let written = Self.written, written.changeCount == UIPasteboard.general.changeCount else { return nil }
        return written.text
    }

    static func write(_ text: String) {
        UIPasteboard.general.string = text
        Self.written = (text: text, changeCount: UIPasteboard.general.changeCount)
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
