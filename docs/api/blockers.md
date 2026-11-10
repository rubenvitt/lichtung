# Blocker (LIC-75)

Ein Blocker ist ein „Blockiert“-Termin, den der Server in einen Kalender schreibt, weil in einem anderen Kalender ein Termin liegt. Wer den Zielkalender sieht (etwa die Kollegen im Arbeitskalender), sieht nur: Diese Zeit ist belegt. Lichtung legt die Blocker an, zieht sie beim Verschieben nach und entfernt sie beim Löschen.

Code: `server/crates/lichtung-server/src/blockers/`. Schema-Version 11.

## 1. Regeln

Eine Regel verbindet einen **Quellkalender** mit einem **Zielkalender**:

```json
{
  "id": "blr_0193…",
  "source_calendar_id": "cal_privat",
  "target_calendar_id": "cal_arbeit",
  "title": "Blockiert",
  "all_day": false,
  "blocker_count": 12,
  "failed_count": 0,
  "last_error": null,
  "created_at": "2026-11-10T20:00:00Z",
  "updated_at": "2026-11-10T20:00:00Z"
}
```

- `title`: Titel der Blocker, Standard „Blockiert“, höchstens 200 Zeichen.
- `all_day`: auch ganztägige Termine blocken (Standard `false`).
- `blocker_count`: Blocker, die zurzeit für die Regel im Zielkalender stehen. `failed_count` und `last_error`: Blocker, deren letzter Schreibversuch scheiterte, und der Grund (deutsch); der Server versucht es weiter.
- Quelle darf jeder Kalender sein, auch nur lesbare und ICS-Abos. Das Ziel muss für Lichtung beschreibbar sein (nicht nur lesbar, kein ICS-Abo, nicht Microsoft). Quelle und Ziel sind verschieden, und je Paar gibt es eine Regel. Die Gegenrichtung ist erlaubt.
- Quelle und Ziel stehen fest. Wer sie ändern will, löscht die Regel und legt eine neue an.

| Aufruf | Wirkung | Wer |
|---|---|---|
| `GET /api/blocker-rules` | alle Regeln, älteste zuerst | jedes Token für `/api` |
| `GET /api/blocker-rules/{id}` | eine Regel (`404` wenn unbekannt) | jedes Token für `/api` |
| `POST /api/blocker-rules` | Regel anlegen: `source_calendar_id`, `target_calendar_id`, optional `title`, `all_day` → `201` | Geräte der App und Browser-Sitzungen |
| `PATCH /api/blocker-rules/{id}` | `title`, `all_day` ändern → `200`; alle Blocker der Regel werden neu geschrieben | Geräte der App und Browser-Sitzungen |
| `DELETE /api/blocker-rules/{id}` | Regel löschen → `204`; ihre Blocker verschwinden aus dem Zielkalender | Geräte der App und Browser-Sitzungen |

Fehler: `400 calendar_not_found` (Kalender unbekannt), `400 calendar_read_only` (Ziel nicht beschreibbar), `400 invalid_request` (gleicher Kalender, Titel zu lang), `409 conflict` (Paar gibt es schon), `403` für API-Tokens beim Ändern. Eine Regel schreibt in den Zielkalender, was immer dessen `mcp_write` sagt; darum dürfen nur die App und Browser-Sitzungen Regeln ändern, wie bei Buchungslinks.

Regeln stehen nicht im Pull. Die App lädt sie beim Öffnen der Einstellungen.

## 2. Der Blocker

- Er trägt **nur die Zeit** seines Termins: Beginn, Ende, bei Serien die Regel (`RRULE`, `RDATE`, `EXDATE`) und verschobene Vorkommen. Titel, Ort, Notizen, Link, Teilnehmer und Erinnerungen des Termins bleiben in dessen Kalender.
- Titel aus der Regel, `availability: busy`, keine Teilnehmer, also keine Einladungen.
- Er nennt seinen Termin in `X-LICHTUNG-BLOCKER-OF`; das Event-Objekt zeigt das als `blocker_of` (die ID des Termins). Die Apps blenden ihn damit genau hinter seinem Termin aus, egal wie er heißt.

**Was blockt:** jeder Termin der Quelle, außer abgesagten (`status: cancelled`), als frei markierten (`availability: free`), Einladungen, die ich abgelehnt habe, und ganztägigen ohne `all_day`. Bei Serien gilt das je Vorkommen: Ein abgesagtes oder freies Vorkommen fehlt im Blocker (als `EXDATE`). Blockt die Serie selbst nicht, bekommt sie keinen Blocker. Ein Termin, der selbst ein Blocker ist, wird nie gespiegelt, so dass Regeln in beide Richtungen einander nicht füttern.

**Wer nicht will, dass ein bestimmter Termin blockt,** markiert ihn als frei. Ein Blocker, der von Hand geändert oder gelöscht wird, kommt beim nächsten Abgleich zurück: Er folgt immer seinem Termin.

## 3. Abgleich

Der Server gleicht ab nach jeder Änderung (gesammelt, 2 Sekunden), sobald sich eine Regel ändert, und alle 10 Minuten. Er betrachtet die Termine der Quelle, die nicht länger als einen Tag vorbei sind:

- kein Blocker → anlegen (die UID merkt er sich vor dem Schreiben, so dass ein Blocker, dessen Antwort verloren ging, wiedergefunden und nicht doppelt angelegt wird),
- Termin, Regel oder Blocker geändert → Blocker neu schreiben,
- Termin blockt nicht mehr → Blocker löschen,
- Termin oder Regel gelöscht → Blocker löschen.

Blocker vergangener Termine bleiben, wie sie sind. Geschrieben wird über den normalen Schreibweg (Herkunft `server`), aber ohne Konflikteinträge: Hat der Anbieter einen neueren Stand, gilt der, und der nächste Durchlauf schreibt erneut. Ein gescheiterter Schreibversuch (Anbieter nicht erreichbar, abgelehnt) steht an der Regel (`failed_count`, `last_error`) und wird bei jedem Durchlauf wiederholt.
