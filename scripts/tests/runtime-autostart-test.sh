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
mkdir -p "$root/etc/systemd/system/nginx.service.d"
printf '[Service]\nTimeoutStopSec=30s\n' > "$root/etc/systemd/system/nginx.service.d/90-operator.conf"
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
nginx_dropin=/etc/systemd/system/nginx.service.d/10-panel-startup.conf
cmp "$REPO/templates/systemd/nginx-panel-startup.conf" "$root$nginx_dropin"
[[ " ${MANAGED_FILES[*]} " == *" $nginx_dropin "* ]]
grep -Fq "$nginx_dropin" "$REPO/uninstall.sh"
[[ "$(cat "$root/etc/systemd/system/nginx.service.d/90-operator.conf")" == $'[Service]\nTimeoutStopSec=30s' ]]
pass 'fresh and repeated upgrades install Nginx startup recovery while preserving operator overrides'

# Verify that the actual rollback restores pre-upgrade files, without touching host paths.
MANAGED_FILES=(
    "$root/etc/systemd/system/panel-fpm@.service.d/10-panel-runtime-order.conf"
    "$root/etc/systemd/system/site-.service.d/10-panel-runtime-order.conf"
    "$root$nginx_dropin"
)
BACKUP_DIR="$fixture/backup"
STATE_CHECKED=false
mkdir -p "$BACKUP_DIR/files$(dirname "${MANAGED_FILES[0]}")"
printf '# old drop-in\n' > "$BACKUP_DIR/files${MANAGED_FILES[0]}"
rollback >/dev/null 2>&1
[[ "$(cat "${MANAGED_FILES[0]}")" == '# old drop-in' && ! -e "${MANAGED_FILES[1]}" ]]
[[ -f "$old_unit" && -f "$root/etc/systemd/system/site-.service.d/90-operator.conf" ]]
[[ ! -e "$root$nginx_dropin" && -f "$root/etc/systemd/system/nginx.service.d/90-operator.conf" ]]
pass 'failed upgrade restores old drop-ins and removes only newly managed files'
mkdir -p "$BACKUP_DIR/files$(dirname "${MANAGED_FILES[2]}")"
printf '# previous Nginx startup policy\n' > "$BACKUP_DIR/files${MANAGED_FILES[2]}"
cp "$REPO/templates/systemd/nginx-panel-startup.conf" "$root$nginx_dropin"
rollback >/dev/null 2>&1
[[ "$(cat "$root$nginx_dropin")" == '# previous Nginx startup policy' ]]
[[ "$(cat "$root/etc/systemd/system/nginx.service.d/90-operator.conf")" == $'[Service]\nTimeoutStopSec=30s' ]]
pass 'failed repeated upgrade restores the previous Nginx policy and retains operator overrides'

# Exercise the real activation function: persistent enablement precedes reload/start.
nginx() { printf 'nginx %s\n' "$*" >> "$trace"; return "${NGINX_TEST_EXIT:-0}"; }
systemctl() {
    printf '%s\n' "$*" >> "$trace"
    if [[ "$1" == enable ]]; then return "${ENABLE_EXIT:-0}"; fi
    if [[ "$1" == is-active ]]; then return "${ACTIVE_EXIT:-0}"; fi
}
: > "$trace"
ACTIVE_EXIT=0
activate_nginx_proxy
[[ "$(cat "$trace")" == $'nginx -t\nenable nginx.service\nis-active --quiet nginx.service\nreload nginx.service' ]]
: > "$trace"
ACTIVE_EXIT=3
activate_nginx_proxy
[[ "$(cat "$trace")" == $'nginx -t\nenable nginx.service\nis-active --quiet nginx.service\nstart nginx.service' ]]
pass 'Nginx is enabled persistently and active proxies are reloaded instead of restarted'

# Run under errexit, as the real installer does; do not put the function in an if.
: > "$trace"
ENABLE_EXIT=1
set +e
(set -e; activate_nginx_proxy) >/dev/null 2>&1
activation_status=$?
set -e
[[ "$activation_status" != 0 ]]
! grep -Eq '^(start|reload|restart) ' "$trace"
pass 'failed boot enablement stops installation before service activation'
ENABLE_EXIT=0

grep -qx 'Wants=network-online.target' "$REPO/templates/systemd/nginx-panel-startup.conf"
grep -qx 'After=network-online.target nss-lookup.target' "$REPO/templates/systemd/nginx-panel-startup.conf"
grep -qx 'Restart=on-failure' "$REPO/templates/systemd/nginx-panel-startup.conf"
grep -qx 'RestartSec=10s' "$REPO/templates/systemd/nginx-panel-startup.conf"
grep -qx 'StartLimitIntervalSec=120' "$REPO/templates/systemd/nginx-panel-startup.conf"
grep -qx 'StartLimitBurst=6' "$REPO/templates/systemd/nginx-panel-startup.conf"
! grep -Eq '^(ExecStart|ExecStop|Requires=|PartOf=|BindsTo=)' "$REPO/templates/systemd/nginx-panel-startup.conf"
pass 'Nginx waits for boot networking and retries failures with a bounded rate and no agent lifecycle coupling'

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
