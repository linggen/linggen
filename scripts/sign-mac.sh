#!/bin/bash
# Sign a release Mach-O so a browser download opens without the "damaged"
# dialog: Developer ID + hardened runtime + secure timestamp, then notarize
# (zip submit — a bare binary cannot be stapled; Gatekeeper finds the ticket
# online). With no Developer ID in the keychain it falls back to ad-hoc, which
# is what local dev builds use.
#
#   ./scripts/sign-mac.sh <binary> [entitlements.plist]
#
# Credentials (same as linggen-app's secrets/signing.env, which is sourced when
# APPLE_SIGNING_IDENTITY is unset; override the path with LINGGEN_SIGNING_ENV):
#   APPLE_SIGNING_IDENTITY  "Developer ID Application: … (9MFA5K7LUP)"
#   notarization — one of:
#     APPLE_API_KEY + APPLE_API_ISSUER + APPLE_API_KEY_PATH   (ASC API key)
#     NOTARY_PROFILE          (xcrun notarytool store-credentials <name>)
#     APPLE_ID + APPLE_TEAM_ID + APPLE_APP_PASSWORD
# LINGGEN_SKIP_NOTARIZE=1 signs without notarizing (a quick local check).
set -euo pipefail

BIN="${1:-}"
ENTITLEMENTS="${2:-}"
[ -f "$BIN" ] || { echo "usage: $0 <binary> [entitlements.plist]" >&2; exit 1; }

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  env_file="${LINGGEN_SIGNING_ENV:-$ROOT_DIR/../linggen-app/secrets/signing.env}"
  # shellcheck disable=SC1090
  [ -f "$env_file" ] && . "$env_file"
fi

adhoc() {
  echo "   ⚠️  $1 — ad-hoc signing $(basename "$BIN") (a browser download will be blocked)"
  codesign --force --sign - "$BIN"
  exit 0
}

[ -n "${APPLE_SIGNING_IDENTITY:-}" ] || adhoc "no APPLE_SIGNING_IDENTITY"
identities="$(security find-identity -v -p codesigning 2>/dev/null || true)"
grep -qF "$APPLE_SIGNING_IDENTITY" <<<"$identities" || adhoc "'$APPLE_SIGNING_IDENTITY' is not in the keychain"

args=(--force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY")
[ -n "$ENTITLEMENTS" ] && args+=(--entitlements "$ENTITLEMENTS")
codesign "${args[@]}" "$BIN"
codesign --verify --strict --verbose=2 "$BIN" 2>&1 | sed 's/^/   /'
echo "   signed: $APPLE_SIGNING_IDENTITY"

[ "${LINGGEN_SKIP_NOTARIZE:-0}" = 1 ] && { echo "   notarization skipped (LINGGEN_SKIP_NOTARIZE=1)"; exit 0; }

if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
  auth=(--key "$APPLE_API_KEY_PATH" --key-id "${APPLE_API_KEY:?}" --issuer "${APPLE_API_ISSUER:?}")
elif [ -n "${NOTARY_PROFILE:-}" ]; then
  auth=(--keychain-profile "$NOTARY_PROFILE")
elif [ -n "${APPLE_APP_PASSWORD:-}" ]; then
  auth=(--apple-id "${APPLE_ID:?}" --team-id "${APPLE_TEAM_ID:?}" --password "$APPLE_APP_PASSWORD")
else
  echo "Error: signed with Developer ID but no notarization credentials (see $0)" >&2
  exit 1
fi

zip="$(mktemp -d)/$(basename "$BIN").zip"
ditto -c -k --keepParent "$BIN" "$zip"
echo "   notarizing $(basename "$BIN") (notarytool --wait)…"
out="$(xcrun notarytool submit "$zip" "${auth[@]}" --wait 2>&1)" || true
rm -rf "$(dirname "$zip")"
echo "$out" | sed 's/^/   /'
if ! grep -q 'status: Accepted' <<<"$out"; then
  id="$(grep -m1 -oE 'id: [0-9a-f-]{36}' <<<"$out" | awk '{print $2}')"
  [ -n "$id" ] && xcrun notarytool log "$id" "${auth[@]}" 2>&1 | sed 's/^/   /'
  echo "Error: notarization of $(basename "$BIN") was not accepted" >&2
  exit 1
fi
echo "   notarized (no staple for a bare binary; Gatekeeper checks online)"
