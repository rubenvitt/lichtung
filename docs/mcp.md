# MCP-Endpunkt

Lichtung hat unter `/mcp` einen MCP-Server, über den Claude den Kalender liest und schreibt: Termine abfragen, freie Zeiten finden, Termine anlegen, ändern und löschen, Einladungen beantworten, Einmal-Buchungslinks anlegen. Grundlage ist Spec Abschnitt 7.

## Einrichtung

### claude.ai (Custom Connector)

In claude.ai unter **Einstellungen → Connectors → Custom Connector hinzufügen** die URL

```
https://lichtung.rubeen.dev/mcp
```

eintragen. claude.ai findet den Autorisierungsserver von Lichtung selbst (`/.well-known/oauth-protected-resource`), registriert sich und öffnet die deutsche Freigabeseite. Dort mit Pocket-ID anmelden und bestätigen. Danach arbeitet claude.ai mit kurzlebigen Access-Tokens (`loa_…`, 1 Stunde), die es selbst erneuert. Verbundene Clients stehen unter `GET /api/oauth/clients` und lassen sich in der App einzeln trennen.

### Per API-Token (Claude Code, Skripte)

In der App einen API-Token (`lt_…`) erzeugen (`POST /api/tokens`, siehe [Betrieb](betrieb.md#zugänge)) und als Bearer-Header mitschicken, z. B. in Claude Code:

```sh
claude mcp add --transport http lichtung https://lichtung.rubeen.dev/mcp \
  --header "Authorization: Bearer lt_…"
```

Der Name des Tokens erscheint als Herkunft der Änderungen (`mcp:<Tokenname>`, bei claude.ai `mcp:oauth:<Client>`), z. B. in Konflikten.

### Zugang im Detail

- Angenommen werden OAuth-Access-Tokens für die Ressource `<öffentliche URL>/mcp`, API-Tokens und der Bootstrap-Token. Geräte-Tokens der App (`ld_…`) und Browser-Sitzungen werden abgewiesen.
- Ohne oder mit ungültigem Token antwortet `/mcp` mit `401` und `WWW-Authenticate: Bearer resource_metadata="<öffentliche URL>/.well-known/oauth-protected-resource"` (bei einem abgewiesenen Token zusätzlich `error="invalid_token"`).
- Transport: Streamable HTTP ohne Sitzungen. `POST /mcp` nimmt eine JSON-RPC-Nachricht oder einen Batch; `GET` und `DELETE` antworten `405`. Protokollversionen `2025-06-18` und `2025-03-26`. Ein mitgeschickter `Origin`-Header muss zur öffentlichen URL passen.

## Schreibschutz pro Kalender

- Jeder Kalender hat die Einstellung `mcp_write` (in der App: „Claude darf hier schreiben“). Ist sie aus, lehnt der Server **jedes** Schreiben über MCP in diesem Kalender ab: `create_event`, `update_event`, `delete_event`, auch `respond_to_invitation` und `create_opening_link` (ein Buchungslink schreibt Platzhalter und den gebuchten Termin in seinen Zielkalender). Der Fehler heißt `mcp_write_forbidden` und nennt den Kalender; freigeben kann das nur der Nutzer in der App. Die Apps selbst betrifft die Einstellung nicht.
- **Standard:** Neue Kalender bekommen `mcp_write` = beschreibbar beim Anbieter. Kalender von ICS-Konten und (später) Microsoft-Konten starten immer mit `mcp_write` aus: Darüber kommt der Arbeitskalender, und Claude darf dort nur schreiben, wenn es ausdrücklich erlaubt ist.
- Ändern dürfen die Einstellung nur die App und Browser-Sitzungen, nicht API- oder OAuth-Tokens. So kann Claude den Schutz nicht selbst aufheben.
- **Kein Standardkalender:** `create_event` verlangt immer `calendar_id`. Fehlt sie, kommt `calendar_id_missing` mit dem Hinweis auf `list_calendars`.
- Nur lesbare Kalender (geteilt ohne Schreibrecht, ICS-Abos) lehnen Schreiben mit `calendar_read_only` ab.

## Zeitangaben

- Zeitpunkte **immer mit Offset** nach RFC 3339: `2026-11-03T14:00:00+01:00` oder `2026-11-03T13:00:00Z`. Eine Uhrzeit ohne Offset wird mit einer Erklärung abgelehnt.
- Ganztägige Termine als Datum `JJJJ-MM-TT`. `end` ist bei ganztägigen Terminen der **letzte Tag einschließlich** (ein eintägiger Termin hat `start` = `end`), in der Eingabe wie in der Ausgabe.
- Zeiträume (`start`/`end` der Abfragen): ein Datum als `start` ist Tagesbeginn, ein Datum als `end` schließt diesen Tag ganz ein; Zeitpunkte gelten wie angegeben (`end` ausschließlich). Höchstens 400 Tage.
- `time_zone` (IANA, z. B. `Europe/Berlin`): bei Abfragen die Zeitzone der Ausgabe und von Datumsangaben (Standard: `TZ` des Servers); beim Anlegen und Ändern die Zeitzone, in der der Termin gespeichert wird. Ohne `time_zone` nimmt `create_event` die Zeitzone des Servers, wenn ihr Offset zum angegebenen passt, sonst UTC; `update_event` behält die Zeitzone des Termins.

## Werkzeuge

| Werkzeug | Parameter (Pflicht **fett**) | Ergebnis |
|---|---|---|
| `list_calendars` | – | `calendars`: `id`, `name`, `account` (`id`, `kind`, `name`), `color`, `writable` (Anbieter), `mcp_writable` (= `writable` und `mcp_write`), `can_invite`, `hidden`, `openings_busy`, `all_day_blocks`, `is_work` (Arbeitskalender) |
| `list_calendar_sets` | – | `calendar_sets`: `id`, `name`, `calendar_ids` |
| `query_events` | **`start`**, **`end`**, `query`, `calendar_ids`, `calendar_set_id`, `include_cancelled`, `time_zone`, `limit` (1–1000, Standard 250) | `events` (Vorkommen, nach Beginn sortiert), `count`, `truncated` |
| `find_free_times` | **`start`**, **`end`**, **`min_duration_minutes`**, `calendar_ids`, `calendar_set_id`, `day_start`/`day_end` (`hh:mm`), `weekdays` (`mo`…`su`), `buffer_minutes`, `time_zone`, `limit` (Standard 100) | `free`: `start`, `end`, `duration_minutes`; `calendar_ids` |
| `list_invitations` | `start` (Standard: jetzt), `end` (Standard: 180 Tage später), `calendar_ids`, `time_zone`, `limit` | `invitations`: wie ein Vorkommen, dazu `occurrence`, `pending_occurrences`, `can_respond` |
| `create_event` | **`calendar_id`**, **`title`**, **`start`**, **`end`**, `time_zone`, `location`, `notes`, `url`, `recurrence` (RRULE), `alarms` (Minuten vor Beginn), `availability` (`busy`/`free`), `attendees` (`email`, `name`, `role`) | `events`: der angelegte Termin mit `event_id` |
| `update_event` | **`event_id`**, `occurrence`, `scope`, dieselben Felder wie `create_event` (außer `calendar_id`; `null` entfernt `location`, `notes`, `url`, `recurrence`, `alarms`, `attendees`), `move_to_calendar_id` | `events`: neuer Stand |
| `delete_event` | **`event_id`**, `occurrence`, `scope` | `deleted_event_ids` bzw. `events` (gekürzte Serie) |
| `respond_to_invitation` | **`event_id`**, **`response`** (`accepted`/`tentative`/`declined`), `occurrence`, `comment` | `events`: neuer Stand mit `self_status` |
| `create_opening_link` | **`duration_minutes`** (5–720), **`calendar_id`**, `title`, `suggestions` (höchstens 20, je `start` und optional `end` mit Offset), `time_zone` | `id`, `url`, `title`, `duration_minutes`, `calendar`, `time_zone`, `suggestions_only`, `suggestions` (`start`, `end`, `placeholder_status`, `placeholder_event_id`), `windows`, `min_notice_minutes`, `horizon_days` |

Die genauen Schemas samt Beschreibungen liefert `tools/list`; sie sind in [`server/crates/lichtung-server/tests/snapshots/mcp_tools.json`](../server/crates/lichtung-server/tests/snapshots/mcp_tools.json) eingefroren.

**Ein Vorkommen** in `query_events` hat: `event_id`, `recurrence_id` (bei Serien der ursprüngliche Beginn, sonst `null`), `is_recurring`, `title`, `notes`, `location`, `url`, `start`, `end`, `all_day`, `duration_minutes`, `event_time_zone` (Zeitzone des Termins, `null` bei ganztägigen und floating Terminen), `status` (`confirmed`, `tentative`, `cancelled`), `availability`, `organizer`, `attendees`, `self_status` (eigene Antwort, `needs_action` = offene Anfrage) und `calendar` (`id`, `name`, `mcp_writable`).

**Suche:** `query` filtert auf Titel, Ort und Notiz; alle Wörter müssen vorkommen, Groß- und Kleinschreibung egal. Ohne `query` kommt alles im Zeitraum. Ohne Kalenderfilter fragt `query_events` alle nicht ausgeblendeten Kalender ab.

**Serien:** `update_event` und `delete_event` verlangen bei Serien `scope`: `this` (nur dieses Vorkommen), `this_and_following` (dieses und alle folgenden) oder `all` (die ganze Serie); `this` und `this_and_following` brauchen `occurrence` = `recurrence_id` aus `query_events`. Fehlt `scope`, kommt `scope_required`, fehlt `occurrence`, kommt `occurrence_required`. Bei Einzelterminen gilt immer `all`. Wird bei `update_event` nur `start` geändert, bleibt die Dauer erhalten. `respond_to_invitation` ohne `occurrence` beantwortet die ganze Serie.

**Freie Zeiten** nach den Regeln der Openings (Spec Abschnitt 6): Es zählen die gewählten Kalender, ohne Auswahl alle mit `openings_busy`. Abgesagte Termine, Termine mit `availability: free` und eigene abgelehnte Einladungen blockieren nicht; ganztägige Termine nur in Kalendern mit `all_day_blocks`. Platzhalter von Einmal-Links („Reserviert: …“, siehe [`api/openings.md`](api/openings.md)) zählen immer als belegt, auch außerhalb der gewählten Kalender. `buffer_minutes` hält vor und nach jedem Termin frei. Liegen zu viele Termine im Zeitraum, kommt `too_many_events` statt eines unsicheren Ergebnisses.

**Einmal-Links** (`create_opening_link`, Spec Abschnitte 6 und 7, Vertrag in [`api/openings.md`](api/openings.md)): legt einen Einmal-Link (`kind: once`) über dieselbe Funktion an wie die App und liefert die `url` der Buchungsseite zum Weitergeben. Der Gast bucht genau einen Termin; danach ist der Link verbraucht. Links laufen nicht ab; Vorlagen (feste Links) und das Löschen gibt es nur in der App.
- **Zielkalender:** muss es geben (`calendar_not_found`), beim Anbieter beschreibbar sein (`calendar_read_only`), Einladungen verschicken können (`calendar_cannot_invite`) und für Claude freigegeben sein (`mcp_write_forbidden`). Geprüft wird vor jedem Schreiben; ein abgelehnter Aufruf legt nichts an.
- **Mit `suggestions`** bietet der Link genau diese Zeiten an. Jeder Vorschlag dauert genau `duration_minutes` (`end` weglassen oder passend angeben), liegt in der Zukunft und höchstens 365 Tage voraus, keiner doppelt; sonst `invalid_arguments` mit Hinweis. Für jeden Vorschlag legt der Server sofort einen Platzhalter „Reserviert: <Titel>“ im Zielkalender an (`placeholder_status: reserved`); er blockiert die Zeit für andere Links und `find_free_times`, bis der Link gebucht oder gelöscht ist oder die Zeit vorbei ist. Lässt sich ein Platzhalter nicht anlegen, wird der Link nicht angelegt (`provider_unavailable` bzw. der Code des Anbieters).
- **Ohne `suggestions`** bietet der Link freie Zeiten nach den Standardregeln: Montag bis Freitag 09:00–17:00 in `time_zone` (Standard: Zeitzone des Servers), frühestens 4 Stunden ab jetzt, höchstens 21 Tage voraus. Das Ergebnis nennt `windows`, `min_notice_minutes` und `horizon_days`.
- `title` ist der Titel des gebuchten Termins (Standard „Termin (<Dauer> Minuten)“). Zeiten der Vorschläge kommen in `time_zone` des Links zurück.
- Die App sieht den Link sofort (Pull, Objekttyp `opening`) und kann ihn dort löschen.

## Fehler

Werkzeugfehler kommen als Ergebnis mit `isError: true`, deutscher Meldung (was falsch war und wie es richtig geht) und `structuredContent.error` (`code`, `message`). Protokollfehler sind JSON-RPC-Fehler (`-32700`, `-32600`, `-32601`, `-32602`; ein unbekanntes Werkzeug ist `-32602`).

| `code` | Bedeutung |
|---|---|
| `invalid_arguments` | Parameter fehlt, unbekannt oder falsch geformt (z. B. Uhrzeit ohne Offset) |
| `calendar_id_missing` | `create_event` oder `create_opening_link` ohne `calendar_id` |
| `calendar_not_found`, `calendar_set_not_found` | unbekannte ID |
| `mcp_write_forbidden` | Kalender für Claude gesperrt (`mcp_write` aus) |
| `calendar_read_only` | Kalender beim Anbieter nur lesbar |
| `calendar_cannot_invite` | `create_opening_link`: Zielkalender kann keine Einladungen verschicken (`can_invite` false) |
| `scope_required`, `occurrence_required` | Serie ohne `scope` bzw. ohne `occurrence` |
| `event_not_found`, `occurrence_not_found` | unbekannter Termin bzw. unbekanntes Vorkommen |
| `invalid_event`, `invalid_request`, `scope_not_supported` | vom Schreibdienst abgelehnt (z. B. ungültige RRULE) |
| `conflict` | Der Termin hat sich beim Anbieter geändert, **der Anbieter gewinnt**. Die Änderung wurde nicht übernommen; `current` zeigt den aktuellen Stand (leer, wenn der Termin gelöscht wurde), `conflict_id` den Konflikt, den die App anzeigt |
| `provider_rejected` | Anbieter lehnt dauerhaft ab |
| `provider_unavailable` | Anbieter gerade nicht erreichbar; später erneut versuchen (`retry_after_seconds`) |
| `too_many_events` | `find_free_times`: Zeitraum kürzer wählen |
