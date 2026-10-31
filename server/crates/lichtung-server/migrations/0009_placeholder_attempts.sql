-- Lichtung schema, version 9: Platzhalter, deren Anlegen läuft oder
-- unklar ausging (LIC-59).
--
-- Vor dem PUT eines Platzhalters legt der Server dessen UID hier ab, und
-- erst wenn die Event-ID am Vorschlag steht, verschwindet die Zeile wieder
-- (in derselben Transaktion). Scheitert das Anlegen, nachdem der Anbieter
-- den Termin schon geschrieben hat (Zeitüberschreitung nach dem PUT,
-- lokaler Speicherfehler), bleibt die Zeile stehen. Ist der Vorschlag
-- dann weg (der Link wurde verworfen), sucht die Aufräumrunde den Termin
-- über Kalender und UID und legt ihn in `placeholder_removals`, aber nur,
-- wenn Titel und Beginn noch so sind wie angelegt (`title`, `start_utc`):
-- Wurde er beim Anbieter inzwischen geändert, gewinnt der Anbieter und er
-- bleibt stehen. Kennt der Server ihn (noch) nicht, wartet die Zeile auf
-- den nächsten Abgleich, längstens 7 Tage.
-- Kein Fremdschlüssel: Vorschlag und Link sind dann meist schon gelöscht.
CREATE TABLE placeholder_attempts (
    uid           TEXT PRIMARY KEY,
    calendar_id   TEXT    NOT NULL,
    opening_id    TEXT    NOT NULL,
    suggestion_id TEXT    NOT NULL,
    title         TEXT    NOT NULL,
    start_utc     INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);
