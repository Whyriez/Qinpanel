#!/usr/bin/env bash
# Root-only, fixed service entry point. EnvironmentFile fields are data, never eval.
set -Eeuo pipefail
set +x
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
STATE_DIR=/var/lib/panel-updater

validate_request() {
    [[ "${UPDATE_ID:-}" =~ ^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$ ]]
    [[ "${UPDATE_VERSION:-}" =~ ^v(0|[1-9][0-9]{0,7})\.(0|[1-9][0-9]{0,7})\.(0|[1-9][0-9]{0,7})$ ]]
    [[ "${UPDATE_DIGEST:-}" =~ ^[a-f0-9]{64}$ ]]
}
write_state() {
    local phase="$1" finished=null
    [[ "$phase" == running ]] || finished="$(date +%s)"
    printf '{"id":"%s","version":"%s","digest":"%s","phase":"%s","started_at":%s,"finished_at":%s}\n' \
        "$UPDATE_ID" "$UPDATE_VERSION" "$UPDATE_DIGEST" "$phase" "$started" "$finished" > "$STATE_DIR/state.json.worker"
    chmod 0600 "$STATE_DIR/state.json.worker"
    mv -f "$STATE_DIR/state.json.worker" "$STATE_DIR/state.json"
}
main() {
    validate_request || { echo 'Identitas update tidak valid.' >&2; return 1; }
    umask 077
    started="$(date +%s)"
    if [[ "${1:-}" == --finalize ]]; then
        if grep -Fq "\"id\":\"$UPDATE_ID\"" "$STATE_DIR/state.json" && grep -Eq '"phase":"(queued|running)"' "$STATE_DIR/state.json"; then write_state interrupted; fi
        return 0
    fi
    exec 8>"$STATE_DIR/worker.lock"
    flock -n 8 || { echo 'Updater lain sedang berjalan.' >&2; return 1; }
    # Never automatically replay an interrupted update after reboot.
    grep -Fq "\"id\":\"$UPDATE_ID\"" "$STATE_DIR/state.json"
    grep -q '"phase":"queued"' "$STATE_DIR/state.json"
    trap 'write_state failed' ERR
    trap 'write_state interrupted; exit 130' INT TERM
    write_state running
    # The bootstrap comes from the installed verified package, never from branch main.
    # Clearing the environment excludes URL/repository/skip-check overrides.
    env -i PATH="$PATH" HOME=/root NUSA_PANEL_VERSION="$UPDATE_VERSION" \
        NUSA_PANEL_EXPECTED_SHA256="$UPDATE_DIGEST" \
        /bin/bash /usr/local/libexec/panel-bootstrap --upgrade --unattended
    write_state succeeded
    trap - ERR INT TERM
}
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then main "$@"; fi
