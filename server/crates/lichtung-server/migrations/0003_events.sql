-- Lichtung schema, version 3: events (one row per calendar object
-- resource, i.e. one UID in one calendar) with the iCalendar original, and
-- the sync state the CalDAV engine needs (LIC-8).

-- `calendar-user-address-set` of the account (JSON array of lower-case
-- e-mail addresses without `mailto:`), from discovery. Decides `is_self`
-- and `self_status` of attendees.
ALTER TABLE accounts ADD COLUMN user_addresses TEXT NOT NULL DEFAULT '[]';

-- 1 once the server refused `sync-collection` for this calendar; the sync
-- then always compares ETags (and skips unchanged ctags) instead.
ALTER TABLE calendars ADD COLUMN sync_unsupported INTEGER NOT NULL DEFAULT 0;

-- Events. `ics` is the calendar object exactly as the provider sent it,
-- only with line endings normalized to CRLF (REPORT answers travel inside
-- XML, whose parser turns CRLF into LF). It is never rewritten by the sync
-- and is the source of truth; every other column is derived from it and
-- only serves queries. If the data cannot be read, the row is stored all
-- the same with `parse_error` set and empty derived columns.
--
-- `start_utc` / `range_end_utc` (Unix ms) bound all occurrences of the
-- object (series with overrides included); `range_end_utc` is NULL for
-- endless series. Floating times and dates are computed in the server's
-- default zone, so queries add a day of slack on both sides.
-- `version` counts changes of this event (starting at 1), `seq` is the
-- change log value (see `changes.rs`).
CREATE TABLE events (
    id            TEXT PRIMARY KEY,
    calendar_id   TEXT    NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    href          TEXT    NOT NULL,
    etag          TEXT,
    uid           TEXT,
    ics           TEXT    NOT NULL,
    parse_error   TEXT,
    title         TEXT,
    start_utc     INTEGER,
    range_end_utc INTEGER,
    is_recurring  INTEGER NOT NULL DEFAULT 0,
    all_day       INTEGER NOT NULL DEFAULT 0,
    status        TEXT,
    transparency  TEXT,
    version       INTEGER NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    seq           INTEGER NOT NULL,
    UNIQUE (calendar_id, href)
);
CREATE INDEX events_seq ON events (seq);
CREATE INDEX events_calendar_uid ON events (calendar_id, uid);
CREATE INDEX events_range ON events (start_utc, range_end_utc);
