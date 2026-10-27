#!/usr/bin/env bash
# Rootless worker/bootstrap/upgrade-contract fixtures. No agent or host service.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/qpanel-update.XXXXXXXX")"
fixture="$(cd "$fixture" && pwd -P)"
[[ "$fixture" == /*/qpanel-update.* && "$fixture" != / ]] || exit 99
trap 'rm -rf -- "$fixture"' EXIT
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }
id=12345678-1234-1234-1234-123456789abc
digest="$(printf 'a%.0s' {1..64})"
for outcome in success failure; do
    directory="$fixture/$outcome"; mkdir -p "$directory"
    printf '{"id":"%s","phase":"queued"}\n' "$id" > "$directory/state.json"
    code=0
    bash -c '
        source "$1/scripts/panel-update-worker.sh"
        STATE_DIR="$2"; UPDATE_ID="$3"; UPDATE_VERSION=v0.2.0; UPDATE_DIGEST="$4"
        flock() { [[ "$*" == "-n 8" ]]; }
        env() {
            [[ "$1" == -i && "$*" == *"NUSA_PANEL_VERSION=v0.2.0"* && "$*" == *"NUSA_PANEL_EXPECTED_SHA256=$UPDATE_DIGEST"* && "$*" == *"/bin/bash /usr/local/libexec/panel-bootstrap --upgrade --unattended" ]] || return 99
            printf "worker invoked trusted bootstrap\n" > "$STATE_DIR/trace"
            [[ "$5" != impossible ]] || return 99
            [[ "$fixture_outcome" == success ]] || return 17
        }
        fixture_outcome="$5"
        main
    ' fixture "$REPO" "$directory" "$id" "$digest" "$outcome" || code=$?
    if [[ "$outcome" == success ]]; then [[ "$code" == 0 ]]; grep -q '"phase":"succeeded"' "$directory/state.json"
    else [[ "$code" != 0 ]]; grep -q '"phase":"failed"' "$directory/state.json"; fi
    [[ -s "$directory/trace" ]]
    pass "worker $outcome stores durable terminal state while executing only the mocked trusted bootstrap"
done
source "$REPO/scripts/panel-update-worker.sh"
STATE_DIR="$fixture/finalize"; mkdir -p "$STATE_DIR"
UPDATE_ID="$id"; UPDATE_VERSION=v0.2.0; UPDATE_DIGEST="$digest"
printf '{"id":"%s","phase":"running"}\n' "$id" > "$STATE_DIR/state.json"
main --finalize
grep -q '"phase":"interrupted"' "$STATE_DIR/state.json"
main --finalize
grep -q '"phase":"interrupted"' "$STATE_DIR/state.json"
pass 'service finalizer marks interrupted work without replaying it or replacing terminal states'

source "$REPO/scripts/bootstrap-install.sh"
printf 'trusted package\n' > "$fixture/package.tar.gz"
(cd "$fixture"; sha256sum package.tar.gz > SHA256SUMS)
NUSA_PANEL_EXPECTED_SHA256="$(sha256sum "$fixture/package.tar.gz" | awk '{print $1}')"
verify_checksum "$fixture/SHA256SUMS" "$fixture/package.tar.gz" package.tar.gz
NUSA_PANEL_EXPECTED_SHA256="$digest"
if verify_checksum "$fixture/SHA256SUMS" "$fixture/package.tar.gz" package.tar.gz >/dev/null 2>&1; then exit 1; fi
pass 'asset changed after administrator check is rejected even when its published checksum matches'

# Map the installer's fixed state/lock paths into a private tree before sourcing it.
root="$fixture/root"; mkdir -p "$root/etc/panel" "$root/var/lib/panel" "$root/run/lock"
sed -e "s|/etc/panel|$root/etc/panel|g" -e "s|/var/lib/panel|$root/var/lib/panel|g" -e "s|/run/lock|$root/run/lock|g" "$REPO/install.sh" > "$fixture/install.sh"
cp "$REPO/UPGRADE_FROM" "$fixture/UPGRADE_FROM"
source "$fixture/install.sh"
runuser() { [[ "$*" == *'SELECT EXISTS(SELECT 1 FROM users'* ]] || return 99; printf '%s\n' "${admin_exists:-1}"; }
touch "$root/var/lib/panel/panel.db"
printf 'v0.1.0 -\n' > "$root/etc/panel/installed-release"
RELEASE_VERSION=v0.2.0
validate_upgrade; [[ "$UPGRADE" == true ]]
pass 'existing installation is upgraded in place with its administrator retained'
RELEASE_VERSION=v0.0.9
if validate_upgrade >/dev/null 2>&1; then exit 1; fi
pass 'downgrade is rejected before any service stops or migrations'
printf 'v0.3.0\n' > "$fixture/UPGRADE_FROM"
RELEASE_VERSION=v0.4.0
if validate_upgrade >/dev/null 2>&1; then exit 1; fi
printf 'v0.1.0\n' > "$fixture/UPGRADE_FROM"
pass 'an incompatible source version is rejected before changing existing files'
RELEASE_VERSION=v0.2.0; admin_exists=0
if validate_upgrade >/dev/null 2>&1; then exit 1; fi
pass 'upgrade refuses to turn a missing administrator into a fresh installation'
(
    command() { [[ "$1" == -v ]] || return 99; return 0; }
    apt-get() { echo 'Host package manager must not run during a panel update' >&2; return 99; }
    UPGRADE=true
    install_packages
)
pass 'panel upgrade does not install or upgrade host packages and tenant runtimes'
admin_exists=1
if command -v flock >/dev/null; then
    acquire_install_lock
    if bash -c 'exec 9>"$1"; flock -n 9' fixture "$root/run/lock/panel-install.lock"; then exit 1; fi
    exec 9>&-
    pass 'installer lock excludes simultaneous version updates using a temporary lock'
else
    lock_held=false
    flock() { [[ "$*" == '-n 9' && "$lock_held" == false ]] || return 1; lock_held=true; }
    acquire_install_lock
    if acquire_install_lock >/dev/null 2>&1; then exit 1; fi
    exec 9>&-
    pass 'installer refuses a held lock (mock; real flock runs in Linux CI)'
fi

unit="$REPO/templates/systemd/panel-update.service"
grep -qx 'ExecStart=/usr/local/libexec/panel-update-worker' "$unit"
! grep -Eq '^(PartOf|BindsTo|Requires)=.*panel-(api|agent)' "$unit"
grep -qx 'Restart=no' "$unit"
pass 'worker service is independent of API/agent restarts and never replays automatically'
printf '%s panel update checks passed (fixtures only).\n' "$count"
