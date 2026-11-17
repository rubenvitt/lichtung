-- Lichtung schema, version 12: stille Push-Nachrichten (APNs, LIC-27).
--
-- Die App eines Geräts meldet ihren APNs-Device-Token (Hex) und die
-- Umgebung, für die er ausgestellt ist (`sandbox` bei Builds aus Xcode,
-- `production` bei TestFlight). Ändern sich Termine, weckt der Server die
-- App mit einer stillen Push-Nachricht: Sie zieht nach und plant ihre
-- Erinnerungen neu, auch wenn sie nicht geöffnet war. Ein Token gehört
-- höchstens einem Gerät (eine neue Anmeldung auf demselben iPhone übernimmt
-- ihn).
ALTER TABLE devices ADD COLUMN apns_token TEXT;
ALTER TABLE devices ADD COLUMN apns_environment TEXT;
ALTER TABLE devices ADD COLUMN apns_updated_at INTEGER;
CREATE UNIQUE INDEX devices_apns_token ON devices (apns_token) WHERE apns_token IS NOT NULL;
