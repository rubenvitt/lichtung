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
| `TZ` | Zeitzone für die Backup-Stunde | Image: `Europe/Berlin` |
| `RUST_LOG` | Log-Filter (tracing) | `info,tower_http=info` |

`LICHTUNG_SECRET_KEY` gehört zusätzlich in den Passwortmanager: Ohne ihn sind gespeicherte Zugangsdaten nicht mehr lesbar, auch nicht aus Backups. Nicht nachträglich ändern.

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
- **Wer Zugänge verwalten darf:** eine Browser-Sitzung oder ein Gerät der App. API-, Bootstrap- und OAuth-Tokens dürfen keine Tokens anlegen oder widerrufen. Abweichung von Zunder (dort nur der Browser): Lichtung hat keine Weboberfläche, Tokens werden in der App erzeugt (Spec, Abschnitt 7).
- **CSRF:** Mit Cookie angemeldete, ändernde Anfragen brauchen den Header `X-Lichtung-Client` (beliebiger Wert).
- **Geräte-Login der App:** OAuth-Code-Flow mit PKCE über `ASWebAuthenticationSession`. Die App registriert sich unter `/oauth/register` mit `client_name` (= Gerätename) und einer Redirect-URI wie `dev.rubeen.lichtung:/oauth`, fragt `/oauth/authorize` mit `resource=<public>/api` und tauscht den Code unter `/oauth/token` gegen einen Geräte-Token. Geräte-Tokens gibt es nur für App-Schemata oder localhost, nie für Web-Dienste. Abmelden: `/oauth/revoke` mit dem eigenen Token.
- **claude.ai:** Connector mit der URL `https://lichtung.rubeen.dev/mcp` anlegen (der Endpunkt folgt in LIC-12); Discovery, Registrierung und Freigabe laufen automatisch über die deutsche Freigabeseite.

Fehler kommen als `{"error":{"code":"…","message":"…"}}` mit deutscher Meldung; die OAuth-Endpunkte antworten im OAuth-Format `{"error","error_description"}`.

## Backups

Jede Nacht zur `LICHTUNG_BACKUP_HOUR` schreibt der Server `/data/backups/lichtung-<Datum>-<Zeit>.db` (konsistent per `VACUUM INTO`) und behält die letzten 14. Dabei räumt er abgelaufene Sitzungen und OAuth-Reste auf. Wiederherstellen: Container stoppen, Backup als `data/lichtung.db` ablegen (`-wal`/`-shm` entfernen), starten. Die Backups enthalten die verschlüsselten Zugangsdaten; lesbar sind sie nur mit dem passenden `LICHTUNG_SECRET_KEY`.
