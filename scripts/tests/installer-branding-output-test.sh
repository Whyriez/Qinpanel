#!/usr/bin/env bash
# Render the actual bootstrap with local archives and mocked downloads/install.
# No network, root commands, package manager or system services are run.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }
no_old_brand() { ! grep -Eiq 'nusa[ _-]*panel|qonpanel|qpanel|vpanel' "$1"; }

for prefix in qinpanel qonpanel nusa-panel; do
    package="$prefix-v0.1.0-x86_64-unknown-linux-gnu"
    mkdir -p "$fixture/$package"
    printf '#!/usr/bin/env bash\nprintf "Qinpanel fixture installer\\n"\n' > "$fixture/$package/install.sh"
    # Future-dated archived files used to print one warning per legacy filename.
    tar --mtime='2035-01-01' -czf "$fixture/$package.tar.gz" -C "$fixture" "$package"
    (cd "$fixture"; sha256sum "$package.tar.gz" > "$prefix.sums")
    (
        source "$REPO/scripts/bootstrap-install.sh"
        check_root() { :; }; check_dependencies() { :; }; detect_os() { :; }
        resolve_latest_version() { printf 'v0.1.0\n'; }
        uname() { printf 'x86_64\n'; }
        download_file() {
            local name="${1##*/}"
            printf '%s\n' "$name" >> "$fixture/$prefix.downloads"
            if [[ "$name" == SHA256SUMS ]]; then cp "$fixture/$prefix.sums" "$2";
            elif [[ "$name" == "$package.tar.gz" ]]; then cp "$fixture/$package.tar.gz" "$2";
            else return 22; fi
        }
        main --unattended
    ) > "$fixture/$prefix.output" 2>&1
    grep -Fq 'Qinpanel v0.1.0 (x86_64-unknown-linux-gnu)' "$fixture/$prefix.output"
    grep -Fq 'Qinpanel fixture installer' "$fixture/$prefix.output"
    no_old_brand "$fixture/$prefix.output"
    expected_names=()
    for brand in qinpanel qonpanel nusa-panel; do
        expected_names+=("$brand-v0.1.0-x86_64-unknown-linux-gnu.tar.gz")
        [[ "$brand" == "$prefix" ]] && break
    done
    expected_names+=(SHA256SUMS)
    diff -u <(printf '%s\n' "${expected_names[@]}") "$fixture/$prefix.downloads"
    pass 'verified release prefers the canonical name and prints only Qinpanel branding'
done

if (
    source "$REPO/scripts/bootstrap-install.sh"
    check_root() { :; }; check_dependencies() { :; }; detect_os() { :; }
    resolve_latest_version() { printf 'v0.1.0\n'; }
    uname() { printf 'x86_64\n'; }
    download_file() {
        local name="${1##*/}"
        printf '%s\n' "$name" >> "$fixture/corrupt.downloads"
        if [[ "$name" == SHA256SUMS ]]; then cp "$fixture/qinpanel.sums" "$2";
        else cp "$fixture/qinpanel-v0.1.0-x86_64-unknown-linux-gnu.tar.gz" "$2"; printf 'corrupt\n' >> "$2"; fi
    }
    main --unattended
) > "$fixture/corrupt.output" 2>&1; then exit 1; fi
grep -Fq 'Release checksum verification failed.' "$fixture/corrupt.output"
! grep -Fq 'Qinpanel fixture installer' "$fixture/corrupt.output"
diff -u <(printf '%s\n' qinpanel-v0.1.0-x86_64-unknown-linux-gnu.tar.gz SHA256SUMS) "$fixture/corrupt.downloads"
pass 'canonical checksum failure never falls back to a different archive or executes an installer'

if (
    source "$REPO/scripts/bootstrap-install.sh"
    check_root() { :; }; check_dependencies() { :; }; detect_os() { :; }
    resolve_latest_version() { printf 'v0.1.0\n'; }
    uname() { printf 'x86_64\n'; }
    download_file() { return 22; }
    main --unattended
) > "$fixture/download-error" 2>&1; then exit 1; fi
grep -Fq 'Failed to download the Qinpanel v0.1.0 release' "$fixture/download-error"
no_old_brand "$fixture/download-error"
pass 'failed primary and compatible downloads report only Qinpanel branding'

(
    unset QINPANEL_OWNER QINPANEL_REPO QINPANEL_VERSION QINPANEL_EXPECTED_SHA256
    unset QONPANEL_OWNER QONPANEL_REPO QONPANEL_VERSION QONPANEL_EXPECTED_SHA256
    unset NUSA_PANEL_OWNER NUSA_PANEL_REPO NUSA_PANEL_VERSION NUSA_PANEL_EXPECTED_SHA256
    source "$REPO/scripts/bootstrap-install.sh"
    [[ "$GITHUB_REPO" == Qinpanel ]]
    for operation in missing-checksum missing-archive missing-entry; do
        case "$operation" in
            missing-checksum) args=("$fixture/qonpanel-missing" "$fixture/$package.tar.gz" "$package.tar.gz") ;;
            missing-archive) args=("$fixture/$prefix.sums" "$fixture/nusa-panel-missing" "$package.tar.gz") ;;
            missing-entry) args=("$fixture/$prefix.sums" "$fixture/$package.tar.gz" qonpanel-not-in-manifest.tar.gz) ;;
        esac
        if verify_checksum "${args[@]}" > "$fixture/error" 2>&1; then exit 1; fi
        no_old_brand "$fixture/error"
        [[ -s "$fixture/error" ]]
    done
)
pass 'checksum errors do not print legacy archive paths or names'

(
    export QINPANEL_OWNER=current-owner QONPANEL_OWNER=previous-owner NUSA_PANEL_OWNER=legacy-owner
    export QINPANEL_REPO=Qinpanel QONPANEL_REPO=previous-repo NUSA_PANEL_REPO=legacy-repo
    export QINPANEL_VERSION=v1.2.3 QONPANEL_VERSION=v1.2.2 NUSA_PANEL_VERSION=v1.2.1
    export QINPANEL_EXPECTED_SHA256=current-digest QONPANEL_EXPECTED_SHA256=previous-digest
    source "$REPO/scripts/bootstrap-install.sh"
    [[ "$GITHUB_OWNER" == current-owner && "$GITHUB_REPO" == Qinpanel ]]
    [[ "$(resolve_latest_version)" == v1.2.3 && "$NUSA_PANEL_EXPECTED_SHA256" == current-digest ]]
)
pass 'Qinpanel environment settings take priority over both compatibility aliases'

(
    unset QINPANEL_OWNER QINPANEL_REPO QINPANEL_VERSION QINPANEL_EXPECTED_SHA256
    export QONPANEL_OWNER=previous-owner QONPANEL_REPO=previous-repo QONPANEL_VERSION=v1.2.2
    export QONPANEL_EXPECTED_SHA256=previous-digest NUSA_PANEL_VERSION=v1.2.1
    source "$REPO/scripts/bootstrap-install.sh"
    [[ "$GITHUB_OWNER" == previous-owner && "$GITHUB_REPO" == previous-repo ]]
    [[ "$(resolve_latest_version)" == v1.2.2 && "$NUSA_PANEL_EXPECTED_SHA256" == previous-digest ]]
)
pass 'previous environment settings still work for installed upgrade callers'

printf '%s installer branding output checks passed (local archives and mocks only).\n' "$count"
