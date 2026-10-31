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
- `App/`: SwiftUI-Code der Apps (Ansichten A–D, Jahr, Schnelleingabe, Terminanfragen, Demo-Daten, Anmeldung, Einstellungen). Ohne Mac/Xcode entstanden: Gebaut wird er nur in der CI (Job `apps-macos`), auf einem Gerät gelaufen ist er noch nie.
  - Erster Start: Server-Adresse eingeben, Anmeldung im Browserfenster (`webAuthenticationSession`), Token in der Keychain. „Ohne Server ausprobieren“ startet den Demo-Modus.
  - Stand in `Application Support/Lichtung/cache.json`, SSE solange die App offen ist. Bei `401` bleiben Termine und Outbox, die App bittet um neue Anmeldung.
  - Einstellungen: Mac im Fenster „Einstellungen …“ (⌘,), iPhone über das Zahnrad in der Toolbar.
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
