#!/usr/bin/env bash
# Ubuntu 24.04 disposable VM only. No daemon agent and no host listeners.
set -Eeuo pipefail
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
fail() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
[[ "$EUID" == 0 ]] || fail 'Run as root in a disposable Ubuntu 24.04 VM.'
source /etc/os-release
[[ "$ID" == ubuntu && "$VERSION_ID" == 24.04 ]] || fail 'Ubuntu 24.04 is required.'
systemd-detect-virt --vm --quiet || fail 'A full virtual machine is required.'
getent group panel >/dev/null || fail 'Install the panel service group first.'
nginx_bin="$(command -v nginx)" || fail 'Install nginx in the VM first.'
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture="$(mktemp -d /var/lib/panel/nginx-sandbox-XXXXXXXX)"
trap 'rm -rf "$fixture"' EXIT
cat > "$fixture/nginx.conf" <<'EOF'
error_log stderr;
events { worker_connections 16; }
http {
    access_log off;
    server { listen 0.0.0.0:80; return 204; }
    server { listen 0.0.0.0:443; return 204; }
}
EOF
# Verify privileged-port policy inside the same isolated network namespace as
# nginx; an unprivileged-port threshold of zero would invalidate the negative test.
cat > "$fixture/check.sh" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
threshold="$(cat /proc/sys/net/ipv4/ip_unprivileged_port_start)"
if (( threshold <= 443 )); then
    echo 'Privileged-port threshold must exceed 443 for this acceptance test.' >&2
    exit 77
fi
exec "$1" -t -c "$2"
EOF
positive=(--property=PrivateNetwork=yes)
negative=(--property=PrivateNetwork=yes)
while IFS='=' read -r key value; do
    value="${value%$'\r'}"
    case "$key" in
        User|Group|NoNewPrivileges|ProtectSystem|ProtectHome|ReadWritePaths|ReadOnlyPaths|InaccessiblePaths|PrivateTmp|PrivateDevices|MemoryDenyWriteExecute|RestrictAddressFamilies|ProtectKernelModules|ProtectKernelTunables|ProtectControlGroups|LockPersonality)
            positive+=(--property="$key=$value")
            negative+=(--property="$key=$value") ;;
        CapabilityBoundingSet)
            [[ " $value " == *' CAP_NET_BIND_SERVICE '* ]] || fail 'Template lacks the bind capability.'
            [[ " $value " != *' CAP_SYS_ADMIN '* && "$value" != '~'* ]] || fail 'Unbounded capabilities are forbidden.'
            positive+=(--property="$key=$value")
            read -ra capabilities <<< "$value"
            restricted=()
            for capability in "${capabilities[@]}"; do
                [[ "$capability" == CAP_NET_BIND_SERVICE ]] || restricted+=("$capability")
            done
            negative+=(--property="$key=${restricted[*]}") ;;
    esac
done < "$repo/templates/systemd/panel-agent.service"
status=0
output="$(systemd-run --wait --collect --pipe --unit="panel-nginx-deny-${RANDOM}" \
    "${negative[@]}" /usr/bin/bash "$fixture/check.sh" "$nginx_bin" "$fixture/nginx.conf" 2>&1)" || status=$?
[[ "$status" != 0 && "$output" == *'bind()'* && "$output" == *'Permission denied'* ]] || {
    printf '%s\n' "$output" >&2
    fail 'Missing bind capability did not reproduce the expected failure.'
}
systemd-run --wait --collect --pipe --unit="panel-nginx-allow-${RANDOM}" \
    "${positive[@]}" /usr/bin/bash "$fixture/check.sh" "$nginx_bin" "$fixture/nginx.conf"
printf 'PASS: nginx -t fails without CAP_NET_BIND_SERVICE and passes with the updated agent sandbox.\n'
