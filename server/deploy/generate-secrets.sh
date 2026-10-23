#!/bin/bash
set -e

# ============================================================================
# Secret Generation Script for Lichtung
# ============================================================================
# Same pattern as Zunder:
#   - The committed template .env.example is never modified. The filled-in
#     file is written to .env (ignored by git, mode 600), which compose.yaml
#     loads via env_file.
#   - Prompts go to stderr, so $(prompt_value ...) captures only the value.
#   - Values are escaped for sed and replaced without `sed -i`, so values
#     containing / & | (e.g. base64 keys) and macOS sed work.
# ============================================================================

echo "Secret Generation Script for Lichtung"
echo "================================================"
echo ""

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

# Work next to this script, regardless of the current directory
cd "$(dirname "$0")"

TEMPLATE_FILE=".env.example"
ENV_FILE=".env"
TEMP_FILE="${ENV_FILE}.tmp"

# Random alphanumeric secret (default 32 characters)
generate_secret() {
    local length=${1:-32}
    openssl rand -base64 $((length * 2)) | tr -d "=+/\n" | cut -c1-"$length"
}

# 32 random bytes as base64 (LICHTUNG_SECRET_KEY)
generate_key() {
    openssl rand -base64 32 | tr -d "\n"
}

# Hidden input; prompts go to stderr.
prompt_secret() {
    local var_name=$1
    local description=$2
    local secret_value

    echo -e "${YELLOW}Enter ${description} (${var_name}):${NC}" >&2
    read -rs secret_value
    echo "" >&2

    if [ -z "$secret_value" ]; then
        echo -e "${RED}Error: Value cannot be empty${NC}" >&2
        exit 1
    fi

    echo "$secret_value"
}

# Visible input for non-secret values.
prompt_value() {
    local var_name=$1
    local description=$2
    local value

    echo -e "${YELLOW}Enter ${description} (${var_name}):${NC}" >&2
    read -r value

    if [ -z "$value" ]; then
        echo -e "${RED}Error: Value cannot be empty${NC}" >&2
        exit 1
    fi

    echo "$value"
}

# Escape a value for use as sed replacement with "|" as delimiter
escape_sed() {
    printf '%s' "$1" | sed -e 's/[\\|&]/\\&/g'
}

# Replace "NAME=PLACEHOLDER" with "NAME=value" in TEMP_FILE
set_var() {
    local name=$1
    local placeholder=$2
    local value=$3

    case "$value" in
        *$'\n'*)
            echo -e "${RED}Error: ${name} must not contain line breaks${NC}" >&2
            exit 1
            ;;
    esac

    sed -e "s|^${name}=${placeholder}\$|${name}=$(escape_sed "$value")|" "$TEMP_FILE" > "${TEMP_FILE}.new"
    mv "${TEMP_FILE}.new" "$TEMP_FILE"
}

if [ ! -f "$TEMPLATE_FILE" ]; then
    echo -e "${RED}Error: $TEMPLATE_FILE not found${NC}"
    echo "Please run this script next to .env.example (server/deploy/ of the Lichtung repository)"
    exit 1
fi

if ! command -v openssl > /dev/null 2>&1; then
    echo -e "${RED}Error: openssl is required${NC}"
    exit 1
fi

if [ -f "$ENV_FILE" ]; then
    echo -e "${YELLOW}$ENV_FILE exists and will be overwritten.${NC}"
    echo -e "${YELLOW}Achtung: Ein neuer LICHTUNG_SECRET_KEY macht gespeicherte Zugangsdaten unlesbar.${NC}"
    echo -e "${YELLOW}Abbrechen mit Strg+C, sonst Enter.${NC}"
    read -r _
    echo ""
fi

echo "Generating secrets..."
echo ""

# Copy base template, readable only by the current user
umask 077
cp "$TEMPLATE_FILE" "$TEMP_FILE"
trap 'rm -f "$TEMP_FILE" "${TEMP_FILE}.new"' EXIT

# ============================================================================
# GENERATED variables
# ============================================================================
set_var LICHTUNG_SESSION_SECRET WILL_BE_GENERATED "$(generate_secret 64)"
set_var LICHTUNG_SECRET_KEY WILL_BE_GENERATED "$(generate_key)"

echo -e "${GREEN}Generated random secrets${NC}"
echo ""

# ============================================================================
# MANUAL variables
# ============================================================================
echo "Please provide the following credentials:"
echo "   (Pocket-ID: OIDC-Client mit Callback https://lichtung.rubeen.dev/auth/callback)"
echo ""

LICHTUNG_OIDC_ISSUER=$(prompt_value "LICHTUNG_OIDC_ISSUER" "Pocket-ID Issuer URL, e.g. https://id.example.com")
set_var LICHTUNG_OIDC_ISSUER CHANGEME_MANUAL "$LICHTUNG_OIDC_ISSUER"

LICHTUNG_OIDC_CLIENT_ID=$(prompt_value "LICHTUNG_OIDC_CLIENT_ID" "Pocket-ID Client ID")
set_var LICHTUNG_OIDC_CLIENT_ID CHANGEME_MANUAL "$LICHTUNG_OIDC_CLIENT_ID"

LICHTUNG_OIDC_CLIENT_SECRET=$(prompt_secret "LICHTUNG_OIDC_CLIENT_SECRET" "Pocket-ID Client Secret")
set_var LICHTUNG_OIDC_CLIENT_SECRET CHANGEME_MANUAL "$LICHTUNG_OIDC_CLIENT_SECRET"

LICHTUNG_OIDC_ALLOWED_EMAIL=$(prompt_value "LICHTUNG_OIDC_ALLOWED_EMAIL" "E-Mail of the Pocket-ID account allowed to log in")
set_var LICHTUNG_OIDC_ALLOWED_EMAIL CHANGEME_MANUAL "$LICHTUNG_OIDC_ALLOWED_EMAIL"

# ============================================================================

# Every placeholder must be gone now
if grep -Eq '=(WILL_BE_GENERATED|CHANGEME_MANUAL)$' "$TEMP_FILE"; then
    echo -e "${RED}Error: unreplaced placeholders left:${NC}"
    grep -E '=(WILL_BE_GENERATED|CHANGEME_MANUAL)$' "$TEMP_FILE" | cut -d= -f1
    exit 1
fi

# Move result into place (template stays untouched)
mv "$TEMP_FILE" "$ENV_FILE"

echo ""
echo "================================================"
echo -e "${GREEN}All secrets configured!${NC}"
echo ""
echo "Written: $(pwd)/$ENV_FILE (mode 600)"
echo ""
echo "Next steps (on the server, in ~/server/lichtung):"
echo "   sudo chown 10001:10001 data   # once, the container runs as uid 10001"
echo "   docker compose pull && docker compose up -d"
echo ""
echo "Security reminders:"
echo "   - Never commit .env to git"
echo "   - Keep a copy of LICHTUNG_SECRET_KEY in the password manager (needed to read backups)"
echo "   - If you generated it on another machine: copy it with scp, then delete the local copy"
