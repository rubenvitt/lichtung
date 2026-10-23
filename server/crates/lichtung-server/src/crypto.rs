//! Encryption of provider credentials at rest (spec section 4: "Zugangsdaten
//! liegen verschlüsselt in der Datenbank").
//!
//! - AES-256-GCM with the key from `LICHTUNG_SECRET_KEY` (`ring`).
//! - A fresh random 96-bit nonce per encryption.
//! - Format: `v1 (0x01) || nonce (12 bytes) || ciphertext || tag (16 bytes)`.
//! - Associated data: the account ID, so a blob copied to another account
//!   does not decrypt.
//! - A key check value in `meta` detects a wrong key at startup, before any
//!   credentials are needed ([`verify_or_init_key_check`]).

use std::fmt;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::config::SECRET_KEY_LEN;

/// Format version byte of a sealed blob.
pub const FORMAT_V1: u8 = 1;
/// Length of the GCM tag.
const TAG_LEN: usize = 16;
/// `meta` key of the key check value.
pub const KEY_CHECK_META_KEY: &str = "secret_key_check";
/// Associated data of the key check value (cannot collide with an account
/// ID, which starts with `acc_`).
const KEY_CHECK_AAD: &[u8] = b"lichtung:key-check";
/// Plaintext of the key check value.
const KEY_CHECK_PLAINTEXT: &[u8] = b"lichtung-key-check-v1";

/// Decryption failed: wrong key, wrong associated data, tampered or
/// truncated blob, or an unknown format version. Deliberately carries no
/// detail.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Zugangsdaten lassen sich nicht entschlüsseln")]
pub struct DecryptError;

/// AES-256-GCM cipher for credentials. `Debug` never shows the key.
pub struct Cipher {
    key: LessSafeKey,
}

impl fmt::Debug for Cipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cipher(AES-256-GCM, ***)")
    }
}

impl Cipher {
    pub fn new(key: &[u8; SECRET_KEY_LEN]) -> Self {
        let unbound = UnboundKey::new(&AES_256_GCM, key).expect("32-byte key fits AES-256-GCM");
        Cipher {
            key: LessSafeKey::new(unbound),
        }
    }

    /// Encrypts `plaintext` bound to `aad`.
    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).expect("operating system RNG is available");
        let mut in_out = plaintext.to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad),
                &mut in_out,
            )
            .expect("AES-GCM sealing of a small buffer cannot fail");
        let mut out = Vec::with_capacity(1 + NONCE_LEN + in_out.len());
        out.push(FORMAT_V1);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&in_out);
        out
    }

    /// Decrypts a blob produced by [`Cipher::seal`] with the same `aad`.
    pub fn open(&self, aad: &[u8], blob: &[u8]) -> Result<Vec<u8>, DecryptError> {
        if blob.len() < 1 + NONCE_LEN + TAG_LEN || blob[0] != FORMAT_V1 {
            return Err(DecryptError);
        }
        let nonce: [u8; NONCE_LEN] = blob[1..1 + NONCE_LEN].try_into().expect("length checked");
        let mut in_out = blob[1 + NONCE_LEN..].to_vec();
        let plain = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad),
                &mut in_out,
            )
            .map_err(|_| DecryptError)?;
        Ok(plain.to_vec())
    }
}

/// A secret string (password) whose `Debug` output is redacted, so it never
/// ends up in logs. Deserializes from a plain JSON string; deliberately not
/// `Serialize` outside of [`StoredCredentials`].
#[derive(Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    /// The secret value. Only pass it to the provider client.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// What is encrypted per account: a JSON object, so later providers (e.g.
/// OAuth refresh tokens for Microsoft) can add fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCredentials {
    pub password: Secret,
    /// ICS subscriptions (LIC-14): the full feed URL, which usually carries
    /// a secret token in its path or query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<Secret>,
    /// ICS subscriptions: Basic-auth user name, if the feed needs one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<Secret>,
}

impl StoredCredentials {
    /// Encrypts the credentials for the account `account_id`.
    pub fn seal(&self, cipher: &Cipher, account_id: &str) -> Vec<u8> {
        let json = serde_json::to_vec(self).expect("credentials serialize");
        cipher.seal(account_id.as_bytes(), &json)
    }

    /// Decrypts the credentials of the account `account_id`.
    pub fn open(cipher: &Cipher, account_id: &str, blob: &[u8]) -> Result<Self, DecryptError> {
        let json = cipher.open(account_id.as_bytes(), blob)?;
        serde_json::from_slice(&json).map_err(|_| DecryptError)
    }
}

/// The German error when `LICHTUNG_SECRET_KEY` does not match the database.
pub const WRONG_KEY_MESSAGE: &str = "LICHTUNG_SECRET_KEY passt nicht zu dieser Datenbank: \
    Der Prüfwert lässt sich mit diesem Schlüssel nicht entschlüsseln, gespeicherte \
    Zugangsdaten der Kalenderkonten wären unlesbar. Den Schlüssel setzen, mit dem die \
    Datenbank angelegt wurde (Passwortmanager). Ist er verloren: \
    `lichtung-server reset-secret-key` mit dem neuen Schlüssel ausführen und danach die \
    Konten in der App neu verbinden (siehe docs/betrieb.md).";

/// Checks the key against the check value in `meta`, or stores one if the
/// database has none yet (fresh database or first start after the
/// migration). Fails with [`WRONG_KEY_MESSAGE`] when the key does not match.
pub fn verify_or_init_key_check(conn: &Connection, cipher: &Cipher) -> anyhow::Result<()> {
    let stored: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [KEY_CHECK_META_KEY],
            |r| r.get(0),
        )
        .optional()?;
    match stored {
        Some(blob) => match cipher.open(KEY_CHECK_AAD, &blob) {
            Ok(plain) if plain == KEY_CHECK_PLAINTEXT => Ok(()),
            _ => anyhow::bail!(WRONG_KEY_MESSAGE),
        },
        None => {
            write_key_check(conn, cipher)?;
            Ok(())
        }
    }
}

/// Stores (or replaces) the key check value for `cipher`.
pub fn write_key_check(conn: &Connection, cipher: &Cipher) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![
            KEY_CHECK_META_KEY,
            cipher.seal(KEY_CHECK_AAD, KEY_CHECK_PLAINTEXT)
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> [u8; 32] {
        std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(n))
    }

    #[test]
    fn round_trip_and_format() {
        let c = Cipher::new(&key(1));
        let blob = c.seal(b"acc_1", b"app-passwort");
        assert_eq!(blob[0], FORMAT_V1);
        assert_eq!(blob.len(), 1 + 12 + b"app-passwort".len() + 16);
        assert_eq!(c.open(b"acc_1", &blob).unwrap(), b"app-passwort");
        // A fresh nonce every time.
        let again = c.seal(b"acc_1", b"app-passwort");
        assert_ne!(blob, again);
        assert_ne!(blob[1..13], again[1..13]);
        // The plaintext does not appear in the blob.
        assert!(!blob.windows(12).any(|w| w == b"app-passwort"));
    }

    #[test]
    fn wrong_key_aad_or_tampering_fails() {
        let c = Cipher::new(&key(1));
        let blob = c.seal(b"acc_1", b"geheim");
        assert_eq!(
            Cipher::new(&key(2)).open(b"acc_1", &blob),
            Err(DecryptError)
        );
        // Swapped between accounts.
        assert_eq!(c.open(b"acc_2", &blob), Err(DecryptError));
        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(c.open(b"acc_1", &tampered), Err(DecryptError));
        let mut version = blob.clone();
        version[0] = 2;
        assert_eq!(c.open(b"acc_1", &version), Err(DecryptError));
        assert_eq!(c.open(b"acc_1", &blob[..20]), Err(DecryptError));
        assert_eq!(c.open(b"acc_1", &[]), Err(DecryptError));
    }

    #[test]
    fn stored_credentials_are_bound_to_the_account() {
        let c = Cipher::new(&key(3));
        let creds = StoredCredentials {
            password: Secret::new("app-passwort-1234"),
            ..Default::default()
        };
        let blob = creds.seal(&c, "acc_a");
        assert_eq!(StoredCredentials::open(&c, "acc_a", &blob).unwrap(), creds);
        assert!(StoredCredentials::open(&c, "acc_b", &blob).is_err());
    }

    #[test]
    fn secrets_are_redacted_in_debug_output() {
        let creds = StoredCredentials {
            password: Secret::new("app-passwort-1234"),
            ..Default::default()
        };
        let debug = format!("{creds:?} {:?}", Cipher::new(&key(1)));
        assert!(!debug.contains("app-passwort"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }

    #[test]
    fn feed_credentials_are_optional_fields() {
        // Blobs written before LIC-14 hold only the password.
        let old: StoredCredentials = serde_json::from_str(r#"{"password":"x"}"#).unwrap();
        assert_eq!((old.url, old.username), (None, None));
        let creds = StoredCredentials {
            url: Some(Secret::new("https://feeds.example.org/k.ics?token=t")),
            ..Default::default()
        };
        let c = Cipher::new(&key(4));
        let blob = creds.seal(&c, "acc_f");
        assert_eq!(StoredCredentials::open(&c, "acc_f", &blob).unwrap(), creds);
        assert!(!format!("{creds:?}").contains("token"));
    }

    #[test]
    fn key_check_detects_a_wrong_key() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB NOT NULL)")
            .unwrap();
        let right = Cipher::new(&key(1));
        verify_or_init_key_check(&conn, &right).unwrap();
        verify_or_init_key_check(&conn, &right).unwrap();
        let err = verify_or_init_key_check(&conn, &Cipher::new(&key(2))).unwrap_err();
        assert!(err.to_string().contains("LICHTUNG_SECRET_KEY passt nicht"));
        // Resetting writes a check value for the new key.
        write_key_check(&conn, &Cipher::new(&key(2))).unwrap();
        verify_or_init_key_check(&conn, &Cipher::new(&key(2))).unwrap();
        assert!(verify_or_init_key_check(&conn, &right).is_err());
    }
}
