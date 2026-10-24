//! Server configuration from environment variables (bootstrap only, spec
//! section 3): public URL, OIDC, secrets. Everything else (accounts,
//! calendars, openings) is managed in the apps and lives in the database.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use base64::Engine;
use url::Url;

/// Minimum length of `LICHTUNG_SESSION_SECRET`.
pub const MIN_SESSION_SECRET_LEN: usize = 32;

/// Length of `LICHTUNG_SECRET_KEY` in bytes.
pub const SECRET_KEY_LEN: usize = 32;

/// Key used to hash session IDs when dev auth runs without a session secret.
const DEV_SESSION_SECRET: &str = "lichtung-dev-auth-session-secret-not-for-production";

/// Fixed key for encrypting provider credentials under dev auth when
/// `LICHTUNG_SECRET_KEY` is not set. Never used in production: without dev
/// auth the variable is mandatory.
pub const DEV_SECRET_KEY: [u8; SECRET_KEY_LEN] = *b"lichtung-dev-key-not-for-prod!!!";

/// CalDAV entry point of Fastmail.
pub const FASTMAIL_CALDAV_URL: &str = "https://caldav.fastmail.com/dav/";
/// CalDAV entry point of iCloud.
pub const ICLOUD_CALDAV_URL: &str = "https://caldav.icloud.com/";

/// Configuration error with a German, user-facing message.
#[derive(Debug, thiserror::Error)]
#[error("Konfigurationsfehler: {0}")]
pub struct ConfigError(pub String);

fn err<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(msg.into()))
}

/// OpenID Connect settings (Pocket-ID).
#[derive(Clone)]
pub struct OidcConfig {
    pub issuer: Url,
    pub client_id: String,
    /// `None` = public client (PKCE only).
    pub client_secret: Option<String>,
    pub allowed_subject: Option<String>,
    pub allowed_email: Option<String>,
}

impl fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcConfig")
            .field("issuer", &self.issuer.as_str())
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "***"))
            .field("allowed_subject", &self.allowed_subject)
            .field("allowed_email", &self.allowed_email)
            .finish()
    }
}

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub public_url: Url,
    pub oidc: Option<OidcConfig>,
    /// Key for hashing session IDs (HMAC-SHA256) and sealing the login
    /// cookie.
    pub session_secret: String,
    /// Key for encrypting provider credentials in the database
    /// (`LICHTUNG_SECRET_KEY`, see [`crate::crypto`]).
    pub secret_key: [u8; SECRET_KEY_LEN],
    pub bootstrap_token: Option<String>,
    pub dev_auth: bool,
    /// Local hour (0–23) of the nightly backup.
    pub backup_hour: u8,
    /// CalDAV base URL for `kind: fastmail` accounts. Not configurable
    /// through the environment; tests point it at a mock server.
    pub fastmail_url: Url,
    /// CalDAV base URL for `kind: icloud` accounts (like `fastmail_url`).
    pub icloud_url: Url,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("bind", &self.bind)
            .field("data_dir", &self.data_dir)
            .field("public_url", &self.public_url.as_str())
            .field("oidc", &self.oidc)
            .field("session_secret", &"***")
            .field("secret_key", &"***")
            .field(
                "bootstrap_token",
                &self.bootstrap_token.as_ref().map(|_| "***"),
            )
            .field("dev_auth", &self.dev_auth)
            .field("backup_hour", &self.backup_hour)
            .field("fastmail_url", &self.fastmail_url.as_str())
            .field("icloud_url", &self.icloud_url.as_str())
            .finish()
    }
}

/// Parses `LICHTUNG_SECRET_KEY`: 64 hex characters or base64 (standard or
/// URL-safe, with or without padding) of exactly 32 bytes.
pub fn parse_secret_key(raw: &str) -> Result<[u8; SECRET_KEY_LEN], ConfigError> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let raw = raw.trim();
    let bytes = if raw.len() == 2 * SECRET_KEY_LEN && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        (0..SECRET_KEY_LEN)
            .map(|i| u8::from_str_radix(&raw[2 * i..2 * i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|_| ConfigError("LICHTUNG_SECRET_KEY ist kein gültiges Hex".into()))?
    } else {
        [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
            .iter()
            .find_map(|engine| engine.decode(raw).ok())
            .ok_or_else(|| {
                ConfigError(
                    "LICHTUNG_SECRET_KEY muss base64 oder 64 Hex-Zeichen sein \
                     (erzeugen mit: openssl rand -base64 32)"
                        .into(),
                )
            })?
    };
    let key: [u8; SECRET_KEY_LEN] = bytes.try_into().map_err(|b: Vec<u8>| {
        ConfigError(format!(
            "LICHTUNG_SECRET_KEY muss genau {SECRET_KEY_LEN} Byte lang sein, nicht {} \
             (erzeugen mit: openssl rand -base64 32)",
            b.len()
        ))
    })?;
    if key.iter().all(|b| *b == key[0]) {
        return err("LICHTUNG_SECRET_KEY ist kein zufälliger Schlüssel (alle Bytes gleich)");
    }
    Ok(key)
}

fn parse_flag(key: &str, value: Option<&str>) -> Result<bool, ConfigError> {
    match value {
        None | Some("false") | Some("0") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(other) => err(format!("{key} muss true oder false sein: {other}")),
    }
}

/// `LICHTUNG_BIND`, or its default: loopback with dev auth, all interfaces
/// otherwise.
fn parse_bind(value: Option<&str>, dev_auth: bool) -> Result<SocketAddr, ConfigError> {
    match value {
        Some(v) => v
            .parse()
            .map_err(|_| ConfigError(format!("LICHTUNG_BIND ist keine Adresse: {v}"))),
        None if dev_auth => Ok(SocketAddr::from(([127, 0, 0, 1], 8080))),
        None => Ok(SocketAddr::from(([0, 0, 0, 0], 8080))),
    }
}

/// Reads a variable through `lookup`; empty values count as unset.
fn getter(lookup: impl Fn(&str) -> Option<String>) -> impl Fn(&str) -> Option<String> {
    move |key: &str| {
        lookup(key)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    }
}

impl Config {
    /// Reads the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// The bind address alone (for `lichtung-server healthcheck`, which must
    /// not depend on the secrets being present).
    pub fn bind_from_env() -> Result<SocketAddr, ConfigError> {
        Self::bind_from_lookup(|key| std::env::var(key).ok())
    }

    /// Like [`Config::bind_from_env`] through `lookup` (testable).
    pub fn bind_from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<SocketAddr, ConfigError> {
        let get = getter(lookup);
        let dev_auth = parse_flag("LICHTUNG_DEV_AUTH", get("LICHTUNG_DEV_AUTH").as_deref())?;
        parse_bind(get("LICHTUNG_BIND").as_deref(), dev_auth)
    }

    /// Reads the configuration through `lookup` (testable). Empty values
    /// count as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = getter(lookup);
        let flag = |key: &str| parse_flag(key, get(key).as_deref());
        let dev_auth = flag("LICHTUNG_DEV_AUTH")?;
        let dev_auth_allow_remote = flag("LICHTUNG_DEV_AUTH_ALLOW_REMOTE")?;

        // Dev auth makes every request the owner, so it listens on loopback
        // only unless explicitly told otherwise (a container cannot bind its
        // host's loopback; it publishes the port with `-p 127.0.0.1:…`).
        let bind = parse_bind(get("LICHTUNG_BIND").as_deref(), dev_auth)?;
        if dev_auth && !bind.ip().is_loopback() && !dev_auth_allow_remote {
            return err(format!(
                "LICHTUNG_DEV_AUTH=true lauscht nur auf einer lokalen Adresse \
                 (LICHTUNG_BIND ist {bind}); im Container \
                 LICHTUNG_DEV_AUTH_ALLOW_REMOTE=true setzen und den Port nur lokal \
                 veröffentlichen"
            ));
        }
        let data_dir = PathBuf::from(get("LICHTUNG_DATA_DIR").unwrap_or_else(|| "./data".into()));

        let public_url = match get("LICHTUNG_PUBLIC_URL") {
            Some(v) => Url::parse(&v)
                .map_err(|_| ConfigError(format!("LICHTUNG_PUBLIC_URL ist keine URL: {v}")))?,
            None => Url::parse(&format!("http://localhost:{}", bind.port()))
                .expect("static URL is valid"),
        };
        if !matches!(public_url.scheme(), "http" | "https") || public_url.host_str().is_none() {
            return err("LICHTUNG_PUBLIC_URL muss eine http(s)-URL sein");
        }
        if dev_auth && !is_loopback_url(&public_url) {
            return err(
                "LICHTUNG_DEV_AUTH=true ist nur mit einer lokalen LICHTUNG_PUBLIC_URL \
                 (localhost, 127.0.0.1, ::1) erlaubt",
            );
        }

        let oidc = match (get("LICHTUNG_OIDC_ISSUER"), get("LICHTUNG_OIDC_CLIENT_ID")) {
            (None, None) => None,
            (Some(issuer), Some(client_id)) => {
                let issuer = Url::parse(&issuer).map_err(|_| {
                    ConfigError(format!("LICHTUNG_OIDC_ISSUER ist keine URL: {issuer}"))
                })?;
                let allowed_subject = get("LICHTUNG_OIDC_ALLOWED_SUBJECT");
                let allowed_email = get("LICHTUNG_OIDC_ALLOWED_EMAIL").map(|e| e.to_lowercase());
                if allowed_subject.is_none() && allowed_email.is_none() {
                    return err(
                        "LICHTUNG_OIDC_ALLOWED_SUBJECT oder LICHTUNG_OIDC_ALLOWED_EMAIL muss gesetzt sein",
                    );
                }
                Some(OidcConfig {
                    issuer,
                    client_id,
                    client_secret: get("LICHTUNG_OIDC_CLIENT_SECRET"),
                    allowed_subject,
                    allowed_email,
                })
            }
            _ => {
                return err("LICHTUNG_OIDC_ISSUER und LICHTUNG_OIDC_CLIENT_ID gehören zusammen");
            }
        };

        let session_secret = match get("LICHTUNG_SESSION_SECRET") {
            Some(s) if s.len() >= MIN_SESSION_SECRET_LEN => s,
            Some(_) => {
                return err(format!(
                    "LICHTUNG_SESSION_SECRET muss mindestens {MIN_SESSION_SECRET_LEN} Zeichen lang sein"
                ))
            }
            None if oidc.is_some() && !dev_auth => {
                return err("LICHTUNG_SESSION_SECRET ist für den OIDC-Login erforderlich")
            }
            None => DEV_SESSION_SECRET.to_owned(),
        };

        let secret_key = match get("LICHTUNG_SECRET_KEY") {
            Some(raw) => parse_secret_key(&raw)?,
            None if dev_auth => DEV_SECRET_KEY,
            None => {
                return err(
                    "LICHTUNG_SECRET_KEY ist erforderlich: Schlüssel für die Zugangsdaten der \
                     Kalenderkonten, 32 Byte als base64 oder 64 Hex-Zeichen \
                     (erzeugen mit: openssl rand -base64 32)",
                )
            }
        };

        let backup_hour = match get("LICHTUNG_BACKUP_HOUR") {
            None => 3,
            Some(v) => match v.parse::<u8>() {
                Ok(h) if h < 24 => h,
                _ => return err(format!("LICHTUNG_BACKUP_HOUR muss 0 bis 23 sein: {v}")),
            },
        };

        Ok(Self {
            bind,
            data_dir,
            public_url,
            oidc,
            session_secret,
            secret_key,
            bootstrap_token: get("LICHTUNG_BOOTSTRAP_TOKEN"),
            dev_auth,
            backup_hour,
            fastmail_url: default_fastmail_url(),
            icloud_url: default_icloud_url(),
        })
    }

    /// A configuration for tests and local tooling: everything in `data_dir`,
    /// no OIDC, no dev auth, the dev secret key.
    pub fn for_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            data_dir: data_dir.into(),
            public_url: Url::parse("http://localhost:8080").expect("static URL is valid"),
            oidc: None,
            session_secret: DEV_SESSION_SECRET.to_owned(),
            secret_key: DEV_SECRET_KEY,
            bootstrap_token: None,
            dev_auth: false,
            backup_hour: 3,
            fastmail_url: default_fastmail_url(),
            icloud_url: default_icloud_url(),
        }
    }

    /// Whether the fixed development key encrypts the credentials.
    pub fn uses_dev_secret_key(&self) -> bool {
        self.secret_key == DEV_SECRET_KEY
    }

    /// Path of the SQLite database.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("lichtung.db")
    }

    /// Directory of the nightly backups.
    pub fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    /// Session cookies are `Secure` unless the public URL is plain-http
    /// loopback (local development).
    pub fn secure_cookies(&self) -> bool {
        !(self.public_url.scheme() == "http" && is_loopback_url(&self.public_url))
    }

    /// The OIDC redirect URI (`<public_url>/auth/callback`).
    pub fn oidc_redirect_uri(&self) -> Url {
        self.public_url
            .join("auth/callback")
            .expect("joining a static path is valid")
    }

    /// The origin (`scheme://host[:port]`) of the public URL.
    pub fn public_origin(&self) -> String {
        self.public_url.origin().ascii_serialization()
    }
}

fn default_fastmail_url() -> Url {
    Url::parse(FASTMAIL_CALDAV_URL).expect("static URL is valid")
}

fn default_icloud_url() -> Url {
    Url::parse(ICLOUD_CALDAV_URL).expect("static URL is valid")
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A valid test key (base64 of 32 bytes 0x00..0x1f).
    const KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    fn key_bytes() -> [u8; 32] {
        std::array::from_fn(|i| i as u8)
    }

    fn cfg(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| map.get(k).cloned())
    }

    /// Like [`cfg`] with a valid secret key.
    fn cfg_key(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let mut all = vec![("LICHTUNG_SECRET_KEY", KEY_B64)];
        all.extend_from_slice(vars);
        cfg(&all)
    }

    #[test]
    fn defaults() {
        let c = cfg_key(&[]).unwrap();
        assert_eq!(c.bind.to_string(), "0.0.0.0:8080");
        assert_eq!(c.data_dir, PathBuf::from("./data"));
        assert_eq!(c.public_url.as_str(), "http://localhost:8080/");
        assert_eq!(c.backup_hour, 3);
        assert!(!c.dev_auth);
        assert!(c.oidc.is_none());
        assert!(!c.secure_cookies());
        assert_eq!(c.db_path(), PathBuf::from("./data/lichtung.db"));
        assert_eq!(c.secret_key, key_bytes());
        assert!(!c.uses_dev_secret_key());
    }

    #[test]
    fn full_oidc_config() {
        let c = cfg_key(&[
            ("LICHTUNG_BIND", "127.0.0.1:9000"),
            ("LICHTUNG_PUBLIC_URL", "https://lichtung.example.org"),
            ("LICHTUNG_OIDC_ISSUER", "https://id.example.org"),
            ("LICHTUNG_OIDC_CLIENT_ID", "lichtung"),
            ("LICHTUNG_OIDC_CLIENT_SECRET", "geheim"),
            ("LICHTUNG_OIDC_ALLOWED_EMAIL", "Person@Example.org"),
            (
                "LICHTUNG_SESSION_SECRET",
                "0123456789abcdef0123456789abcdef",
            ),
            ("LICHTUNG_BACKUP_HOUR", "23"),
        ])
        .unwrap();
        let oidc = c.oidc.as_ref().unwrap();
        assert_eq!(oidc.allowed_email.as_deref(), Some("person@example.org"));
        assert!(c.secure_cookies());
        assert_eq!(
            c.oidc_redirect_uri().as_str(),
            "https://lichtung.example.org/auth/callback"
        );
        assert_eq!(c.public_origin(), "https://lichtung.example.org");
        assert_eq!(c.backup_hour, 23);
        // Secrets never show up in debug output (logs).
        let debug = format!("{c:?}");
        assert!(!debug.contains("geheim"), "{debug}");
        assert!(!debug.contains("0123456789abcdef"), "{debug}");
    }

    #[test]
    fn rejects_invalid_values() {
        assert!(cfg_key(&[("LICHTUNG_BIND", "nope")]).is_err());
        assert!(cfg_key(&[("LICHTUNG_BACKUP_HOUR", "24")]).is_err());
        assert!(cfg_key(&[("LICHTUNG_DEV_AUTH", "yes")]).is_err());
        assert!(cfg_key(&[("LICHTUNG_PUBLIC_URL", "ftp://lichtung.example.org")]).is_err());
        assert!(cfg_key(&[("LICHTUNG_OIDC_ISSUER", "https://id.example.org")]).is_err());
        // OIDC without allowed person
        assert!(cfg_key(&[
            ("LICHTUNG_OIDC_ISSUER", "https://id.example.org"),
            ("LICHTUNG_OIDC_CLIENT_ID", "lichtung"),
            (
                "LICHTUNG_SESSION_SECRET",
                "0123456789abcdef0123456789abcdef"
            ),
        ])
        .is_err());
        // OIDC without session secret
        assert!(cfg_key(&[
            ("LICHTUNG_OIDC_ISSUER", "https://id.example.org"),
            ("LICHTUNG_OIDC_CLIENT_ID", "lichtung"),
            ("LICHTUNG_OIDC_ALLOWED_SUBJECT", "abc"),
        ])
        .is_err());
        assert!(cfg_key(&[("LICHTUNG_SESSION_SECRET", "kurz")]).is_err());
    }

    #[test]
    fn secret_key_is_required_outside_dev_auth() {
        let e = cfg(&[]).unwrap_err();
        assert!(e.to_string().contains("LICHTUNG_SECRET_KEY"), "{e}");
        // Dev auth falls back to the fixed development key.
        let c = cfg(&[("LICHTUNG_DEV_AUTH", "true")]).unwrap();
        assert_eq!(c.secret_key, DEV_SECRET_KEY);
        assert!(c.uses_dev_secret_key());
        // A given key wins under dev auth too.
        let c = cfg_key(&[("LICHTUNG_DEV_AUTH", "true")]).unwrap();
        assert_eq!(c.secret_key, key_bytes());
    }

    #[test]
    fn secret_key_formats() {
        let expected = key_bytes();
        let hex: String = expected.iter().map(|b| format!("{b:02x}")).collect();
        for raw in [
            KEY_B64,
            KEY_B64.trim_end_matches('='),
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=\n",
            hex.as_str(),
            &hex.to_uppercase(),
        ] {
            assert_eq!(parse_secret_key(raw).unwrap(), expected, "{raw}");
        }
        // URL-safe alphabet
        let urlsafe = parse_secret_key("-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_-_8").unwrap();
        assert_eq!(urlsafe[0], 0xfb);
        for bad in [
            "zu-kurz",
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd", // 30 bytes
            &format!("{hex}00"),                        // 33 bytes as hex
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", // all zero
            "kein base64 !",
        ] {
            assert!(parse_secret_key(bad).is_err(), "{bad}");
        }
        assert!(cfg(&[("LICHTUNG_SECRET_KEY", "zu-kurz")]).is_err());
    }

    #[test]
    fn dev_auth_only_on_loopback() {
        assert!(cfg(&[("LICHTUNG_DEV_AUTH", "true")]).unwrap().dev_auth);
        assert!(
            cfg(&[
                ("LICHTUNG_DEV_AUTH", "true"),
                ("LICHTUNG_PUBLIC_URL", "http://127.0.0.1:5173")
            ])
            .unwrap()
            .dev_auth
        );
        assert!(cfg(&[
            ("LICHTUNG_DEV_AUTH", "true"),
            ("LICHTUNG_PUBLIC_URL", "https://lichtung.example.org")
        ])
        .is_err());
    }

    #[test]
    fn dev_auth_binds_loopback_only() {
        let c = cfg(&[("LICHTUNG_DEV_AUTH", "true")]).unwrap();
        assert_eq!(c.bind.to_string(), "127.0.0.1:8080");
        assert_eq!(c.public_url.as_str(), "http://localhost:8080/");
        assert!(cfg(&[
            ("LICHTUNG_DEV_AUTH", "true"),
            ("LICHTUNG_BIND", "[::1]:9000")
        ])
        .is_ok());
        for bind in ["0.0.0.0:8080", "[::]:8080", "192.0.2.2:8080"] {
            assert!(
                cfg(&[("LICHTUNG_DEV_AUTH", "true"), ("LICHTUNG_BIND", bind)]).is_err(),
                "{bind}"
            );
        }
        // explicit escape hatch for containers
        let c = cfg(&[
            ("LICHTUNG_DEV_AUTH", "true"),
            ("LICHTUNG_BIND", "0.0.0.0:8080"),
            ("LICHTUNG_DEV_AUTH_ALLOW_REMOTE", "true"),
        ])
        .unwrap();
        assert!(c.dev_auth);
        assert!(cfg(&[("LICHTUNG_DEV_AUTH_ALLOW_REMOTE", "ja")]).is_err());
        // without dev auth the default stays on all interfaces
        assert_eq!(cfg_key(&[]).unwrap().bind.to_string(), "0.0.0.0:8080");
    }

    #[test]
    fn healthcheck_reads_only_the_bind_address() {
        let map: HashMap<&str, &str> = [("LICHTUNG_BIND", "0.0.0.0:9000")].into();
        let bind = Config::bind_from_lookup(|k| map.get(k).map(|v| v.to_string())).unwrap();
        assert_eq!(bind.to_string(), "0.0.0.0:9000");
        let dev: HashMap<&str, &str> = [("LICHTUNG_DEV_AUTH", "true")].into();
        let bind = Config::bind_from_lookup(|k| dev.get(k).map(|v| v.to_string())).unwrap();
        assert_eq!(bind.to_string(), "127.0.0.1:8080");
    }
}
