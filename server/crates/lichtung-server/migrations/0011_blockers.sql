-- Lichtung schema, version 11: Blocker (LIC-75).
--
-- Eine Blocker-Regel spiegelt die Termine eines Quellkalenders als
-- „Blockiert“-Termine in einen Zielkalender. Kein Fremdschlüssel auf die
-- Kalender: Verschwindet einer, bleibt die Regel sichtbar und tut nichts.
-- `all_day` = 1: auch ganztägige Termine blocken. `updated_at` ändert sich
-- mit Titel oder `all_day`; danach schreibt der Abgleich alle Blocker der
-- Regel neu.
CREATE TABLE blocker_rules (
    id                 TEXT PRIMARY KEY,
    source_calendar_id TEXT    NOT NULL,
    target_calendar_id TEXT    NOT NULL,
    title              TEXT    NOT NULL,
    all_day            INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

-- Ein Quelltermin einer Regel und sein Blocker. `uid` steht hier, bevor
-- der Blocker beim Anbieter angelegt wird; `event_id` erst, wenn er
-- gespeichert ist. So findet der Abgleich einen Blocker wieder, dessen
-- Anlegen unklar ausging. `uid` NULL: Der Quelltermin blockt (in dieser
-- Fassung) nichts. `source_version`, `rule_updated_at` und
-- `blocker_version` halten fest, welchen Stand der Blocker spiegelt; weicht
-- einer davon ab, schreibt der Abgleich ihn neu. Kein Fremdschlüssel: Ist
-- die Regel oder der Quelltermin weg, entfernt der Abgleich erst den
-- Blocker, dann die Zeile.
CREATE TABLE blockers (
    rule_id         TEXT    NOT NULL,
    source_event_id TEXT    NOT NULL,
    uid             TEXT,
    event_id        TEXT,
    source_version  INTEGER NOT NULL,
    rule_updated_at INTEGER NOT NULL,
    blocker_version INTEGER,
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (rule_id, source_event_id)
);
CREATE INDEX blockers_event ON blockers (event_id);
