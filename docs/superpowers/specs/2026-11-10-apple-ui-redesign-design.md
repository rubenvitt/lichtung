# Lichtung – Neugestaltung der Apple-Apps

- **Datum:** 2026-11-10
- **Status:** Entwurf, wartet auf Freigabe
- **Ergänzt:** `2026-10-23-lichtung-design.md` §5 (Ansichten). Wo sich beide widersprechen, gilt dieses Dokument.
- **Vorlage zum Ansehen:** `assets/2026-11-10-apple-ui-mockup.html` (im Browser öffnen, hell/dunkel umschaltbar, Tage und Termine anklickbar). Sie ist verbindlich für Aufbau, Farben und Rangfolge, nicht für jeden Pixel.

## 1. Warum

Die Apps sind ohne Xcode entstanden und nie auf einem Gerät gelaufen. Der erste echte Blick (Screenshot KW 46) zeigt: Die Wochenansicht ist kaum benutzbar.

- Ein einziger langer Termin zwingt alle anderen Termine des Tages in schmale Streifen.
- „Blockiert“-Termine und derselbe Termin in mehreren Kalendern stehen mehrfach nebeneinander.
- Drei Schriften (Serife, gerundet, Standard) ohne erkennbare Regel.
- Die Seitenleiste listet die Woche ein zweites Mal auf und nimmt ein Drittel des Fensters.

Ziel in den Worten des Nutzers: „ein toller Kalender, für macOS und iOS“, „radikal“ überarbeitet. Zwei frühere Entwürfe fielen durch, weil sie „sehr stark nach KI-generiert“ aussahen (Neongrün auf Fast-Schwarz, Karten, Leuchten, Kapseln, Etiketten in Großbuchstaben). Entwurf 3 wurde angenommen.

## 2. Ziele, Nicht-Ziele, Anti-Ziele

**Ziele**

1. Woche und Tag sind auf einen Blick lesbar, auch an vollen Arbeitstagen.
2. Die App hat ein eigenes, ruhiges Gesicht, das auf macOS und iOS zu Hause ist.
3. Mac und iPhone folgen derselben Idee: ein Tag im Mittelpunkt.
4. Blocker und Mehrfach-Termine stören nicht mehr, gehen aber nicht verloren.

**Nicht-Ziele**

- Keine neuen Funktionen außer der Zusammenfassung von Terminen (§6). Blocker selbst anlegen ist LIC-75.
- Keine Änderung an Server, Sync-API oder MCP.
- Kein Umbau von Editor, Einstellungen, Anmeldung, Terminanfragen, Buchungslinks. Sie bekommen nur die neuen Farben und Schriften (§4), ihr Aufbau bleibt. *(Meine Setzung.)*
- Kein eigenes iPad-Layout. Das iPad zeigt wie bisher die iPhone-Oberfläche. *(Meine Setzung.)*
- Bestehende Bedienung bleibt erhalten: Ziehen im Raster, Tastatur, Kontextmenüs, Kopieren/Einfügen, Rückgängig, Reisezeit, zweite Zeitzone.

**Anti-Ziele**

- Es sieht wieder nach Baukasten aus: Karten mit Schatten, Leuchteffekte, Kapseln überall, Großbuchstaben-Etiketten, mit „·“ verkettete Angaben, eine grelle Akzentfarbe.
- Termine werden so schmal, dass ihr Titel nicht mehr lesbar ist.
- Ein ausgeblendeter Blocker verdeckt einen echten Termin.

## 3. Leitidee

**Heute ist die Lichtung.** Die Oberfläche hat keine eigene Akzentfarbe. Der heutige Tag ist die einzige Spalte, auf die Licht fällt. Farbe kommt nur von den Kalendern des Nutzers. Das ist die eine auffällige Sache; alles andere bleibt still.

## 4. Gestaltung (`Theme.swift`)

### Farben

| Name | dunkel | hell | Verwendung |
|---|---|---|---|
| Grund (`shade`) | `#1B1D1B` | `#ECEEE8` | Fenster, alle Tage außer heute |
| Licht-Fläche (`lit`) | `#F4E9C9` mit 7,5 % Deckung | `#FFFFFF` | Spalte von heute |
| Licht (`light`) | `#F4E9C9` | `#1B1D1A` | Zahl von heute, Hauptknopf (Beitreten, +), Auswahl in Reglern |
| auf Licht (`onLight`) | `#1B1D1B` | `#FFFFFF` | Schrift auf Hauptknöpfen |
| Text 1 / 2 / 3 | `#ECEBE4` / `#9A9C93` / `#63655E` | `#1B1D1A` / `#5F635B` / `#A1A49C` | Rangfolge. Text 3 nur für Stundenzahlen und Linienbeschriftung, nie für Termintitel |
| Sonne (`sun`) | `#F0B840` | `#C58500` | Jetzt-Linie und ihre Uhrzeit, sonst nichts |
| Linie | Weiß 6 % | Schwarz 7 % | Stundenlinien, Trennungen |

- `Theme.accent`, `accentDeep`, `now`, `onNow` (Waldgrün, Rot-Orange) entfallen. Das Asset `AccentColor` wird auf `light` gesetzt, damit Systemregler nicht grün sind.
- Im dunklen Modus verläuft die Licht-Fläche senkrecht: voll zwischen etwa 8 und 17 Uhr, zu den Rändern auslaufend. *(Meine Setzung: feste Uhrzeiten, kein Sonnenstand; im Code als `shortcut:` vermerken.)* Im hellen Modus ist die Spalte einfach weiß.
- Wochenenden bekommen keine eigene Tönung mehr; ihre Tageszahl steht in Text 2.

### Schrift

- Nur San Francisco (System). `Theme.display` (Serife) und die gerundeten Ziffern entfallen.
- Ziffern in Uhrzeiten und Tageszahlen: gleich breit (`monospacedDigit`).
- Schmale Tagesspalten setzen Termintext in schmaler Breite (`.fontWidth(.condensed)`), der Fokus-Tag in normaler.
- Der Fokus-Tag ist ausgeschrieben und groß: „Dienstag“ leicht, „10“ halbfett (etwa 26 pt am Mac, 32 pt am iPhone). Andere Tage: „Mo 9“ in Textgröße.
- Keine Großbuchstaben-Etiketten (`SectionLabel` entfällt bzw. wird normale halbfette Zeile). Keine mit „·“ verketteten Statuszeilen; ganze Sätze („Sport beginnt in 20 Minuten“).

### Formen

- Termine: Radius 4, flach, kein Schatten, kein Leuchten. Hauptknöpfe Radius 7. Kapselform nur dort, wo das System sie von sich aus zeichnet (Toolbar, Umschalter) und an der Leiste unten am iPhone.
- Toolbars, Umschalter und Blätter sind Systembausteine. Kein selbst gebautes Glas; was das System beim Bauen mit dem aktuellen SDK liefert, wird genommen.

### Termin im Raster: drei Zustände

| Zustand | Aussehen |
|---|---|
| vorbei | keine Fläche, Farbkante links mit 40 % Deckung, Titel in Text 2 |
| kommt | Fläche in Kalenderfarbe (dunkel 30 %, hell 20 % auf Grund), volle Farbkante 3 pt, Titel in Text 1 |
| läuft | volle Kalenderfarbe, dunkle Schrift |

- Ausgewählt: 1,5 pt Rahmen in Text 1, 1 pt Abstand.
- Ein Termin unter etwa 28 pt Höhe ist eine Zeile: Titel, im Fokus-Tag dahinter die Startzeit. Darüber: Titel, darunter „13:00–15:00“, im Fokus-Tag mit Ort.
- Im Fokus-Tag zeigt ein kommender oder laufender Termin mit Videolink (`VideoLink`) ab etwa 60 pt Höhe den Knopf „Beitreten“.
- Mindestkontrast für jeden Text 4,5:1 in beiden Modi; „vorbei“ deshalb Text 2 statt Text 3. Zustand nie nur über Farbe: VoiceOver nennt „vorbei“ bzw. „läuft gerade“.

## 5. Aufbau

### Raster (Tag, Woche) – `DayGridView`, Rechnung in `LichtungKit/Views/GridLayout.swift`

**Fokus-Tag.** In der Woche ist ein Tag etwa 2,1-mal so breit wie die übrigen. Er ist ein eigener Zustand (`focusDay`), nicht `selectedDay`: Er wechselt nur durch Klick oder Tipp auf einen Tageskopf, durch ←/→ (Tag für Tag), durch „Heute“, durch Blättern (gleicher Wochentag der neuen Woche) und durch einen Sprung aus Suche oder Mini-Monat. Ein Klick in leere Fläche oder das Wandern der Markierung mit ↑/↓/Tab ändert die Breiten nicht, damit nichts unter dem Zeiger wegrutscht. *(Meine Setzung.)* Die Breiten gleiten (0,3 s, bei „Bewegung reduzieren“ ohne Übergang). Beim Öffnen ist heute der Fokus-Tag. Ganztägige Balken und das Ziehen in andere Tage rechnen mit den ungleichen Spaltenbreiten. In der Tagesansicht gibt es nur den Fokus-Tag.

**Nebeneinander nur bei echter Überschneidung.** Die Stunde ist am Mac mindestens 56 pt hoch, damit 15 Minuten eine lesbare Zeile ergeben. Termine unter 15 Minuten gelten für Darstellung und Überschneidung als 15 Minuten lang (wie bisher `minimumMinutes`); folgt der nächste Termin innerhalb dieser Zeit, stehen die beiden nebeneinander. Darüber hinaus gibt es keine künstliche Verlängerung.

**Lange Termine sind Rahmen.** Ein zeitgebundener Termin von mindestens 4 Stunden, der sich mit mindestens einem anderen Termin des Tages überschneidet, wird nicht in die Spaltenrechnung genommen. Er erscheint als schmale Linie (1,5 pt) am linken Spaltenrand über seine ganze Dauer, mit seinem Titel in klein am oberen Ende (im Fokus-Tag mit Zeit). Die übrigen Termine rücken in seinem Zeitraum um etwa 10 pt ein. Der Rahmen ist anklickbar (Linie und Titel) und per Tastatur erreichbar wie jeder Termin; verschieben und in der Länge ändern geht für Rahmen über Detail/Editor, nicht per Ziehen. *(Meine Setzung.)* Ein langer Termin ohne Überschneidung bleibt ein normaler Block. Mehrere Rahmen am selben Tag liegen als Linien nebeneinander (je 5 pt).

- „Mindestens 4 Stunden“ meint den Teil, der an diesem Tag liegt (22–6 Uhr ist an keinem der beiden Tage ein Rahmen).
- Zum Rahmen macht nur die Überschneidung mit einem Termin, der selbst kein langer ist. Zwei lange Termine, die sich nur gegenseitig überschneiden, bleiben Blöcke nebeneinander.
- Zustände am Rahmen: vorbei = Linie mit 40 % und Titel in Text 2, läuft = volle Linie und Titel in Text 1.

**Wenn es trotzdem eng wird.** Mehr als drei Spalten nebeneinander in einem schmalen Tag bzw. mehr als fünf im Fokus-Tag: die ersten bleiben, der Rest wird zu einem Feld „+2“, das den Tag zum Fokus-Tag macht (schmaler Tag) bzw. die Termine als Liste zeigt (Fokus-Tag). *(Meine Setzung.)*

**Jetzt-Linie.** 1,5 pt in Sonne, nur in der Spalte von heute, hinter den Terminblöcken. Die Uhrzeit steht in der Stundenspalte auf Höhe der Linie und verdrängt dort die Stundenzahl.

**Kopf.** Kein KW-Feld mehr im Raster (steht in der Toolbar). Ganztägige Termine wie bisher als Balken, im neuen Stil (Radius 3, Fläche in Kalenderfarbe, keine Emoji-Sonderbehandlung).

Rahmen-Erkennung, Spaltenrechnung und „+n“ sind reine Rechnung im Kit und bekommen Tests (`LayoutFixtures` erweitern), darunter der Fall aus dem Screenshot: Termin 8–17 Uhr plus drei kurze Termine ergibt einen Rahmen und volle Breite für die kurzen.

### Mac-Fenster – `MacRootView`

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ● ● ●  November 2026, KW 46   [ Termin eintragen oder suchen ⌘K ]  [Tag Woche Monat Jahr] [‹ Heute ›] │
├───────────────────────────────────────────────────┬──────────────────────┤
│  Mo 9   Dienstag 10        Mi 11  Do 12  Fr 13 …  │ Vorbereitung KI@…    │
│  ganztägige Balken                                │ Dienstag, 13 bis 15  │
│  ┌────┬──────────────┬────┬────┬────┬────┬────┐   │ [ Beitreten ]        │
│  │    │  (Licht)     │    │    │    │    │    │   │                      │
│  │    │              │    │    │    │    │    │   │ Heute noch           │
│  │    │              │    │    │    │    │    │   │ 11:00 Sport          │
│  └────┴──────────────┴────┴────┴────┴────┴────┘   │ Offene Einladungen   │
└───────────────────────────────────────────────────┴──────────────────────┘
```

- **Die linke Seitenleiste entfällt.** Ihr Inhalt zieht um:
  - Mini-Monat: Popover am Titel „November 2026“.
  - Kalender-Set: Menü in der Toolbar.
  - Liste der nächsten Termine: ersetzt durch die rechte Spalte.
- **Rechte Spalte** (etwa 240 pt, ein-/ausblendbar, kein Kartenrahmen, nur Abstand). Von oben:
  1. Der ausgewählte Termin: Titel, Zeit als Satz, Ort, „läuft gerade“, Kalender und „steht auch in …“ (§6), Hauptknopf „Beitreten“ bei Videolink, Zu-/Absage bei Einladungen. Ohne Auswahl: der nächste Termin von heute. Ein Klick auf einen Termin wählt ihn aus und zeigt ihn hier; das Blatt mit `EventDetailView` öffnet erst Doppelklick, Return oder Leertaste. *(Meine Setzung; bisher öffnete der einfache Klick das Blatt.)*
  2. „Heute noch“: restliche Termine von heute mit Uhrzeit und Farbkante, danach „danach frei“ mit der Uhrzeit. Ist nichts mehr: „Heute nichts mehr“.
  3. „Offene Einladungen“, nur wenn es welche gibt; Klick öffnet die bestehende Liste.
- **Ein Feld für Eintragen und Suchen** in der Mitte der Toolbar, ⌘K und ⌘F setzen den Fokus. Beim Tippen erscheint darunter eine Liste: oben „Anlegen: …“ mit der Vorschau der Schnelleingabe (`QuickEntryPreview`), darunter Suchtreffer (`EventSearch`). Return nimmt die vorgewählte Zeile, ↑/↓ wählen, Esc schließt. Vorgewählt ist nach ⌘K die Zeile „Anlegen“, nach ⌘F der erste Treffer (dann stehen die Treffer oben). *(Meine Setzung.)* Die Leiste `QuickEntryBar` unten und `.searchable` entfallen am Mac. ⌘N öffnet weiter den Editor.
- **Trackpad-Gesten** *(Nachtrag 2026-11-10, Wunsch des Nutzers)*: Waagerechtes Wischen mit zwei Fingern blättert im Raster, im Monat und im Jahr um eine Einheit (Woche, Tag, Monat, Jahr), ein Schritt je Geste, Richtung wie beim Scrollen des Systems. Senkrechtes Scrollen bleibt unberührt; eine überwiegend senkrechte Bewegung blättert nicht. Aufziehen mit zwei Fingern (Pinch) ändert im Raster die Höhe der Stunde zwischen 40 und 120 pt; der Wert wird gemerkt.
- Monat und Jahr behalten ihren Aufbau und bekommen Farben, Schrift und die Licht-Fläche für heute.

### iPhone – `PhoneRootView`

```
┌─────────────────────────┐
│ Dienstag 10        ⚙ ✉ │  großer Titel, Knöpfe wie bisher
│ Sport beginnt in 20 Min │  ein Satz zum nächsten Termin
│ Mo Di Mi Do Fr Sa So    │  Wochenstreifen, wischbar
│  9 10 11 12 13 14 15    │
│ ganztägig               │
│ ┌─────────────────────┐ │
│ │ Raster des Tages    │ │
│ └─────────────────────┘ │
│ ( Tag  Woche  Monat  Liste )  (+) │  unten, am Daumen
└─────────────────────────┘
```

- Kopf je Ansicht: Tag „Dienstag 10“ mit dem Satz zum nächsten Termin; Woche „November“ mit „KW 46“ darunter; Monat und Liste „November 2026“. Der Wochenstreifen steht in Tag und Liste. „Heute“ ist ein Textknopf oben links, sichtbar nur, wenn heute nicht gezeigt wird. *(Meine Setzung.)*
- Start ist der Tag (bisher die Liste). *(Meine Setzung; die zuletzt gewählte Ansicht wird weiter gemerkt.)*
- Ansichten: Tag, Woche, Monat, Liste. „3 Tage“ entfällt, weil die Woche mit Fokus-Tag das abdeckt. *(Meine Setzung.)* Ein gespeicherter Wert „3 Tage“ wird zu „Woche“.
- Woche am iPhone: dasselbe Raster mit Fokus-Tag; schmale Tage zeigen nur Farbflächen mit Titelanfang.
- Der Umschalter wandert von oben nach unten in eine Leiste; rechts daneben der runde Knopf „+“. Tipp öffnet die Schnelleingabe, wie bisher der runde Knopf. Die Suche bleibt über das System (`.searchable`).
- Seitliches Wischen blättert wie bisher.
- Liste (`AgendaView`) und Monat behalten ihren Aufbau, im neuen Stil; der DayTicker der Liste wird der Wochenstreifen von oben.

## 6. Termine zusammenfassen (neu, im Kit)

Neuer Baustein in `LichtungKit` (Vorschlag `Features/OccurrenceMerge.swift`), rein rechnend und getestet. Er sitzt zwischen `CalendarQuery.occurrences` und den Ansichten und gilt in Raster, Liste, Monat, Jahr, rechter Spalte und für lokale Erinnerungen. Die Suche zeigt weiter alles.

1. **Gleicher Termin in mehreren Kalendern.** Vorkommen mit gleichem Beginn, gleichem Ende, gleicher Ganztägigkeit und gleichem Titel (ohne Rand-Leerzeichen, Groß/Klein egal) in verschiedenen Kalendern werden zu einem. Gezeigt wird, in dieser Rangfolge: das Vorkommen mit offener Einladung (damit Zu- und Absage erreichbar bleiben), sonst eines aus einem schreibbaren Kalender, sonst nach der Reihenfolge des Nutzers (`CalendarOrder`).
2. **Blocker-Regel.** Ein Vorkommen, dessen Titel einem der eingestellten Blocker-Titel entspricht (ohne Rand-Leerzeichen, Groß/Klein egal) und das genau denselben Beginn und dasselbe Ende hat wie ein anderes, nicht als Blocker erkanntes Vorkommen in einem anderen Kalender, wird ausgeblendet. Ein Blocker ohne solchen Partner bleibt sichtbar.
3. **Am Termin sichtbar.** Der gezeigte Termin kennt die Kalender der zusammengefassten Vorkommen. Rechte Spalte und Detail zeigen „Steht auch in Arbeit, DRK“ mit Farbpunkten; im Detail lässt sich jedes der anderen Vorkommen öffnen. Ändern und Löschen wirken nur auf das gezeigte Vorkommen.
   Erinnerungen gehen nicht verloren: `ReminderPlanner` bekommt je Gruppe die Erinnerungszeiten aller zusammengefassten Vorkommen (auch des Blockers) und plant jede Zeit einmal. Hat ein ausgeblendeter Blocker selbst eine offene Einladung, wird er nicht ausgeblendet.
4. **Einstellung** unter „Allgemein“: Schalter „Blocker ausblenden“ (an) und die Liste der Titel (Vorgabe: „Blockiert“). Ein zweiter Schalter „Gleiche Termine zusammenfassen“ (an). Gespeichert je Gerät in `UserDefaults`. *(Meine Setzung: `shortcut:` kein Abgleich zwischen Geräten, bis LIC-75 echte Verknüpfungen bringt.)*

Im Raster und in der Liste ist ein zusammengefasster Termin nicht besonders markiert; der Hinweis steht erst in der rechten Spalte und im Detail. *(Meine Setzung.)*

## 7. Umsetzung und Prüfung

- Reihenfolge: (1) Kit: Rasterrechnung und Zusammenfassen mit Tests, (2) Theme und Termin-Darstellung, (3) Raster mit Fokus-Tag, (4) Mac-Fenster, (5) iPhone, (6) übrige Ansichten im neuen Stil, README anpassen.
- Xcode ist auf dem Entwicklungsrechner vorhanden. Jede Etappe wird lokal gebaut (macOS und iOS-Simulator) und im Demo-Modus angesehen; Bildschirmfotos hell und dunkel gehören zum Abschluss. Der Vermerk „Ohne Xcode entstanden“ fällt in Dateien weg, die so geprüft wurden.
- `DemoData` bekommt eine volle Woche nach dem Muster von KW 46: ein Tag mit Rahmen, ein Blocker-Paar, ein Termin in mehreren Kalendern, kurze Termine dicht hintereinander.
- `swift test` im Kit bleibt grün.
- Mindestversionen bleiben iOS 18 und macOS 15.

## 8. Dem Builder überlassen

- Genaue Maße, Abstände und Schriftgrößen, solange die Vorlage erkennbar bleibt.
- Wie die ungleichen Spaltenbreiten in `GridInteraction` und `LaneLayout` einfließen.
- Technik des Feldes in der Mac-Toolbar (Popover, eigenes Fenster), solange Tastatur und VoiceOver gehen.
- Gestaltung von Monat, Jahr, Liste, Detail, Editor und Einstellungen innerhalb der Regeln aus §4.
- Ob die rechte Spalte unter einer Fensterbreite von selbst verschwindet.

## 9. Risiken

- **Rahmen sind neu.** Ob eine dünne Linie als Termin verstanden wird, zeigt erst der Alltag. Rückweg: Rahmen als blasse Fläche hinter den Terminen.
- **Einfacher Klick öffnet kein Blatt mehr.** Gewöhnung; Doppelklick und Return bleiben.
- **Titelregel für Blocker** blendet einen echten Termin aus, der zufällig „Blockiert“ heißt und zeitgleich mit einem anderen liegt. Er bleibt über „Steht auch in …“ erreichbar; die Regel ist abschaltbar.
- **Der Code ist nie gelaufen.** Beim ersten lokalen Bauen können Fehler auftauchen, die mit der Gestaltung nichts zu tun haben. Sie werden behoben, soweit sie die Ansichten betreffen, sonst gemeldet.
