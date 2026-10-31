#!/usr/bin/env bash
# Baut die Mac-App für ein GitHub Release: archivieren, mit Developer ID signieren,
# notarisieren, heften, zippen und für Sparkle signieren. Ergebnis in build/release/:
#   Lichtung-<VERSION>.zip   Anhang des Releases (Download und Update)
#   appcast.xml              Sparkle-Feed; die App liest ihn über
#                            releases/latest/download/appcast.xml (project.yml, SUFeedURL)
# Die CI ruft es nach semantic-release auf (.github/workflows/release.yml).
#
# Umgebung:
#   VERSION                     Pflicht, z. B. 1.4.0 (ohne „v“)
#   ASC_KEY_ID, ASC_ISSUER_ID   Pflicht, App Store Connect API-Key für notarytool
#   ASC_KEY_PATH                Standard ~/.appstoreconnect/private_keys/AuthKey_${ASC_KEY_ID}.p8
#   SPARKLE_KEY_FILE            Pflicht, privater EdDSA-Schlüssel (Secret SPARKLE_PRIVATE_KEY)
#   SIGN_IDENTITY               Standard „Developer ID Application“ (im Schlüsselbund)
#   BUILD_NUMBER                Standard UTC-Zeitstempel JJJJMMTT.HHMM wie bei TestFlight;
#                               Sparkle vergleicht diese Nummer (CFBundleVersion)
#   NOTARIZE=0                  ohne Notarisierung (nur zum Testen)
set -euo pipefail

cd "$(dirname "$0")/.."

: "${VERSION:?VERSION fehlt}"
: "${SPARKLE_KEY_FILE:?SPARKLE_KEY_FILE fehlt}"
NOTARIZE="${NOTARIZE:-1}"
if [[ "$NOTARIZE" == 1 ]]; then
  : "${ASC_KEY_ID:?ASC_KEY_ID fehlt}"
  : "${ASC_ISSUER_ID:?ASC_ISSUER_ID fehlt}"
  ASC_KEY_PATH="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_${ASC_KEY_ID}.p8}"
  [[ -f "$ASC_KEY_PATH" ]] || { echo "API-Key nicht gefunden: $ASC_KEY_PATH" >&2; exit 1; }
fi
SIGN_IDENTITY="${SIGN_IDENTITY:-Developer ID Application}"
BUILD_NUMBER="${BUILD_NUMBER:-$(date -u +%Y%m%d.%H%M)}"
# Gleiche Hauptversion wie das Paket in project.yml.
SPARKLE_VERSION="2.10.1"
REPO_URL="https://github.com/rubenvitt/lichtung"
TEAM_ID="H95J852PKP"

OUT="build/release"
ARCHIVE="$OUT/Lichtung-macOS.xcarchive"
ZIP="$OUT/Lichtung-$VERSION.zip"
rm -rf "$OUT"
mkdir -p "$OUT"

echo "Version $VERSION, Build $BUILD_NUMBER"

# Sparkle-Werkzeuge (sign_update)
SPARKLE_DIR="$OUT/sparkle"
mkdir -p "$SPARKLE_DIR"
curl -fsSL "https://github.com/sparkle-project/Sparkle/releases/download/$SPARKLE_VERSION/Sparkle-$SPARKLE_VERSION.tar.xz" \
  | tar -xJ -C "$SPARKLE_DIR"

xcodegen generate

xcodebuild -project Lichtung.xcodeproj -scheme Lichtung-macOS -configuration Release \
  -destination 'generic/platform=macOS' -archivePath "$ARCHIVE" \
  MARKETING_VERSION="$VERSION" CURRENT_PROJECT_VERSION="$BUILD_NUMBER" \
  CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY="$SIGN_IDENTITY" DEVELOPMENT_TEAM="$TEAM_ID" \
  OTHER_CODE_SIGN_FLAGS="--timestamp" \
  archive

cat > "$OUT/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>developer-id</string>
  <key>signingStyle</key><string>manual</string>
  <key>signingCertificate</key><string>$SIGN_IDENTITY</string>
  <key>teamID</key><string>$TEAM_ID</string>
</dict>
</plist>
PLIST

xcodebuild -exportArchive -archivePath "$ARCHIVE" \
  -exportOptionsPlist "$OUT/ExportOptions.plist" -exportPath "$OUT/export"

APP="$OUT/export/Lichtung.app"
codesign --verify --deep --strict --verbose=2 "$APP"

if [[ "$NOTARIZE" == 1 ]]; then
  # notarytool will ein Archiv; danach das Ticket an die App heften und neu zippen.
  ditto -c -k --keepParent "$APP" "$OUT/notarize.zip"
  xcrun notarytool submit "$OUT/notarize.zip" \
    --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" --wait
  xcrun stapler staple "$APP"
  rm "$OUT/notarize.zip"
  spctl --assess --type execute --verbose=2 "$APP"
fi

ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"

# Ergibt: sparkle:edSignature="…" length="…"
SIGNATURE=$("$SPARKLE_DIR/bin/sign_update" --ed-key-file "$SPARKLE_KEY_FILE" "$ZIP")
MIN_SYSTEM=$(/usr/libexec/PlistBuddy -c "Print :LSMinimumSystemVersion" "$APP/Contents/Info.plist" 2>/dev/null || echo 15.0)
PUB_DATE=$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")

cat > "$OUT/appcast.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Lichtung</title>
    <link>$REPO_URL</link>
    <item>
      <title>Lichtung $VERSION</title>
      <pubDate>$PUB_DATE</pubDate>
      <sparkle:version>$BUILD_NUMBER</sparkle:version>
      <sparkle:shortVersionString>$VERSION</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>$MIN_SYSTEM</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>$REPO_URL/releases/tag/v$VERSION</sparkle:fullReleaseNotesLink>
      <enclosure url="$REPO_URL/releases/download/v$VERSION/$(basename "$ZIP")" type="application/octet-stream" $SIGNATURE />
    </item>
  </channel>
</rss>
XML

echo "Fertig: $ZIP und $OUT/appcast.xml"
