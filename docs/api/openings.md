# Openings (Buchungslinks) in der API von lichtung-server

- **Stand:** 2026-10-26 (LIC-31, LIC-32). Ergänzt die [Sync-API](sync-api.md); Grundlagen (Format, Zeitpunkte, Fehlerform, Anmeldung) gelten wie dort.
- Spec: [`docs/superpowers/specs/2026-10-23-lichtung-design.md`](../superpowers/specs/2026-10-23-lichtung-design.md), Abschnitt 6.

Ein Opening ist ein Buchungslink. Gäste buchen über die öffentliche Seite `<öffentliche URL>/o/<token>` (LIC-33, nicht Teil dieses Dokuments) einen freien Termin im Zielkalender; der Anbieter verschickt die Einladung. Die App verwaltet die Links über `/api/openings`.

## 1. Arten

| `kind` | Was | Verbrauch |
|---|---|---|
| `once` | **Einmal-Link:** Dauer und Zielkalender, optional konkrete Vorschläge. | Nach einer Buchung verbraucht (`booked_at`). |
| `template` | **Fester Link (Vorlage):** Titel, Dauer, Zielkalender, Wochenfenster, Puffer, Mindestvorlauf, Buchungshorizont, optional fester Videolink. | Beliebig oft buchbar. |

- **Links laufen nicht ab.** Ein Link gilt, bis er gelöscht ist (ein Einmal-Link bietet nach der Buchung nur nichts mehr an).
- **Einmal-Link mit Vorschlägen** (`suggestions_only: true`): bietet genau seine Vorschläge an, die noch nicht begonnen haben und noch frei sind. Sind alle vorbei, bietet er nichts mehr an (kein Rückfall auf die Regeln).
- **Einmal-Link ohne Vorschläge:** bietet freie Zeiten nach denselben Regeln wie eine Vorlage, mit den Standardwerten (Abschnitt 3), wenn nichts anderes angegeben ist.

## 2. Objekt `Opening`

```json
{
  "id": "opn_0190…",
  "kind": "template",
  "url": "https://lichtung.example.org/o/Zb3…43 Zeichen…",
  "title": "30 Minuten Kennenlernen",
  "duration_minutes": 30,
  "calendar_id": "cal_01",
  "time_zone": "Europe/Berlin",
  "windows": [{"weekday": "mo", "start": "09:00", "end": "17:00"},
              {"weekday": "tu", "start": "09:00", "end": "12:00"}],
  "buffer_before_minutes": 10,
  "buffer_after_minutes": 10,
  "min_notice_minutes": 240,
  "horizon_days": 21,
  "video_url": "https://meet.example.org/kennenlernen",
  "suggestions_only": false,
  "suggestions": [],
  "booked_at": null,
  "booked_event_id": null,
  "created_at": "2026-10-26T08:00:00.123Z",
  "updated_at": "2026-10-26T08:00:00.123Z"
}
```

| Feld | Bedeutung |
|---|---|
| `url` | Öffentliche Buchungsseite. Der letzte Teil ist eine nicht erratbare Kennung (32 Zufallsbytes, base64url, 43 Zeichen). Sie ändert sich nie; wer den Link nicht mehr weitergeben will, löscht ihn und legt einen neuen an. |
| `windows` | Wochenfenster in `time_zone`: `weekday` `mo` … `su`, `start`/`end` als `hh:mm` (24 Stunden, Ende nach Beginn, nicht über Mitternacht). Mehrere Fenster pro Tag sind erlaubt, überlappende verschmelzen. Gilt nicht für Vorschläge. |
| `buffer_before_minutes`, `buffer_after_minutes` | So lange bleibt vor Beginn bzw. nach Ende eines gebuchten Termins frei. Puffer dürfen über das Fenster hinausreichen. Gelten auch für Vorschläge. |
| `min_notice_minutes` | Mindestvorlauf: früheste Buchung ab jetzt. Gilt nicht für Vorschläge. |
| `horizon_days` | Buchungshorizont: Beginn spätestens so viele Tage ab jetzt. Gilt nicht für Vorschläge. |
| `video_url` | Fester Videolink oder `null`. Die Buchungsseite übernimmt ihn in den Termin. |
| `suggestions` | Nur bei `once`: Vorschläge, die noch nicht begonnen haben, frühester zuerst (siehe unten). Vergangene fehlen. |
| `booked_at`, `booked_event_id` | Nur bei `once`: Zeitpunkt der Buchung und das angelegte Event; sonst `null`. |

**Vorschlag:**

```json
{"id": "sug_0190…", "start": "2026-11-03T09:00:00Z", "end": "2026-11-03T09:30:00Z",
 "placeholder_event_id": "ev_0190…", "placeholder_status": "reserved"}
```

`placeholder_status`: `reserved` (Platzhalter liegt im Zielkalender), `missing` (Platzhalter wurde außerhalb von Lichtung gelöscht; der Vorschlag bleibt buchbar und blockiert seine Zeit für andere Links und `find_free_times` weiter, im Kalender steht er aber nicht mehr), `pending` (wird gerade angelegt).

## 3. Regeln für freie Zeit

- **Frei** heißt wie bei `find_free_times` (Spec Abschnitt 6): kein Termin in einem Kalender mit `openings_busy`; abgesagte Termine, Termine mit `availability: free` und eigene abgelehnte Einladungen blockieren nicht; ganztägige Termine nur in Kalendern mit `all_day_blocks`. Termine ohne Dauer blockieren eine Minute.
- **Platzhalter** anderer Einmal-Links blockieren immer, auch wenn ihr Kalender nicht für Openings zählt. Die eigenen Platzhalter eines Links blockieren ihn nicht.
- **Slots** einer Vorlage oder eines Einmal-Links ohne Vorschläge: Termine der Länge `duration_minutes` innerhalb der Fenster. Beginne liegen im Raster ab Fensterbeginn: alle 15 Minuten bei Links bis 15 Minuten, sonst alle 30 Minuten (Fenster ab 09:00 → 09:00, 09:30, …). Frühester Beginn jetzt + Mindestvorlauf (aufgerundet aufs Raster), spätester Beginn vor jetzt + Horizont, Ende innerhalb des Fensters.
- **Vorschläge:** buchbar, solange sie nicht begonnen haben und frei sind (mit Puffern).
- **Standardwerte** beim Anlegen: `time_zone` die Zeitzone des Servers (`TZ`), `windows` Montag bis Freitag 09:00–17:00, Puffer 0, `min_notice_minutes` 240 (4 Stunden), `horizon_days` 21, `video_url` `null`, `title` bei `once` „Termin (<Dauer> Minuten)“.
- Die Buchungsseite prüft einen Slot unmittelbar vor dem Schreiben erneut gegen den aktuellen Stand.

## 4. Platzhalter (Einmal-Links mit Vorschlägen)

- Für jeden Vorschlag legt der Server beim Anlegen des Links einen Termin im Zielkalender an: Titel „Reserviert: <Titel des Links>“, ohne Teilnehmer, `availability: busy`, mit einer Notiz, dass Lichtung ihn verwaltet. Er geht wie jede andere Änderung zum Anbieter und erscheint im Pull. Konflikte dieser Schreibvorgänge tragen `origin: server`.
- **Entfernt** werden Platzhalter
  - bei der Buchung: alle des Links, auch der des gebuchten Vorschlags (die Buchung legt den echten Termin an);
  - beim Löschen des Links: alle;
  - wenn die Zeit eines Vorschlags begonnen hat: dessen Platzhalter, der Vorschlag fällt weg. Der Server räumt beim Start und alle 15 Minuten auf; vergangene Vorschläge zeigt und bietet er auch vorher schon nicht mehr an.
- **Fehler:** Lässt sich ein Platzhalter nicht anlegen, wird der Link nicht angelegt (schon angelegte Platzhalter entfernt der Server wieder), und die Antwort nennt den Grund (`502 provider_unavailable` oder `400` mit dem Code des Schreibpfads, z. B. `provider_rejected`). Lässt sich ein Platzhalter nicht löschen, weil der Anbieter gerade nicht erreichbar ist, versucht es die nächste Aufräumrunde erneut; der Link ist trotzdem gelöscht bzw. gebucht. Wurde ein Platzhalter beim Anbieter inzwischen geändert, gewinnt der Anbieter: Er bleibt stehen und ein Konflikt (`origin: server`) macht das sichtbar. Wurde er gelöscht, ist nichts mehr zu tun.
- Ändert sich der Titel eines Links, behalten bestehende Platzhalter ihren Titel.

## 5. Aufrufe

| Aufruf | Zweck | Wer |
|---|---|---|
| `GET /api/openings` | alle Links, neueste zuerst | jedes Token für `/api` |
| `GET /api/openings/{id}` | ein Link (`404` wenn unbekannt) | jedes Token für `/api` |
| `POST /api/openings` | Link anlegen → `201` mit dem Opening | Geräte der App und Browser-Sitzungen |
| `PATCH /api/openings/{id}` | Link ändern → `200` mit dem Opening | Geräte der App und Browser-Sitzungen |
| `DELETE /api/openings/{id}` | Link löschen, Platzhalter entfernen → `204` | Geräte der App und Browser-Sitzungen |

API-Tokens (`lt_…`) dürfen Links nur lesen (`403 forbidden`): Ein Link schreibt Platzhalter in seinen Zielkalender, unabhängig von `mcp_write`, und API-Tokens öffnen auch `/mcp`. Claude legt Einmal-Links später über das MCP-Werkzeug `create_opening_link` an (LIC-45).

**Anlegen:**

```json
{"kind": "once", "title": "Abstimmung", "duration_minutes": 30, "calendar_id": "cal_01",
 "suggestions": [{"start": "2026-11-03T10:00:00+01:00"},
                 {"start": "2026-11-04T14:00:00+01:00", "end": "2026-11-04T14:30:00+01:00"}]}
```

```json
{"kind": "template", "title": "30 Minuten Kennenlernen", "duration_minutes": 30,
 "calendar_id": "cal_01", "time_zone": "Europe/Berlin",
 "windows": [{"weekday": "mo", "start": "09:00", "end": "17:00"}],
 "buffer_before_minutes": 10, "buffer_after_minutes": 10,
 "min_notice_minutes": 240, "horizon_days": 21, "video_url": "https://meet.example.org/kennenlernen"}
```

- Pflicht: `kind`, `duration_minutes` (5 bis 720), `calendar_id`; bei `template` auch `title` (1 bis 200 Zeichen, ohne Steuerzeichen).
- Optional: `title`, `time_zone` (IANA), `windows` (1 bis 50), `buffer_before_minutes`/`buffer_after_minutes` (0 bis 720), `min_notice_minutes` (0 bis 43200), `horizon_days` (1 bis 365), `video_url` (`http(s)`, höchstens 2000 Zeichen; leer = keiner).
- `suggestions` nur bei `once`: höchstens 20, `start` (RFC 3339 mit Offset) in der Zukunft und höchstens 365 Tage voraus, `end` optional und sonst `start` + Dauer; ein Vorschlag dauert genau `duration_minutes`, keiner doppelt.
- Unbekannte Felder, ungültige Werte: `400 invalid_request`.

**Ändern** (`PATCH`): `title`, `duration_minutes`, `calendar_id`, `time_zone`, `windows` (ersetzt alle Fenster), Puffer, `min_notice_minutes`, `horizon_days`, `video_url` (`null` entfernt ihn). Nur genannte Felder ändern sich. `kind` und `suggestions` lassen sich nicht ändern (`400`). Bei einem Einmal-Link mit Vorschlägen bleiben auch `calendar_id` und `duration_minutes` fest (die Platzhalter liegen schon), sonst `400 invalid_request`. Ein neuer `calendar_id` wird geprüft wie beim Anlegen. Die `url` bleibt.

**Zielkalender:** muss existieren, für Lichtung beschreibbar sein und Einladungen verschicken können (`can_invite`, Abschnitt 3 der Sync-API). Sonst `400` mit

| `code` | Bedeutung |
|---|---|
| `calendar_not_found` | den Kalender gibt es nicht |
| `calendar_read_only` | nur lesbar (geteilt ohne Schreibrecht, ICS-Abo, Microsoft vor der Graph-Anbindung) |
| `calendar_cannot_invite` | der Kalender kann keine Einladungen verschicken |

Verliert ein Zielkalender später diese Eigenschaften oder verschwindet er, bleibt der Link sichtbar, bietet aber nichts mehr an, bis ein passender Kalender gesetzt ist (`PATCH`).

## 6. Im Pull

Links kommen über den Pull der Sync-API als eigener Objekttyp `opening` (Daten wie in Abschnitt 2), Löschungen als `{"type": "opening", "op": "delete", "id": …}`. Jede Änderung (Anlegen, Ändern, Platzhalter angelegt, Vorschlag verfallen, Buchung) erscheint dort, also auch Links, die Claude anlegt. Clients, die den Typ nicht kennen, überspringen ihn.

## 7. Für die Buchungsseite (LIC-33, serverintern)

Die Seite liest den Link über `App::opening_by_token`, die Slots über `App::opening_slots` (Zeitraum und „jetzt“ als Parameter) und bucht so: `App::opening_booking_guard` halten, `App::check_opening_slot` prüft den Slot erneut, Termin anlegen, `App::complete_opening_booking` markiert einen Einmal-Link als gebucht und entfernt seine Platzhalter (`409 conflict`, wenn er schon gebucht ist). Siehe `server/crates/lichtung-server/src/openings/mod.rs`.
