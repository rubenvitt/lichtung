# lichtung-caldav

CalDAV-Protokollschicht für Lichtung: Discovery, Kalenderliste, Delta-Sync per `sync-collection` (RFC 6578) mit ETag/ctag-Fallback, `calendar-multiget`, Schreiben mit `If-Match`/`If-None-Match`. iCalendar wird nur als String transportiert, nicht geparst.

## Tests

```sh
cd server
cargo test -p lichtung-caldav
```

Die normalen Tests laufen gegen einen Mock-Server mit erfundenen Antworten im Stil von Fastmail und iCloud. `tests/icloud.rs` deckt die iCloud-Besonderheiten ab: Discovery über einen Partitions-Host, Erinnerungslisten, `inbox`/`outbox`/`notification`, geteilte Kalender (`cs:shared`, `cs:shared-owner`, `cs:invite`, nur lesende Freigaben), Sync-Tokens im iCloud-Stil, 403 `valid-sync-token`, 401, 403 und 503 mit `Retry-After`.

Zugangsdaten gehen an den Ursprung der Basis-URL und per HTTPS an Hosts derselben Elterndomain. Bei iCloud sind das `caldav.icloud.com` und `pNN-caldav.icloud.com`, auch wenn die href den Port `:443` ausdrücklich nennt. `DAV:owner` wird nur über den Pfad mit dem Principal verglichen, weil iCloud den Principal auf `caldav.icloud.com` meldet, die Kalender aber auf dem Partitions-Host liegen.

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
