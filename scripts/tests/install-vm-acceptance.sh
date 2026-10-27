#!/usr/bin/env bash
# MANUAL ONLY: modifies an explicitly designated disposable VM. Never run on host.
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
set -Eeuo pipefail
[[ "${NUSA_DISPOSABLE_VM:-}" == yes ]] || {
    echo 'Refusing: set NUSA_DISPOSABLE_VM=yes only inside a disposable test VM.' >&2; exit 1;
}
[[ "$(id -u)" == 0 ]] || { echo 'Run as root inside the disposable VM.' >&2; exit 1; }
systemd-detect-virt --vm >/dev/null || { echo 'A full VM is required.' >&2; exit 1; }
mode="${1:-}"
case "$mode" in
    --install|--upgrade)
        release="${2:?Provide extracted release directory}"
        args=(--unattended)
        [[ "$mode" != --upgrade ]] || args+=(--upgrade)
        PATH=/usr/local/bin:/usr/bin:/bin /bin/bash "$release/install.sh" "${args[@]}"
        # A second install must preserve DB/account/token and all tenant sites.
        before="$(runuser -u panel-api -- sqlite3 /var/lib/panel/panel.db 'SELECT key,value FROM settings ORDER BY key;')"
        /bin/bash "$release/install.sh" "${args[@]}"
        after="$(runuser -u panel-api -- sqlite3 /var/lib/panel/panel.db 'SELECT key,value FROM settings ORDER BY key;')"
        [[ "$before" == "$after" ]]
        ;;
    --after-reboot) ;;
    *) echo 'Usage: install-vm-acceptance.sh --install|--upgrade <release-dir> | --after-reboot' >&2; exit 1 ;;
esac
port="$(cat /etc/panel/public-port)"
for service in panel-agent panel-api nginx; do systemctl is-active --quiet "$service"; done
[[ "$(stat -c '%U:%G:%a' /run/panel)" == root:panel:750 ]]
[[ "$(stat -c '%U:%G:%a' /run/panel/agent.sock)" == root:panel:660 ]]
runuser -u panel-api -- /usr/local/bin/panelctl ipc-check
curl -fsS http://127.0.0.1:7700/healthz
curl -fsS "http://127.0.0.1:$port/healthz"
# API restart must not unlink the agent socket.
inode="$(stat -c '%i' /run/panel/agent.sock)"
systemctl restart panel-api
[[ "$(stat -c '%i' /run/panel/agent.sock)" == "$inode" ]]
runuser -u panel-api -- /usr/local/bin/panelctl ipc-check
# Start API from / as the correct identity without relying on WorkingDirectory.
systemctl stop panel-api
trap 'systemctl start panel-api' EXIT
cd /
runuser -u panel-api -- env PANEL_ENVIRONMENT=production /usr/local/bin/panel-api &
probe_pid=$!
for attempt in {1..20}; do
    if curl -fsS http://127.0.0.1:7700/healthz >/dev/null; then break; fi
    sleep 1
done
curl -fsS http://127.0.0.1:7700/healthz
kill "$probe_pid"
wait "$probe_pid" || true
systemctl start panel-api
trap - EXIT
panelctl doctor
echo 'Manual VM checks passed. For reboot persistence: reboot, then rerun --after-reboot.'
