# Lichtung – Design

- **Datum:** 2026-10-23
- **Status:** Entwurf, wartet auf Freigabe
- **Repo:** öffentlich, Apache-2.0

Lichtung ist eine eigene Kalender-App für macOS und iOS, in Bedienung und Gestaltung sehr eng an Fantastical angelehnt, mit einem selbst gehosteten Server dahinter. Der Name steht für die freie Stelle im Wald: die Lücke im Kalender, die andere bei mir buchen können.

## 1. Ziel, Warum, Erfolgskriterien

Lichtung ersetzt das Fantastical-Abo. Fantastical ist in Bedienung und Gestaltung sehr gut, das ist der Maßstab. Lichtung soll es nicht neu erfinden, sondern für eine Person mit eigenem Server nachbauen, plus zwei Dinge, die nur ein eigener Server leisten kann: ein MCP-Endpunkt für Claude und Buchungslinks für freie Zeiten.

**Lichtung ist fertig, wenn Fantastical gekündigt werden kann.** Das ist der Fall, wenn alle fünf Punkte erfüllt sind:

1. **Im Alltag gut benutzbar**, auf Mac und iPhone, auch unterwegs.
2. **Alle Kalender sind verbunden:** Fastmail, iCloud und der Arbeitskalender (Microsoft 365 / Exchange).
3. **Modern und macht Spaß:** Die App fühlt sich so flüssig und durchdacht an wie Fantastical. Pflicht dafür (Nutzer: „alles“):
   - im Raster per Ziehen anlegen, verschieben und in der Länge ändern
   - Terminsuche
   - Rückgängig
   - Tastaturbedienung und Kontextmenüs auf dem Mac
   - Termin duplizieren
   - Videolinks in Terminen erkennen, Beitreten mit einem Klick
   - Reisezeit
   - mehrere Zeitzonen
4. **Terminanfragen annehmen und ablehnen** geht direkt in der App, auf beiden Plattformen.
5. **Die Planungs-Skills, die heute das Fantastical-MCP nutzen**, laufen gegen das MCP von Lichtung.

**Nutzerkreis:** eine Person. Mehrbenutzerbetrieb wird nicht gebaut. Gäste auf Buchungsseiten sind keine Nutzer.

### Wie der Kalender heute genutzt wird

- **Morgens:** App öffnen, den Tag überblicken. Kurz danach Tagesplanung mit Claude, die über MCP Zeitblöcke anlegt.
- **Tagsüber und unterwegs:** schnell Termine von Hand eintragen (Beispiel: Folgetermin beim Arzt), und zwar **als Satz** („Zahnarzt Di 14 Uhr“), und schnell den Überblick gewinnen.
- **Terminanfragen** annehmen oder ablehnen. Sie kommen meist aus dem Arbeitskalender.
- **Buchungslinks (Openings):** Nach einer Absprache geht ein Link raus, die andere Person bucht einen passenden Slot.

## 2. Nicht-Ziele und Anti-Ziele

**Nicht in der ersten Version:**

- Globales Tastenkürzel für die Schnelleingabe aus jeder App heraus (auf dem Mac). Vorgemerkt für später; in v1 läuft die Schnelleingabe aus der App.
- Widgets und Apple Watch. *(Meine Setzung, vom Nutzer nicht beanstandet.)*
- Automatisch erzeugte Zoom-/Teams-/Meet-Links bei Buchungen. Ein fester Videolink pro Vorlage reicht.
- Umbuchen durch den Gast. Ein Gast sagt über die Einladung ab und bucht neu.
- Aufgaben/Erinnerungen (Reminders). Aufgaben liegen in einem anderen System.
- KI im Produkt. Der Satz-Parser ist regelbasiert; Intelligenz kommt von außen über MCP.
- Android, Web-Oberfläche für den Kalender selbst (die Buchungsseite ist die einzige Web-Oberfläche).

**Anti-Ziele (wäre ein Fehlschlag, auch wenn es technisch läuft):**

- Die App fühlt sich träge oder „wie ein Web-Wrapper“ an.
- Einladungen funktionieren nur halb: Gäste bekommen keine echte Einladung, Zu- und Absagen kommen nicht zurück.
- Termine gehen beim Sync verloren oder verdoppeln sich.
- Claude schreibt über MCP in den Arbeitskalender, ohne dass das ausdrücklich erlaubt ist.
- Die Buchungsseite verrät Details aus dem Kalender (Titel, Orte, Teilnehmer).

## 3. Architektur

**Entscheidung des Nutzers: kein EventKit.** Lichtung spricht wie Fantastical direkt mit den Kalenderanbietern. Grund: EventKit kann keine Einladungen (keine Teilnehmer anlegen, nicht annehmen/ablehnen), und „Einladungen müssen einfach funktionieren“. Mit dem direkten Weg ist das System einheitlich.

**Entscheidung des Nutzers: Der Server synchronisiert alles, die Apps sprechen nur mit dem Server.**

```
 Fastmail (CalDAV) ─┐
 iCloud (CalDAV) ───┤       ┌──────── lichtung-server (Rust, axum, SQLite) ────────┐
 Arbeit (Graph) ────┼──────▶│ Anbieter-Sync │ Termine/Serien │ Sync-API für Apps     │
 ICS-Abos (lesend) ─┘       │ Openings      │ Buchungsseite  │ /mcp für Claude       │
                            └───────────────────────────────────────────────────────┘
                                    ▲                    ▲               ▲
                     macOS-App ─────┤      iOS-App ──────┘      Gäste (Browser)
                     (SwiftUI, lokaler Cache)
```

| Baustein | Aufgabe |
|---|---|
| `lichtung-server` | Rust/axum/SQLite. Hält die Zugangsdaten, synchronisiert alle Konten, ist die Autorität für den Zustand, liefert Sync-API, MCP und Buchungsseite. Ein Container hinter einem Reverse Proxy. |
| Swift-Package (Kern) | Gemeinsame Logik der Apps: Modelle, lokaler Cache, Sync-Client, Satz-Parser, Serienexpansion für die Anzeige. |
| macOS-App, iOS-App | Native SwiftUI. Xcode-Projekt per xcodegen (`project.yml`), nicht eingecheckt. Bundle-ID-Präfix `dev.rubeen`. |

**Vorbilder im eigenen Bestand** *(meine Setzung)*: Server-Aufbau, Login, Tokens, Deployment und MCP wie im Projekt Zunder (Rust/axum/SQLite, OIDC über Pocket-ID, Container hinter Traefik). App-Aufbau wie im Projekt Rudel (Kern als Swift-Package mit `swift test`, xcodegen).

**Konfiguration:** Keine Konfiguration über Umgebungsvariablen außer dem Bootstrap (öffentliche URL, OIDC, Schlüssel für die Verschlüsselung der Zugangsdaten). Konten, Zugangsdaten, Kalender-Einstellungen und Openings werden in der App gepflegt.

## 4. Konten und Anbieter-Sync

| Konto | Weg | Schreiben | Einladungen |
|---|---|---|---|
| Fastmail | CalDAV, App-Passwort | ja | Server-seitiges Scheduling (RFC 6638): Teilnehmer am Termin → Fastmail verschickt; PARTSTAT ändern → Fastmail antwortet. **Im Spike S2 zu prüfen.** |
| iCloud | CalDAV, App-spezifisches Passwort | ja | wie Fastmail, **im Spike S2 zu prüfen** |
| Arbeitskalender | Microsoft Graph (delta query) | ja | Graph: Termin mit Teilnehmern anlegen, `accept`/`decline`/`tentativelyAccept` |
| ICS-Abos | HTTP-Abruf, lesend | nein | nein |

- **Arbeitskalender hängt an einer Graph-Freigabe** beim Arbeitgeber. Der Nutzer beantragt sie, falls nötig. Bis dahin ist der Arbeitskalender **nur lesend über einen ICS-Feed** angebunden (ein solcher Feed existiert bereits). Ein zweiter Kalender des Arbeitskontos, der nicht im Feed steckt, fehlt bis dahin. Annehmen/Ablehnen von Arbeitsanfragen geht erst mit Graph, ist aber Kündigungskriterium (Abschnitt 1).
- **Sync-Verfahren** *(Builder)*: CalDAV mit `sync-collection` (RFC 6578), Fallback über ctag/ETag; Graph über delta query; ICS per Intervall mit ETag/Last-Modified. Intervalle und Push-Mechanismen (z. B. Graph-Subscriptions) wählt der Builder.
- **Gespeichert wird das iCalendar-Original** (bzw. das Graph-Objekt) pro Termin, damit nichts verloren geht, was Lichtung nicht modelliert. Abgeleitet daraus: normalisierte Felder und expandierte Vorkommen für Abfragen.
- **Serien:** RRULE, EXDATE, RDATE, überschriebene Vorkommen (RECURRENCE-ID), Zeitzonen inkl. Sommerzeit und ganztägige Termine müssen korrekt sein. Beim Bearbeiten: „nur dieser“, „dieser und folgende“, „alle“.
- **Schreiben:** mit `If-Match`/ETag bzw. Graph-Änderungsschutz. **Konflikt:** Die Version des Anbieters gewinnt, die lokale Änderung wird dem Nutzer sichtbar gemeldet. Nichts verschwindet still.
- **Geteilte Kalender** (in Fastmail oder iCloud freigegebene Kalender anderer) gehören dazu, so wie sie im Konto erscheinen, mit den Rechten, die die Freigabe hat (nur lesen oder auch schreiben) *(meine Setzung)*. Die Verfügbarkeit anderer Personen (Graph `getSchedule`) und delegierte Kalender sind kein Ziel.
- **Zugangsdaten** liegen verschlüsselt in der Datenbank (Schlüssel aus dem Bootstrap).
- **Vollständiger Spiegel:** Termine aller Konten liegen vollständig (mit Titeln und Teilnehmern) auf dem eigenen Server, auch die des Arbeitskontos. Das ist eine bewusste Entscheidung; MCP und Openings brauchen den vollständigen Stand.

## 5. Apps

### Ansichten

Alle Ansichten kommen in die erste Version, **A und C sind die Hauptansichten** und bekommen den meisten Feinschliff. Entwürfe aus dem Brainstorming: Tagesleiste + Liste, Zeitraster, Mac-Seitenleiste + Woche, Monat.

| | iPhone | Mac |
|---|---|---|
| **A** Tagesleiste + Liste (DayTicker oben, Termine als endlose Liste darunter) | Startbildschirm | in der Seitenleiste (siehe C) |
| **B** Zeitraster Tag / 3 Tage / Woche | ja | Tag / Woche |
| **C** Seitenleiste (Mini-Monat + Liste der nächsten Termine) + Raster | – | Hauptfenster |
| **D** Monat | ja | ja |
| **Jahr** | – | ja |

- **Kalender-Sets** wie in Fantastical (z. B. „Arbeit“, „Privat“): Kalender gruppiert ein- und ausblenden. *(Meine Setzung, vom Nutzer nicht beanstandet.)*
- **Terminanfragen** erscheinen gesammelt (Badge/Liste) und lassen sich direkt annehmen, mit Vielleicht beantworten oder ablehnen, auf beiden Plattformen.
- **Termin anlegen und bearbeiten:** inkl. Teilnehmer (→ echte Einladung über den Anbieter), Ort, Notiz, Erinnerungen, Wiederholung, Zielkalender.
- **Erinnerungen** an Termine stellt die App als lokale Benachrichtigungen aus den VALARM-/Graph-Erinnerungen ein *(Builder: Zeitfenster, Aktualisierung)*. Doppelte Benachrichtigungen durch parallel eingerichtete System-Kalenderkonten sind ein Einrichtungsthema, kein Code-Thema (siehe Risiken).

### Schnelleingabe per Satz

- Ein Eingabefeld, das einen deutschen Satz versteht („Zahnarzt Di 14 Uhr“, „Team-Sync morgen 9–9:30 in Raum 2“, „jeden Montag 7:30 Review“, „Urlaub 3.–7.11.“) und **schon beim Tippen eine Vorschau** zeigt: Titel, Tag, Uhrzeit/Dauer, Kalender, Wiederholung.
- **Regelbasierter eigener Parser, ohne KI**, im Swift-Kern, offline, deterministisch *(meine Setzung)*. Englisch ist nicht nötig.
- Umfangreich getestet über eine Tabelle Eingabe → erwartetes Ergebnis mit festem „Jetzt“.
- Auf dem Mac in v1 aus der App heraus; globales Kürzel später.

### Offline und Live-Updates

- Die Apps halten einen lokalen Cache (Zeitraum *Builder*, z. B. −3 bis +12 Monate) und sind damit auch offline lesbar und schnell.
- Offline angelegte oder geänderte Termine werden als ausstehende Änderungen gehalten und nachgesendet. Konflikte nach Abschnitt 4.
- Live-Updates bei geöffneter App vom Server (z. B. SSE wie in Zunder).
- **Erinnerungen müssen auch für Termine zuverlässig kommen, die die App nie gesehen hat** (z. B. morgens per MCP angelegte Zeitblöcke). Dafür schickt der Server stille Push-Nachrichten über APNs, die App aktualisiert daraufhin Cache und lokale Benachrichtigungen *(meine Setzung; ein bezahlter Apple-Developer-Account ist vorhanden, Verteilung über TestFlight bzw. direkte Installation)*.

## 6. Openings (Buchungslinks)

Beide Arten gibt es, beide sind bestätigt:

- **Einmal-Link:** In der App Dauer und Zielkalender wählen, optional konkrete Vorschläge markieren, Link kopieren. Nach einer Buchung ist der Link verbraucht.
- **Markierte Vorschläge blockieren die Zeit** (Nutzer): Der Server legt für jeden Vorschlag einen Platzhalter-Termin ohne Teilnehmer im Zielkalender an. Platzhalter zählen überall als belegt (andere Openings, `find_free_times`), nur nicht für ihren eigenen Link. Bei der Buchung werden die übrigen Platzhalter des Links entfernt, beim Löschen des Links alle.
- **Links laufen nicht ab** (Nutzer). Vorschläge, deren Zeit vorbei ist, fallen weg, ihre Platzhalter werden entfernt *(meine Setzung)*. Die App zeigt offene Einmal-Links als Liste zum Löschen.
- **Fester Link (Vorlage):** z. B. „30 Minuten Kennenlernen“ mit Zeitfenstern (z. B. werktags 9–17 Uhr), Puffer vor/nach Terminen, Mindestvorlauf (z. B. 4 h), Buchungshorizont (z. B. 3 Wochen), optional festem Videolink.
- **Zielkalender** ist pro Link/Vorlage wählbar, aber nur unter Kalendern, die Einladungen verschicken können (Fastmail sofort, Arbeitskalender nach der Graph-Freigabe, iCloud nach Spike S2).
- **Frei** heißt: kein Termin in einem einbezogenen Kalender. Pro Kalender einstellbar, ob er für Openings zählt; ganztägige Termine blockieren nur, wenn das für den Kalender eingestellt ist. Abgesagte Termine und als „frei“ markierte (TRANSP/showAs free) blockieren nicht.
- **Buchungsseite:** schlicht, Deutsch und Englisch, Zeitzone des Gastes. Der Gast gibt Name, E-Mail und optional eine Notiz an. Die Seite zeigt **nur freie Slots**, nie Termindetails.
- **Buchen:** Der Server prüft unmittelbar vor dem Schreiben erneut gegen den aktuellen Stand, legt den Termin mit dem Gast als Teilnehmer im Zielkalender an, und der Anbieter verschickt die Einladung von der eigenen Adresse. Absagen läuft über die Einladung.
- **Sicherheit:** Links mit nicht erratbaren Kennungen, Rate-Limiting, Schutz gegen Massenbuchungen und Spam *(Mechanismus: Builder)*. Die Buchungsseite ist der einzige öffentlich erreichbare Teil ohne Login.

## 7. MCP

Unter `/mcp`, Zugang wie bei Zunder: OAuth für Claude.ai und API-Tokens, die in der App erzeugt und einzeln widerrufen werden.

Mindestumfang (Parität zu dem, was die Planungs-Skills heute bei Fantastical nutzen):

| Werkzeug | Zweck |
|---|---|
| `list_calendars`, `list_calendar_sets` | IDs, Namen, Konto, beschreibbar ja/nein |
| `query_events` | Zeitraum (ohne Suchbegriff alles), optional Kalender-Filter. Liefert auch Beschreibung, Dauer, Ganztägig, Teilnehmer, Status |
| `create_event`, `update_event`, `delete_event` | **`calendar_id` ist Pflicht**, kein Default-Kalender. Serien mit Geltungsbereich (dieser / folgende / alle) |
| `find_free_times` | freie Zeiten in einem Zeitraum nach denselben Regeln wie Openings |
| `list_invitations`, `respond_to_invitation` | offene Anfragen, annehmen / vielleicht / ablehnen |
| `create_opening_link` | Einmal-Link erzeugen |

- **Schreibschutz pro Kalender für MCP:** In der App wird pro Kalender festgelegt, ob Claude dort schreiben darf. **Standard für den Arbeitskalender: nein.** Der Schreibschutz sperrt auch `respond_to_invitation` für diesen Kalender *(meine Setzung)*. Hintergrund: Fantastical schreibt ohne Kalender-ID still in seinen Default-Kalender; ein Fehlgriff darf hier nicht im Arbeitskalender landen.
- Zeiten immer mit Zeitzone; Zeiträume als ISO-Datum. Fehlermeldungen sagen, was falsch war und wie es richtig geht.

## 8. Login und Betrieb

- Login der Apps über OIDC (Pocket-ID) mit Geräte-Tokens, wie in Zunder.
- Ein Container, SQLite, Backups; Deployment wie Zunder (Image in der CI, Compose hinter Traefik) *(Details: Builder)*.
- Tests: Sync-Logik gegen aufgezeichnete Antworten (synthetisch) und gegen einen lokalen CalDAV-Server *(Builder: z. B. Radicale)*; Serienexpansion und Parser mit Tabellentests.
- **Öffentliches Repo:** Testdaten, Fixtures, Screenshots und Beispiele sind ausnahmslos erfunden. Keine echten Termine, Namen, Adressen oder Kalender-IDs.

## 9. Etappen *(meine Setzung, Reihenfolge änderbar)*

| Etappe | Inhalt | Ergebnis |
|---|---|---|
| **S Spikes** | **S1 ist das Tor vor allem anderen:** Graph-Zugang zum Arbeitskalender (Freigabe nötig? delta, accept/decline). S2: Fastmail und iCloud per CalDAV: Teilnehmer anlegen → wird eingeladen? PARTSTAT setzen → geht Antwort raus? S3: ICS-Feed des Arbeitskalenders lesbar | Weg pro Anbieter bestätigt, Graph-Freigabe ggf. beantragt |
| **E1 Server-Kern** | Konten, Sync Fastmail/iCloud/ICS, Serien, Sync-API, MCP (lesen + schreiben mit Schreibschutz) | Planungs-Skills können vom Fantastical-MCP umziehen |
| **E2 Apps** | iPhone A/B/D, Mac C/B/D/Jahr, Termin-Editor, Satz-Parser, Sets, Cache, Erinnerungen | tägliche Nutzung möglich |
| **E3 Einladungen + Openings** | Teilnehmer, Anfragen beantworten, Einmal-Links, Vorlagen, Buchungsseite | Openings ersetzen Fantastical |
| **E4 Arbeitskalender voll** | Graph-Sync statt ICS, Anfragen aus dem Arbeitskalender beantworten | **Fantastical kündbar** |

## 10. Entscheidungen und verworfene Alternativen

| Entscheidung | Verworfen | Warum |
|---|---|---|
| Direkter Anbieter-Sync auf dem Server (Nutzer) | EventKit auf den Geräten | EventKit kann keine Einladungen; zwei Datenwege wären uneinheitlich |
| Server synchronisiert, Apps sind Clients (Nutzer) | Geräte spiegeln zum Server | MCP und Openings sind immer aktuell, auch wenn kein Gerät wach ist; keine Schreib-Warteschlange |
| MCP auf dem Server (Nutzer) | MCP in der Mac-App | funktioniert überall (Claude.ai, iPhone, Routinen), nicht nur am Mac |
| Native SwiftUI | Web/Tauri | Fantastical-Gefühl ist nur nativ erreichbar |
| Regelbasierter Parser | LLM/Apple Foundation Models | Vorschau beim Tippen braucht Tempo und Determinismus |
| Eigene Openings | selbst gehostetes cal.com | ein System statt zwei, Openings nutzen denselben Termin-Spiegel |
| Öffentliches Repo (Nutzer) | privat | wie Zunder; Datenschutz über erfundene Testdaten |

## 11. Risiken

- **Graph-Freigabe beim Arbeitgeber** wird abgelehnt oder dauert: Dann bleibt der Arbeitskalender lesend über ICS, und die Kriterien 2 und 4 sind nicht erfüllbar. Deshalb ist S1 das Tor vor allem anderen. Bei Ablehnung entscheidet der Nutzer zwischen Stopp (Tendenz) und Weiterbau mit „Arbeitsanfragen bleiben in Outlook“ als angepasstem Kriterium. Der Nutzer erwartet, die Freigabe zu bekommen.
- **Sync-Korrektheit** (Serien, Zeitzonen, Ausnahmen, Konflikte) ist der teuerste Teil. Gegenmittel: iCalendar-Original speichern, Tabellentests, Anbieter gewinnt bei Konflikt.
- **CalDAV-Scheduling** verhält sich bei Fastmail oder iCloud anders als erwartet (Spike S2). Fallback, falls nötig: Einladung als iMIP-Mail vom Server *(nur wenn S2 scheitert)*.
- **Doppelte Benachrichtigungen,** wenn dieselben Konten parallel im System-Kalender von iOS/macOS eingerichtet bleiben. Lösung beim Umstieg: dort Benachrichtigungen ausschalten oder Konten entfernen.
- **Abdriften ins Hobby:** großes Projekt. Gegenmittel: Etappen mit nutzbarem Ergebnis, Fantastical läuft parallel bis E4.

## 12. Dem Builder überlassen

Sync-Intervalle und Push-Mechanismen, Datenbankschema, Cache-Zeitraum der Apps, lokale Speichertechnik in Swift, Rate-Limiting-Verfahren, genaue MCP-Parameternamen, Gestaltung der Buchungsseite, Testserver für CalDAV, Details des Deployments.
