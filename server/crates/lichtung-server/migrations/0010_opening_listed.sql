-- Lichtung schema, version 10: feste Links auf der öffentlichen Startseite.
--
-- `listed` = 1: Die Vorlage steht auf der Startseite `<public_url>/` mit
-- Titel, Dauer und Link zu ihrer Buchungsseite. Nur Vorlagen (`template`)
-- lassen sich aufnehmen; Einmal-Links bleiben immer unsichtbar. Ohne
-- aufgenommene Vorlagen zeigt `/` weiter nur, was hier läuft.
ALTER TABLE openings ADD COLUMN listed INTEGER NOT NULL DEFAULT 0;
