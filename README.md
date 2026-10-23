# Lichtung

Eigene Kalender-App für macOS und iOS, in Bedienung eng an Fantastical angelehnt, mit selbst gehostetem Server: synchronisiert Fastmail, iCloud und Microsoft 365 direkt, bietet einen MCP-Endpunkt für Claude und Buchungslinks für freie Zeiten.

Eine Lichtung ist die freie Stelle im Wald: die Lücke im Kalender, die andere buchen können.

## Stand

Im Aufbau. Die Spec steht unter [`docs/superpowers/specs/2026-10-23-lichtung-design.md`](docs/superpowers/specs/2026-10-23-lichtung-design.md). Der Server entsteht unter `server/`, die Apps unter `apple/`.

## Server

`lichtung-server` (Rust, axum, SQLite) liegt unter [`server/`](server), ein eigener Cargo-Workspace. Login über Pocket-ID (OIDC), API-Tokens, Geräte-Login der Apps und OAuth für claude.ai; Container und Compose-Stack nach dem Vorbild von Zunder. Konfiguration, Inbetriebnahme und lokaler Start mit Dev-Auth: [`docs/betrieb.md`](docs/betrieb.md).

```sh
cd server
cargo test --workspace
LICHTUNG_DEV_AUTH=true cargo run -p lichtung-server
```

## Lizenz

Apache-2.0
