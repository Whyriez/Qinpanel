#!/usr/bin/env bash
# Rootless snapshots/rollback with a Coreutils 9.1 regression model.
# All service calls and host-state copies are mocked. Only this fixture is writable.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$REPO/install.sh"
case "$(uname -s)" in
    MINGW*|MSYS*) export MSYS="${MSYS:+$MSYS }winsymlinks:sys" ;;
esac
fixture="$(mktemp -d "${TMPDIR:-/tmp}/qpanel-snapshot-test.XXXXXXXX")"
fixture="$(cd "$fixture" && pwd -P)"
[[ "$fixture" == /*/qpanel-snapshot-test.* && "$fixture" != / ]] || exit 99
trap 'rm -rf -- "$fixture"' EXIT
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }

# Intercept every systemd call, including read-only probes. A fresh host has no units.
systemctl() { printf 'systemctl:%s\n' "$*" >> "$trace"; return 4; }
mktemp() {
    [[ "$*" == '-d /var/backups/panel/install-XXXXXXXX' ]] || return 99
    printf '%s\n' "$snapshot"
}
cp() {
    local source destination argument
    source="${@: -2:1}"; destination="${@: -1}"
    # Optional host-state branches are simulated, never copied from this machine.
    case "$source" in /etc/panel|/var/lib/panel) return 0 ;; esac
    [[ "$source" == "$fixture/"* && "$destination" == "$fixture/"* ]] || return 99
    printf 'cp:%s:%s\n' "$source" "$destination" >> "$trace"
    for argument in "$@"; do
        if [[ "$argument" == --parents && "$source" == /* ]]; then
            printf "cp: 'etc/nginx/sites-enabled': No such file or directory (Coreutils 9.1 model)\n" >&2
            return 1
        fi
    done
    [[ "$source" != "${copy_failure:-}" ]] || return 74
    command cp "$@"
}
mkdir() {
    local argument
    for argument in "$@"; do
        [[ "$argument" == -* || "$argument" == "$fixture/"* ]] || return 99
        [[ "$argument" != "${mkdir_failure:-}" ]] || return 73
    done
    command mkdir "$@"
}
chmod() {
    [[ $# == 2 && "$2" == "$fixture/"* ]] || return 99
    printf 'chmod:%s:%s\n' "$1" "$2" >> "$trace"
    command chmod "$@"
}
reset_fixture() {
    root="$fixture/$1"
    snapshot="$root/snapshot"
    trace="$root/trace"
    command mkdir -p "$snapshot" "$root/origin/etc/nginx/sites-available" \
        "$root/origin/etc/nginx/sites-enabled" "$root/origin/usr/local/bin"
    : > "$trace"
    TRANSACTION=false; STATE_SAVED=false; STATE_CHECKED=false
    API_WAS_ACTIVE=false; AGENT_WAS_ACTIVE=false; NGINX_WAS_ACTIVE=false
    API_WAS_ENABLED=false; AGENT_WAS_ENABLED=false; NGINX_WAS_ENABLED=false
    copy_failure=""; mkdir_failure=""
    available="$root/origin/etc/nginx/sites-available/panel.conf"
    enabled="$root/origin/etc/nginx/sites-enabled/default"
    missing="$root/origin/etc/nginx/sites-enabled/absent.conf"
    printf 'original config\n' > "$available"
    printf 'original enabled\n' > "$enabled"
    printf 'fixture binary\n' > "$root/origin/usr/local/bin/panel-api"
    chmod 0640 "$available"
    MANAGED_FILES=("$root/origin/usr/local/bin/panel-api" "$available" "$enabled" "$missing")
}

reset_fixture bookworm
# Execute the actual begin_transaction from an unrelated working directory.
(cd "$root/origin/usr/local/bin"; begin_transaction
    [[ "$TRANSACTION" == true && "$STATE_CHECKED" == true ]])
for path in "${MANAGED_FILES[@]:0:3}"; do
    cmp -s "$path" "$snapshot/files$path"
done
[[ ! -e "$snapshot/files$missing" ]]
pass 'absolute managed files snapshot successfully with Coreutils 9.1 behavior'
[[ "$(stat -c '%a' "$available")" == "$(stat -c '%a' "$snapshot/files$available")" ]]
pass 'snapshot preserves original file mode and contents'
grep -Fxq "chmod:0700:$snapshot" "$trace"
if [[ "$(uname -s)" == Linux ]]; then [[ "$(stat -c '%a' "$snapshot")" == 700 ]]; fi
pass 'snapshot requests owner-only mode 0700 (Unix enforcement checked on Linux)'

reset_fixture 'spaces in path'
begin_transaction
cmp -s "$available" "$snapshot/files$available"
pass 'snapshot handles absolute paths containing spaces'

reset_fixture copy-failure
copy_failure="$available"
status=0
# Use a child with errexit, as in the real installer; no conditional suppresses it.
output="$(export -f cp mkdir mktemp systemctl chmod
    export fixture trace snapshot copy_failure mkdir_failure
    bash -c 'source "$1/install.sh"; shift; MANAGED_FILES=("$@"); begin_transaction; echo UNEXPECTED_CONTINUATION' \
        test "$REPO" "${MANAGED_FILES[@]}" 2>&1)" || status=$?
[[ "$status" != 0 && "$output" != *UNEXPECTED_CONTINUATION* ]]
! grep -q '^systemctl:' "$trace"
pass 'snapshot copy failure aborts before inspecting or stopping services'

reset_fixture mkdir-failure
mkdir_failure="$snapshot/files${available%/*}"
status=0
output="$(export -f cp mkdir mktemp systemctl chmod
    export fixture trace snapshot copy_failure mkdir_failure
    bash -c 'source "$1/install.sh"; shift; MANAGED_FILES=("$@"); begin_transaction; echo UNEXPECTED_CONTINUATION' \
        test "$REPO" "${MANAGED_FILES[@]}" 2>&1)" || status=$?
[[ "$status" != 0 && "$output" != *UNEXPECTED_CONTINUATION* ]]
! grep -q '^systemctl:' "$trace"
pass 'snapshot directory failure aborts before services are stopped'

reset_fixture rollback
begin_transaction
STATE_CHECKED=false # State database I/O is out of scope; never touch /var/lib/panel.
printf 'changed config\n' > "$available"
printf 'new managed file\n' > "$missing"
systemctl() { printf 'systemctl:%s\n' "$*" >> "$trace"; return 0; }
rollback >/dev/null 2>&1
[[ "$(cat "$available")" == 'original config' && ! -e "$missing" ]]
pass 'rollback reads the unchanged snapshot layout and removes only new managed files'

# Git Bash can represent symlinks through MSYS without Windows admin privileges.
reset_fixture symlinks
command rm -f "$enabled"
MSYS=winsymlinks:sys ln -s ../sites-available/panel.conf "$enabled"
if [[ -L "$enabled" ]]; then
    systemctl() { printf 'systemctl:%s\n' "$*" >> "$trace"; return 4; }
    begin_transaction
    [[ -L "$snapshot/files$enabled" ]]
    [[ "$(readlink "$snapshot/files$enabled")" == ../sites-available/panel.conf ]]
    pass 'snapshot preserves relative Nginx symlink without dereferencing'

    STATE_CHECKED=false
    command rm -f "$enabled"
    printf 'replacement config\n' > "$enabled"
    systemctl() { return 0; }
    rollback >/dev/null 2>&1
    [[ -L "$enabled" && "$(readlink "$enabled")" == ../sites-available/panel.conf ]]
    pass 'rollback restores the original relative symlink'

    reset_fixture dangling
    command rm -f "$enabled"
    MSYS=winsymlinks:sys ln -s ../sites-available/not-yet-created.conf "$enabled"
    begin_transaction
    [[ -L "$snapshot/files$enabled" && ! -e "$snapshot/files$enabled" ]]
    [[ "$(readlink "$snapshot/files$enabled")" == ../sites-available/not-yet-created.conf ]]
    pass 'snapshot includes dangling symlinks without requiring target contents'
else
    printf 'NOTE symlink fixtures unavailable; run these branches in Linux CI.\n'
fi
printf '%s snapshot checks passed (rootless fixture, no host services).\n' "$count"
