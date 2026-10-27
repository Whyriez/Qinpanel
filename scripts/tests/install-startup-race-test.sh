#!/usr/bin/env bash
# Safe local regression tests: fake clock, mock HTTP/ss/systemd, no host services.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if [[ "${1:-}" == --scenario ]]; then
    scenario="$2"
    trace="$3"
    source "$REPO/install.sh"
    record() { printf '%s\n' "$*" >> "$trace"; }
    deny() { echo 'Unexpected host command' >&2; exit 99; }
    # Every path that could mutate the host is stubbed before main is invoked.
    runuser() { deny; }; stat() { deny; }; chown() { deny; }; chmod() { deny; }
    check_platform() { :; }; find_release() { :; }; install_packages() { :; }
    check_port_available() { :; }; install() { :; }
    begin_transaction() { TRANSACTION=true; }
    setup_identity() { :; }; install_binaries_and_units() { :; }
    acquire_install_lock() { :; }; validate_upgrade() { :; }; record_installed_release() { :; }
    initialize_database() { :; }; install_nginx_proxy() { record proxy:staged; }
    wait_agent_ready() { record agent:ready; }
    configure_firewall() { record firewall; }
    rollback() { record rollback; echo ROLLBACK_CALLED; }
    hostname() { printf '192.0.2.10\n'; }
    # The unchanged final socket/SQLite checks are outside this startup fixture.
    # Keep the real public health gate; do not execute CLI or use real paths.
    validate_health() { record final:validation; wait_health "http://127.0.0.1:${PANEL_PUBLIC_PORT}/healthz"; }

    PORT_EXPLICIT=true
    PANEL_PUBLIC_PORT=19090
    clock_ms=0
    api_started=false
    proxy_ready=false
    case "$scenario" in existing-nginx|nginx-reload-failure) proxy_ready=true ;; esac
    sleep() {
        [[ "$1" == 0.2 ]] || deny
        clock_ms=$((clock_ms + 200))
        record "poll:$clock_ms"
    }
    systemctl() {
        record "systemctl:$*"
        case "$1" in
            enable) return 0 ;;
            start|reload)
                case "$2" in
                    panel-agent.service) return 0 ;;
                    panel-api.service) api_started=true ;;
                    nginx.service)
                        case "$scenario" in nginx-start-failure|nginx-reload-failure) return 1 ;; esac
                        proxy_ready=true ;;
                    *) deny ;;
                esac ;;
            is-active) [[ "$proxy_ready" == true ]] ;;
            *) deny ;;
        esac
    }
    curl() {
        local url="${!#}"
        [[ "$api_started" == true ]] || deny
        [[ " $* " == *' --max-time 2 '* && " $* " == *' --connect-timeout 1 '* ]] || deny
        case "$url" in
            http://127.0.0.1:7700/healthz)
                record health:internal
                case "$scenario" in
                    delayed)
                        if (( clock_ms < 500 )); then echo 'curl: Connection refused' >&2; return 7; fi ;;
                    sqlite-slow)
                        if (( clock_ms < 2000 )); then echo 'curl: Connection refused' >&2; return 7; fi ;;
                    timeout) echo 'curl: Connection refused' >&2; return 7 ;;
                    redirect) printf '302'; return 0 ;;
                esac
                record ready:internal
                printf '200' ;;
            http://127.0.0.1:19090/healthz)
                [[ "$proxy_ready" == true ]] || deny
                record health:public
                [[ "$scenario" != proxy-timeout ]] || return 7
                printf '200' ;;
            *) deny ;;
        esac
    }
    ss() {
        record binding:checked
        grep -qx ready:internal "$trace" || deny
        [[ "$*" == '-H -lnt sport = :7700' ]] || deny
        case "$scenario" in
            no-listener) return 0 ;;
            ss-error) return 2 ;;
            unsafe-v4) printf 'LISTEN 0 511 0.0.0.0:7700 0.0.0.0:*\n' ;;
            unsafe-star) printf 'LISTEN 0 511 *:7700 *:*\n' ;;
            unsafe-v6) printf 'LISTEN 0 511 [::]:7700 [::]:*\n' ;;
            unsafe-specific) printf 'LISTEN 0 511 192.0.2.10:7700 0.0.0.0:*\n' ;;
            mixed)
                printf 'LISTEN 0 511 127.0.0.1:7700 0.0.0.0:*\n'
                printf 'LISTEN 0 511 [::]:7700 [::]:*\n' ;;
            duplicate)
                printf 'LISTEN 0 511 127.0.0.1:7700 0.0.0.0:*\n\n'
                printf '  LISTEN   0   511   127.0.0.1:7700   0.0.0.0:*\n' ;;
            *) printf 'LISTEN 0 511 127.0.0.1:7700 0.0.0.0:*\n' ;;
        esac
    }
    nginx() {
        [[ "$*" == -t ]] || deny
        grep -qx ready:internal "$trace" || deny
        grep -qx binding:checked "$trace" || deny
        record nginx:validated
        [[ "$scenario" != nginx-invalid ]]
    }
    # Independent shell invocation preserves errexit/ERR trap semantics.
    main --unattended
    exit 0
fi

fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }

for scenario in delayed sqlite-slow loopback duplicate existing-nginx timeout redirect \
    no-listener ss-error unsafe-v4 unsafe-star unsafe-v6 unsafe-specific mixed proxy-timeout \
    nginx-invalid nginx-start-failure nginx-reload-failure; do
    trace="$fixture/$scenario.trace"
    : > "$trace"
    status=0
    output="$(bash "$0" --scenario "$scenario" "$trace" 2>&1)" || status=$?
    [[ "$output" != *'curl: Connection refused'* ]]
    case "$scenario" in
        delayed|sqlite-slow|loopback|duplicate|existing-nginx)
            [[ "$status" == 0 && "$output" == *'Installation complete'* ]]
            ! grep -qx rollback "$trace"
            # Exactly one binding probe, after internal readiness and before proxy activation.
            [[ "$(grep -c '^binding:checked$' "$trace")" == 1 ]]
            [[ "$(grep -c '^health:public$' "$trace")" == 1 ]]
            case "$scenario" in
                delayed) grep -qx poll:600 "$trace" ;;
                sqlite-slow) grep -qx poll:2000 "$trace" ;;
                loopback|duplicate|existing-nginx) ! grep -q '^poll:' "$trace" ;;
            esac
            if [[ "$scenario" == existing-nginx ]]; then
                grep -qx 'systemctl:reload nginx.service' "$trace"
                ! grep -qx 'systemctl:start nginx.service' "$trace"
            else
                grep -qx 'systemctl:start nginx.service' "$trace"
            fi
            ;;
        *)
            [[ "$status" != 0 && "$output" == *ROLLBACK_CALLED* && "$output" != *'Installation complete'* ]]
            [[ "$(grep -c '^rollback$' "$trace")" == 1 ]]
            case "$scenario" in
                timeout|redirect)
                    [[ "$output" == *'Health check timeout'* && "$output" == *'127.0.0.1:7700/healthz'* ]]
                    [[ "$(grep -c '^health:internal$' "$trace")" == 75 ]]
                    [[ "$(grep -c '^poll:' "$trace")" == 74 ]]
                    ! grep -q '^binding:' "$trace" ;;
                proxy-timeout)
                    [[ "$output" == *'Health check timeout'* && "$output" == *'127.0.0.1:19090/healthz'* ]]
                    [[ "$(grep -c '^health:public$' "$trace")" == 75 ]] ;;
                ss-error) [[ "$output" == *'Could not inspect the API listener with ss'* ]] ;;
                nginx-*) grep -qx nginx:validated "$trace" ;;
                *)
                    [[ "$output" == *'API listener validation failed.'* && "$output" == *'Expected:'* && "$output" == *'Detected:'* ]]
                    case "$scenario" in
                        no-listener) [[ "$output" == *'<none>'* ]] ;;
                        unsafe-v4) [[ "$output" == *'0.0.0.0:7700'* ]] ;;
                        unsafe-star) [[ "$output" == *'*:7700'* ]] ;;
                        unsafe-v6|mixed) [[ "$output" == *'[::]:7700'* ]] ;;
                        unsafe-specific) [[ "$output" == *'192.0.2.10:7700'* ]] ;;
                    esac ;;
            esac
            case "$scenario" in
                proxy-timeout) ;;
                nginx-*) ! grep -q '^health:public$' "$trace" ;;
                *) ! grep -q '^nginx:' "$trace"; ! grep -q '^health:public$' "$trace" ;;
            esac
            ! grep -qx firewall "$trace"
            ;;
    esac
    pass "startup scenario: $scenario"
done

# Missing units are expected on fresh installs; state detection must be quiet.
output="$(bash -c '
    source "$1/install.sh"
    MANAGED_FILES=()
    mktemp() { printf /mock-backup; }; chmod() { :; }; mkdir() { :; }; cp() { :; }
    systemctl() { echo "Failed to get unit file state: No such file or directory" >&2; return 4; }
    begin_transaction
    [[ "$TRANSACTION" == true && "$STATE_CHECKED" == true ]]
    [[ "$API_WAS_ENABLED" == false && "$AGENT_WAS_ENABLED" == false && "$NGINX_WAS_ENABLED" == false ]]
' test "$REPO" 2>&1)"
[[ -z "$output" ]]
pass 'fresh install state detection suppresses missing-unit noise'

# Bootstrap output remains private until health gates; migration failures still fail installation.
for mode in uninitialized migration-failure; do
    status=0
    output="$(bash -c '
        source "$1/install.sh"
        runuser() {
            [[ "$*" == "-u panel-api -- /usr/local/bin/panelctl --db-path /var/lib/panel/panel.db admin bootstrap --installer-output" ]] || exit 99
            [[ "$probe_mode" != migration-failure ]] || return 1
            printf "created admin_123456abcdef Qp!1234567890abcdef1234567890abcdef\n"
        }
        chown() { :; }; chmod() { :; }
        probe_mode="$2"
        initialize_database
        [[ "$ADMIN_USERNAME" == admin_123456abcdef ]]
    ' test "$REPO" "$mode" 2>&1)" || status=$?
    [[ "$output" != *Qp!* ]]
    if [[ "$mode" == uninitialized ]]; then
        [[ "$status" == 0 && -z "$output" ]]
    else
        [[ "$status" != 0 && "$output" == *'Account initialization or SQLite migration failed'* ]]
    fi
    pass "internal account bootstrap: $mode"
done

printf '%s startup race checks passed (mocked; no VM/agent/services executed).\n' "$count"
