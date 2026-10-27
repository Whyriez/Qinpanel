#!/usr/bin/env bash
# scripts/uninstall.sh: Convenience wrapper calling root uninstall.sh
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "${SCRIPT_DIR}/uninstall.sh" "$@"
