-- Lichtung schema, version 2: calendar accounts with encrypted credentials,
-- their calendars, calendar sets, and the change log for the sync API.
-- Timestamps are Unix milliseconds. Object IDs carry a prefix (`acc_`,
-- `cal_`, `set_`) followed by a UUID v7 as 32 hex characters.

-- Small key/value store for server state, e.g. the encrypted key check
-- value that detects a wrong LICHTUNG_SECRET_KEY at startup.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value BLOB NOT NULL
);

-- Change log (sync API, LIC-11). One global, strictly increasing counter.
-- Every change to a synced object stores a fresh value in the object's
-- `seq` column; deletions leave a tombstone with their own value. The pull
-- cursor is the highest value a client has seen.
CREATE TABLE change_counter (
    id  INTEGER PRIMARY KEY CHECK (id = 1),
    seq INTEGER NOT NULL
);
INSERT INTO change_counter (id, seq) VALUES (1, 0);

CREATE TABLE tombstones (
    seq        INTEGER PRIMARY KEY,
    type       TEXT    NOT NULL,
    id         TEXT    NOT NULL,
    deleted_at INTEGER NOT NULL
);
CREATE INDEX tombstones_deleted ON tombstones (deleted_at);

-- Calendar accounts. `credentials` is AES-256-GCM (`v1 || nonce ||
-- ciphertext+tag`, AAD = account ID) of a JSON object with the password; it
-- is NULL after `lichtung-server reset-secret-key` until the account is
-- connected again.
CREATE TABLE accounts (
    id             TEXT PRIMARY KEY,
    kind           TEXT    NOT NULL CHECK (kind IN ('fastmail', 'icloud', 'caldav', 'ics', 'microsoft')),
    name           TEXT    NOT NULL,
    username       TEXT    NOT NULL,
    base_url       TEXT    NOT NULL,
    principal_url  TEXT,
    home_url       TEXT,
    -- Server announced `calendar-auto-schedule` (RFC 6638) at discovery.
    auto_schedule  INTEGER NOT NULL DEFAULT 0,
    credentials    BLOB,
    status_state   TEXT    NOT NULL DEFAULT 'ok'
                   CHECK (status_state IN ('ok', 'syncing', 'error', 'auth_failed')),
    status_message TEXT,
    last_sync_at   INTEGER,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    seq            INTEGER NOT NULL
);
CREATE UNIQUE INDEX accounts_identity ON accounts (kind, base_url, username);
CREATE INDEX accounts_seq ON accounts (seq);

-- Calendars of an account. Only collections that can hold events are
-- stored (VTODO-only collections are skipped). Provider fields (name,
-- description, provider_color, writable, can_invite, shared) follow the
-- provider on every refresh; settings (color, hidden, mcp_write,
-- openings_busy, all_day_blocks, position) belong to the user.
-- `sync_token`/`ctag` are written by the event sync only.
CREATE TABLE calendars (
    id             TEXT PRIMARY KEY,
    account_id     TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_href    TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    description    TEXT,
    provider_color TEXT,
    color          TEXT,
    writable       INTEGER NOT NULL,
    can_invite     INTEGER NOT NULL,
    shared         INTEGER NOT NULL,
    sync_token     TEXT,
    ctag           TEXT,
    hidden         INTEGER NOT NULL DEFAULT 0,
    mcp_write      INTEGER NOT NULL,
    openings_busy  INTEGER NOT NULL DEFAULT 1,
    all_day_blocks INTEGER NOT NULL DEFAULT 0,
    position       INTEGER NOT NULL,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    seq            INTEGER NOT NULL,
    UNIQUE (account_id, remote_href)
);
CREATE INDEX calendars_seq ON calendars (seq);

-- Calendar sets ("Arbeit", "Privat") and their ordered members.
CREATE TABLE calendar_sets (
    id         TEXT PRIMARY KEY,
    name       TEXT    NOT NULL,
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    seq        INTEGER NOT NULL
);
CREATE INDEX calendar_sets_seq ON calendar_sets (seq);

CREATE TABLE calendar_set_members (
    set_id      TEXT    NOT NULL REFERENCES calendar_sets(id) ON DELETE CASCADE,
    calendar_id TEXT    NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    PRIMARY KEY (set_id, calendar_id)
);
CREATE INDEX calendar_set_members_calendar ON calendar_set_members (calendar_id);
