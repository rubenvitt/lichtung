-- Lichtung schema, version 5: sync API for the apps (LIC-11).

-- Results of pushed mutations, by the client's mutation ID: the same ID
-- again gets the stored result without a second write (idempotency).
-- Only final results (applied, conflict, rejected) are kept; `retry` is
-- tried again. Kept 30 days.
CREATE TABLE mutation_results (
    id         TEXT PRIMARY KEY,
    origin     TEXT    NOT NULL,
    result     TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX mutation_results_created ON mutation_results (created_at);

-- Highest `seq` of tombstones removed by the cleanup (older than 90 days).
-- A pull with a cursor below it may have missed deletions: `410
-- cursor_expired`.
ALTER TABLE change_counter ADD COLUMN purged_through INTEGER NOT NULL DEFAULT 0;
