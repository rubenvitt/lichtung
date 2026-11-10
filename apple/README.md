# Apps (macOS und iOS)

- `LichtungKit/`: gemeinsamer Kern als Swift-Package. Nur Foundation, kein SwiftUI, damit `swift test` auch unter Linux läuft.
  - `QuickEntry/`: Satz-Parser der Schnelleingabe („Zahnarzt Di 14 Uhr“) und Text der Vorschau.
  - `Model/`: Kalendertag, Wochentag, Wiederholungsregel.
  - `API/`: Objekte und Mutationen der Sync-API (`docs/api/sync-api.md`).
  - `Series/`: Serienexpansion für die Anzeige.
  - `Store/`: lokaler Stand mit Outbox (ausstehende Offline-Änderungen), Cache-Datei, Abfragen.
  - `Sync/`: HTTP-Client (Pull, Push), SSE, Geräte-Anmeldung (PKCE), Verwaltungs-API.
  - `Session/`: Server-Adresse, Ablauf der Anmeldung (Discovery, Registrierung, Token), Geräte-Token mit 401-Signal, Live-Abgleich (SSE + Senden), Startentscheidung.
  - `Settings/`: Regeln und Texte der Einstellungen (Reihenfolge, Kontostatus, Schreibschutz für Claude, Zeitzonen-Auswahl).
- `project.yml`: xcodegen-Projekt mit `Lichtung-iOS` und `Lichtung-macOS` (Bundle-Präfix `dev.rubeen`). Das `.xcodeproj` wird nicht eingecheckt.
- `scripts/testflight.sh`: archiviert, signiert und lädt die iOS-App zu TestFlight hoch. Die CI ruft es bei jedem Push auf `main` auf, der `apple/` ändert (`.github/workflows/testflight.yml`); lokal mit `XCODE_ACCOUNT=1`.
- `scripts/release-macos.sh`: baut die Mac-App für ein GitHub Release (Developer ID, Notarisierung, Zip, Sparkle-Feed `appcast.xml`). Die CI ruft es auf, wenn semantic-release auf `main` ein neues Release anlegt (`.github/workflows/release.yml`, Regeln in `/.releaserc.json`: `feat` → Minor, `fix` → Patch, `BREAKING CHANGE` → Major).
- Auto-Update (Mac): Sparkle, Feed `releases/latest/download/appcast.xml`, Menü „Lichtung → Nach Updates suchen …“ (`App/macOS/AppUpdater.swift`). Der öffentliche EdDSA-Schlüssel steht in `project.yml` (`SUPublicEDKey`), der private im Secret `SPARKLE_PRIVATE_KEY`. Secrets der Release-Pipeline: `MACOS_CERTIFICATE_P12` (Developer ID Application, base64), `MACOS_CERTIFICATE_PASSWORD`, `SPARKLE_PRIVATE_KEY`, dazu die API-Key-Secrets von TestFlight für die Notarisierung.
- `App/`: SwiftUI-Code der Apps (Tag, Woche, Monat, Liste, Jahr, Schnelleingabe, Terminanfragen, Demo-Daten, Anmeldung, Einstellungen). Die Apps sind lokal gebaut und im Demo-Modus am Mac und im iOS-Simulator angesehen; Bedienung wie Ziehen, Tastatur und Menüs hat bisher nur der Nutzer von Hand ausprobiert. Dateien, die noch den Kopfvermerk „Ohne Xcode entstanden“ tragen, sind nur in der CI gebaut (Job `apps-macos`), nicht im Demo-Modus angesehen.
  - Gestaltung zentral in `App/Shared/Theme.swift` (Spec `docs/superpowers/specs/2026-11-10-apple-ui-redesign-design.md`): keine Akzentfarbe, Farbe kommt nur von den Kalendern. Heute liegt als einziger Tag auf der Licht-Fläche. Eine Schrift (San Francisco), gleich breite Ziffern. Termine flach mit Farbkante in drei Zuständen: vorbei ohne Fläche in Text 2, kommend getönt, laufend in voller Kalenderfarbe.
  - iPhone: Tag, Woche, Monat und Liste in der Leiste unten, Start im Tag. Oben der Wochenstreifen, Wischen blättert. „+“ öffnet die Schnelleingabe als Blatt.
  - Mac: ein Fenster ohne Seitenleiste, rechts eine Spalte mit dem markierten Termin, „Heute noch“ und offenen Einladungen. Ein Feld in der Toolbar zum Eintragen und Suchen (⌘K, ⌘F). Wischen mit zwei Fingern blättert, Aufziehen ändert die Stundenhöhe.
  - Termine zusammenfassen (Einstellungen → Allgemein, je Gerät): Gleiche Termine aus mehreren Kalendern erscheinen einmal, Blocker nach Titel werden ausgeblendet. Das Detail nennt die übrigen Kalender unter „Steht auch in“. Regeln im Kern unter `Features/OccurrenceMerge.swift`.
  - Erster Start: Server-Adresse eingeben, Anmeldung im Browserfenster (`webAuthenticationSession`), Token in der Keychain. „Ohne Server ausprobieren“ startet den Demo-Modus.
  - Stand in `Application Support/Lichtung/cache.json`, SSE solange die App offen ist. Bei `401` bleiben Termine und Outbox, die App bittet um neue Anmeldung.
  - Einstellungen: Mac im Fenster „Einstellungen …“ (⌘,), iPhone über das Zahnrad in der Toolbar.
  - Erinnerungen: lokale Mitteilungen für die nächsten sieben Tage aus dem Stand (höchstens 64, iOS-Grenze), nach jedem Abgleich und beim Öffnen neu eingeplant; Tipp öffnet den Termin, Schalter unter „Allgemein“. Regeln im Kern unter `Features/Reminders.swift` (LIC-68). APNs für Termine, die die App nie gesehen hat, folgt mit LIC-27.
  - Terminanfragen: Ablage-Knopf mit Zahl in der Toolbar (iPhone: Blatt, Mac: Popover), Annehmen/Vielleicht/Ablehnen in der Liste, im Detail und im Kontextmenü. Regeln im Kern unter `Features/Invitations.swift`.
  - Buchungslinks (Openings): in den Einstellungen unter „Buchungslinks“ (Mac: eigener Reiter). Offene Einmal-Links kopieren, teilen (iPhone) und löschen (Wischen, Kontextmenü), feste Links anlegen und ändern, neuer Einmal-Link mit Dauer, Zielkalender und optional Vorschlägen; die URL landet nach dem Anlegen in der Zwischenablage. Nur mit Server, der Demo-Modus zeigt erfundene Links. Objekt im Kern unter `API/OpeningModels.swift`, Regeln und Texte unter `Features/Openings.swift`.

```sh
cd apple/LichtungKit
swift test

# auf dem Mac
cd apple
xcodegen generate
open Lichtung.xcodeproj
```

Testdaten sind erfunden (Spec §8).
