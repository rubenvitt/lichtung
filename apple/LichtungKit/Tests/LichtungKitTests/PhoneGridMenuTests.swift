import Foundation
@testable import LichtungKit
import Testing

/// LIC-54: Menü an ziehbaren Blöcken im iPhone-Raster und Verfall der Kopie,
/// wenn die Zwischenablage anderswo überschrieben wurde.
@Suite("iPhone: Menü am Block und Zwischenablage")
struct PhoneGridMenuTests {
    typealias D = DraftFixtures

    @Test func releaseWithoutMoveOpensMenu() {
        let h = 1152.0 // 48 Punkte pro Stunde, 12 Punkte pro Rasterschritt
        #expect(GridInteraction.opensMenuOnRelease(dx: 0, dy: 0, columnHeight: h))
        // Zittern des Fingers bleibt Menü.
        #expect(GridInteraction.opensMenuOnRelease(dx: 3, dy: -4, columnHeight: h))
        #expect(GridInteraction.opensMenuOnRelease(dx: 9, dy: 0, columnHeight: h))
        // Ab der Schwelle ist es ein Zug, in jede Richtung.
        #expect(!GridInteraction.opensMenuOnRelease(dx: -10, dy: 0, columnHeight: h))
        #expect(!GridInteraction.opensMenuOnRelease(dx: 8, dy: 8, columnHeight: h))
        // Ein halber Rasterschritt verschiebt schon um 15 Minuten: kein Menü.
        #expect(!GridInteraction.opensMenuOnRelease(dx: 0, dy: 6, columnHeight: h))
        #expect(!GridInteraction.opensMenuOnRelease(dx: 0, dy: -7, columnHeight: h))
        #expect(GridInteraction.opensMenuOnRelease(dx: 0, dy: 5.9, columnHeight: h))
        // Bei höherem Raster (96 Punkte pro Stunde) greift nur die Strecke des Fingers.
        #expect(GridInteraction.opensMenuOnRelease(dx: 0, dy: 9, columnHeight: 2880))
    }

    @Test func pasteboardMarkHoldsTextUntilChanged() {
        let mark = PasteboardMark(text: "Team-Sync", changeCount: 7)
        #expect(mark.text(atChangeCount: 7) == "Team-Sync")
        #expect(mark.text(atChangeCount: 8) == nil)
    }

    /// Beim Aktivwerden: Hat eine andere App kopiert, passt die Kopie nicht mehr.
    @Test func clipboardExpiresWhenPasteboardChanged() {
        let (occ, _) = D.single()
        let clip = EventClipboard(occ, calendar: .lichtung(timeZone: D.tz))
        let mark = PasteboardMark(text: clip.text, changeCount: 3)
        #expect(clip.matches(mark.text(atChangeCount: 3)))
        #expect(!clip.matches(mark.text(atChangeCount: 4)))
        #expect(!clip.matches(nil))
    }
}
