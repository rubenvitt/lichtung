# Apps (macOS und iOS)

- `LichtungKit/`: gemeinsamer Kern als Swift-Package. Nur Foundation, kein SwiftUI, damit `swift test` auch unter Linux läuft.
  - `QuickEntry/`: Satz-Parser der Schnelleingabe („Zahnarzt Di 14 Uhr“) und Text der Vorschau.
  - `Model/`: Kalendertag, Wochentag, Wiederholungsregel.

```sh
cd apple/LichtungKit
swift test
```

Testdaten sind erfunden (Spec §8).
