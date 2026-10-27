#!/usr/bin/env bash
# Presentation-only fixtures: source functions, never execute the installer.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$REPO/install.sh"
[[ "$LC_ALL" == C ]]
fixture="$(mktemp -d)"
trap 'rm -f -- "$fixture/plain" "$fixture/upgrade" "$fixture/style" "$fixture/narrow" "$fixture/help"; rmdir -- "$fixture"' EXIT
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }
username=admin_123456abcdef
password=Qp!1234567890abcdef1234567890abcdef
PANEL_PUBLIC_PORT=19090
RELEASE_VERSION=v0.1.0
BACKUP_DIR=/fixture/install-recovery

read_bootstrap_result "created $username $password"
TERM=xterm
unset NO_COLOR
print_installation_summary 192.168.3.103 > "$fixture/plain"
grep -Fq 'Qinpanel | Installation complete' "$fixture/plain"
grep -Fq 'http://192.168.3.103:19090' "$fixture/plain"
grep -Fq 'v0.1.0' "$fixture/plain"
grep -Fq 'Passed: API, agent, Nginx and SQLite' "$fixture/plain"
grep -Fq 'public access is not verified' "$fixture/plain"
grep -Fq 'not a full website backup' "$fixture/plain"
pass 'summary distinguishes local checks, access URL and installer recovery copy'
[[ "$(grep -oF "$password" "$fixture/plain" | wc -l)" == 1 ]]
[[ -z "$ADMIN_USERNAME" && -z "$ADMIN_PASSWORD" ]]
grep -Fq 'shown only once' "$fixture/plain"
pass 'fresh password appears once and credential variables are cleared'
for text in '/profile' '2FA' 'panelctl --help' 'panelctl admin --help' \
    'sudo panelctl admin list' 'sudo panelctl doctor' 'panelctl backup --help' \
    'reset-password --username USERNAME' 'Recovery help: panelctl admin reset-password --help'; do
    grep -Fq "$text" "$fixture/plain"
done
pass 'next steps and complete CLI recovery reference use English'
! grep -Fq $'\033' "$fixture/plain"
pass 'redirected output has no ANSI escapes even with an xterm environment'

UPGRADE=true
read_bootstrap_result preserved
print_installation_summary '[2001:db8::1]' > "$fixture/upgrade"
grep -Fq 'Qinpanel | Upgrade complete' "$fixture/upgrade"
grep -Fq 'http://[2001:db8::1]:19090' "$fixture/upgrade"
grep -Fq 'sign-in details are unchanged' "$fixture/upgrade"
grep -Fq 'No new password was generated.' "$fixture/upgrade"
! grep -Eq '^  Password[[:space:]]+:' "$fixture/upgrade"
! grep -Fq "$password" "$fixture/upgrade"
pass 'upgrade preserves existing accounts and a correctly bracketed IPv6 URL'

# A mock only selects the renderer's interactive branch; no PTY is required.
install_terminal_is_tty() { return 0; }
TERM=xterm
unset NO_COLOR
setup_installation_style
print_installation_banner > "$fixture/style"
grep -Fq $'\033[36m' "$fixture/style"
grep -Fq $'\033[0m' "$fixture/style"
pass 'interactive renderer uses restrained accent color and resets formatting'
for no_color_value in '' 1; do
    NO_COLOR="$no_color_value"
    setup_installation_style
    print_installation_banner > "$fixture/style"
    ! grep -Fq $'\033' "$fixture/style"
done
pass 'NO_COLOR disables ANSI even when the variable is set to an empty string'
unset NO_COLOR
TERM=dumb
setup_installation_style
print_installation_banner > "$fixture/style"
! grep -Fq $'\033' "$fixture/style"
pass 'TERM=dumb disables styling after a colored render'

COLUMNS=40
print_installation_rule > "$fixture/narrow"
[[ "$(head -n 1 "$fixture/narrow" | wc -c)" -le 40 ]]
print_summary_field 'Password' "$password" >> "$fixture/narrow"
grep -Fq "$password" "$fixture/narrow"
print_summary_field 'Literal' '100% \path $(false)' >> "$fixture/narrow"
grep -Fq '100% \path $(false)' "$fixture/narrow"
pass 'narrow separators fit while full values and literal shell characters remain intact'
for width in bad 0 000 999999999999999999999999999; do
    COLUMNS="$width"; print_installation_rule > "$fixture/narrow"
    [[ -s "$fixture/narrow" ]]
done
pass 'invalid terminal widths fall back without arithmetic errors'
help_status=0
parse_args --help > "$fixture/help" || help_status=$?
[[ "$help_status" == 2 ]]
grep -Fq 'Usage: ./install.sh [options]' "$fixture/help"
grep -Fq 'Upgrades keep existing accounts.' "$fixture/help"
if parse_args --port > "$fixture/help" 2>&1; then exit 1; fi
grep -Fq -- '--port requires a value' "$fixture/help"
pass 'help and input errors use English without invoking installation'
! grep -Eq 'Instalasi|Penggunaan|Gagal|dipertahankan|PERINGATAN|Langkah berikutnya|Bantuan CLI' "$REPO/install.sh"
pass 'installer presentation does not retain the previous Indonesian messages'
(
    source "$REPO/scripts/bootstrap-install.sh"
    TERM=xterm; NO_COLOR=''; COLUMNS=40
    setup_colors
    print_bootstrap_banner
    log_step '[1/7] Checking environment...'
    log_sub 'Literal % \t $(false)'
) > "$fixture/style"
! grep -Fq $'\033' "$fixture/style"
grep -Fq 'Literal % \t $(false)' "$fixture/style"
[[ "$(grep '^  --' "$fixture/style" | wc -L)" -le 40 ]]
pass 'bootstrap matches the terminal style and prints literal messages without ANSI in logs'
printf '%s installer presentation checks passed (no services, network or host writes).\n' "$count"
