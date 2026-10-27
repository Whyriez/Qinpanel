#!/usr/bin/env bash
# Rootless filesystem fixtures: nginx/systemd/dpkg are mocked; no listeners or host writes.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$REPO/install.sh"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/qinpanel-host-guard-test.XXXXXXXX")"
fixture="$(cd "$fixture" && pwd -P)"
[[ "$fixture" == /*/qinpanel-host-guard-test.* && "$fixture" != / ]] || exit 99
trap 'rm -rf -- "$fixture"' EXIT
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }
systemctl() { echo 'Unexpected service mutation' >&2; exit 99; }
stock='server { listen 80 default_server; server_name _; root /var/www/html; }'
stock_digest="$(printf '%s\n' "$stock" | md5sum)"; stock_digest="${stock_digest%% *}"
printf 'symlink probe\n' > "$fixture/probe-source"
ln -s "$fixture/probe-source" "$fixture/probe-link"
symlinks_supported=false
if [[ -L "$fixture/probe-link" ]]; then symlinks_supported=true
else printf 'NOTE native symlinks unavailable; Debian link cases run in Linux CI (Rust mocks cover them locally).\n'; fi
rm -f "$fixture/probe-link"
default_link_preserved() { [[ "$symlinks_supported" == false || -L "$nginx_root/sites-enabled/default" ]]; }
reset_fixture() {
    nginx_root="$fixture/$1/nginx"
    mkdir -p "$nginx_root/sites-available" "$nginx_root/sites-enabled" "$nginx_root/conf.d"
    printf 'events {}\nhttp {\n include %s/conf.d/*.conf;\n include %s/sites-enabled/*;\n}\n' "$nginx_root" "$nginx_root" > "$nginx_root/nginx.conf"
    printf '%s\n' "$stock" > "$nginx_root/sites-available/default"
    printf '%s\n' "$stock" > "$nginx_root/conf.d/default.conf"
    if [[ "$symlinks_supported" == true ]]; then ln -s ../sites-available/default "$nginx_root/sites-enabled/default"; fi
    printf 'server { listen 80; server_name customer.example; }\n' > "$nginx_root/conf.d/customer.conf"
    checks=0; fail_syntax=false; missing_metadata=false
}
dpkg-query() {
    [[ "$1" == -W && "$2" == '-f=${Conffiles}\n' ]] || return 99
    [[ "$missing_metadata" == false ]] || return 1
    printf ' %s %s\n' "$nginx_root/sites-available/default" "$stock_digest"
    printf ' %s %s\n' "$nginx_root/conf.d/default.conf" "$stock_digest"
}
nginx() {
    checks=$((checks + 1))
    if [[ "$*" == '-t -c '* ]]; then
        [[ "$3" == "$STAGING_DIR/nginx.conf" ]]
        grep -Fq "include $STAGING_DIR/conf.d/*.conf;" "$3"
        grep -Fq "include $STAGING_DIR/sites-enabled/*;" "$3"
        [[ ! -e "$STAGING_DIR/conf.d/default.conf" && ! -L "$STAGING_DIR/sites-enabled/default" ]]
        cmp -s "$REPO/templates/nginx/nusa-reject-unknown.conf" "$STAGING_DIR/conf.d/00-qpanel-reject-unknown.conf"
        [[ "$fail_syntax" == false ]] || return 1
    else
        [[ "$*" == -t ]]
        [[ ! -e "$nginx_root/conf.d/default.conf" && ! -L "$nginx_root/sites-enabled/default" ]]
        cmp -s "$REPO/templates/nginx/nusa-reject-unknown.conf" "$nginx_root/conf.d/00-qpanel-reject-unknown.conf"
    fi
}

reset_fixture stock
cp "$nginx_root/nginx.conf" "$fixture/main.original"
install_nginx_proxy_at "$nginx_root" "$fixture/public-port"
[[ "$checks" == 2 && "$(cat "$fixture/public-port")" == 17890 ]]
cmp -s "$nginx_root/nginx.conf" "$fixture/main.original"
[[ "$(cat "$nginx_root/sites-available/default")" == "$stock" ]]
grep -Fq customer.example "$nginx_root/conf.d/customer.conf"
if [[ "$symlinks_supported" == true ]]; then
    compgen -G "$nginx_root/sites-available/default.qpanel-disabled-*" >/dev/null
    [[ -L "$nginx_root/sites-enabled/nusa-panel.conf" ]]
fi
compgen -G "$nginx_root/conf.d/default.conf.qpanel-disabled-*" >/dev/null
pass 'install guard before first website, quarantine verified defaults, preserve main/custom configs'
install_nginx_proxy_at "$nginx_root" "$fixture/public-port"
[[ "$checks" == 4 ]]
pass 'upgrade is idempotent and validates staged plus live configuration'
generation=0
for brand in Qpanel 'Nusa Panel'; do
    generation=$((generation + 1))
    sed "1s/Qinpanel/$brand/" "$REPO/templates/nginx/nusa-reject-unknown.conf" > "$nginx_root/conf.d/00-qpanel-reject-unknown.conf"
    install_nginx_proxy_at "$nginx_root" "$fixture/public-port"
    cmp -s "$REPO/templates/nginx/nusa-reject-unknown.conf" "$nginx_root/conf.d/00-qpanel-reject-unknown.conf"
    pass "historical guard generation $generation upgrades to Qinpanel without losing ownership"
done
checks=4


# A binary-only update leaves this legacy proxy behind. Full upgrade must
# replace it with the trusted Tunnel metadata contract, retaining a custom port.
printf 'server { listen 19090; location / { proxy_set_header Host $host; proxy_set_header X-Forwarded-Proto $scheme; } }\n' > "$nginx_root/sites-available/nusa-panel.conf"
PANEL_PUBLIC_PORT=19090
install_nginx_proxy_at "$nginx_root" "$fixture/public-port"
proxy="$nginx_root/sites-available/nusa-panel.conf"
[[ "$checks" == 6 && "$(cat "$fixture/public-port")" == 19090 ]]
grep -Fq 'listen 19090;' "$proxy"
grep -Fq 'proxy_set_header Host $qpanel_public_authority;' "$proxy"
grep -Fq 'proxy_set_header X-Qpanel-Cf-Visitor $qpanel_connector_visitor;' "$proxy"
grep -Fq 'proxy_set_header CF-Visitor "";' "$proxy"
! grep -Fq 'proxy_set_header Host $host;' "$proxy"
pass 'upgrade replaces legacy origin proxy metadata while retaining the chosen port'
PANEL_PUBLIC_PORT=17890

changed_files=(conf.d/default.conf)
if [[ "$symlinks_supported" == true ]]; then changed_files+=(sites-available/default); fi
for changed in "${changed_files[@]}"; do
    reset_fixture "modified-${changed//\//-}"
    printf 'operator custom configuration\n' > "$nginx_root/$changed"
    if install_nginx_proxy_at "$nginx_root" "$fixture/public-port" >/dev/null 2>&1; then exit 1; fi
    [[ "$checks" == 0 && -e "$nginx_root/conf.d/default.conf" ]]
    default_link_preserved
    [[ ! -e "$nginx_root/conf.d/00-qpanel-reject-unknown.conf" ]]
    grep -Fq 'operator custom' "$nginx_root/$changed"
    pass "modified package config preserved: $changed"
done
reset_fixture no-metadata
missing_metadata=true
if install_nginx_proxy_at "$nginx_root" "$fixture/public-port" >/dev/null 2>&1; then exit 1; fi
[[ "$checks" == 0 ]]; default_link_preserved
pass 'missing package checksum fails closed without modifying stock files'
reset_fixture custom-guard
printf 'operator guard\n' > "$nginx_root/conf.d/00-qpanel-reject-unknown.conf"
if install_nginx_proxy_at "$nginx_root" "$fixture/public-port" >/dev/null 2>&1; then exit 1; fi
[[ "$(cat "$nginx_root/conf.d/00-qpanel-reject-unknown.conf")" == 'operator guard' && "$checks" == 0 ]]
pass 'custom guard is preserved'
reset_fixture commented-include
sed -i 's| include .*conf.d/| # include /commented/conf.d/|' "$nginx_root/nginx.conf"
if install_nginx_proxy_at "$nginx_root" "$fixture/public-port" >/dev/null 2>&1; then exit 1; fi
[[ "$checks" == 0 ]]; default_link_preserved
pass 'commented include cannot masquerade as an active guard integration'
reset_fixture invalid-syntax
fail_syntax=true
if install_nginx_proxy_at "$nginx_root" "$fixture/public-port" >/dev/null 2>&1; then exit 1; fi
[[ "$checks" == 1 && -e "$nginx_root/conf.d/default.conf" ]]; default_link_preserved
[[ ! -e "$nginx_root/conf.d/00-qpanel-reject-unknown.conf" ]]
pass 'failed candidate syntax never replaces live configuration'
printf '%s unknown-host installer checks passed; no live Nginx was started.\n' "$count"
