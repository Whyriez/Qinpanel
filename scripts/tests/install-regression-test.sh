#!/usr/bin/env bash
# Safe local tests: no agent, real services, firewall changes, or production writes.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$REPO/install.sh"
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }

[[ "$PATH" == /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin ]]
pass 'canonical installer PATH'
[[ "$PANEL_PUBLIC_PORT" == 17890 ]]
pass 'default public port'
for valid in 1024 17890 19090 65535; do
    PANEL_PUBLIC_PORT="$valid"; validate_port; [[ "$PANEL_PUBLIC_PORT" == "$valid" ]]
done
pass 'valid configurable ports'
for invalid in '' 80 443 1023 7700 65536 99999999999999999999 -1 12.3 '17890;id' abc; do
    PANEL_PUBLIC_PORT="$invalid"
    if validate_port >/dev/null 2>&1; then echo "Accepted invalid port: $invalid" >&2; exit 1; fi
done
pass 'invalid, internal and injectable ports rejected'
parse_args --port 19090 --unattended --upgrade
[[ "$PANEL_PUBLIC_PORT" == 19090 && "$PORT_EXPLICIT" == true && "$UPGRADE" == true ]]
pass 'argument forwarding'
if parse_args --port >/dev/null 2>&1; then exit 1; fi
pass 'missing port value rejected'
if parse_args --unknown >/dev/null 2>&1; then exit 1; fi
pass 'unknown option rejected'
restore_digest="$(printf 'a%.0s' {1..64})"
parse_args --restore-digest "$restore_digest"
[[ "$RESTORE_DIGEST" == "$restore_digest" ]]
RESTORE_DIGEST=""
pass 'installer accepts a canonical root-approved migration digest'
if parse_args --restore-digest bad >/dev/null 2>&1; then exit 1; fi
pass 'installer rejects an invalid migration approval digest'
if parse_args --restore-digest >/dev/null 2>&1; then exit 1; fi
pass 'installer rejects a missing migration approval digest'
PANEL_PUBLIC_PORT=17890
config="$(render_panel_site)"
[[ "$config" == *'listen 17890;'* && "$config" == *'listen [::]:17890;'* ]]
[[ "$config" == *'proxy_pass http://127.0.0.1:7700;'* && "$config" == *'proxy_set_header Upgrade $http_upgrade;'* ]]
[[ "$config" == *'client_max_body_size 100m;'* ]]
pass 'IPv4/IPv6 public proxy, bounded upload chunks and WebSocket headers'
[[ "$config" == *'map $remote_addr $qpanel_connector_visitor {'* ]]
[[ "$config" == *'default "";'* && "$config" == *'127.0.0.1 $http_cf_visitor;'* && "$config" == *'::1 $http_cf_visitor;'* ]]
[[ "$config" == *'proxy_set_header Host $qpanel_public_authority;'* ]]
[[ "$config" == *'proxy_set_header X-Forwarded-Proto $scheme;'* ]]
[[ "$config" == *'proxy_set_header X-Qpanel-Cf-Visitor $qpanel_connector_visitor;'* ]]
for header in CF-Visitor X-Forwarded-Host Forwarded; do
    [[ "$config" == *"proxy_set_header $header \"\";"* ]]
done
pass 'proxy preserves public authority and forwards HTTPS offload metadata only from a loopback connector'
PANEL_PUBLIC_PORT=19090
[[ "$(render_panel_site)" == *'listen 19090;'* ]]
pass 'custom port rendering'

# Simulate a conflicting listener; must fail without calling systemctl/kill.
ss() { printf 'LISTEN 0 511 0.0.0.0:19090 *:* users:(("other",pid=42,fd=6))\n'; }
systemctl() { echo 'Unexpected service mutation' >&2; exit 99; }
if check_port_available >/dev/null 2>&1; then exit 1; fi
pass 'occupied public port rejected without stopping services'
ss() { return 0; }
check_port_available
pass 'unused public port accepted'

# Mock health failures. Installer must stop at a failed component.
systemctl() { return 1; }
if validate_health >/dev/null 2>&1; then exit 1; fi
pass 'failed service fails installer health gate'
curl() { return 1; }
sleep() { :; }
if wait_health http://127.0.0.1:7700/healthz >/dev/null 2>&1; then exit 1; fi
pass 'failed HTTP health gate returns nonzero'
curl() { printf '200'; }
wait_health http://127.0.0.1:7700/healthz
pass 'healthy HTTP endpoint accepted'

# Only the public panel port may be added to an existing firewall.
ufw() {
    if [[ "$1" == status ]]; then printf 'Status: active\n'; else printf '%s\n' "$*"; fi
}
SKIP_FIREWALL=false
firewall_output="$(configure_firewall)"
[[ "$firewall_output" == *'allow 19090/tcp'* ]]
[[ "$firewall_output" != *'allow 7700'* && "$firewall_output" != *'allow 80/'* && "$firewall_output" != *'allow 443/'* ]]
pass 'active UFW opens only the configured public port'
SKIP_FIREWALL=true
[[ -z "$(configure_firewall)" ]]
pass 'skip firewall preserves existing rules'

api="$REPO/templates/systemd/panel-api.service"
agent="$REPO/templates/systemd/panel-agent.service"
grep -qx 'User=panel-api' "$api"
grep -qx 'Group=panel' "$api"
grep -qx 'WorkingDirectory=/var/lib/panel' "$api"
grep -qx 'ReadWritePaths=/var/lib/panel' "$api"
grep -qx 'ProtectSystem=strict' "$api"
! grep -q '^RuntimeDirectory=' "$api"
pass 'API service group, working directory, sandbox and runtime lifecycle'
grep -qx 'User=root' "$agent"
grep -qx 'Group=panel' "$agent"
grep -qx 'RuntimeDirectory=panel' "$agent"
grep -qx 'RuntimeDirectoryMode=0750' "$agent"
grep -qx 'RuntimeDirectoryPreserve=yes' "$agent"
grep -qx 'ExecStartPre=/usr/bin/setfacl -m g:panel-tenants:--x,u:www-data:--x -- /run/panel' "$agent"
pass 'agent alone owns boot-time runtime directory'

# Root inside the service is still restricted by its capability bounding set.
# nginx -t must be able to validate HTTP/HTTPS listeners without broadening
# the API identity or disabling the agent's remaining sandbox restrictions.
agent_caps="$(sed -n 's/^CapabilityBoundingSet=//p' "$agent")"
[[ " $agent_caps " == *' CAP_NET_BIND_SERVICE '* ]]
[[ " $agent_caps " == *' CAP_SYS_CHROOT '* ]]
[[ " $agent_caps " != *' CAP_SYS_ADMIN '* && "$agent_caps" != '~'* && -n "$agent_caps" ]]
! grep -q '^AmbientCapabilities=.*CAP_NET_BIND_SERVICE' "$api"
grep -qx 'NoNewPrivileges=true' "$agent"
pass 'root agent may validate privileged nginx listeners with a bounded capability set; API gets no ambient bind capability'
proc_helper="$REPO/templates/systemd/panel-proc-hardening.service"
grep -qx 'Type=oneshot' "$proc_helper"
grep -qx 'User=root' "$proc_helper"
grep -qx 'ExecStart=/usr/bin/mount -o remount,hidepid=2,gid=panel /proc' "$proc_helper"
grep -qx 'CapabilityBoundingSet=CAP_SYS_ADMIN' "$proc_helper"
grep -qx 'NoNewPrivileges=true' "$proc_helper"
! grep -Eq '^(PrivateTmp|PrivateDevices|PrivateMounts|ProtectSystem|ProtectHome|ProtectKernelTunables|ProtectKernelModules|ProtectControlGroups|ReadWritePaths|ReadOnlyPaths|InaccessiblePaths|RootDirectory|RootImage|ProtectProc|ProcSubset)=' "$proc_helper"
! grep -q '^CapabilityBoundingSet=.*CAP_SYS_ADMIN' "$agent"
grep -q 'templates/systemd/panel-proc-hardening.service.* /etc/systemd/system/panel-proc-hardening.service' "$REPO/install.sh"
grep -q '^    /etc/systemd/system/panel-proc-hardening.service$' "$REPO/install.sh"
grep -q 'panel-proc-hardening.service' "$REPO/uninstall.sh"
pass 'host procfs helper has fixed mount command and isolated privilege without agent CAP_SYS_ADMIN'
grep -qx 'ProtectSystem=strict' "$agent"
grep -qx 'ProtectHome=false' "$agent"
grep -qx 'NoNewPrivileges=true' "$agent"
grep -qx 'ReadWritePaths=/etc /home /opt /usr /var /run' "$agent"
pass 'agent permits tenant lifecycle and runtime/package installation while keeping strict default protection'

# Guard the production runtime layout as well as the unit directive. PHP uses the
# host package manager, so allowing only /var/lib/panel cannot support installation.
grep -q 'DEFAULT_NODE_BASE_DIR: &str = "/opt/panel/runtimes/node"' "$REPO/crates/panel-runtime/src/node.rs"
grep -q 'DEFAULT_PYTHON_BASE_DIR: &str = "/opt/panel/runtimes/python"' "$REPO/crates/panel-runtime/src/python.rs"
grep -Fq 'crate::platform::install(runner, &pkg_strs)' "$REPO/crates/panel-agent/src/php.rs"
pass 'Node/Python runtime roots and PHP package installation match the agent write policy'
grep -q 'DEFAULT_DATABASE_PATH.*"/var/lib/panel/panel.db"' "$REPO/crates/panel-core/src/production.rs"
grep -q 'DEFAULT_DATABASE_PATH' "$REPO/crates/panel-api/src/config.rs"
grep -q 'DEFAULT_DATABASE_PATH' "$REPO/crates/panel-cli/src/main.rs"
pass 'API and CLI share an absolute production database default'
grep -q 'for path in panel.db panel.db-wal panel.db-shm panel.operations.db panel.operations.db-wal panel.operations.db-shm;' "$REPO/install.sh"
pass 'installer rollback includes operational database and its WAL/SHM together with core state'
grep -q 'runner: ubuntu-22.04$' "$REPO/.github/workflows/release.yml"
grep -q 'runner: ubuntu-22.04-arm$' "$REPO/.github/workflows/release.yml"
grep -q 'scripts/check-glibc.sh' "$REPO/.github/workflows/release.yml"
grep -q 'scripts/package-release.sh' "$REPO/scripts/build-release.sh"
grep -q 'templates/nginx/nusa-panel.conf' "$REPO/scripts/package-release.sh"
pass 'release baseline gate and proxy template packaging'

# Exercise the actual failure trap in an isolated child with every host mutation mocked.
for failing in validate_health install_nginx_proxy; do
    status=0
    output="$(bash -c '
        source "$1/install.sh"
        PORT_EXPLICIT=true
        check_platform() { :; }; find_release() { :; }; install_packages() { :; }
        check_port_available() { :; }; install() { :; }
        begin_transaction() { TRANSACTION=true; }
        setup_identity() { :; }; install_binaries_and_units() { :; }
        acquire_install_lock() { :; }; validate_upgrade() { :; }; record_installed_release() { :; }
        initialize_database() { ADMIN_USERNAME=fixture_admin; ADMIN_PASSWORD=fixture_secret_never_print; }; install_nginx_proxy() { :; }
        systemctl() { :; }; wait_agent_ready() { :; }; validate_health() { :; }; configure_firewall() { :; }
        wait_health() { :; }; validate_api_binding() { :; }; activate_nginx_proxy() { :; }
        rollback() { echo ROLLBACK_CALLED; }
        if [[ "$2" == validate_health ]]; then
            validate_health() { return 1; }
        else
            install_nginx_proxy() { return 1; }
        fi
        main --unattended
    ' test "$REPO" "$failing" 2>&1)" || status=$?
    [[ "$status" != 0 && "$output" == *ROLLBACK_CALLED* && "$output" != *'Installation complete'* && "$output" != *fixture_secret_never_print* ]]
    pass "$failing failure invokes rollback and suppresses success"
done

# Exercise file rollback with a temporary rootless fixture and mocked service manager.
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
original="$fixture/panel.conf"
new="$fixture/new-unit.service"
tenant="$fixture/tenant.conf"
printf 'old panel\n' > "$original"
printf 'tenant unchanged\n' > "$tenant"
BACKUP_DIR="$fixture/backup"
mkdir -p "$BACKUP_DIR/files"
mkdir -p "$BACKUP_DIR/files${original%/*}"
cp -a -- "$original" "$BACKUP_DIR/files$original"
printf 'broken candidate\n' > "$original"
printf 'new file\n' > "$new"
MANAGED_FILES=("$original" "$new")
STATE_SAVED=false
systemctl() { return 0; }
rollback >/dev/null 2>&1
[[ "$(cat "$original")" == 'old panel' && ! -e "$new" && "$(cat "$tenant")" == 'tenant unchanged' ]]
pass 'rollback restores previous artifacts, removes new artifacts and preserves tenant fixture'

# The release gate examines required symbol versions, without executing any binary.
for version in GLIBC_2.34 GLIBC_2.35 GLIBC_2.39 none; do
    status=0
    output="$(bash -c '
        FAKE_GLIBC="$2"; export FAKE_GLIBC
        readelf() { printf "%s\n" "$FAKE_GLIBC"; }; export -f readelf
        bash "$1/scripts/check-glibc.sh" fixture-binary
    ' test "$REPO" "$version" 2>&1)" || status=$?
    if [[ "$version" == GLIBC_2.34 ]]; then [[ "$status" == 0 ]]; else [[ "$status" != 0 ]]; fi
    pass "GLIBC release gate: $version"
done
printf '%s regression checks passed. VM acceptance was not run.\n' "$count"
bash "$REPO/scripts/tests/install-startup-race-test.sh"
bash "$REPO/scripts/tests/installer-platform-test.sh"
bash "$REPO/scripts/tests/platform-bootstrap-test.sh"
bash "$REPO/scripts/tests/install-snapshot-test.sh"
bash "$REPO/scripts/tests/release-build-test.sh"
bash "$REPO/scripts/tests/nginx-unknown-host-test.sh"
bash "$REPO/scripts/tests/admin-bootstrap-installer-test.sh"
bash "$REPO/scripts/tests/installer-presentation-test.sh"
bash "$REPO/scripts/tests/installer-branding-output-test.sh"
bash "$REPO/scripts/tests/runtime-autostart-test.sh"
bash "$REPO/scripts/tests/panel-update-test.sh"

bash "$REPO/scripts/tests/qonpanel-brand-test.sh"
bash "$REPO/scripts/tests/distribution-bootstrap-test.sh"
