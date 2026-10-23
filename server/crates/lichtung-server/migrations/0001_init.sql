-- Lichtung schema, version 1: owner, browser sessions, API tokens, devices
-- and the built-in OAuth 2.1 authorization server (/mcp, device login).
-- Timestamps are Unix milliseconds (INTEGER), IDs canonical UUID strings
-- (v7). Secrets (tokens, codes, session IDs) are stored as hashes only.

CREATE TABLE users (
    id           TEXT PRIMARY KEY,
    oidc_subject TEXT UNIQUE,
    email        TEXT,
    name         TEXT,
    created_at   INTEGER NOT NULL
);

-- Browser sessions: `id_hash` is HMAC-SHA256 of the cookie value, keyed with
-- LICHTUNG_SESSION_SECRET.
CREATE TABLE sessions (
    id_hash      TEXT PRIMARY KEY,
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
);
CREATE INDEX sessions_expires ON sessions (expires_at);

-- API tokens (`lt_…`, SHA-256 hex), for /api and /mcp.
CREATE TABLE api_tokens (
    id           TEXT PRIMARY KEY,
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    token_hash   TEXT    NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at   INTEGER
);
CREATE INDEX api_tokens_user ON api_tokens (user_id);

-- Dynamically registered public clients (RFC 7591).
CREATE TABLE oauth_clients (
    id                 TEXT PRIMARY KEY,
    name               TEXT    NOT NULL,
    redirect_uris_json TEXT    NOT NULL,
    created_at         INTEGER NOT NULL,
    last_used_at       INTEGER,
    revoked_at         INTEGER
);
CREATE INDEX oauth_clients_created ON oauth_clients (created_at);

-- Authorization codes (`loc_…`): short-lived, single use, bound to client,
-- redirect URI, PKCE challenge and resource. `family_id` names the token
-- family issued for the code (revoked when the code is replayed).
CREATE TABLE oauth_codes (
    code_hash      TEXT PRIMARY KEY,
    client_id      TEXT    NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    user_id        TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redirect_uri   TEXT    NOT NULL,
    code_challenge TEXT    NOT NULL,
    resource       TEXT    NOT NULL,
    scope          TEXT,
    family_id      TEXT    NOT NULL,
    created_at     INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    used_at        INTEGER
);
CREATE INDEX oauth_codes_expires ON oauth_codes (expires_at);

-- Access (`loa_…`) and refresh (`lor_…`) tokens for /mcp. One
-- authorization = one family; refresh tokens rotate within the family, and
-- replaying a rotated refresh token revokes the whole family.
CREATE TABLE oauth_tokens (
    id         TEXT PRIMARY KEY,
    token_hash TEXT    NOT NULL UNIQUE,
    kind       TEXT    NOT NULL CHECK (kind IN ('access', 'refresh')),
    client_id  TEXT    NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    user_id    TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    family_id  TEXT    NOT NULL,
    resource   TEXT    NOT NULL,
    scope      TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at    INTEGER,
    revoked_at INTEGER
);
CREATE INDEX oauth_tokens_family ON oauth_tokens (family_id);
CREATE INDEX oauth_tokens_client ON oauth_tokens (client_id);
CREATE INDEX oauth_tokens_expires ON oauth_tokens (expires_at);

-- Devices: the iOS/macOS app logs in through the built-in authorization
-- server (resource <public>/api) and receives a device token (`ld_…`,
-- SHA-256 hex) that does not expire and is revoked per device. No foreign
-- key to oauth_clients: registered clients are purged after a day, the
-- device stays. `family_id` is the authorization code's family, so
-- replaying the code revokes the device.
CREATE TABLE devices (
    id           TEXT PRIMARY KEY,
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    token_hash   TEXT    NOT NULL UNIQUE,
    family_id    TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at   INTEGER
);
CREATE INDEX devices_user ON devices (user_id);
CREATE INDEX devices_family ON devices (family_id);
