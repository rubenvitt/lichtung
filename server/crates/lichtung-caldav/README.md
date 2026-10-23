# lichtung-caldav

CalDAV-Protokollschicht für Lichtung: Discovery, Kalenderliste, Delta-Sync per `sync-collection` (RFC 6578) mit ETag/ctag-Fallback, `calendar-multiget`, Schreiben mit `If-Match`/`If-None-Match`. iCalendar wird nur als String transportiert, nicht geparst.

## Tests

```sh
cd server
cargo test -p lichtung-caldav
```

Die normalen Tests laufen gegen einen Mock-Server mit erfundenen Antworten im Stil von Fastmail und iCloud.

### Radicale-Integrationstest

Der Test `tests/radicale.rs` ist mit `#[ignore]` markiert und spricht mit einem echten CalDAV-Server. So lässt er sich lokal mit Radicale laufen:

```sh
python3 -m venv /tmp/radicale-venv
/tmp/radicale-venv/bin/pip install radicale

cat > /tmp/radicale.conf <<'EOF'
[server]
hosts = 127.0.0.1:5232

[auth]
type = none

[storage]
filesystem_folder = /tmp/radicale-data
EOF

/tmp/radicale-venv/bin/radicale --config /tmp/radicale.conf &

cd server
LICHTUNG_TEST_CALDAV_URL=http://127.0.0.1:5232/ \
  cargo test -p lichtung-caldav --test radicale -- --ignored
```

Optional: `LICHTUNG_TEST_CALDAV_USER` und `LICHTUNG_TEST_CALDAV_PASSWORD` (Standard: `lichtung`, leer). Der Test legt einen eigenen Kalender an und löscht ihn am Ende wieder.
