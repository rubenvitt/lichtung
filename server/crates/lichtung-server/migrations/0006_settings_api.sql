-- Lichtung schema, version 6: Arbeitskalender und Clients der Geräte
-- (LIC-46, LIC-49).

-- Der Arbeitskalender (Spec §7): in der App gekennzeichnet, Standard für
-- Kalender von Microsoft-Konten. Wer ihn kennzeichnet, sperrt damit auch das
-- Schreiben über MCP (`mcp_write`), außer im selben Aufruf anders gesetzt.
ALTER TABLE calendars ADD COLUMN is_work INTEGER NOT NULL DEFAULT 0;

-- Der OAuth-Client, über den sich ein Gerät angemeldet hat. Die App merkt
-- sich ihren Client je Server und meldet sich damit erneut an; solange ein
-- Gerät des Clients aktiv ist, räumt der Server ihn nicht weg, und ein
-- umbenanntes Gerät benennt auch seinen Client um. Kein Fremdschlüssel: Ein
-- weggeräumter Client lässt das Gerät bestehen. Geräte von vor dieser
-- Migration haben keinen Client.
ALTER TABLE devices ADD COLUMN client_id TEXT;
CREATE INDEX devices_client ON devices (client_id);
