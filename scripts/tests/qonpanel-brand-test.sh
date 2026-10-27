#!/usr/bin/env bash
# Hermetic branding/upgrade fixtures. No service, VM, network or panel daemon.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
source "$REPO/scripts/bootstrap-install.sh"
[[ "$GITHUB_REPO" == Qinpanel ]]
[[ "$(build_asset_name v0.2.0 x86_64-unknown-linux-gnu)" == qinpanel-v0.2.0-x86_64-unknown-linux-gnu.tar.gz ]]
count=2
for brand in qinpanel qonpanel nusa-panel; do
    root="$brand-v0.2.0-x86_64-unknown-linux-gnu"
    mkdir -p "$fixture/$root" "$fixture/extract-$brand"
    printf '#!/usr/bin/env bash\nprintf "fixture installer: %%s\\n" "$*"\n' > "$fixture/$root/install.sh"
    tar -czf "$fixture/$root.tar.gz" -C "$fixture" "$root"
    (cd "$fixture" && sha256sum "$root.tar.gz" > SHA256SUMS)
    (
        TMP_DIR=""
        check_root() { :; }; detect_os() { :; }; check_dependencies() { :; }
        resolve_latest_version() { echo v0.2.0; }; uname() { echo x86_64; }
        download_file() {
            if [[ "$1" == */SHA256SUMS ]]; then cp "$fixture/SHA256SUMS" "$2"
            elif [[ "$1" == */"$root.tar.gz" ]]; then cp "$fixture/$root.tar.gz" "$2"
            else return 22; fi
        }
        main --upgrade --unattended > "$fixture/$brand.log"
    )
    grep -q 'fixture installer: --upgrade --unattended' "$fixture/$brand.log"
    grep -q 'Qinpanel Installer' "$fixture/$brand.log"
    count=$((count + 2))
done
# Canonical env settings override legacy aliases; old aliases remain supported.
value="$(QONPANEL_VERSION=v0.2.0 NUSA_PANEL_VERSION=v0.1.0 bash -c 'source "$1/scripts/bootstrap-install.sh"; resolve_latest_version' test "$REPO")"
[[ "$value" == v0.2.0 ]]; count=$((count + 1))
value="$(NUSA_PANEL_VERSION=v0.1.0 bash -c 'source "$1/scripts/bootstrap-install.sh"; resolve_latest_version' test "$REPO")"
[[ "$value" == v0.1.0 ]]; count=$((count + 1))
printf '%s Qinpanel branding/compatibility checks passed.\n' "$count"
