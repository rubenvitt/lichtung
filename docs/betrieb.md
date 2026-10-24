# Betrieb

Wie `lichtung-server` läuft: Container, Konfiguration, Login, Tokens, Backups. Aufbau und Muster stammen aus Zunder (gleicher Autor); hier steht, was für Lichtung gilt.

## Überblick

- **Ein Container** `ghcr.io/rubenvitt/lichtung-server` (amd64 und arm64), gebaut aus [`server/Dockerfile`](../server/Dockerfile) von der CI ([`.github/workflows/server.yml`](../.github/workflows/server.yml)).
- **Daten** liegen unter `/data`: SQLite-Datenbank `lichtung.db` (WAL-Modus) und nächtliche Backups in `/data/backups/` (`VACUUM INTO`, die letzten 14 bleiben).
- **Port** 8080 im Container, nach außen nur über Traefik.
- **Login** über Pocket-ID (OIDC). Die Apps melden sich als Gerät an und bekommen einen Geräte-Token; Werkzeuge nutzen API-Tokens; claude.ai verbindet sich per OAuth.
- **Stack-Dateien** in [`server/deploy/`](../server/deploy): `compose.yaml`, `.env.example` (Vorlage) und `generate-secrets.sh`. Auf dem Server unter `~/server/lichtung/`, betrieben mit `docker compose`.
- **Domain** `lichtung.rubeen.dev` (öffentlich): claude.ai muss `/mcp` erreichen, Gäste später die Buchungsseite.

Ohne Anmeldung erreichbar sind nur `/`, `/healthz`, die Login-Strecke `/auth/*` und der OAuth-Autorisierungsserver (`/.well-known/oauth-*`, `/oauth/*`). Alles unter `/api/*` verlangt eine Sitzung, einen API-Token oder einen Geräte-Token.

## Umgebungsvariablen

Über Umgebungsvariablen läuft nur der Bootstrap. Konten, Zugangsdaten, Kalender und Openings werden in der App gepflegt. Leere Werte gelten als nicht gesetzt.

| Variable | Bedeutung | Standard |
|---|---|---|
| `LICHTUNG_PUBLIC_URL` | öffentliche URL, Basis für OIDC-Callback, OAuth-Issuer und Ressourcen | `http://localhost:<Port>` |
| `LICHTUNG_BIND` | Adresse und Port | `0.0.0.0:8080`, mit Dev-Auth `127.0.0.1:8080` |
| `LICHTUNG_DATA_DIR` | Datenverzeichnis (`lichtung.db`, `backups/`) | `./data` (Image: `/data`) |
| `LICHTUNG_OIDC_ISSUER` | Basis-URL von Pocket-ID | – |
| `LICHTUNG_OIDC_CLIENT_ID` | Client-ID aus Pocket-ID | – |
| `LICHTUNG_OIDC_CLIENT_SECRET` | Client-Secret aus Pocket-ID (leer = öffentlicher Client nur mit PKCE) | – |
| `LICHTUNG_OIDC_ALLOWED_EMAIL` | die eine Person, die sich anmelden darf (verifizierte E-Mail) | – |
| `LICHTUNG_OIDC_ALLOWED_SUBJECT` | Alternative zur E-Mail: `sub` des Kontos; hat Vorrang | – |
| `LICHTUNG_SESSION_SECRET` | mindestens 32 Zeichen; Pflicht mit OIDC. Ändern meldet alle Browser-Sitzungen ab | – |
| `LICHTUNG_SECRET_KEY` | Schlüssel für die Zugangsdaten der Kalenderkonten: 32 Byte als base64 oder 64 Hex-Zeichen (`openssl rand -base64 32`). **Pflicht**, außer mit Dev-Auth | – |
| `LICHTUNG_BOOTSTRAP_TOKEN` | fester API-Token für die Inbetriebnahme oder Tests | – |
| `LICHTUNG_BACKUP_HOUR` | Stunde des nächtlichen Backups (0–23, Zeitzone `TZ`) | `3` |
| `LICHTUNG_DEV_AUTH` | `true` = Anmeldung ohne OIDC, nur über localhost. **Nie in Produktion** | `false` |
| `LICHTUNG_DEV_AUTH_ALLOW_REMOTE` | `true` = Dev-Auth auch mit nicht lokaler `LICHTUNG_BIND` (nur für einen lokalen Container mit `-p 127.0.0.1:…`) | `false` |
| `TZ` | Zeitzone für die Backup-Stunde und für floating/ganztägige Termine bei Server-Abfragen | Image: `Europe/Berlin` |
| `RUST_LOG` | Log-Filter (tracing) | `info,tower_http=info` |

`LICHTUNG_SECRET_KEY` gehört zusätzlich in den Passwortmanager: Ohne ihn sind gespeicherte Zugangsdaten nicht mehr lesbar, auch nicht aus Backups. Nicht nachträglich ändern (siehe [Schlüssel für die Zugangsdaten](#schlüssel-für-die-zugangsdaten)).

## Schlüssel für die Zugangsdaten

Die Zugangsdaten der Kalenderkonten (App-Passwörter von Fastmail, iCloud, CalDAV-Servern) liegen nur verschlüsselt in der Datenbank: AES-256-GCM mit `LICHTUNG_SECRET_KEY`, zufälliger Nonce pro Verschlüsselung, die Konto-ID als zusätzliche Prüfung (ein Eintrag lässt sich nicht einem anderen Konto unterschieben). Passwörter stehen in keiner API-Antwort und in keinem Log.

- **Prüfwert:** Beim ersten Start legt der Server in der Tabelle `meta` einen mit dem Schlüssel verschlüsselten Prüfwert ab. Bei jedem weiteren Start prüft er ihn. Passt `LICHTUNG_SECRET_KEY` nicht zur Datenbank (falscher Schlüssel, Tippfehler, Dev-Schlüssel), **startet der Server nicht** und meldet: „LICHTUNG_SECRET_KEY passt nicht zu dieser Datenbank …“. Dann den richtigen Schlüssel aus dem Passwortmanager setzen.
- **Schlüssel verloren:** Die gespeicherten Zugangsdaten sind dann nicht mehr zu retten. Neuen Schlüssel erzeugen und eintragen, dann bei gestopptem Server einmal
  ```sh
  docker compose run --rm lichtung lichtung-server reset-secret-key
  ```
  ausführen (Dienstname wie in `compose.yaml`). Das verwirft alle gespeicherten Zugangsdaten und trägt den Prüfwert für den neuen Schlüssel ein. Konten, Kalender, Einstellungen und Sets bleiben; die Konten stehen auf `auth_failed` und werden in der App **neu verbunden** (App-Passwort erneut eingeben, ggf. beim Anbieter ein neues erzeugen).
- **Backups** enthalten die Zugangsdaten nur verschlüsselt; ein Backup ist nur zusammen mit dem Schlüssel, der beim Backup galt, vollständig.

## Erstinbetriebnahme

### 1. OIDC-Client in Pocket-ID anlegen

In der Pocket-ID-Administration unter **OIDC Clients → Add OIDC Client**:

| Feld | Wert |
|---|---|
| Name | `Lichtung` |
| Callback URLs | `https://lichtung.rubeen.dev/auth/callback` |
| Public Client | aus (Lichtung ist ein vertraulicher Client mit Secret) |
| PKCE | an |

Client-ID, Client-Secret und die Basis-URL von Pocket-ID (Issuer) notieren. Lichtung liest `<Issuer>/.well-known/openid-configuration`.

### 2. Ordner anlegen, Geheimnisse erzeugen, starten

```sh
mkdir -p ~/server/lichtung/data && cd ~/server/lichtung
for f in compose.yaml .env.example generate-secrets.sh; do
  curl -fsSLO "https://raw.githubusercontent.com/rubenvitt/lichtung/main/server/deploy/$f"
done
chmod +x generate-secrets.sh
sudo chown 10001:10001 data   # der Container läuft als uid/gid 10001
./generate-secrets.sh         # erzeugt Session-Secret und Schlüssel, fragt OIDC ab
docker compose pull && docker compose up -d
curl -fsS https://lichtung.rubeen.dev/healthz   # {"status":"ok","version":"…"}
```

Voraussetzungen wie bei Zunder: externes Netzwerk `proxy` mit Traefik (Entrypoint `https`, Resolver `letsencrypt`) und ein DNS-Eintrag für `lichtung.rubeen.dev`.

### Automatisches Ausrollen

Nach jedem grünen Push auf `main`, der `server/` ändert, legt die CI ein GitHub-Deployment an (Umgebung `production`, `payload.tag` = Commit-SHA, `payload.image` = `ghcr.io/rubenvitt/lichtung-server`). Der `~/server/deploy-agent` greift es auf, sobald Lichtung dort eingetragen ist; ohne Eintrag bleibt es in der Warteschlange.

## Lokal starten (Dev-Auth)

```sh
cd server
LICHTUNG_DEV_AUTH=true cargo run -p lichtung-server
curl -s http://localhost:8080/api/me
```

Mit Dev-Auth gilt jede Anfrage an `localhost` als angemeldet (Browser-Sitzung), der Server lauscht nur auf `127.0.0.1`, und ohne `LICHTUNG_SECRET_KEY` wird ein fester Entwicklungsschlüssel benutzt (mit Warnung im Log). Eine so entstandene Datenbank nie in Produktion übernehmen. Ändernde Anfragen brauchen wie im Browser den Header `X-Lichtung-Client`:

```sh
curl -s -X POST http://localhost:8080/api/tokens \
  -H 'X-Lichtung-Client: curl' -H 'Content-Type: application/json' \
  -d '{"name":"werkzeug"}'
```

## Zugänge

| Zugang | Form | gilt für | verwaltet unter |
|---|---|---|---|
| Browser-Sitzung | Cookie `lichtung_session`, 30 Tage gleitend | `/api`, OAuth-Freigabe | `/auth/login`, `/auth/logout` |
| API-Token | `lt_…` | `/api` und `/mcp` | `/api/tokens` |
| Geräte-Token | `ld_…`, läuft nicht ab | nur `/api` | `/api/devices` |
| OAuth (claude.ai) | Access `loa_…` (1 h), Refresh `lor_…` (30 Tage, rotierend) | nur `/mcp` | `/api/oauth/clients` |
| Bootstrap-Token | Wert von `LICHTUNG_BOOTSTRAP_TOKEN` | wie ein API-Token | Umgebung |

- Gespeichert werden nur Hashes (Tokens und Codes SHA-256, Sitzungen HMAC-SHA256).
- **Kalenderkonten und Kalender-Einstellungen:** Konten anlegen, ändern, löschen sowie Kalender-Einstellungen und Kalender-Sets ändern dürfen wie bei den Zugängen nur Browser-Sitzung und Geräte. API-Tokens dürfen lesen und einen Abgleich anstoßen, aber nichts ändern: Sie gelten auch für `/mcp`, und der Schreibschutz pro Kalender (`mcp_write`) soll nicht von Claude selbst aufgehoben werden können.
- **Wer Zugänge verwalten darf:** eine Browser-Sitzung oder ein Gerät der App. API-, Bootstrap- und OAuth-Tokens dürfen keine Tokens anlegen oder widerrufen. Abweichung von Zunder (dort nur der Browser): Lichtung hat keine Weboberfläche, Tokens werden in der App erzeugt (Spec, Abschnitt 7).
- **CSRF:** Mit Cookie angemeldete, ändernde Anfragen brauchen den Header `X-Lichtung-Client` (beliebiger Wert).
- **Geräte-Login der App:** OAuth-Code-Flow mit PKCE über `ASWebAuthenticationSession`. Die App registriert sich unter `/oauth/register` mit `client_name` (= Gerätename) und einer Redirect-URI wie `dev.rubeen.lichtung:/oauth`, fragt `/oauth/authorize` mit `resource=<public>/api` und tauscht den Code unter `/oauth/token` gegen einen Geräte-Token. Geräte-Tokens gibt es nur für App-Schemata oder localhost, nie für Web-Dienste. Abmelden: `/oauth/revoke` mit dem eigenen Token.
- **claude.ai:** Connector mit der URL `https://lichtung.rubeen.dev/mcp` anlegen; Discovery, Registrierung und Freigabe laufen automatisch über die deutsche Freigabeseite. Werkzeuge, Schreibschutz und Einrichtung per API-Token: [`docs/mcp.md`](mcp.md).

Fehler kommen als `{"error":{"code":"…","message":"…"}}` mit deutscher Meldung; die OAuth-Endpunkte antworten im OAuth-Format `{"error","error_description"}`.

## Kalender-Abgleich

- **Rhythmus:** Beim Start gleicht der Server alle Konten ab, danach jedes Konto alle 5 Minuten (ICS-Abos alle 15 Minuten); „Jetzt synchronisieren“ in der App (`POST /api/accounts/{id}/sync`) stößt sofort einen Lauf an. Pro Konto läuft höchstens ein Abgleich gleichzeitig, Konten laufen unabhängig voneinander.
- **Verfahren:** CalDAV mit `sync-collection` (RFC 6578). Ohne Token, mit abgelaufenem Token oder bei Servern ohne `sync-collection` vergleicht der Server alle ETags (bei unverändertem ctag gar nicht). Neue und geänderte Termine holt er per `calendar-multiget`.
- **Nichts geht verloren, nichts doppelt:** Gespeichert wird das iCalendar-Original (Zeilenenden auf CRLF vereinheitlicht), auch wenn es nicht lesbar ist. Die Änderungen einer Runde und der neue Sync-Token landen in einer Transaktion; bricht ein Lauf ab (Netz, Fehler beim Anbieter, Neustart), wiederholt der nächste die Runde.
- **Status:** `syncing` während des Laufs, danach `ok`. Vorübergehende Fehler setzen `error` mit Meldung; der nächste Versuch kommt nach 1, 2, 4 … höchstens 60 Minuten. Bei `auth_failed` (Anbieter lehnt das App-Passwort ab) versucht es der Server erst wieder, wenn in der App neue Zugangsdaten gespeichert sind.
- **ICS-Abos** (`kind: ics`, z. B. der veröffentlichte Outlook-Kalender der Arbeit): nur lesend, ein Kalender pro Abo, Claude darf dort nie schreiben (`mcp_write` aus). Der Server ruft den Feed alle 15 Minuten ab, bedingt mit `If-None-Match`/`If-Modified-Since`; `304` heißt „nichts zu tun“. Sonst zerlegt er den Feed pro UID und vergleicht jeden Termin über einen Inhalts-Hash ohne `DTSTAMP` mit dem Bestand (neu, geändert, entfernt), alles in einer Transaktion. Grenzen: 20 MB, 60 s, höchstens 5 Weiterleitungen. `webcal://` wird zuerst über HTTPS versucht, `webcals://` nur über HTTPS.
- **Geheime Feed-URL:** Die URL eines Abos enthält meist ein Token und gilt wie ein Passwort: Sie liegt nur verschlüsselt in der Datenbank (wie die App-Passwörter, mit `LICHTUNG_SECRET_KEY`), Logs und API-Antworten zeigen höchstens Host und Pfad ohne Query und ohne schlüsselartige Pfadteile. Wird ein Link bekannt, beim Anbieter einen neuen erzeugen (Outlook: Veröffentlichung beenden und neu veröffentlichen) und in der App eintragen. Nach `reset-secret-key` stehen auch ICS-Konten auf `auth_failed` und brauchen die URL neu.
- **Drosselung:** Antwortet ein Anbieter mit 429 oder 503 und `Retry-After` (iCloud tut das bei vielen Anfragen), bricht der Lauf ab, statt die übrigen Kalender weiter abzufragen. Der nächste Versuch kommt frühestens nach der verlangten Pause, mindestens aber nach dem üblichen Abstand.
- **Zeitzone des Servers:** Floating-Zeiten und ganztägige Termine rechnet der Server in `TZ` (Image: `Europe/Berlin`), falls dort eine IANA-Zone steht, sonst in UTC.

### iCloud

- **App-spezifisches Passwort:** iCloud nimmt für CalDAV nur App-spezifische Passwörter an. Erzeugen unter appleid.apple.com → Anmelden und Sicherheit → App-spezifische Passwörter (die Apple-ID braucht dafür die Zwei-Faktor-Authentifizierung). In der App `iCloud` wählen, Benutzername ist die E-Mail-Adresse der Apple-ID. Wer das normale Apple-ID-Passwort eingibt, bekommt `invalid_credentials` mit genau diesem Hinweis. Wird das Passwort bei Apple widerrufen oder das Apple-ID-Passwort geändert (das widerruft alle App-spezifischen Passwörter), steht das Konto auf `auth_failed`. Dann ein neues Passwort erzeugen und in der App eintragen.
- **Adressen:** Lichtung beginnt bei `https://caldav.icloud.com/`. iCloud leitet auf einen Partitions-Host weiter (`pNN-caldav.icloud.com`), dort liegen die Kalender. Die Zugangsdaten gehen nur per HTTPS an Hosts unter `icloud.com`.
- **Geteilte Kalender:** Kalender, die andere mit dem Konto teilen, erscheinen als eigene Kalender mit `shared: true` und den Rechten der Freigabe. Bei einer nur lesenden Freigabe sind `writable` und `mcp_write` `false`, und `mcp_write` lässt sich dort auch nicht einschalten. Eigene Kalender, die man selbst mit anderen teilt, bleiben normale Kalender (`shared: false`). Eine Freigabe muss in der Kalender-App am Mac oder iPhone erst angenommen sein, bevor sie über CalDAV sichtbar wird. Zieht jemand die Freigabe zurück, verschwindet der Kalender beim nächsten Abgleich samt Terminen.
- **Nicht übernommen:** Erinnerungslisten (nur Aufgaben), Eingang/Ausgang für Einladungen und Mitteilungen.

## Schreiben und Konflikte

- **Schreiben:** Änderungen aus den Apps und über MCP schreibt der Server sofort zum Anbieter: neue Termine mit `If-None-Match: *`, Änderungen und Löschungen mit `If-Match` auf dem gespeicherten ETag. Ausgangspunkt ist immer das gespeicherte iCalendar-Original, Felder, die Lichtung nicht kennt, bleiben erhalten. Ein Schreibvorgang und der Abgleich desselben Kalenders laufen nie gleichzeitig.
- **Konflikt (der Anbieter gewinnt):** Hat sich der Termin seit dem Stand der App geändert (andere `version`, oder der Anbieter antwortet 412 bzw. 404), holt der Server den Termin sofort neu, verwirft die Änderung und legt einen Konflikt an. Die App zeigt ihn mit der verworfenen Änderung, bis jemand ihn wegklickt (`POST /api/conflicts/{id}/dismiss`). Nichts verschwindet still; im Log steht `write conflict, provider wins`.
- **Rechte:** Nur lesbare Kalender (geteilt ohne Schreibrecht, ICS-Abos) lehnen Änderungen ab (`calendar_read_only`). `mcp_write` gilt nur für Claude (`/mcp`): Ist es aus, lehnt der Server jedes Schreiben über MCP in diesem Kalender ab, auch Antworten auf Einladungen (`mcp_write_forbidden`). Neue Kalender von ICS- und Microsoft-Konten (Arbeitskalender) starten mit `mcp_write` aus.
- **Anbieter nicht erreichbar:** Die App behält die Änderung und schickt sie später erneut. Lehnt der Anbieter das App-Passwort ab, steht das Konto auf `auth_failed`.

## Aufbewahrung

- **Löschungen** (Tombstones für den Pull der Apps) bleiben 90 Tage. Eine App, die länger nicht synchronisiert hat, bekommt `410 cursor_expired` und lädt ihren Cache neu; ausstehende Änderungen behält sie.
- **Ergebnisse von Mutationen** (für die Idempotenz) bleiben 30 Tage.
- **Konflikte** bleiben, bis sie weggeklickt werden.
- Aufgeräumt wird nachts zusammen mit dem Backup.

## Backups

Jede Nacht zur `LICHTUNG_BACKUP_HOUR` schreibt der Server `/data/backups/lichtung-<Datum>-<Zeit>.db` (konsistent per `VACUUM INTO`) und behält die letzten 14. Dabei räumt er abgelaufene Sitzungen, OAuth-Reste und alte Sync-Daten auf (siehe [Aufbewahrung](#aufbewahrung)). Wiederherstellen: Container stoppen, Backup als `data/lichtung.db` ablegen (`-wal`/`-shm` entfernen), starten. Die Backups enthalten die verschlüsselten Zugangsdaten; lesbar sind sie nur mit dem passenden `LICHTUNG_SECRET_KEY`.
