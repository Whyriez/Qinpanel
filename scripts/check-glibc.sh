#!/usr/bin/env bash
set -euo pipefail
# AlmaLinux 9 is the oldest libc baseline; Debian/Ubuntu/AlmaLinux 10 are newer.
[[ $# -gt 0 ]] || { echo 'Usage: check-glibc.sh <ELF binary>...' >&2; exit 1; }
for binary in "$@"; do
    symbols="$(readelf --version-info "$binary")"
    versions="$(grep -oE 'GLIBC_[0-9]+\.[0-9]+(\.[0-9]+)?' <<< "$symbols" | sort -Vu)"
    [[ -n "$versions" ]] || { echo "No GLIBC requirements found: $binary" >&2; exit 1; }
    newest="$(printf '%s\n' "$versions" | tail -n 1)"
    [[ "$(printf '%s\n' "$newest" GLIBC_2.34 | sort -V | tail -n 1)" == GLIBC_2.34 ]] || {
        echo "Unsupported GLIBC baseline: $binary requires $newest (maximum GLIBC_2.34)" >&2
        exit 1
    }
    echo "$binary: $newest (compatible baseline)"
done
