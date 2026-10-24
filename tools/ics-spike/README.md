# ICS-Feed-Spike (LIC-14, Spike S3)

Prüft, ob der veröffentlichte ICS-Feed des Arbeitskalenders (Outlook/Exchange) so aussieht, wie `lichtung-server` ihn erwartet (Spec Abschnitt 4, Spike S3):

1. **Abruf:** Status, Größe, Weiterleitungen, Content-Type; liegt der Feed unter dem Limit von 20 MB?
2. **Bedingte Abrufe:** Liefert der Server `ETag` und/oder `Last-Modified`, und antwortet ein zweiter Abruf mit `If-None-Match`/`If-Modified-Since` mit `304`? Ändert sich der Inhalt zwischen zwei Abrufen nur im `DTSTAMP`?
3. **Inhalt (nur Struktur):** Anzahl Termine, Serien, Overrides, Overrides ohne Master, doppelte UIDs mit `RECURRENCE-ID`, Zeitzonen (TZIDs, auch Windows-Namen), X-Properties, Verteilung von `X-MICROSOFT-CDO-BUSYSTATUS`, `TRANSP`, `STATUS`, `CLASS`, Zeitraum (Monate).

Das Skript gibt **keine Inhalte** aus: keine Titel, Orte, Beschreibungen, Personen, UIDs und nicht die URL. Die Kopfzeilen `PRODID`, `METHOD`, `X-WR-TIMEZONE` und `X-PUBLISHED-TTL` erscheinen im Klartext (Produkt- und Zonennamen); der Kalendername nur als „vorhanden ja/nein“.

Nur Python-Standardbibliothek (python3 ≥ 3.9). Die URL kommt aus der Umgebung oder wird verdeckt abgefragt und nirgends gespeichert. **Die Feed-URL enthält ein Geheimnis: nicht ins Repo, in einen Chat oder in ein Ticket kopieren.**

## Vorbereitung

In Outlook im Web: **Einstellungen → Kalender → Freigegebene Kalender → Kalender veröffentlichen**, Kalender und „Kann alle Details anzeigen“ wählen, dann den **ICS-Link** kopieren (nicht den HTML-Link). Ein bereits vorhandener Link geht genauso.

## Ablauf

```sh
# URL wird verdeckt abgefragt (taucht nicht in der Shell-History auf)
python3 tools/ics-spike/spike.py

# alternativ über die Umgebung (Vorsicht: History) und als JSON
LICHTUNG_SPIKE_ICS_URL='https://…' python3 tools/ics-spike/spike.py --json

# Feed mit Anmeldung (selten): Passwort wird abgefragt oder aus LICHTUNG_SPIKE_ICS_PASSWORD gelesen
python3 tools/ics-spike/spike.py --user <benutzername>
```

- `webcal://` und `webcals://` werden zu `https://`. Spricht der Server nur HTTP, die URL mit `http://` angeben.
- `--wait <s>` legt die Pause zwischen erstem und zweitem Abruf fest (Standard 5 s). Für die Frage, ob sich der Inhalt ohne Änderung im Kalender trotzdem ändert, einmal mit `--wait 120` laufen lassen.
- Das Skript ruft den Feed vier- bis fünfmal ab (erster Abruf, bedingt mit beiden Validatoren, mit jedem einzeln, unbedingt).

Die Ausgabe ist eine Markdown-Tabelle zum Einfügen.

## Ergebnis festhalten

Bitte als Kommentar an LIC-14 (Ausgabe des Skripts genügt), vorher kurz prüfen, dass keine Inhalte darin stehen:

| Frage | Ergebnis |
|---|---|
| HTTP-Status, Dauer, Weiterleitungen | |
| Größe (unter 20 MB?) | |
| Content-Type / -Encoding | |
| ETag / Last-Modified vorhanden | |
| Zweiter Abruf mit Bedingung → 304? (beide / nur ETag / nur Last-Modified) | |
| Inhalt gleich ohne `DTSTAMP`? ETag stabil? | |
| `PRODID` | |
| VEVENTs / UIDs / Einzeltermine / Serien | |
| Overrides (davon ohne Master) | |
| Doppelte Master / doppelte Overrides | |
| TZIDs (benutzt, ohne VTIMEZONE) | |
| BUSYSTATUS-Verteilung | |
| TRANSP fehlt bei wie vielen? | |
| X-Properties (Termine) | |
| Zeitraum (Monate) | |
| Auffälligkeiten | |

## Was der Server daraus macht

- **Kein 304:** Der Server lädt alle 15 Minuten den ganzen Feed und vergleicht pro Termin einen Inhalts-Hash ohne `DTSTAMP`; unveränderte Termine bekommen keine neue Version. Das kostet nur Bandbreite.
- **Inhalt ändert sich nur im `DTSTAMP`:** abgedeckt (Hash ohne `DTSTAMP`). Ändern sich weitere Felder bei jedem Abruf (z. B. `SEQUENCE` oder `LAST-MODIFIED`), bitte als Auffälligkeit notieren: Dann steigt bei jedem Abruf die Version aller Termine.
- **Overrides ohne Master** (Einladungen zu einzelnen Terminen fremder Serien) und **doppelte Overrides** (gleiche UID und `RECURRENCE-ID`, der Server behält den neuesten nach `SEQUENCE`, `LAST-MODIFIED`, `DTSTAMP`) werden verarbeitet.
- **TZIDs ohne VTIMEZONE** mit Windows-Namen übersetzt `lichtung-ical`; unbekannte Namen landen als `time_zone: null` mit korrektem `utc`.
- **BUSYSTATUS** bestimmt `availability`, wenn `TRANSP` fehlt: `FREE` und `WORKINGELSEWHERE` frei, `TENTATIVE`, `BUSY`, `OOF` belegt.
