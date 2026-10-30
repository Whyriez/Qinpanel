#!/usr/bin/env bash
# Exercise the actual CI bootstrap and installer with package-manager mocks only.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -f -- "$fixture/bootstrap.sh" "$fixture/commands"; rmdir -- "$fixture"' EXIT
awk '
    /^      - name: Bootstrap container tools$/ { found=1; next }
    found && /^      - uses:/ { exit }
    found && /^        run: \|$/ { commands=1; next }
    found && commands { sub(/^          /, ""); print }
' "$REPO/.github/workflows/platform-compatibility.yml" > "$fixture/bootstrap.sh"
[[ -s "$fixture/bootstrap.sh" ]]
# Never edit the host's repository files when exercising the new CI prelude.
# Actual repository rewrites/retries are covered by almalinux-repo-ci-test.py.
CI_FIXTURE_REPOS="$fixture/repos"
sed -i 's@/etc/yum.repos.d/almalinux\*\.repo@"$CI_FIXTURE_REPOS"/almalinux*.repo@' "$fixture/bootstrap.sh"

dnf() { printf 'dnf %s\n' "$*" >> "$fixture/commands"; }
apt-get() { printf 'apt-get %s\n' "$*" >> "$fixture/commands"; }
command() {
    case "${1:-}:${2:-}" in
        -v:dnf) [[ "$family" == rpm ]] ;;
        -v:curl) [[ "$curl_available" == yes ]] ;;
        *) builtin command "$@" ;;
    esac
}

for family in rpm deb; do
    for curl_available in yes no; do
        : > "$fixture/commands"
        (source "$fixture/bootstrap.sh")
        if [[ "$family" == rpm ]]; then
            ! grep -Fq 'apt-get' "$fixture/commands"
            ! grep -Eq '^dnf .*install (.* )?curl( |$)' "$fixture/commands"
            if [[ "$curl_available" == yes ]]; then
                ! grep -Fq 'curl-minimal' "$fixture/commands"
            else
                grep -Fxq 'dnf -y install curl-minimal' "$fixture/commands"
            fi
        else
            ! grep -Fq 'dnf' "$fixture/commands"
            grep -Fxq 'apt-get update -qq' "$fixture/commands"
            grep -Fq 'install -y --no-install-recommends git curl' "$fixture/commands"
        fi
        printf 'PASS container bootstrap family=%s existing-curl=%s\n' "$family" "$curl_available"
    done
done

source "$REPO/install.sh"
UPGRADE=false
PACKAGE_FAMILY=rpm
family=rpm
for curl_available in yes no; do
    : > "$fixture/commands"
    install_packages
    ! grep -Eq '^dnf .*install (.* )?curl( |$)' "$fixture/commands"
    grep -Fq 'install nginx nftables tar ca-certificates' "$fixture/commands"
    if [[ "$curl_available" == yes ]]; then
        ! grep -Fq 'curl-minimal' "$fixture/commands"
    else
        grep -Fxq 'dnf -y install curl-minimal' "$fixture/commands"
    fi
    printf 'PASS RPM installer preserves or supplies curl existing-curl=%s\n' "$curl_available"
done
printf '6 bootstrap/curl checks passed (mock package managers, no host services).\n'
