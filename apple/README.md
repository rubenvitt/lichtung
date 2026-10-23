# Apps (macOS und iOS)

- `LichtungKit/`: gemeinsamer Kern als Swift-Package. Nur Foundation, kein SwiftUI, damit `swift test` auch unter Linux läuft.
  - `QuickEntry/`: Satz-Parser der Schnelleingabe („Zahnarzt Di 14 Uhr“) und Text der Vorschau.
  - `Model/`: Kalendertag, Wochentag, Wiederholungsregel.
  - `API/`: Objekte und Mutationen der Sync-API (`docs/api/sync-api.md`).
  - `Series/`: Serienexpansion für die Anzeige.
  - `Store/`: lokaler Stand mit Outbox (ausstehende Offline-Änderungen), Cache-Datei, Abfragen.
  - `Sync/`: HTTP-Client (Pull, Push), SSE, Geräte-Anmeldung (PKCE).

```sh
cd apple/LichtungKit
swift test
```

Testdaten sind erfunden (Spec §8).
