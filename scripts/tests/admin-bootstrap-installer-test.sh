#!/usr/bin/env bash
# Rootless installer contract: all production mutations are mocked.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$REPO/install.sh"
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }
fixture="$(mktemp -d)"
trap 'rm -f "$fixture/output" "$fixture/trace"; rmdir "$fixture"' EXIT
username=admin_123456abcdef
password=Qp!1234567890abcdef1234567890abcdef

read_bootstrap_result "created $username $password"
[[ "$ADMIN_USERNAME" == "$username" && "$ADMIN_PASSWORD" == "$password" ]]
pass 'installer accepts the bounded generated credential protocol'
PANEL_PUBLIC_PORT=19090
BACKUP_DIR=/fixture/snapshot
print_installation_summary 203.0.113.10 > "$fixture/output"
output="$(cat "$fixture/output")"
[[ "$output" == *'Qinpanel | Installation complete'* && "$output" == *'http://203.0.113.10:19090'* ]]
grep -Eq "^  Username[[:space:]]+: $username$" "$fixture/output"
grep -Fq "$password" "$fixture/output"
[[ "$output" == *'panelctl --help'* && "$output" == *'panelctl admin --help'* && "$output" == *'reset-password --username USERNAME'* ]]
[[ "$output" != *'Token pengaturan'* && -z "$ADMIN_USERNAME" && -z "$ADMIN_PASSWORD" ]]
pass 'successful summary shows credentials once, URL, CLI help, then clears secrets'
read_bootstrap_result preserved
UPGRADE=true
print_installation_summary '[2001:db8::1]' > "$fixture/output"
output="$(cat "$fixture/output")"
[[ "$output" == *'Qinpanel | Upgrade complete'* && "$output" == *'http://[2001:db8::1]:19090'* && "$output" == *'sign-in details are unchanged'* ]]
! grep -Eq '^  Password[[:space:]]+:' "$fixture/output"
[[ "$output" != *"$password"* ]]
pass 'upgrade summary preserves accounts and never prints a password'
UPGRADE=false

for bad in '' created "created unsafe $password" "created $username weak" "created $username $password extra" $'created admin_123456abcdef secret\nmore' '$(false)' 'preserved extra'; do
    if read_bootstrap_result "$bad" >/dev/null 2>&1; then exit 1; fi
    [[ -z "$ADMIN_USERNAME" && -z "$ADMIN_PASSWORD" ]]
done
pass 'malformed CLI output is rejected without eval or secret retention'

runuser() {
    [[ "$*" == '-u panel-api -- /usr/local/bin/panelctl --db-path /var/lib/panel/panel.db admin bootstrap --installer-output' ]] || return 99
    printf '%s\n' "$*" >> "$fixture/trace"
    printf 'created %s %s\n' "$username" "$password"
}
chown() { [[ "$*" == '-R panel-api:panel /var/lib/panel' ]]; }
chmod() { [[ "$1" == 0640 ]]; }
initialize_database > "$fixture/output"
[[ ! -s "$fixture/output" && "$ADMIN_USERNAME" == "$username" && "$ADMIN_PASSWORD" == "$password" ]]
[[ "$(wc -l < "$fixture/trace")" == 1 ]]
pass 'database initialization uses only service-identity bootstrap and defers secret output'

runuser() { return 1; }
if initialize_database > "$fixture/output" 2>&1; then exit 1; fi
! grep -Fq "$password" "$fixture/output"
pass 'failed database bootstrap never prints captured credentials'

printf '%s admin installer checks passed (no services or production writes).\n' "$count"
