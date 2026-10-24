# CalDAV-Scheduling-Spike (LIC-6, LIC-13)

Prüft an einem echten Konto, ob server-seitiges Scheduling nach RFC 6638 trägt (Spec Abschnitt 4, Spike S2):

1. **Einladen:** Termin mit Teilnehmer per CalDAV anlegen → verschickt der Anbieter die Einladung?
2. **Antworten:** eigene `PARTSTAT` an einer erhaltenen Einladung ändern → geht die Antwort an den Organisator?

Nur Python-Standardbibliothek (python3 ≥ 3.9). Zugangsdaten kommen aus der Umgebung oder werden abgefragt und nirgends gespeichert. **Nichts davon ins Repo oder in einen Chat kopieren.**

## Vorbereitung

- **Fastmail:** Einstellungen → Privatsphäre & Sicherheit → App-Passwörter → neues Passwort mit Zugriff „Kalender (CalDAV)“. Benutzername ist die Fastmail-Adresse.
- **iCloud:** appleid.apple.com → App-spezifische Passwörter. Benutzername ist die Apple-ID.
- Eine **zweite Adresse** als Teilnehmer, deren Postfach du einsehen kannst (z. B. eine andere eigene Adresse).

```sh
export LICHTUNG_SPIKE_USER='<deine Adresse>'
# Passwort wird abgefragt; alternativ LICHTUNG_SPIKE_PASSWORD setzen (landet dann in der Shell-History, wenn du es tippst)
```

## Ablauf

```sh
# 0. Was meldet der Server? (calendar-auto-schedule, Postfächer, Kalender, Rechte, geteilte Kalender)
python3 tools/caldav-spike/spike.py --provider fastmail discover

# 1. Einladen: legt übermorgen 10:00 einen 30-Minuten-Termin „Lichtung-Spike: Testeinladung“ an
python3 tools/caldav-spike/spike.py --provider fastmail invite --attendee <zweite-adresse>
#    → Ausgabe zeigt ATTENDEE-Zeilen; SCHEDULE-STATUS=1.2 = zugestellt. Postfach der zweiten Adresse prüfen.

# 2. Antworten: von der zweiten Adresse an die Hauptadresse einladen (z. B. aus einem anderen Kalender),
#    dann offene Anfragen auflisten und eine beantworten
python3 tools/caldav-spike/spike.py --provider fastmail requests
python3 tools/caldav-spike/spike.py --provider fastmail respond --href <href aus requests> --status ACCEPTED
#    → beim Organisator prüfen, ob die Zusage ankommt (und wie sie aussieht: Mail mit iMIP-Anhang?)

# 3. Aufräumen (als Organisator verschickt das eine Absage an den Teilnehmer)
python3 tools/caldav-spike/spike.py --provider fastmail delete --href <href aus invite>
```

Mit `-v` gibt das Skript die Rohantworten des Servers auf stderr aus (enthält Termindaten, nicht ins Repo kopieren).

Für iCloud dasselbe mit `--provider icloud` (Teil von LIC-13), siehe nächster Abschnitt.

## iCloud (LIC-13)

**App-spezifisches Passwort:** appleid.apple.com → Anmelden und Sicherheit → App-spezifische Passwörter → „+“, Name z. B. „Lichtung-Spike“. Das Passwort hat die Form `xxxx-xxxx-xxxx-xxxx`. Benutzername ist die E-Mail-Adresse der Apple-ID. Das normale Apple-ID-Passwort lehnt iCloud für CalDAV ab (401); das Skript weist dann darauf hin. Nach dem Spike das Passwort dort wieder widerrufen. Für Lichtung selbst ein eigenes App-spezifisches Passwort erzeugen.

**Wie iCloud antwortet** (das Skript folgt dem selbst):

- `/.well-known/caldav` leitet mit 301 auf `https://caldav.icloud.com/` um. Der Principal liegt unter `/<DSID>/principal/`, die DSID ist eine Nummer des Kontos. Sie steht mit `-v` in der Ausgabe und gehört nicht in Kommentare oder ins Repo.
- Das Calendar-Home liegt auf einem Partitions-Host, z. B. `https://p42-caldav.icloud.com:443/<DSID>/calendars/`. Alle weiteren Anfragen gehen dorthin. Das Skript schickt die Zugangsdaten nur an die Basis-URL und per HTTPS an `*.icloud.com`. Weiterleitungen folgt es selbst, auch bei PROPFIND und REPORT, und behält dabei Methode und Inhalt (`urllib` folgt ihnen dort nicht).
- In der Kalenderliste stehen auch Erinnerungslisten (nur `VTODO`), `inbox`, `outbox` und `notification`. Das Skript zeigt nur Kalender. Bei geteilten Kalendern steht „geteilt mit mir“ oder „von mir geteilt“, bei nur lesender Freigabe „nur lesen“.

**Ablauf:**

```sh
export LICHTUNG_SPIKE_USER='<Apple-ID>'
python3 tools/caldav-spike/spike.py --provider icloud discover
python3 tools/caldav-spike/spike.py --provider icloud invite --attendee <zweite-adresse>
# Postfach der zweiten Adresse prüfen; von dort an die Apple-ID einladen, dann:
python3 tools/caldav-spike/spike.py --provider icloud requests
python3 tools/caldav-spike/spike.py --provider icloud respond --href <href aus requests> --status ACCEPTED
python3 tools/caldav-spike/spike.py --provider icloud delete --href <href aus invite>
```

`invite` legt den Termin im ersten eigenen schreibbaren Kalender an, nie in einem geteilten. Wer einen geteilten Kalender ausprobieren will (z. B. ob eine nur lesende Freigabe ein PUT mit 403 ablehnt), gibt `--calendar <href aus discover>` an.

**Ergebnis für LIC-13** (als Kommentar an LIC-13, ohne DSID, Adressen oder Termindetails):

| Frage | Ergebnis |
|---|---|
| `/.well-known/caldav` → Partitions-Host gefunden? | ja / nein |
| `calendar-auto-schedule` im DAV-Header? | ja / nein |
| Schedule-Inbox/-Outbox vorhanden? | ja / nein |
| Einladung beim Teilnehmer angekommen? | ja / nein, nach wie vielen Sekunden, Absender |
| `SCHEDULE-STATUS` am Teilnehmer nach dem PUT | z. B. `1.2` |
| Antwort (`respond`) beim Organisator angekommen? | ja / nein |
| Absage beim Löschen verschickt? | ja / nein |
| Geteilte Kalender: als „geteilt mit mir“ erkannt, Rechte stimmen? | ja / nein |
| Nur lesende Freigabe: PUT abgelehnt (403)? | ja / nein / nicht geprüft |
| Auffälligkeiten | z. B. 503 mit `Retry-After`, Fehlercodes |

## Ergebnis festhalten

Bitte als Kommentar an LIC-6 (bzw. LIC-13), ohne Adressen oder Termindetails:

| Frage | Ergebnis |
|---|---|
| `calendar-auto-schedule` im DAV-Header? | ja / nein |
| Schedule-Inbox/-Outbox vorhanden? | ja / nein |
| Einladung beim Teilnehmer angekommen? | ja / nein, nach wie vielen Sekunden |
| `SCHEDULE-STATUS` am Teilnehmer nach dem PUT | z. B. `1.2` |
| Antwort (`respond`) beim Organisator angekommen? | ja / nein |
| Absage beim Löschen verschickt? | ja / nein |
| Auffälligkeiten | z. B. Absender, Mail-Format, Fehlercodes |

Fällt einer der Punkte durch, gilt der Fallback aus der Spec (Abschnitt 11): Einladungen als iMIP-Mail vom Server.
