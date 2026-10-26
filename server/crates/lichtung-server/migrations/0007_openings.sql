-- Lichtung schema, version 7: Openings (Buchungslinks, LIC-31) und ihre
-- Platzhalter (LIC-32). Spec §6, API in docs/api/openings.md.

-- Ein Opening ist ein Einmal-Link (`once`, nach einer Buchung verbraucht)
-- oder eine Vorlage (`template`, beliebig oft buchbar). Links laufen nicht
-- ab. `token` ist die nicht erratbare Kennung der öffentlichen Adresse
-- `<public_url>/o/<token>` (32 Zufallsbytes base64url); sie steht im
-- Klartext hier, weil die App den Link jederzeit wieder anzeigt.
--
-- Regeln für freie Zeit: `windows` ist eine JSON-Liste von Wochenfenstern
-- (`[{"weekday": "mo", "start": "09:00", "end": "17:00"}]`) in
-- `time_zone` (IANA). `suggested` = 1: Der Einmal-Link bietet nur seine
-- Vorschläge an (auch wenn alle verfallen sind), sonst freie Zeiten nach
-- den Regeln. `calendar_id` ohne Fremdschlüssel: Verschwindet der
-- Zielkalender, bleibt der Link sichtbar und bietet nichts mehr an.
CREATE TABLE openings (
    id                    TEXT PRIMARY KEY,
    kind                  TEXT    NOT NULL CHECK (kind IN ('once', 'template')),
    token                 TEXT    NOT NULL UNIQUE,
    title                 TEXT    NOT NULL,
    duration_minutes      INTEGER NOT NULL,
    calendar_id           TEXT    NOT NULL,
    time_zone             TEXT    NOT NULL,
    windows               TEXT    NOT NULL,
    buffer_before_minutes INTEGER NOT NULL,
    buffer_after_minutes  INTEGER NOT NULL,
    min_notice_minutes    INTEGER NOT NULL,
    horizon_days          INTEGER NOT NULL,
    video_url             TEXT,
    suggested             INTEGER NOT NULL DEFAULT 0,
    booked_at             INTEGER,
    booked_event_id       TEXT,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    seq                   INTEGER NOT NULL
);
CREATE INDEX openings_seq ON openings (seq);

-- Vorschläge eines Einmal-Links, je mit ihrem Platzhalter-Termin im
-- Zielkalender (`event_id`, NULL solange er angelegt wird). Verfallene
-- Vorschläge (Beginn vorbei) und die eines gebuchten Links werden
-- gelöscht, ihr Platzhalter wandert in `placeholder_removals`.
CREATE TABLE opening_suggestions (
    id         TEXT PRIMARY KEY,
    opening_id TEXT    NOT NULL REFERENCES openings(id) ON DELETE CASCADE,
    start_utc  INTEGER NOT NULL,
    end_utc    INTEGER NOT NULL,
    event_id   TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX opening_suggestions_opening ON opening_suggestions (opening_id, start_utc);
CREATE INDEX opening_suggestions_range ON opening_suggestions (start_utc, end_utc);

-- Platzhalter, die beim Anbieter noch zu löschen sind. Gelingt das Löschen
-- nicht (Anbieter nicht erreichbar), versucht es die Aufräumrunde erneut.
-- Kein Fremdschlüssel: Der Link ist dann meist schon gelöscht.
CREATE TABLE placeholder_removals (
    event_id   TEXT PRIMARY KEY,
    opening_id TEXT    NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL
);
