#!/usr/bin/env bash
# Rootless installer/upgrade fixture. Never runs services, agents, or setfacl.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$REPO/install.sh"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/qpanel-autostart.XXXXXXXX")"
fixture="$(cd "$fixture" && pwd -P)"
[[ "$fixture" == /*/qpanel-autostart.* && "$fixture" != / ]] || exit 99
trap 'rm -rf -- "$fixture"' EXIT
root="$fixture/root"
trace="$fixture/trace"
mkdir -p "$root" "$fixture/bin"
BIN_SOURCE="$fixture/bin"
for binary in panel-agent panel-api panelctl; do printf 'fixture binary\n' > "$BIN_SOURCE/$binary"; done
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }

# Redirect all installation destinations to the fixture; strip owner/group options.
install() {
    local directory=false mode=0755 argument
    while [[ "$1" == -* ]]; do
        case "$1" in
            -d) directory=true; shift ;;
            -o|-g) shift 2 ;;
            -m) mode="$2"; shift 2 ;;
            *) return 99 ;;
        esac
    done
    if [[ "$directory" == true ]]; then
        for argument in "$@"; do
            [[ "$argument" == /etc/systemd/system/* || "$argument" == /usr/local/libexec ]] || return 99
            command install -d -m "$mode" "$root$argument"
        done
    else
        [[ "$1" == "$REPO/"* || "$1" == "$BIN_SOURCE/"* ]] || return 99
        [[ "$2" == /etc/systemd/system/* || "$2" == /usr/local/bin/* || "$2" == /usr/local/libexec/* ]] || return 99
        command mkdir -p "$root$(dirname "$2")"
        command install -m "$mode" "$1" "$root$2"
    fi
}
mv() {
    [[ "$1" == -f && ( "$2" == /usr/local/bin/.*.new || "$2" == /usr/local/libexec/.*.new ) && ( "$3" == /usr/local/bin/* || "$3" == /usr/local/libexec/* ) ]] || return 99
    command mv -f "$root$2" "$root$3"
}
systemctl() { printf '%s\n' "$*" >> "$trace"; }

old_unit="$root/etc/systemd/system/site-old.service"
mkdir -p "$(dirname "$old_unit")" "$root/etc/systemd/system/site-.service.d"
printf '[Service]\nUser=p_alice\n' > "$old_unit"
printf '# operator configuration\n' > "$root/etc/systemd/system/site-.service.d/90-operator.conf"
install_binaries_and_units
for name in panel-fpm@.service.d site-.service.d; do
    path="/etc/systemd/system/$name/10-panel-runtime-order.conf"
    cmp "$REPO/templates/systemd/panel-runtime-order.conf" "$root$path"
    [[ " ${MANAGED_FILES[*]} " == *" $path "* ]]
    grep -Fq "$path" "$REPO/uninstall.sh"
done
grep -qx 'daemon-reload' "$trace"
pass 'installer applies boot ordering to legacy FPM instances and site units with rollback/uninstall coverage'
[[ "$(cat "$old_unit")" == $'[Service]\nUser=p_alice' ]]
[[ "$(cat "$root/etc/systemd/system/site-.service.d/90-operator.conf")" == '# operator configuration' ]]
pass 'existing unit and operator drop-ins are preserved'

install_binaries_and_units
[[ "$(grep -c '^daemon-reload$' "$trace")" == 2 ]]
cmp "$REPO/templates/systemd/panel-agent.service" "$root/etc/systemd/system/panel-agent.service"
pass 'repeat upgrade retains the complete agent and runtime ordering configuration'

# Verify that the actual rollback restores pre-upgrade files, without touching host paths.
MANAGED_FILES=(
    "$root/etc/systemd/system/panel-fpm@.service.d/10-panel-runtime-order.conf"
    "$root/etc/systemd/system/site-.service.d/10-panel-runtime-order.conf"
)
BACKUP_DIR="$fixture/backup"
STATE_CHECKED=false
mkdir -p "$BACKUP_DIR/files$(dirname "${MANAGED_FILES[0]}")"
printf '# old drop-in\n' > "$BACKUP_DIR/files${MANAGED_FILES[0]}"
rollback >/dev/null 2>&1
[[ "$(cat "${MANAGED_FILES[0]}")" == '# old drop-in' && ! -e "${MANAGED_FILES[1]}" ]]
[[ -f "$old_unit" && -f "$root/etc/systemd/system/site-.service.d/90-operator.conf" ]]
pass 'failed upgrade restores old drop-ins and removes only newly managed files'

agent="$root/etc/systemd/system/panel-agent.service"
grep -qx 'ExecStartPre=/usr/bin/setfacl -m g:panel-tenants:--x,u:www-data:--x -- /run/panel' "$agent"
grep -qx 'RuntimeDirectoryMode=0750' "$agent"
grep -qx 'RuntimeDirectoryPreserve=yes' "$agent"
grep -qx 'Wants=panel-agent.service' "$REPO/templates/systemd/panel-runtime-order.conf"
grep -qx 'After=panel-agent.service' "$REPO/templates/systemd/panel-runtime-order.conf"
! grep -q '^Requires=' "$REPO/templates/systemd/panel-runtime-order.conf"
grep -Fq 'getent group panel-tenants >/dev/null || groupadd -r panel-tenants' "$REPO/install.sh"
pass 'each agent start restores search-only group/web ACL before runtimes, preserving sockets on stop'
printf '%s autostart checks passed (fixture only; no Linux boot executed).\n' "$count"
