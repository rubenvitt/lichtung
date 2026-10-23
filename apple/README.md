# Apps (macOS und iOS)

- `LichtungKit/`: gemeinsamer Kern als Swift-Package. Nur Foundation, kein SwiftUI, damit `swift test` auch unter Linux läuft.
  - `QuickEntry/`: Satz-Parser der Schnelleingabe („Zahnarzt Di 14 Uhr“) und Text der Vorschau.
  - `Model/`: Kalendertag, Wochentag, Wiederholungsregel.
  - `API/`: Objekte und Mutationen der Sync-API (`docs/api/sync-api.md`).
  - `Series/`: Serienexpansion für die Anzeige.
  - `Store/`: lokaler Stand mit Outbox (ausstehende Offline-Änderungen), Cache-Datei, Abfragen.
  - `Sync/`: HTTP-Client (Pull, Push), SSE, Geräte-Anmeldung (PKCE).
- `project.yml`: xcodegen-Projekt mit `Lichtung-iOS` und `Lichtung-macOS` (Bundle-Präfix `dev.rubeen`). Das `.xcodeproj` wird nicht eingecheckt.
- `App/`: SwiftUI-Code der Apps (Ansichten A–D, Jahr, Schnelleingabe, Demo-Daten). Ohne Mac/Xcode entstanden: Gebaut wird er nur in der CI (Job `apps-macos`), auf einem Gerät gelaufen ist er noch nie.

```sh
cd apple/LichtungKit
swift test

# auf dem Mac
cd apple
xcodegen generate
open Lichtung.xcodeproj
```

Testdaten sind erfunden (Spec §8).
