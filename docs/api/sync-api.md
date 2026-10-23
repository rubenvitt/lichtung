# Sync-API von lichtung-server (Vertrag, Version 1)

- **Stand:** 2026-10-23, Entwurf zur Umsetzung (LIC-11). Der Swift-Sync-Client (LIC-16) baut gegen diesen Vertrag.
- **Änderungen** am Vertrag stehen unten im Änderungsprotokoll und werden zusätzlich als Kommentar an LIC-16 gemeldet.
- Spec: [`docs/superpowers/specs/2026-10-23-lichtung-design.md`](../superpowers/specs/2026-10-23-lichtung-design.md), Abschnitte 3–5.

Der Server ist die Autorität. Die Apps halten einen lokalen Cache, lesen ihn über **Pull mit Cursor** nach, schicken eigene Änderungen als **Mutationen** (auch offline gesammelt) und erfahren über **SSE**, dass es Neues gibt.

## 1. Grundlagen

| Thema | Regel |
|---|---|
| Basis | `https://<server>/api/…`, nur HTTPS (lokal auch `http://localhost`) |
| Format | JSON, UTF-8. Unbekannte Felder in Antworten ignoriert der Client (der Server darf Felder ergänzen, ohne die Version zu erhöhen). |
| Zeitpunkte | RFC 3339 in UTC mit `Z`, z. B. `2026-11-03T13:00:00Z` |
| IDs | undurchsichtige Strings mit Präfix (`acc_…`, `cal_…`, `set_…`, `ev_…`, `cf_…`). Nie parsen. |
| Fehler | HTTP-Status plus `{"error": {"code": "…", "message": "…"}}`. `code` ist stabil (snake_case, Tabelle in Abschnitt 9), `message` ist deutsch und für Menschen. |
| Auth | `Authorization: Bearer ld_…` (Geräte-Token, Abschnitt 2). Ohne gültiges Token: `401` mit `code: unauthorized`. |

### Datumswerte (`DateValue`)

Ein Termin beginnt entweder an einem **Tag** (ganztägig) oder zu einem **Zeitpunkt in einer Zeitzone**:

```json
{"date": "2026-11-03"}
```

```json
{"date_time": "2026-11-03T14:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-03T13:00:00Z"}
```

- `date_time` ist die **Wanduhrzeit** ohne Offset. `time_zone` ist ein IANA-Name. Der Server übersetzt Windows-Zonennamen und eigene VTIMEZONEs anbieterseitig nach IANA (bestmöglich).
- `time_zone: null` heißt **floating** (gilt in der Zeitzone des Geräts); dann ist auch `utc` `null`.
- `utc` liefert nur der Server (zum bequemen Sortieren). Schickt der Client einen `DateValue`, ignoriert der Server `utc`.
- Bei ganztägigen Terminen ist `end` **exklusiv**: ein eintägiger Termin am 3.11. hat `start {"date":"2026-11-03"}`, `end {"date":"2026-11-04"}`.

## 2. Anmeldung eines Geräts

Die Apps melden sich wie die Zunder-App über den eingebauten OAuth-Server an (Authorization Code mit PKCE, öffentlicher Client). Das Ergebnis ist ein **Geräte-Token** `ld_…`, das nicht abläuft, nur für `/api` gilt und in der Geräteliste einzeln widerrufen wird.

1. **Discovery:** `GET /.well-known/oauth-authorization-server` → `authorization_endpoint`, `token_endpoint`, `registration_endpoint`, `revocation_endpoint`.
2. **Registrieren** (einmal pro Installation): `POST /oauth/register`
   ```json
   {"client_name": "iPhone", "redirect_uris": ["dev.rubeen.lichtung:/oauth"],
    "token_endpoint_auth_method": "none", "grant_types": ["authorization_code"], "response_types": ["code"]}
   ```
   → `201` mit `client_id`. `client_name` wird zum Gerätenamen in der Liste.
3. **Autorisieren** in `ASWebAuthenticationSession`:
   `GET /oauth/authorize?response_type=code&client_id=…&redirect_uri=dev.rubeen.lichtung:/oauth&code_challenge=…&code_challenge_method=S256&state=…&resource=<public-url>/api`
   Der Server leitet bei Bedarf zum Pocket-ID-Login weiter, zeigt eine Bestätigungsseite und ruft dann `dev.rubeen.lichtung:/oauth?code=…&state=…` auf.
4. **Token holen:** `POST /oauth/token` (`application/x-www-form-urlencoded`):
   `grant_type=authorization_code&code=…&redirect_uri=…&client_id=…&code_verifier=…&resource=<public-url>/api`
   → `{"access_token": "ld_…", "token_type": "Bearer"}` (kein `expires_in`, kein `refresh_token`).
5. **Abmelden:** `POST /oauth/revoke` mit `token=ld_…` oder in der Geräteliste (`DELETE /api/devices/{id}`).

`401` auf einen Aufruf mit Geräte-Token heißt: Token widerrufen, neu anmelden. Der lokale Cache darf bleiben, ausstehende Mutationen auch.

## 3. Objekte

### Account

```json
{
  "id": "acc_01",
  "kind": "fastmail",
  "name": "Fastmail",
  "username": "person@example.org",
  "status": {"state": "ok", "message": null, "last_sync_at": "2026-11-03T06:00:12Z"}
}
```

- `kind`: `fastmail`, `icloud`, `ics`, später `microsoft`.
- `username`: Anmeldename (bei `ics` die URL ohne Zugangsdaten). Passwörter verlassen den Server nie.
- `status.state`: `ok`, `syncing`, `error` (vorübergehend, `message` sagt warum), `auth_failed` (Zugangsdaten erneuern).
- *Ergänzung (LIC-8):* Der Server synchronisiert jedes Konto beim Start und danach alle 5 Minuten. Nach `error` versucht er es mit wachsendem Abstand erneut (1, 2, 4 … höchstens 60 Minuten), `last_sync_at` bleibt dabei der letzte erfolgreiche Abgleich. Ein Konto auf `auth_failed` versucht er nicht mehr von selbst, bis neue Zugangsdaten gespeichert sind (`PATCH /api/accounts/{id}`); `POST /api/accounts/{id}/sync` versucht es trotzdem. `message` bei `error` nennt den betroffenen Kalender, z. B. „Kalender „Privat“: Kalenderanbieter derzeit nicht erreichbar: HTTP 503“.
- *Ergänzung (LIC-7):* `kind: caldav` ist ein beliebiger CalDAV-Server mit eigener Adresse (Selbsthoster, Radicale in Tests). Nur bei diesem `kind` enthält das Account zusätzlich `url` (Server-Adresse). Clients, die `caldav` nicht kennen, zeigen das Konto wie ein unbekanntes an.

### Calendar

```json
{
  "id": "cal_01",
  "account_id": "acc_01",
  "name": "Privat",
  "color": "#3B82F6",
  "provider_color": "#3B82F6",
  "writable": true,
  "can_invite": true,
  "hidden": false,
  "mcp_write": true,
  "openings_busy": true,
  "all_day_blocks": false,
  "position": 0
}
```

- `writable`: Rechte beim Anbieter (bei geteilten Kalendern die Rechte der Freigabe, bei ICS immer `false`).
- `can_invite`: Termine mit Teilnehmern verschicken echte Einladungen.
- Einstellungen (in der App änderbar, Abschnitt 7): `color` (überschreibt `provider_color`), `hidden`, `mcp_write` (darf Claude hier schreiben; Standard `false` für den Arbeitskalender), `openings_busy` (zählt für freie Zeiten), `all_day_blocks` (ganztägige Termine blockieren), `position`.
- *Ergänzung (LIC-7):* `color` ist immer gesetzt: die eigene Farbe, sonst `provider_color`, sonst eine feste Ersatzfarbe. `provider_color` ist `null`, wenn der Anbieter keine Farbe meldet. Farben haben die Form `#RRGGBB` (Großbuchstaben).
- *Ergänzung (LIC-7):* Standardwerte beim Anlegen: `mcp_write` = `writable` (bei `microsoft` später `false`), `openings_busy` `true`, `all_day_blocks` `false`, `hidden` `false`; `position` hängt neue Kalender hinten an (innerhalb eines Kontos in der Reihenfolge des Anbieters). Sammlungen, die nur Aufgaben (VTODO) halten, werden nicht als Kalender übernommen.
- *Ergänzung (LIC-13):* Neues Feld `shared` (bool): `true` für Kalender anderer, die mit dem Konto geteilt sind (z. B. iCloud-Freigaben). Jede Freigabe ist ein eigener Kalender. Eigene Kalender, die man selbst mit anderen teilt, haben `shared: false`. Bei `writable: false` ist `mcp_write` immer `false`: `PATCH` mit `mcp_write: true` lässt den Wert auf `false`, und wird eine Freigabe beim Anbieter nur lesbar, setzt der nächste Abgleich `mcp_write` auf `false`.

### CalendarSet

```json
{"id": "set_01", "name": "Arbeit", "calendar_ids": ["cal_02", "cal_05"], "position": 1}
```

### Event

Ein Event ist **ein iCalendar-Objekt** (eine UID in einem Kalender): ein Einzeltermin oder eine Serie mit ihren geänderten Vorkommen. Die Apps expandieren Serien selbst für die Anzeige (Swift-Kern); der Server liefert die Regel und die Ausnahmen.

```json
{
  "id": "ev_01",
  "calendar_id": "cal_01",
  "uid": "7d1f0c9e-2b1a-4c55-9d0e-000000000001@example.org",
  "version": 4,
  "title": "Team-Sync",
  "location": "Raum 2",
  "notes": "Agenda im Dokument",
  "url": null,
  "start": {"date_time": "2026-11-02T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-02T08:00:00Z"},
  "end": {"date_time": "2026-11-02T09:30:00", "time_zone": "Europe/Berlin", "utc": "2026-11-02T08:30:00Z"},
  "all_day": false,
  "status": "confirmed",
  "availability": "busy",
  "recurrence": {
    "rrule": "FREQ=WEEKLY;BYDAY=MO;UNTIL=20270201T075959Z",
    "rdates": [],
    "exdates": [{"date_time": "2026-11-16T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-16T08:00:00Z"}]
  },
  "overrides": [
    {
      "recurrence_id": {"date_time": "2026-11-09T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T08:00:00Z"},
      "title": "Team-Sync (verschoben)",
      "location": "Raum 2",
      "notes": "Agenda im Dokument",
      "url": null,
      "start": {"date_time": "2026-11-09T10:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T09:00:00Z"},
      "end": {"date_time": "2026-11-09T10:30:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T09:30:00Z"},
      "all_day": false,
      "status": "confirmed",
      "availability": "busy",
      "organizer": null,
      "attendees": [],
      "self_status": null,
      "alarms": [{"minutes_before": 10}],
      "travel_time_minutes": null
    }
  ],
  "organizer": null,
  "attendees": [],
  "self_status": null,
  "alarms": [{"minutes_before": 10}],
  "travel_time_minutes": null,
  "created_at": "2026-10-20T07:12:00Z",
  "updated_at": "2026-10-28T16:40:00Z"
}
```

| Feld | Bedeutung |
|---|---|
| `version` | Ganzzahl, steigt bei jeder Änderung dieses Events (egal ob vom Anbieter, aus einer App oder per MCP). Basis für Konflikterkennung (`base_version`). |
| `status` | `confirmed`, `tentative`, `cancelled` |
| `availability` | `busy` oder `free` (TRANSP bzw. showAs) |
| `recurrence` | `null` bei Einzelterminen. `rrule` ist der RRULE-Wert nach RFC 5545 (ohne `RRULE:`), `rdates`/`exdates` sind `DateValue`-Listen. Hat ein Objekt mehrere RRULEs (selten, von RFC 5545 nicht mehr empfohlen), steht die erste in `rrule` und `recurrence.incomplete` ist `true`; die App zeigt dann einen Hinweis. Sonst fehlt `incomplete` oder ist `false`. |
| `overrides` | geänderte Vorkommen, jeweils **vollständig** (nicht als Differenz), identifiziert über `recurrence_id` (der ursprüngliche Beginn). `status: cancelled` heißt: Vorkommen fällt aus. |
| `organizer` | `{"email": "…", "name": "…"}` oder `null` |
| `attendees` | `[{"email", "name", "role", "status", "is_self"}]`, `role`: `chair`, `required`, `optional`, `non_participant`; `status`: `needs_action`, `accepted`, `declined`, `tentative`, `delegated` |
| `self_status` | eigene Antwort, wenn ich eingeladen bin (nicht Organisator), sonst `null`. `needs_action` = **offene Terminanfrage**. |
| `alarms` | `[{"minutes_before": 10}]` relativ zum Beginn (negativ = nach Beginn) oder `[{"at": "2026-11-02T07:45:00Z"}]` absolut |
| `travel_time_minutes` | Reisezeit (X-APPLE-TRAVEL-DURATION), sonst `null` |

Das iCalendar-Original hält der Server; Felder, die Lichtung nicht modelliert, bleiben beim Bearbeiten erhalten.

*Ergänzungen (LIC-8), keine bestehende Form geändert:*

- **`DateValue`:** Werte in UTC haben `time_zone: "UTC"`. Lässt sich eine Zeitzone nicht nach IANA übersetzen (unbekannter Name ohne Definition, eigene VTIMEZONE ohne passende IANA-Zone), ist `time_zone` `null`, `utc` aber gesetzt: Der Zeitpunkt gilt, die Wanduhrzeit ist die des Anbieters. Nur `time_zone: null` **und** `utc: null` heißt floating. Eigene VTIMEZONEs übersetzt der Server über `X-LIC-LOCATION` oder über den Vergleich ihrer Offsets mit den IANA-Zonen (bevorzugt eine Zone, deren Stadt im Namen steht).
- **`title`** ist `""`, wenn der Termin keinen Titel hat. `end` fehlt nie: ohne DTEND gilt DTSTART + DURATION, sonst ein Tag (ganztägig) bzw. Dauer null.
- **`recurrence.rrule`** kann `null` sein: bei Serien nur aus RDATEs und bei Objekten ohne Master. `rdates` mit Zeitraum (PERIOD) liefern nur ihren Beginn. Eine EXRULE (veraltet) oder ein nicht lesbares RDATE/EXDATE setzt `incomplete: true`.
- **Objekte ohne Master** (nur einzelne Vorkommen einer fremden Serie, z. B. die Einladung zu einem Termin einer Serie): Kopf-Felder aus dem frühesten Vorkommen, `start` ist dessen `recurrence_id` (Ende mit der Dauer des Vorkommens), `recurrence` ist `{"rrule": null, "rdates": [recurrence_id der übrigen Vorkommen], "exdates": []}`, `overrides` enthält alle Vorkommen. So ergibt die normale Expansion genau diese Vorkommen.
- **`overrides`** sind nach `recurrence_id` sortiert und haben keine eigene `id`, `uid`, `version`, `recurrence` oder `overrides`.
- **`alarms`:** Erinnerungen relativ zum Ende (`RELATED=END`) werden auf den Beginn umgerechnet; Platzhalter mit `ACTION:NONE` (Apple) fehlen.
- **`created_at`/`updated_at`** sind die Zeitpunkte, zu denen Lichtung das Event zuerst gesehen bzw. zuletzt geändert gespeichert hat (nicht CREATED/LAST-MODIFIED aus dem iCalendar).
- **Nicht lesbare Termine:** Kann der Server das iCalendar-Original nicht lesen (kaputte Daten, kein VEVENT, kein DTSTART), speichert er es trotzdem und liefert ein Event in dieser Form:
  ```json
  {"id": "ev_09", "calendar_id": "cal_01", "uid": "…", "version": 1,
   "title": "(nicht lesbarer Termin)", "location": null, "notes": null, "url": null,
   "start": null, "end": null, "all_day": false, "status": "confirmed", "availability": "free",
   "recurrence": null, "overrides": [], "organizer": null, "attendees": [], "self_status": null,
   "alarms": [], "travel_time_minutes": null, "created_at": "…", "updated_at": "…",
   "parse_error": "iCalendar-Fehler in Zeile 12: END:VCALENDAR fehlt am Ende der Daten"}
  ```
  `parse_error` (deutsch) gibt es nur in dieser Form; `uid` kann `null` sein. Die App zeigt solche Events nicht im Kalender, höchstens in einer Liste zum Nachsehen. Wird das Original beim Anbieter repariert, kommt das Event mit höherer `version` in normaler Form.

### Conflict

Eine eigene Änderung, die der Server nicht übernehmen konnte, weil der Anbieter inzwischen etwas anderes hat. **Die Version des Anbieters gewinnt**, die verworfene Änderung bleibt als Conflict sichtbar, bis sie jemand wegklickt.

```json
{
  "id": "cf_01",
  "event_id": "ev_01",
  "calendar_id": "cal_01",
  "origin": "app:iPhone",
  "created_at": "2026-11-03T08:01:00Z",
  "message": "Der Termin wurde inzwischen bei Fastmail geändert. Deine Änderung (Beginn 10:00) wurde nicht übernommen.",
  "mutation": {"id": "…", "op": "update_event", "…": "…"}
}
```

- `origin`: `app:<Gerätename>`, `mcp:<Tokenname>` oder `server`.
- `event_id` ist `null`, wenn das Event inzwischen gelöscht ist.
- `mutation` ist die abgelehnte Mutation, wie sie geschickt wurde (für „noch mal anwenden“ in der App).

## 4. Lesen: Pull mit Cursor

`GET /api/sync/pull?since=<cursor>&limit=<n>`

- `since`: zuletzt gespeicherter Cursor, beim ersten Mal `0`.
- `limit`: höchstens so viele Änderungen (Standard 500, Maximum 2000).

```json
{
  "cursor": 1834,
  "has_more": false,
  "changes": [
    {"type": "account", "op": "upsert", "data": {"id": "acc_01", "…": "…"}},
    {"type": "calendar", "op": "upsert", "data": {"id": "cal_01", "…": "…"}},
    {"type": "event", "op": "upsert", "data": {"id": "ev_01", "…": "…"}},
    {"type": "event", "op": "delete", "id": "ev_07"},
    {"type": "conflict", "op": "upsert", "data": {"id": "cf_01", "…": "…"}}
  ]
}
```

- `type`: `account`, `calendar`, `calendar_set`, `event`, `conflict`. `op`: `upsert` (vollständiges Objekt) oder `delete` (nur `id`).
- Änderungen kommen in der Reihenfolge, in der sie passiert sind; jedes Objekt höchstens einmal pro Seite, in seinem neuesten Stand.
- Der Client wendet eine Seite **atomar** an und speichert danach `cursor`. Bei `has_more: true` sofort weiter mit `since=<cursor>`.
- Wird ein Kalender gelöscht, kommen auch die `delete`s seiner Events. Ein `delete` für ein unbekanntes Objekt ist kein Fehler.
- **Kein Zeitfenster:** Der Pull liefert alle Events aller Kalender. Den Cache-Zeitraum der App (z. B. −3 bis +12 Monate) setzt der Client beim Speichern bzw. Anzeigen um. Serien, die in das Fenster hineinreichen, immer behalten.
- **Cursor zu alt:** Hält der Server die Löschungen nicht mehr so weit zurück (mindestens 90 Tage), antwortet er `410` mit `code: cursor_expired`. Dann Cache leeren (ausstehende Mutationen behalten) und mit `since=0` neu ziehen.

## 5. Live-Updates: SSE

`GET /api/sync/events` mit `Accept: text/event-stream`

```
event: cursor
data: 1834

: keep-alive

event: cursor
data: 1837
```

- Sofort nach dem Verbinden kommt der aktuelle Cursor, danach nach jeder Änderung. Ist er größer als der gespeicherte, zieht der Client per Pull nach. Mehrere Benachrichtigungen dürfen zu einer zusammenfallen.
- Kommentarzeilen (`: …`) halten die Verbindung offen und werden ignoriert.
- Bei Verbindungsabbruch mit Backoff neu verbinden (z. B. 1 s, 2 s, 4 s … bis 60 s). SSE ist nur für die offene App gedacht; im Hintergrund weckt künftig eine stille Push-Nachricht (APNs) die App für einen Pull (siehe Abschnitt 8).
- Weitere Ereignistypen kann der Server ergänzen; unbekannte `event:`-Namen ignoriert der Client.

## 6. Schreiben: Mutationen

Eigene Änderungen schickt der Client als Liste von Mutationen. Offline gesammelte Änderungen bleiben in der lokalen Warteschlange (Outbox), bis der Server sie beantwortet hat.

`POST /api/sync/push`

```json
{
  "mutations": [
    {
      "id": "6f1d1a52-0000-4000-8000-000000000001",
      "op": "create_event",
      "calendar_id": "cal_01",
      "event": {
        "title": "Zahnarzt",
        "start": {"date_time": "2026-11-03T14:00:00", "time_zone": "Europe/Berlin"},
        "end": {"date_time": "2026-11-03T15:00:00", "time_zone": "Europe/Berlin"},
        "alarms": [{"minutes_before": 60}]
      }
    }
  ]
}
```

- Höchstens 100 Mutationen pro Aufruf. Der Server bearbeitet sie **in der Reihenfolge der Liste**.
- `id` ist eine vom Client erzeugte UUID und macht die Mutation **idempotent**: Schickt der Client dieselbe `id` noch einmal (z. B. weil die Antwort verloren ging), bekommt er das gespeicherte Ergebnis, ohne dass der Server erneut schreibt. Ergebnisse hält der Server mindestens 30 Tage.
- Der Server schreibt synchron zum Anbieter und antwortet erst danach (Zeitlimit pro Aufruf etwa 30 s).

### Operationen

**`create_event`**: `calendar_id`, `event` (EventInput). Optional `event.uid`, sonst vergibt der Server eine.

**`update_event`**:

```json
{
  "id": "…", "op": "update_event",
  "event_id": "ev_01", "base_version": 4,
  "scope": "this", "occurrence": {"date_time": "2026-11-09T09:00:00", "time_zone": "Europe/Berlin"},
  "changes": {"start": {"date_time": "2026-11-09T10:00:00", "time_zone": "Europe/Berlin"},
              "end": {"date_time": "2026-11-09T10:30:00", "time_zone": "Europe/Berlin"}}
}
```

- `scope`: `all` (Standard, auch für Einzeltermine), `this` (nur dieses Vorkommen), `this_and_following` (dieses und alle folgenden; der Server teilt die Serie, das Ergebnis enthält dann zwei Events).
- `occurrence`: `recurrence_id` des Vorkommens (der **ursprüngliche** Beginn); Pflicht bei `this` und `this_and_following`.
- `changes`: Teilmenge von EventInput. Nur genannte Felder ändern sich, `null` leert ein optionales Feld.
- `calendar_id` in `changes` verschiebt das Event in einen anderen Kalender (nur mit `scope: all`).

**`delete_event`**: `event_id`, `base_version`, `scope`, `occurrence` wie oben.

**`respond`** (Terminanfrage beantworten):

```json
{"id": "…", "op": "respond", "event_id": "ev_09", "occurrence": null, "status": "accepted", "comment": null}
```

`status`: `accepted`, `tentative`, `declined`. `occurrence` beantwortet nur ein Vorkommen einer Serie.

### EventInput

Schreibbare Felder: `title` (Pflicht beim Anlegen), `location`, `notes`, `url`, `start` (Pflicht), `end` (Pflicht), `status`, `availability`, `recurrence` (`{"rrule", "rdates", "exdates"}` oder `null`), `attendees` (`[{"email", "name", "role"}]`; Status setzt der Anbieter), `alarms`, `travel_time_minutes`, beim Anlegen optional `uid`. `all_day` ergibt sich aus der Form von `start`/`end` (`date` vs. `date_time`); beide müssen dieselbe Form haben.

### Antwort

```json
{
  "results": [
    {"id": "6f1d1a52-…-0001", "status": "applied", "events": [{"id": "ev_12", "…": "…"}], "deleted_event_ids": []},
    {"id": "…-0002", "status": "conflict", "conflict": {"id": "cf_03", "…": "…"}, "events": [{"id": "ev_01", "version": 6, "…": "…"}]},
    {"id": "…-0003", "status": "rejected", "error": {"code": "calendar_read_only", "message": "Der Kalender „Feiertage“ ist nur lesbar."}},
    {"id": "…-0004", "status": "retry", "error": {"code": "provider_unavailable", "message": "Fastmail ist gerade nicht erreichbar."}, "retry_after_seconds": 60},
    {"id": "…-0005", "status": "retry", "error": {"code": "not_attempted", "message": "Nicht versucht, weil eine vorherige Änderung warten muss."}}
  ]
}
```

| `status` | Bedeutung | Was der Client tut |
|---|---|---|
| `applied` | übernommen und zum Anbieter geschrieben. `events`: neuer Stand der betroffenen Events (beim Teilen zwei), `deleted_event_ids`: gelöschte. | Mutation aus der Outbox nehmen, Events übernehmen. Beim Anlegen das lokale Platzhalter-Event über die Mutations-`id` dem Server-Event zuordnen. |
| `conflict` | Event wurde seit `base_version` beim Anbieter geändert oder gelöscht; **Anbieter gewinnt**. `conflict` beschreibt die verworfene Änderung, `events` den aktuellen Stand (leer, wenn gelöscht). | Mutation aus der Outbox nehmen, Server-Stand übernehmen, Konflikt sichtbar melden. |
| `rejected` | dauerhaft ungültig (Validierung, Kalender nicht beschreibbar, Event unbekannt). | Mutation aus der Outbox nehmen, lokale Änderung zurücknehmen, Meldung zeigen. |
| `retry` | vorübergehend nicht möglich (Anbieter nicht erreichbar, Zeitlimit). | Mutation behalten und später erneut senden (frühestens nach `retry_after_seconds`). |

- Nach dem ersten `retry` bricht der Server ab; alle folgenden Mutationen bekommen `retry` mit `code: not_attempted`. So bleibt die Reihenfolge erhalten.
- Ein Konflikt wird nur gemeldet, wenn sich das Event seit `base_version` tatsächlich geändert hat. Der Server darf Änderungen zusammenführen, wenn sie andere Felder betreffen; der Client verlässt sich darauf nicht.
- Hängen weitere Mutationen in der Outbox am selben Event, schickt der Client sie nach einem `applied` mit der neuen `version` als `base_version`. Für ein noch nicht angelegtes Event wartet er, bis das `create_event` `applied` ist.
- Alles, was eine Mutation ändert, kommt zusätzlich über Pull/SSE (mit höherer `version`). Doppelt anwenden ist harmlos.

## 7. Einstellungen und Verwaltung (nur online)

Diese Aufrufe gehen nicht über die Outbox. Antworten enthalten das geänderte Objekt; die Änderung erscheint zusätzlich im Pull.

| Aufruf | Zweck |
|---|---|
| `GET /api/me` | angemeldete Person, Servername und -version |
| `GET /api/accounts` | Konten (wie im Pull) |
| `POST /api/accounts` | Konto anlegen: `{"kind": "fastmail", "username": "…", "password": "<App-Passwort>", "name": "Fastmail"}`, bei `icloud` ebenso, bei `ics`: `{"kind": "ics", "url": "https://…", "name": "…"}`. Der Server prüft die Zugangsdaten sofort (`422` mit `invalid_credentials` bzw. `502` mit `provider_unavailable`) und startet den ersten Sync. |
| `PATCH /api/accounts/{id}` | `name`, neue Zugangsdaten (`username`, `password`) oder `url` |
| `DELETE /api/accounts/{id}` | Konto samt Kalendern und Events aus Lichtung entfernen (beim Anbieter bleibt alles) |
| `POST /api/accounts/{id}/sync` | sofort synchronisieren (`202`) |
| `PATCH /api/calendars/{id}` | `color` (oder `null` = Anbieterfarbe), `hidden`, `mcp_write`, `openings_busy`, `all_day_blocks`, `position` |
| `POST /api/calendar-sets`, `PATCH /api/calendar-sets/{id}`, `DELETE /api/calendar-sets/{id}` | Kalender-Sets: `name`, `calendar_ids`, `position` |
| `POST /api/conflicts/{id}/dismiss` | Konflikt wegklicken (erscheint im Pull als `delete`) |
| `GET /api/occurrences?from=<datum>&to=<datum>&calendar_ids=cal_01,cal_02` | Komfort: vom Server expandierte Vorkommen im Zeitraum (`from` inklusiv, `to` exklusiv, ISO-Datum oder RFC 3339). Nicht für den Cache gedacht. |
| `GET /api/devices`, `DELETE /api/devices/{id}` | angemeldete Geräte, einzeln abmelden |
| *Ergänzung (LIC-7):* `GET /api/calendars`, `GET /api/calendar-sets` | Komfort: alle Kalender bzw. Sets wie im Pull (nach `position` sortiert) |
| `GET /api/tokens`, `POST /api/tokens`, `DELETE /api/tokens/{id}` | API-Tokens (`lt_…`) für MCP-Clients; das Token steht nur in der Antwort auf `POST` |

*Ergänzungen (LIC-7) zu den Konten-Aufrufen:*

- `POST /api/accounts` mit `kind: caldav`: `{"kind": "caldav", "url": "https://dav.example.org/", "username": "…", "password": "…", "name": "…"}`. `name` ist bei allen Arten optional (Standard: „Fastmail“, „iCloud“ bzw. der Hostname). `kind: ics` antwortet bis LIC-14 mit `400 invalid_request`.
- Antwort auf `POST /api/accounts` ist `201` mit dem Account; die Kalender des Kontos kommen über den Pull bzw. `GET /api/calendars`. Dasselbe Konto (Art, Server, Benutzername) ein zweites Mal: `409 conflict`.
- `PATCH /api/accounts/{id}`: Neue Zugangsdaten werden erst beim Anbieter geprüft (`422 invalid_credentials` bzw. `502 provider_unavailable`) und nur bei Erfolg gespeichert. Wer nur `username` oder `url` ändert, braucht `password` nicht erneut zu schicken, außer das Konto steht auf `auth_failed` ohne gespeicherte Zugangsdaten (dann `400`). `url` gibt es nur bei `kind: caldav`.
- `POST /api/accounts/{id}/sync` antwortet `202` mit dem Account (`status.state: syncing`); das Ergebnis erscheint im Status (Pull/SSE).

*Ergänzung (LIC-8) zu `GET /api/occurrences`:*

- Parameter: `from` (Pflicht, inklusiv), `to` (Pflicht, exklusiv), jeweils ISO-Datum (`2026-11-03`, Mitternacht in `time_zone`) oder RFC 3339; `calendar_ids` (kommagetrennt, ohne = alle Kalender, auch ausgeblendete); `time_zone` (IANA, für ISO-Daten, floating und ganztägige Termine; ohne = Zeitzone des Servers); `include_cancelled` (`true` liefert auch abgesagte Vorkommen mit `status: cancelled`, Standard `false`). Höchstens 400 Tage. Fehler: `400 invalid_request` (fehlender oder ungültiger Wert, unbekannter Kalender oder Parameter, `to` nicht nach `from`). Erlaubt für jedes Token für `/api`.
- Antwort:
  ```json
  {
    "occurrences": [
      {
        "event_id": "ev_01", "calendar_id": "cal_01", "uid": "…", "version": 4,
        "recurrence_id": {"date_time": "2026-11-09T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T08:00:00Z"},
        "is_override": true,
        "title": "Team-Sync (verschoben)", "location": "Raum 2", "notes": "…", "url": null,
        "start": {"date_time": "2026-11-09T10:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T09:00:00Z"},
        "end": {"date_time": "2026-11-09T10:30:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T09:30:00Z"},
        "all_day": false, "status": "confirmed", "availability": "busy",
        "organizer": null, "attendees": [], "self_status": null,
        "alarms": [{"minutes_before": 10}], "travel_time_minutes": null
      }
    ],
    "truncated": false
  }
  ```
- Ein Vorkommen trägt die Felder des Events bzw. der Ausnahme, aus der es stammt (wie ein Eintrag in `overrides`), dazu `event_id`, `calendar_id`, `uid`, `version` des Events, `recurrence_id` (ursprünglicher Beginn, `null` bei Einzelterminen) und `is_override`. `start`/`end` sind die des Vorkommens.
- Sortiert nach Beginn (UTC), dann Ende. Höchstens 5000 Vorkommen; mehr ergibt `truncated: true`. Nicht lesbare Termine fehlen.
- `PATCH /api/calendars/{id}` und `/api/calendar-sets`: unbekannte Felder und ungültige Werte (Farbe nicht `#RRGGBB`, `position` negativ, unbekannte oder doppelte `calendar_ids`) ergeben `400 invalid_request`. `POST /api/calendar-sets` antwortet `201`, `DELETE`-Aufrufe `204`.
- **Wer darf:** Lesen (`GET`) und `POST /api/accounts/{id}/sync` jedes Token für `/api`. Konten anlegen, ändern, löschen sowie Kalender-Einstellungen und Sets ändern nur Geräte der App und Browser-Sitzungen (`403 forbidden` für API-Tokens `lt_…`): API-Tokens öffnen auch `/mcp`, und `mcp_write` ist der Schutz vor Claude.

## 8. Geplant (noch nicht verbindlich)

- `PUT /api/devices/current/push` mit `{"apns_token": "…", "environment": "sandbox"|"production"}`: stille Push-Nachrichten, damit Erinnerungen auch für Termine kommen, die die App noch nicht gesehen hat (Spec Abschnitt 5). Kommt mit dem Erinnerungs-Teil von E2.
- Openings (E3) bekommen eigene Objekttypen im Pull.

## 9. Fehlercodes

| HTTP | `code` | Bedeutung |
|---|---|---|
| 400 | `invalid_request` | Anfrage fehlerhaft (Feld fehlt, Format falsch); `message` nennt das Feld |
| 401 | `unauthorized` | kein oder ungültiges Token |
| 403 | `forbidden` | mit diesem Token nicht erlaubt |
| 404 | `not_found` | Objekt unbekannt |
| 409 | `conflict` | Zustandskonflikt bei Verwaltungsaufrufen |
| 410 | `cursor_expired` | Pull-Cursor zu alt, neu mit `since=0` |
| 413 | `payload_too_large` | zu viele Mutationen oder zu großer Body |
| 422 | `invalid_credentials` | Zugangsdaten vom Anbieter abgelehnt |
| 429 | `too_many_requests` | zu viele Anfragen, `Retry-After` beachten |
| 502 | `provider_unavailable` | Anbieter nicht erreichbar oder Fehler beim Anbieter |
| 503 | `unavailable` | Server fährt herunter oder ist überlastet |

Zusätzliche `code`s in Mutations-Ergebnissen (`rejected`/`retry`): `calendar_read_only`, `event_not_found`, `invalid_event` (z. B. Ende vor Beginn), `occurrence_not_found`, `scope_not_supported`, `provider_rejected` (Anbieter lehnt dauerhaft ab, `message` enthält den Grund), `provider_unavailable`, `not_attempted`.

## 10. Änderungsprotokoll

| Datum | Änderung |
|---|---|
| 2026-10-23 | Version 1 (Entwurf) |
| 2026-10-23 | Ergänzungen aus LIC-7 (keine bestehende Form geändert): `kind: caldav` mit `url`; `color` immer gesetzt, Standardwerte neuer Kalender, VTODO-Sammlungen ausgelassen; `GET /api/calendars`, `GET /api/calendar-sets`; Statuscodes und Rechte der Konten-, Kalender- und Set-Aufrufe (Abschnitt 7) |
| 2026-10-23 | Ergänzungen aus LIC-8 (keine bestehende Form geändert): Sync-Rhythmus und Status der Konten; `DateValue` mit `time_zone: null` und `utc` (nicht übersetzbare Zone), `time_zone: "UTC"`; `rrule: null`, Objekte ohne Master; Form nicht lesbarer Termine mit `parse_error`; Form von `GET /api/occurrences` mit `time_zone` und `include_cancelled` (Abschnitt 7) |
| 2026-10-23 | Ergänzungen aus LIC-13 (keine bestehende Form geändert): Feld `shared` am Kalender; `mcp_write` ist bei nur lesbaren Kalendern immer `false` |
