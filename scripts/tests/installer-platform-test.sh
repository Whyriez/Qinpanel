#!/usr/bin/env bash
# Pure platform predicates and mocked package operations. No root/services/files.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$REPO/install.sh"
for pair in 'ubuntu 22.04' 'ubuntu 24.04' 'debian 12' 'almalinux 9' 'almalinux 9.7' 'almalinux 10.2'; do
    read -r distro release <<< "$pair"
    supported_distribution "$distro" "$release"
done
for pair in 'ubuntu 26.04' 'debian 13' 'almalinux 8.10' 'almalinux 9;' 'rocky 9' 'almalinux 10.invalid'; do
    read -r distro release <<< "$pair"
    if supported_distribution "$distro" "$release"; then exit 1; fi
done
fixture="$(mktemp -d)"
trap 'rm -f -- "$fixture/commands"; rmdir -- "$fixture"' EXIT
dnf() { printf 'dnf %s\n' "$*" >> "$fixture/commands"; }
apt-get() { printf 'apt-get %s\n' "$*" >> "$fixture/commands"; }
UPGRADE=false
PACKAGE_FAMILY=rpm
install_packages
grep -Fq 'dnf config-manager --set-enabled crb' "$fixture/commands"
grep -Fq 'dnf -y install epel-release' "$fixture/commands"
grep -Fq 'policycoreutils-python-utils' "$fixture/commands"
! grep -Fq 'apt-get' "$fixture/commands"
: > "$fixture/commands"
PACKAGE_FAMILY=deb
install_packages
grep -Fq 'apt-get update -qq' "$fixture/commands"
! grep -Fq 'dnf' "$fixture/commands"
apt-get() { return 1; }
status=0
output="$(install_packages 2>&1)" || status=$?
[[ "$status" != 0 && "$output" == *'check the server UTC time and NTP'* && "$output" == *'checks remain enabled'* ]]
configure_platform_security
commit_platform_security
echo 'PASS OS allowlist, DNF/CRB/EPEL plan, APT preservation, clock error guidance and non-RPM security no-op (mock commands only)'
