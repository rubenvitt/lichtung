# Neugestaltung der Apple-Apps – Umsetzungsplan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Woche und Tag werden lesbar (Fokus-Tag, Rahmen für lange Termine, zusammengefasste Termine), Mac und iPhone bekommen den Aufbau und das Gesicht aus Entwurf 3.

**Architecture:** Alles Rechnende kommt getestet ins Package `LichtungKit` (Rasterrechnung, Spaltenbreiten, Zusammenfassen). Die App-Targets zeichnen nur: `Theme.swift` liefert Farben und Schrift, `DayGridView` das Raster, `MacRootView`/`PhoneRootView` den Rahmen.

**Tech Stack:** Swift 6 (strict concurrency), SwiftUI, Swift Testing (`@Suite`, `#expect`), xcodegen, Xcode 27. Keine neuen Abhängigkeiten.

**Spec:** `docs/superpowers/specs/2026-11-10-apple-ui-redesign-design.md`, Vorlage `docs/superpowers/specs/assets/2026-11-10-apple-ui-mockup.html`. Jeder Ausführende liest beide zuerst.

## Global Constraints

- Mindestversionen bleiben iOS 18 und macOS 15. Keine neue Abhängigkeit.
- Code englisch (Bezeichner, Dateinamen), Kommentare und Oberflächentexte deutsch mit echten Umlauten.
- `LichtungKit` bleibt ohne SwiftUI (nur Foundation).
- Farben nur über `Theme`; Werte exakt aus Spec §4. Kein Grün, kein Rot-Orange, keine Serife, keine gerundete Schrift, keine Schatten oder Leuchteffekte an Terminen, keine Großbuchstaben-Etiketten, keine mit „·“ verketteten Statuszeilen.
- Text an Terminen hat mindestens 4,5:1 Kontrast in beiden Modi; „vorbei“ nutzt Text 2.
- Systembausteine für Toolbar, Umschalter, Blätter; kein selbst gebautes Glas.
- Bestehende Bedienung bleibt: Ziehen, Tastatur, Kontextmenüs, Kopieren/Einfügen, Rückgängig, Reisezeit, zweite Zeitzone.
- Commits nach Conventional Commits auf Deutsch (`feat(apple): …`), Branch `feat/apple-ui-fokus-tag`.
- Prüfbefehle (im Plan kurz „Kit-Tests“, „Mac-Build“, „iOS-Build“):
  - Kit-Tests: `cd apple/LichtungKit && swift test`
  - Mac-Build: `cd apple && xcodegen generate && xcodebuild -scheme Lichtung-macOS -destination 'platform=macOS' -derivedDataPath build/dd CODE_SIGNING_ALLOWED=NO build 2>&1 | grep -E "error:|warning: unused|BUILD"` → `** BUILD SUCCEEDED **`
  - iOS-Build: wie oben mit `-scheme Lichtung-iOS -destination 'generic/platform=iOS Simulator'`
- Der Kopfvermerk „Ohne Xcode entstanden …“ wird in jeder Datei entfernt, die in einer Aufgabe gebaut und im Demo-Modus angesehen wurde.

## Review Focus

1. Tag ohne Termine, Woche ohne Termine: Raster, rechte Spalte („Heute nichts mehr“) und iPhone-Kopf zeigen sinnvolle Leere, kein Absturz. → Test in Task 1 (leere Eingabe), Sichtprüfung in Task 7 und 8.
2. Termin über Mitternacht (22–6 Uhr) und Termin an Umstellungstagen: kein Rahmen, keine negative Höhe. → Tests in Task 1.
3. Zehn gleichzeitige Termine an einem Tag: „+n“ statt unlesbarer Streifen. → Test in Task 1.
4. Blocker-Titel leer oder nur Leerzeichen in den Einstellungen: blendet nichts aus (sonst verschwände jeder titellose Termin). → Test in Task 2.
5. Blocker mit zwei zeitgleichen Partnern, drei gleiche Termine in drei Kalendern: genau ein sichtbarer Termin je Gruppe, nichts doppelt in `alsoIn`. → Tests in Task 2.

---

### Task 1: Rasterrechnung mit Rahmen und „+n“ (Kit)

**Files:**
- Modify: `apple/LichtungKit/Sources/LichtungKit/Views/GridLayout.swift` (Abschnitt `TimelineLayout`)
- Test: `apple/LichtungKit/Tests/LichtungKitTests/ViewLayoutTests.swift` (neue Suite, `LayoutFixtures` mitnutzen)

**Interfaces:**
- Consumes: bestehendes `TimelineLayout.blocks(_:day:calendar:minimumMinutes:)`, `TimelineBlock`.
- Produces:
  ```swift
  public struct TimelineFrame: Hashable, Sendable, Identifiable {  // langer Termin als Randlinie
      public var occurrence: Occurrence
      public var top: Double; public var height: Double   // Anteil der Spalte, 0…1
      public var index: Int                               // 0 = ganz links
      public var id: String { occurrence.id }
  }
  public struct TimelineOverflow: Hashable, Sendable, Identifiable {  // Feld „+n“
      public var hidden: [Occurrence]
      public var top: Double; public var height: Double
      public var column: Int; public var columns: Int
      public var id: String
  }
  public struct TimelineDay: Hashable, Sendable {
      public var blocks: [TimelineBlock]; public var frames: [TimelineFrame]; public var overflows: [TimelineOverflow]
  }
  extension TimelineBlock { public var insetByFrames: Int }  // Zahl der Rahmen, die den Block zeitlich überdecken
  extension TimelineLayout {
      public static let frameMinimumMinutes = 240.0
      public static func day(_ occurrences: [Occurrence], day: LocalDate, calendar: Calendar,
                             maxColumns: Int, minimumMinutes: Double = 15) -> TimelineDay
  }
  ```

Regeln (Spec §5): Rahmen = zeitgebunden, Anteil am Tag ≥ 240 min, überschneidet mindestens einen Termin, dessen Anteil am Tag < 240 min ist. Rahmen fallen aus der Spaltenrechnung; der Rest läuft durch die bestehende Spaltenrechnung. Hat eine Gruppe mehr als `maxColumns` Spalten, bleiben die Spalten `0..<maxColumns-1`, alle Blöcke weiter rechts wandern in ein `TimelineOverflow` in Spalte `maxColumns-1` (ein Feld je Gruppe, Höhe über die verdrängten Blöcke). `blocks(...)` bleibt für Aufrufer außerhalb des Rasters bestehen.

- [ ] **Step 1: Failing tests** (Suite `"Raster: Rahmen und Überlauf"`, `L = LayoutFixtures`)

```swift
@Test func longEventBecomesFrame() {           // Fall aus dem Screenshot
    let d = TimelineLayout.day([L.timed("tech", 12, 8, to: 17), L.timed("a", 12, 11, to: 12),
        L.timed("b", 12, 14, to: 15), L.timed("c", 12, 9, 30, to: 9, 45)], day: L.day(12), calendar: L.cal, maxColumns: 3)
    #expect(d.frames.map(\.occurrence.eventId) == ["tech"])
    #expect(d.blocks.count == 3)
    #expect(d.blocks.allSatisfy { $0.columns == 1 && $0.insetByFrames == 1 })
}
@Test func longEventAloneStaysBlock()          // 9–17 ohne anderen Termin: frames leer, 1 Block
@Test func twoLongEventsOnlyOverlappingEachOtherStayBlocks()  // 8–14 und 10–17: frames leer, columns == 2
@Test func overnightEventIsNoFrame()           // 22–6 Uhr (endDay) + Termin 23–23:30: an keinem Tag ein Rahmen
@Test func twoFramesGetIndexes()               // 8–17, 9–16 + kurzer Termin: indexes [0, 1], Block insetByFrames == 2
@Test func blockOutsideFrameIsNotInset()       // Rahmen 8–12 + Termin 10–11 + Termin 14–15: letzter insetByFrames == 0
@Test func overflowCollapsesExtraColumns() {   // zehn Termine 10–11, maxColumns 3
    // blocks.count == 2, overflows.count == 1, overflows[0].hidden.count == 8, column == 2, columns == 3
}
@Test func shortBackToBackGoSideBySide()       // 9:00–9:05 und 9:05–9:30: columns == 2
@Test func emptyDay()                          // [] → alles leer
@Test func dstDayKeepsPositiveHeights()        // 25.10.2026, Termin 1:30–3:30 + Rahmen 0–6: alle height > 0
```

- [ ] **Step 2:** Kit-Tests mit `--filter "Rahmen"` → schlagen fehl (Typen fehlen).
- [ ] **Step 3:** Typen und `TimelineLayout.day` umsetzen; die Spaltenrechnung aus `blocks` in eine private Funktion ziehen, die beide nutzen.
- [ ] **Step 4:** Kit-Tests ganz → grün.
- [ ] **Step 5:** Commit `feat(apple): Rasterrechnung mit Rahmen für lange Termine und Überlauf`

---

### Task 2: Termine zusammenfassen (Kit)

**Files:**
- Create: `apple/LichtungKit/Sources/LichtungKit/Features/OccurrenceMerge.swift`
- Modify: `apple/LichtungKit/Sources/LichtungKit/Features/Reminders.swift` (`ReminderPlanner.plan`)
- Test: `apple/LichtungKit/Tests/LichtungKitTests/OccurrenceMergeTests.swift`, `ReminderTests.swift`

**Interfaces:**
- Consumes: `Occurrence`, `CalendarInfo` (`writable`, `position`), `CalendarOrder.sorted`, `EventDetails.selfStatus`.
- Produces:
  ```swift
  public struct MergeRules: Hashable, Sendable, Codable {
      public var mergeIdentical = true
      public var hideBlockers = true
      public var blockerTitles = ["Blockiert"]
      public static let `default` = MergeRules()
      public static let off = MergeRules(mergeIdentical: false, hideBlockers: false, blockerTitles: [])
  }
  public struct MergedOccurrence: Hashable, Sendable, Identifiable {
      public var shown: Occurrence
      public var others: [Occurrence]          // zusammengefasste, in Kalenderreihenfolge
      public var id: String { shown.id }
      public var alsoInCalendarIds: [String]   // ohne Doppelte, ohne shown.calendarId
  }
  public enum OccurrenceMerge {
      public static func merge(_ occurrences: [Occurrence], calendars: [String: CalendarInfo], rules: MergeRules) -> [MergedOccurrence]
  }
  // ReminderPlanner.plan bekommt den Parameter `merge: MergeRules = .off`
  ```

Regeln (Spec §6): Schlüssel für „gleich“: `start`, `end`, `isAllDay`, Titel getrimmt und kleingeschrieben, verschiedene Kalender. Rangfolge für `shown`: `selfStatus == .needsAction`, dann schreibbarer Kalender, dann `CalendarOrder`. Blocker: Titel (getrimmt, klein) in `blockerTitles` (ebenso normalisiert, leere Einträge verworfen), gleicher `start`/`end` wie ein Nicht-Blocker in anderem Kalender → wandert in dessen `others`; bei mehreren Partnern in den ersten nach Rangfolge. Blocker mit `needsAction` wird nie ausgeblendet. Die Ausgabe behält die Reihenfolge der Eingabe (Position von `shown`). Erinnerungen: `plan` rechnet wie bisher über alle Vorkommen und verwirft danach Erinnerungen, deren Vorkommen in `others` einer Gruppe liegt, wenn die Gruppe zur selben `fireDate` schon eine hat; die übrig bleibende trägt Titel und `occurrenceId` von `shown`.

- [ ] **Step 1: Failing tests** (eigene kleine Fixture mit drei Kalendern: `work` schreibgeschützt Position 0, `private` schreibbar Position 1, `drk` schreibbar Position 2)

```swift
@Test func identicalInThreeCalendarsBecomeOne()      // 1 Ergebnis, shown.calendarId == "private", alsoIn == ["work", "drk"]
@Test func titleCompareIgnoresCaseAndWhitespace()    // " lehrgang " == "Lehrgang"
@Test func differentEndIsNotMerged()
@Test func blockerWithPartnerIsHidden()              // "Sport" private + "Blockiert" work → 1 Ergebnis, alsoIn == ["work"]
@Test func blockerWithoutPartnerStays()
@Test func blockerWithTwoPartnersGoesToOne()         // 2 Ergebnisse, Blocker in genau einem others
@Test func twoBlockersSameTimeWithoutPartnerStayMergedAsIdentical() // 1 Ergebnis mit Titel "Blockiert"
@Test func pendingInvitationWinsAsShown()            // needsAction im schreibgeschützten Kalender wird shown
@Test func pendingBlockerIsNotHidden()
@Test func emptyBlockerTitleHidesNothing()           // blockerTitles ["", "  "], titelloser Termin + Partner → 2 Ergebnisse
@Test func rulesOffReturnsEverything()
@Test func orderOfInputIsKept()
// ReminderTests:
@Test func mergedGroupRemindsOncePerFireDate()       // zwei gleiche Termine, beide Alarm −10 min → 1 Erinnerung
@Test func blockerAlarmSurvivesWithShownTitle()      // nur der Blocker hat einen Alarm → 1 Erinnerung, Titel "Sport"
```

- [ ] **Step 2:** `swift test --filter OccurrenceMerge` → schlägt fehl.
- [ ] **Step 3:** Umsetzen.
- [ ] **Step 4:** Kit-Tests ganz → grün.
- [ ] **Step 5:** Commit `feat(apple): gleiche Termine und Blocker zusammenfassen`

---

### Task 3: Ungleiche Spaltenbreiten (Kit)

**Files:**
- Create: `apple/LichtungKit/Sources/LichtungKit/Views/ColumnWidths.swift`
- Test: `apple/LichtungKit/Tests/LichtungKitTests/ColumnWidthsTests.swift`

**Interfaces:**
- Produces:
  ```swift
  public struct ColumnWidths: Hashable, Sendable {
      public static let focusWeight = 2.1
      public let fractions: [Double]                         // je Spalte, Summe 1
      public init(columns: Int, focus: Int?)                 // focus nil oder außerhalb → alle gleich
      public func leading(_ column: Int) -> Double           // Anteil links der Spalte
      public func width(from first: Int, through last: Int) -> Double
      public func column(atFraction x: Double) -> Int        // auf 0..<columns begrenzt
      /// Tagesversatz beim Ziehen: Spalte unter dem Zeiger minus Ausgangsspalte.
      public func dayOffset(fromColumn column: Int, pointerFraction x: Double) -> Int
  }
  ```

- [ ] **Step 1: Failing tests**

```swift
@Test func sevenColumnsWithFocus() {
    let w = ColumnWidths(columns: 7, focus: 1)
    #expect(abs(w.fractions.reduce(0, +) - 1) < 1e-9)
    #expect(abs(w.fractions[1] / w.fractions[0] - 2.1) < 1e-9)
    #expect(abs(w.leading(2) - (1 + 2.1) / 8.1) < 1e-9)
}
@Test func noFocusIsEven()                    // fractions alle 1/7
@Test func singleColumn()                     // [1]
@Test func columnAtFractionClamps()           // −0.2 → 0, 1.5 → 6, genau auf der Grenze → rechte Spalte
@Test func dayOffsetAcrossFocus()             // von Spalte 0, Zeiger mitten in Spalte 2 → 2
@Test func widthAcrossAllIsOne()
```

- [ ] **Step 2:** fehlschlagen lassen. **Step 3:** umsetzen. **Step 4:** Kit-Tests grün.
- [ ] **Step 5:** Commit `feat(apple): Spaltenbreiten mit Fokus-Tag`

---

### Task 4: Theme und Termin-Darstellung

**Files:**
- Modify: `apple/App/Shared/Theme.swift`, alle Aufrufer von `Theme.accent`, `accentDeep`, `now`, `onNow`, `sun`, `wash`, `display`, `number`, `MonthTitle`, `SectionLabel`, `NowBadge` (per `grep -rn "Theme\.\|MonthTitle\|SectionLabel\|NowBadge" apple/App`)
- Modify: `EventBlockView`, `NowLine`, `AllDayBarShape`, `DayHeaderCell` in `apple/App/Shared/DayGridView.swift`
- Modify: Asset `AccentColor` (hell `#1B1D1A`, dunkel `#F4E9C9`), Kommentar in `apple/project.yml`

**Interfaces:**
- Produces (in `Theme`):
  ```swift
  static let shade, lit, light, onLight, text1, text2, text3, sun, line: Color   // Werte Spec §4
  static func eventFill(_ color: Color) -> Color        // dunkel 30 %, hell 20 % auf shade
  static let blockRadius: CGFloat = 4; static let buttonRadius: CGFloat = 7
  static func digits(_ style: Font.TextStyle, weight: Font.Weight = .regular) -> Font  // System, monospacedDigit
  enum EventState { case past, upcoming, running }
  static func state(of occ: Occurrence, now: Date) -> EventState
  struct PrimaryButtonStyle: ButtonStyle                // Fläche light, Schrift onLight, Radius 7
  ```
- `EventBlockView` bekommt `var compact: Bool` (schmale Spalte: `.fontWidth(.condensed)`, keine Zeit in einer Zeile) und zeichnet die drei Zustände aus Spec §4, Auswahlrahmen 1,5 pt in `text1`.

- [ ] **Step 1:** `Theme` umbauen: alte Farben und `display`/`number` löschen, neue Werte eintragen, `NowBadge` löschen (Zustand „läuft“ ersetzt sie), `SectionLabel` wird halbfette normale Zeile ohne `uppercased()`/`tracking`, `MonthTitle` ohne Serife und Farbe (Jahr in `text2`).
- [ ] **Step 2:** Aufrufer anpassen, bis Mac-Build und iOS-Build grün sind. Wo bisher `Theme.accent` Auswahl oder Hauptknopf färbte: `Theme.light`/`PrimaryButtonStyle`; wo es „heute“ markierte: `Theme.light` für die Zahl. Wochenend-Tönung (`wash`) entfällt.
- [ ] **Step 3:** `EventBlockView`, `NowLine` (1,5 pt, `sun`, ohne Knopf), `AllDayBarShape` (Radius 3, Fläche `eventFill`) nach Spec §4. VoiceOver: `accessibilityValue` „vorbei“ bzw. „läuft gerade“.
- [ ] **Step 4:** `grep -rn "design: .serif\|design: .rounded\|uppercased()\|shadow(" apple/App` → keine Treffer an Terminen und Titeln. Mac-Build, iOS-Build grün.
- [ ] **Step 5:** Commit `feat(apple): neues Theme ohne Akzentfarbe, Termine mit drei Zuständen`

---

### Task 5: Modell – Fokus-Tag, Zusammenfassen, Demo-Woche

**Files:**
- Modify: `apple/App/Shared/AppModel.swift`, `apple/App/Shared/DemoData.swift`, `apple/App/Shared/ReminderScheduler.swift:96`, `apple/App/Shared/Settings/GeneralSettingsView.swift`, `apple/App/Shared/AppSession.swift`
- Test: `apple/LichtungKit/Tests/LichtungKitTests/AppIntegrationTests.swift` nur, falls dort Demo-Daten geprüft werden

**Interfaces:**
- Consumes: `OccurrenceMerge.merge`, `MergeRules`, `ReminderPlanner.plan(… merge:)`.
- Produces (in `AppModel`):
  ```swift
  var focusDay: LocalDate                       // Regeln Spec §5 „Fokus-Tag“
  var mergeRules: MergeRules                    // didSet → UserDefaults "mergeRules" (JSON)
  func mergedOccurrences(from start: LocalDate, days: Int) -> [MergedOccurrence]
  func occurrences(from:days:) -> [Occurrence]  // liefert ab jetzt nur noch `shown`
  func alsoIn(_ occ: Occurrence) -> [CalendarInfo]
  func otherOccurrences(of occ: Occurrence) -> [Occurrence]
  ```
- Startargument `--demo`: `AppSession` startet ohne Abfrage im Demo-Modus (für Bildschirmfotos).

- [ ] **Step 1:** `focusDay` einführen. Setzen in: Tageskopf-Tipp (neue Methode `focus(_ day:)`, setzt auch `selectedDay`), `perform(.previousDay/.nextDay)`, `goToToday()`, `step(_:)` (gleicher Wochentag der neuen Woche), Sprung aus Suche und Mini-Monat. Nicht in `tapEmptyArea` und `highlightNeighbor`.
- [ ] **Step 2:** `occurrences(from:days:)` über `OccurrenceMerge` leiten; Suche (`query.search`) bleibt ungefiltert. `ReminderScheduler` übergibt `merge: model.mergeRules`.
- [ ] **Step 3:** Einstellungen „Allgemein“: Schalter „Gleiche Termine zusammenfassen“, Schalter „Blocker ausblenden“, darunter Titel-Liste (ein Textfeld, Komma-getrennt, Vorgabe „Blockiert“), Fußzeile: „Ein Blocker wird nur ausgeblendet, wenn er zeitgleich mit einem anderen Termin liegt. Am Termin steht, in welchen Kalendern er noch eingetragen ist.“ Kommentar `// shortcut: je Gerät in UserDefaults, Abgleich kommt mit LIC-75`.
- [ ] **Step 4:** `DemoData`: laufende Woche nach dem Muster KW 46 der Vorlage (Daten im Script der Vorlage, Konstante `raw`): ein Tag mit 8–17-Uhr-Termin plus drei kurzen, „Sport“ privat mit „Blockiert“ im Arbeitskalender, ein Termin gleich in drei Kalendern, drei 15-Minuten-Termine hintereinander, ein Termin mit Teams-Link, zwei ganztägige über die Woche.
- [ ] **Step 5:** Kit-Tests, Mac-Build, iOS-Build grün. Commit `feat(apple): Fokus-Tag und zusammengefasste Termine im Modell, Demo-Woche`

---

### Task 6: Raster mit Fokus-Tag, Rahmen und „+n“

**Files:**
- Modify: `apple/App/Shared/DayGridView.swift`, `apple/App/Shared/GridDrag.swift` (falls dort Spaltenbreite gerechnet wird)

**Interfaces:**
- Consumes: `TimelineLayout.day(… maxColumns:)`, `TimelineFrame`, `TimelineOverflow`, `ColumnWidths`, `model.focusDay`, `model.focus(_:)`, `EventBlockView(compact:)`.
- Produces: `DayGridView(start:dayCount:hourHeight:)` unverändert im Aufruf; neu `static let minimumHourHeight: CGFloat = 56`.

- [ ] **Step 1:** Spaltenbreiten: `ColumnWidths(columns: dayCount, focus: dayCount > 1 ? Index von focusDay : nil)`. Kopf, Tagesspalten, `AllDayRow` (x und Breite über `leading`/`width(from:through:)`) und das Ziehen in andere Tage (`dayOffset(fromColumn:pointerFraction:)` statt `GridInteraction.dayOffset`) nutzen dieselbe Instanz. Übergang `.animation(.snappy(duration: 0.3), value: model.focusDay)`, aus bei `accessibilityReduceMotion`.
- [ ] **Step 2:** Kopf nach Spec: Fokus-Tag „Dienstag“ leicht + Zahl halbfett 26 pt (iPhone-Woche 20 pt), übrige „Mo 9“; heute Zahl in `Theme.light`; KW-Feld im Raster entfernen; Tipp ruft `model.focus(day)`.
- [ ] **Step 3:** `DayColumn` rechnet mit `TimelineLayout.day(…, maxColumns: isFocus ? 5 : 3)`. Rahmen: Linie 1,5 pt bei `x = 3 + 5·index`, Titel 10 pt oben (Fokus-Tag mit Zeit), Trefferfläche mindestens 16 pt breit über die ganze Höhe, Tipp markiert (`highlightedOccurrence`), Kontextmenü wie Blöcke, kein Ziehen. Blöcke rücken um `10·insetByFrames` pt ein. Überlauf: Feld „+n“; im schmalen Tag `model.focus(day)`, im Fokus-Tag Popover mit den Terminen als Liste.
- [ ] **Step 4:** Licht-Fläche hinter der Spalte von heute (dunkel: senkrechter Verlauf, voll 8–17 Uhr, je 90 Minuten auslaufend; hell: `Theme.lit` flächig), Kommentar `// shortcut: feste Uhrzeiten statt Sonnenstand`. Jetzt-Linie hinter den Blöcken, Uhrzeit in der Stundenspalte auf Höhe der Linie in `Theme.sun`; die Stundenzahl im Abstand unter 20 Minuten wird ausgeblendet.
- [ ] **Step 5:** Termine im Fokus-Tag: Ort in der Zeitzeile, Knopf „Beitreten“ (`PrimaryButtonStyle`, klein) bei Videolink, Zustand nicht vorbei, Höhe ≥ 60 pt.
- [ ] **Step 6:** Mac-Build grün. App mit `--demo` starten, Woche und Tag hell und dunkel ansehen und Bildschirmfotos nach `docs/superpowers/specs/assets/2026-11-10-task6-*.png` legen. Von Hand prüfen: Termin in einen anderen Tag über den Fokus-Tag hinweg ziehen, im Fokus-Tag neuen Termin aufziehen, Länge ändern, ←/→.
- [ ] **Step 7:** Commit `feat(apple): Raster mit Fokus-Tag, Rahmen und Überlauf`

---

### Task 7: Mac-Fenster

**Files:**
- Modify: `apple/App/macOS/MacRootView.swift` (`SidebarView` entfällt)
- Create: `apple/App/macOS/InspectorColumn.swift`, `apple/App/macOS/CommandField.swift`
- Modify: `apple/App/Shared/EventDetailView.swift` (Zeile „Steht auch in …“), `apple/App/Shared/LichtungApp.swift` (Menübefehle ⌘K, ⌘F, Spalte ein/aus)

**Interfaces:**
- Consumes: `model.highlightedOccurrence`, `model.selectedOccurrence`, `model.alsoIn`, `model.otherOccurrences`, `QuickEntryPreview`, `model.search`, `VideoLink`, bestehende `invitationsButton()`, `MiniMonthView`.
- Produces: `InspectorColumn()` (Breite 240), `CommandField(mode: Binding<CommandField.Mode>)` mit `enum Mode { case create, search }`.

- [ ] **Step 1:** `NavigationSplitView` ersetzen durch `HStack { Raster; InspectorColumn }` in einem Fenster mit Systemtoolbar. Toolbar: Titel „November 2026, KW 46“ als Knopf mit Popover (`MiniMonthView`), `CommandField` in der Mitte, Umschalter Tag/Woche/Monat/Jahr, `‹ Heute ›`, Menü Kalender-Set, bestehende Knöpfe für Anfragen und Sync-Probleme, Knopf Spalte ein/aus (`@AppStorage("inspectorVisible")`). `QuickEntryBar` und `.searchable` am Mac entfernen.
- [ ] **Step 2:** Klick im Raster markiert nur (`highlightedOccurrence`); Doppelklick, Return, Leertaste öffnen das Blatt wie bisher über `selectedOccurrence`. Prüfen, dass der Doppelklick in leere Fläche weiter anlegt.
- [ ] **Step 3:** `InspectorColumn` nach Spec §5: (1) markierter Termin, sonst nächster von heute: Titel 17 pt halbfett, Zeit als Satz („Dienstag, 13:00 bis 15:00“), Ort, „läuft gerade“, Kalender mit Farbpunkt, „Steht auch in Arbeit, DRK“, „Beitreten“, Zu-/Absage bei offener Einladung. (2) „Heute noch“ mit Uhrzeit und Farbkante, Schluss „danach frei“ bzw. „Heute nichts mehr“. (3) „Offene Einladungen“ nur bei Bedarf. Kein Kartenrahmen.
- [ ] **Step 4:** `CommandField`: Textfeld „Termin eintragen oder suchen“, Popover darunter mit Zeile „Anlegen: <Vorschau>“ und Treffern. ⌘K → `mode = .create` (Anlegen oben und vorgewählt), ⌘F → `.search` (Treffer oben, erster vorgewählt). Return führt die vorgewählte Zeile aus (Anlegen über `model.create(from:)`, Treffer springt zum Tag und markiert), ↑/↓ wählen, Esc leert und gibt den Fokus ans Raster.
- [ ] **Step 5:** `EventDetailView`: Zeile „Steht auch in …“ mit Farbpunkten; Tipp auf einen Kalender öffnet das andere Vorkommen.
- [ ] **Step 6:** Mac-Build grün; mit `--demo` ansehen (hell, dunkel, Spalte aus, leerer Tag), Bildschirmfotos `…-task7-*.png`. Von Hand: ⌘K „Zahnarzt morgen 14 Uhr“ Return legt an; ⌘F „Sport“ Return springt hin; Tab/↑/↓ im Raster; ⌘C/⌘V.
- [ ] **Step 7:** Commit `feat(apple): Mac-Fenster mit rechter Spalte und einem Feld für Eintragen und Suchen`

---

### Task 8: iPhone

**Files:**
- Modify: `apple/App/iOS/PhoneRootView.swift`, `apple/App/Shared/AgendaView.swift` (DayTicker → Wochenstreifen), `apple/App/Shared/QuickEntryBar.swift`

**Interfaces:**
- Consumes: `DayGridView`, `model.focusDay`, `DayTickerView`, `PrimaryButtonStyle`.
- Produces: `PhoneRootView.Mode` = `day, week, month, list`; Rohwert `threeDays` wird beim Lesen zu `.week`; Vorgabe `.day`.

- [ ] **Step 1:** `Mode` umstellen, `@AppStorage("phoneViewMode")` mit Umzug des alten Werts.
- [ ] **Step 2:** Kopf je Ansicht (Spec §5 iPhone): Tag „Dienstag 10“ 32 pt (leicht + halbfett) und ein Satz zum nächsten Termin („Sport beginnt in 20 Minuten“, „Sport läuft bis 11:45“, „Heute nichts mehr“); Woche „November“ mit „KW 46“; Monat und Liste „November 2026“. Wochenstreifen (aus `DayTickerView`) in Tag und Liste. „Heute“ als Textknopf oben links, nur wenn heute nicht gezeigt wird. Zahnrad, Anfragen, Sync-Probleme, Einfügen, Rückgängig bleiben oben rechts.
- [ ] **Step 3:** Leiste unten (`safeAreaInset(edge: .bottom)`): Umschalter Tag/Woche/Monat/Liste in Kapselform, rechts runder Knopf „+“ (`Theme.light`), der die Schnelleingabe als Blatt öffnet; die feste `QuickEntryBar` unten entfällt.
- [ ] **Step 4:** Woche am iPhone: `DayGridView(start: weekStart, dayCount: 7)` mit Fokus-Tag; schmale Tage `compact`.
- [ ] **Step 5:** iOS-Build grün; im Simulator mit `--demo` alle vier Ansichten hell und dunkel ansehen (`xcrun simctl io booted screenshot`), Fotos `…-task8-*.png`. Von Hand: wischen blättert, langes Drücken verschiebt, „+“ legt per Satz an, iPad zeigt dieselbe Oberfläche ohne Bruch.
- [ ] **Step 6:** Commit `feat(apple): iPhone mit Tag als Start, Wochenstreifen und Leiste unten`

---

### Task 9: Übrige Ansichten und Abschluss

**Files:**
- Modify: `apple/App/Shared/MonthView.swift`, Jahr (`YearView`, per grep), `AgendaView.swift`, `EventDetailView.swift`, `EventEditorView.swift`, `SearchResultsView.swift`, `InvitationsView.swift`, `LoginView.swift`, `Settings/*.swift`, `apple/README.md`

- [ ] **Step 1:** Monat und Jahr: heute mit Licht-Fläche und Zahl in `Theme.light`, Balken im Stil der ganztägigen Balken, keine Wochenend-Tönung. Liste: Zeilen mit Farbkante, drei Zustände über Schrift (vorbei `text2`, läuft halbfett mit „läuft gerade“).
- [ ] **Step 2:** Detail, Editor, Anfragen, Anmeldung, Einstellungen: nur Farben, Schrift, `PrimaryButtonStyle`; kein Umbau. Anmeldung ohne grünen Verlauf.
- [ ] **Step 3:** `grep -rn "Ohne Xcode entstanden" apple/App` → in allen angesehenen Dateien entfernt. `apple/README.md`: Absatz zur Gestaltung und zu den iPhone-Ansichten an den neuen Stand anpassen.
- [ ] **Step 4:** Kit-Tests, Mac-Build, iOS-Build grün. Monat, Jahr, Liste, Detail, Editor, Einstellungen je einmal hell und dunkel ansehen, Fotos `…-task9-*.png`.
- [ ] **Step 5:** Commit `feat(apple): übrige Ansichten im neuen Stil, README`
- [ ] **Step 6:** Abschlussprüfung des ganzen Branches (Codex-Review als zweite Meinung, ohne Kundendaten) und Abgleich mit Spec §2 Anti-Ziele anhand der Bildschirmfotos.

## Abhängigkeiten

Task 1, 2, 3 sind unabhängig und können nebeneinander laufen. 4 braucht nichts davon. 5 braucht 2. 6 braucht 1, 3, 4, 5. 7 und 8 brauchen 6 und können nebeneinander laufen (verschiedene Dateien bis auf `EventDetailView`, das nur Task 7 anfasst). 9 zum Schluss.
