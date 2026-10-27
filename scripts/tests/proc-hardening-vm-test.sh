#!/usr/bin/env bash
# Alters the VM's host procfs policy. Never run on a developer host or container.
set -Eeuo pipefail
fail() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
[[ "$EUID" == 0 ]] || fail 'Run as root in a disposable Ubuntu 24.04 VM.'
source /etc/os-release
[[ "$ID" == ubuntu && "$VERSION_ID" == 24.04 ]] || fail 'Ubuntu 24.04 is required.'
systemd-detect-virt --vm --quiet || fail 'A full VM is required; containers are not accepted.'
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
helper=panel-proc-hardening.service
cmp -s "$repo/templates/systemd/$helper" "/etc/systemd/system/$helper" || fail 'Install the updated helper unit first.'
getent group panel >/dev/null || fail 'The panel service group is required.'
getent passwd panel-api >/dev/null || fail 'The non-root panel-api identity is required.'

# Reproduce the actual agent's capability and filesystem namespace restrictions,
# without starting the panel-agent binary or taking ownership of its socket dir.
properties=()
while IFS='=' read -r key value; do
    value="${value%$'\r'}"
    case "$key" in
        User|Group|NoNewPrivileges|ProtectSystem|ProtectHome|ReadWritePaths|ReadOnlyPaths|InaccessiblePaths|PrivateTmp|PrivateDevices|MemoryDenyWriteExecute|RestrictAddressFamilies|CapabilityBoundingSet|ProtectKernelModules|ProtectKernelTunables|ProtectControlGroups|LockPersonality)
            properties+=(--property="$key=$value") ;;
    esac
done < "$repo/templates/systemd/panel-agent.service"
[[ " ${properties[*]} " != *CAP_SYS_ADMIN* ]] || fail 'Agent must not have CAP_SYS_ADMIN.'
[[ " ${properties[*]} " == *'--property=ProtectSystem=strict '* ]] || fail 'Agent strict sandbox is required.'

systemctl daemon-reload
systemd-run --wait --collect --pipe --unit="panel-proc-enable-${RANDOM}" \
    "${properties[@]}" /usr/bin/systemctl enable "$helper"
systemd-run --wait --collect --pipe --unit="panel-proc-apply-${RANDOM}" \
    "${properties[@]}" /usr/bin/systemctl restart "$helper"

options="$(findmnt --task 1 -n -o OPTIONS --target /proc)"
[[ ",$options," == *,hidepid=2,* || ",$options," == *,hidepid=invisible,* ]] || fail "Host hidepid missing: $options"
panel_gid="$(getent group panel | cut -d: -f3)"
[[ ",$options," == *,gid="$panel_gid",* ]] || fail "Panel monitoring group missing: $options"
systemctl is-enabled --quiet "$helper"
systemctl is-active --quiet "$helper"
runuser -u panel-api -- test -r /proc/1/mounts
systemd-run --wait --collect --pipe --unit="panel-proc-read-${RANDOM}" \
    "${properties[@]}" /usr/bin/findmnt --task 1 -n -o OPTIONS --target /proc
printf 'PASS: sandboxed agent can invoke the fixed helper; host hidepid, monitoring and boot policy verified.\n'
printf 'The hardening policy remains enabled in this disposable VM. Reboot and repeat diagnostics for boot acceptance.\n'
