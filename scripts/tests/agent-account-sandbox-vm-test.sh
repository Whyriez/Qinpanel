#!/usr/bin/env bash
# Destructive acceptance check: Ubuntu 24.04 VM only, never run on a developer host.
# Probes runtime/package write paths and a temporary account under the agent sandbox.
set -Eeuo pipefail

fail() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
[[ "$EUID" == 0 ]] || fail 'Run as root inside the disposable Ubuntu 24.04 VM.'
source /etc/os-release
[[ "$ID" == ubuntu && "$VERSION_ID" == 24.04 ]] || fail 'Ubuntu 24.04 is required.'
systemd-detect-virt --vm --quiet || fail 'A full virtual machine is required.'

if [[ "${1:-}" == --inside ]]; then
    probe_name="${2:-}"
    probe_uid="${3:-}"
    [[ "$probe_name" =~ ^p_sbx_[0-9]+$ ]] || fail 'Invalid probe account name.'
    [[ "$probe_uid" =~ ^59[0-9]{3}$ ]] || fail 'Probe UID must be in 59000..59999.'
    probe_home="/home/$probe_name"
    [[ ! -e "$probe_home" ]] || fail 'Probe home already exists.'
    ! getent passwd "$probe_name" >/dev/null || fail 'Probe user already exists.'
    ! getent group "$probe_name" >/dev/null || fail 'Probe group already exists.'
    group_created=false
    user_created=false
    home_created=false
    cleanup() {
        local failed=0
        if [[ "$home_created" == true ]]; then
            rm -f -- "$probe_home/.sandbox-check" || failed=1
            rmdir -- "$probe_home" || failed=1
            home_created=false
        fi
        if [[ "$user_created" == true ]]; then
            userdel "$probe_name" || failed=1
            user_created=false
        fi
        if [[ "$group_created" == true ]] && getent group "$probe_name" >/dev/null; then
            groupdel "$probe_name" || failed=1
            group_created=false
        fi
        return "$failed"
    }
    trap cleanup EXIT

    # Probe the parent directories used by runtime extraction, package state and
    # lifecycle locks. Only unique temporary directories are touched, never APT's
    # actual databases, locks, binaries or configuration. Rename tests atomic writes.
    probe_write() {
        local parent="$1" probe_dir result=0
        probe_dir="$(mktemp -d "$parent/.panel-sandbox.XXXXXXXX")" || return 1
        printf 'runtime write check\n' > "$probe_dir/staged" || result=1
        if [[ "$result" == 0 ]]; then
            mv -- "$probe_dir/staged" "$probe_dir/active" || result=1
        fi
        rm -f -- "$probe_dir/staged" "$probe_dir/active" || result=1
        rmdir -- "$probe_dir" || result=1
        return "$result"
    }
    for parent in /etc /home /opt /usr /var/lib/dpkg /var/lib/apt /var/cache/apt /var/log /run; do
        probe_write "$parent"
        printf 'PASS: create/rename/remove in %s.\n' "$parent"
    done
    # Strict protection must still apply outside the agent's writable directories.
    for parent in /root /boot; do
        if probe_write "$parent" 2>/dev/null; then
            fail "Unexpected write access outside the allow-list: $parent"
        fi
    done

    groupadd --gid "$probe_uid" "$probe_name"
    group_created=true
    useradd --uid "$probe_uid" --gid "$probe_uid" --no-create-home \
        --home-dir "$probe_home" --shell /usr/sbin/nologin "$probe_name"
    user_created=true
    usermod -L "$probe_name"
    install -d -o "$probe_uid" -g "$probe_uid" -m 0700 "$probe_home"
    home_created=true
    printf 'sandbox write check\n' > "$probe_home/.sandbox-check"
    cleanup
    trap - EXIT
    printf 'PASS: runtime paths, protected paths and account lifecycle under agent sandbox.\n'
    exit 0
fi

script_path="$(realpath "${BASH_SOURCE[0]}")"
repo="$(cd "$(dirname "$script_path")/../.." && pwd)"
template="${1:-$repo/templates/systemd/panel-agent.service}"
[[ -r "$template" ]] || fail 'Agent unit template is unreadable.'
getent group panel >/dev/null || fail 'Install the panel in this disposable VM first.'
[[ -d /var/lib/panel && -d /var/log/panel ]] || fail 'Panel state/log directories are required.'
properties=()
while IFS='=' read -r key value; do
    value="${value%$'\r'}"
    case "$key" in
        User|Group|NoNewPrivileges|ProtectSystem|ProtectHome|ReadWritePaths|ReadOnlyPaths|InaccessiblePaths|PrivateTmp|PrivateDevices|MemoryDenyWriteExecute|RestrictAddressFamilies|CapabilityBoundingSet|ProtectKernelModules|ProtectKernelTunables|ProtectControlGroups|LockPersonality)
            properties+=(--property="$key=$value") ;;
    esac
done < "$template"
[[ " ${properties[*]} " == *'--property=ProtectSystem=strict '* ]] || fail 'Strict protection is required.'
probe_name="p_sbx_${RANDOM}${RANDOM}"
probe_uid=59000
while getent passwd "$probe_uid" >/dev/null || getent group "$probe_uid" >/dev/null; do
    probe_uid=$((probe_uid + 1))
    (( probe_uid <= 59999 )) || fail 'No free probe UID/GID in 59000..59999.'
done
systemd-run --wait --collect --pipe --unit="panel-account-sandbox-${RANDOM}" \
    "${properties[@]}" /usr/bin/bash "$script_path" --inside "$probe_name" "$probe_uid"
