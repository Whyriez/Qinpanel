#!/usr/bin/env bash
# Pure URL/metadata tests; curl is mocked, no downloads or installer execution.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$REPO/scripts/bootstrap-install.sh"
count=0
for invalid in http://example.com https://user:secret@example.com https://example.com/ https://example.com/a/../b 'https://example.com/a?token=x' 'https://example.com/a#x' https://example.com//a https://example.com:443; do
    if validate_release_base "$invalid"; then exit 1; fi
    count=$((count + 1))
done
for valid in https://download.arelqon.com/qonpanel https://project.supabase.co/storage/v1/object/public/qonpanel-releases; do
    validate_release_base "$valid"
    RELEASE_BASE_URL="$valid"
    name="$(build_asset_name v1.2.3 x86_64-unknown-linux-gnu)"
    [[ "$name" == qinpanel-v1.2.3-x86_64-unknown-linux-gnu.tar.gz ]]
    [[ "$(build_asset_url v1.2.3 "$name")" == "$valid/releases/v1.2.3/$name" ]]
    [[ "$(build_checksum_url v1.2.3)" == "$valid/releases/v1.2.3/SHA256SUMS" ]]
    curl() { [[ "${*: -1}" == "$RELEASE_BASE_URL/latest.json" && "$*" != *github* ]]; printf '{"tag_name":"v1.2.3"}'; }
    [[ "$(resolve_latest_version)" == v1.2.3 ]]
    count=$((count + 5))
done
curl() { return 22; }
if resolve_latest_version >/dev/null 2>&1; then exit 1; fi
count=$((count + 1))
NUSA_PANEL_VERSION=v1.2.4
[[ "$(resolve_latest_version)" == v1.2.4 ]]
unset NUSA_PANEL_VERSION
count=$((count + 1))
RELEASE_BASE_URL=''
[[ "$(build_checksum_url v1.2.3)" == https://github.com/Whyriez/Qinpanel/releases/download/v1.2.3/SHA256SUMS ]]
count=$((count + 1))
printf '%s distribution bootstrap checks passed (no network).\n' "$count"
