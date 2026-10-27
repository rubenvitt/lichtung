-- Lichtung schema, version 8: Buchungen der Openings (LIC-33, LIC-35).
--
-- Was der Gast auf der Buchungsseite eingibt (Name, Adresse, Notiz), steht
-- nur hier und in der Verwaltung (`bookings` am Opening). Der gebuchte
-- Termin selbst trägt davon nur den Gast als ATTENDEE: Der Anbieter
-- verschickt ihn als Einladung von der eigenen Adresse an die eingegebene
-- Adresse (RFC 6638), Titel und Beschreibung enthalten also nichts, was
-- der Gast bestimmt.
--
-- Einmal-Links haben eine Buchung, Vorlagen beliebig viele. Die Zeilen
-- gehen mit dem Link; die Aufräumrunde löscht Buchungen, die mehr als
-- 365 Tage zurückliegen (gebucht und Termin vorbei).
CREATE TABLE opening_bookings (
    id          TEXT PRIMARY KEY,
    opening_id  TEXT    NOT NULL REFERENCES openings(id) ON DELETE CASCADE,
    event_id    TEXT    NOT NULL,
    start_utc   INTEGER NOT NULL,
    end_utc     INTEGER NOT NULL,
    guest_name  TEXT    NOT NULL,
    guest_email TEXT    NOT NULL,
    note        TEXT,
    created_at  INTEGER NOT NULL
);
CREATE INDEX opening_bookings_opening ON opening_bookings (opening_id, created_at);
CREATE INDEX opening_bookings_created ON opening_bookings (created_at);
