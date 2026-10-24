-- Lichtung schema, version 4: conflicts of the write path (LIC-10).
--
-- A conflict is an own change (from an app, the web or MCP) that the
-- server did not apply because the provider had a different version by
-- then: the provider wins, the dropped change stays visible here until
-- someone dismisses it (tombstone of type `conflict`).
--
-- `event_id` is NULL once the event no longer exists (the deletion takes a
-- fresh `seq` for the conflict, so clients see it). `mutation` is the
-- dropped change as JSON, exactly as it was sent. No foreign keys: a
-- conflict outlives its event and its calendar.
CREATE TABLE conflicts (
    id          TEXT PRIMARY KEY,
    event_id    TEXT,
    calendar_id TEXT,
    origin      TEXT    NOT NULL,
    message     TEXT    NOT NULL,
    mutation    TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    seq         INTEGER NOT NULL
);
CREATE INDEX conflicts_seq ON conflicts (seq);
CREATE INDEX conflicts_event ON conflicts (event_id);
