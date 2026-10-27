#!/usr/bin/env bash
# ==============================================================================
# Qinpanel - Release Packaging Script
# Targets: x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-x86_64-unknown-linux-gnu}"
cd "$SCRIPT_DIR"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/target/release-native/$TARGET}"
BIN_DIR="$CARGO_TARGET_DIR/$TARGET/release"

echo "================================================================================"
echo "          MEMBANGUN PAKET RILIS QONPANEL (${TARGET})              "
echo "================================================================================"


# Validate release metadata before any expensive build.
bash "$SCRIPT_DIR/scripts/package-release.sh" "$TARGET" "${RELEASE_VERSION:-}" --check-version

# 1. Build frontend bundle
if [[ -d "${SCRIPT_DIR}/web" && -f "${SCRIPT_DIR}/web/package.json" ]]; then
    echo "==> Membangun aset antarmuka web (npm run build)..."
    (cd "${SCRIPT_DIR}/web" && npm run build)
fi

# 2. Build Rust release binaries
echo "==> Mengompilasi biner release untuk target ${TARGET}..."
bash "$SCRIPT_DIR/scripts/compile-release.sh" "$TARGET"

bash "${SCRIPT_DIR}/scripts/check-glibc.sh" \
    "$BIN_DIR/panel-api" \
    "$BIN_DIR/panel-agent" \
    "$BIN_DIR/panelctl"

# 3. Use the same complete packaging path as the GitHub release workflow.
bash "$SCRIPT_DIR/scripts/package-release.sh" "$TARGET" "${RELEASE_VERSION:-}"
