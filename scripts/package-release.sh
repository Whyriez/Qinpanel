#!/usr/bin/env bash
# Shared packaging entry point for local builds and GitHub Releases.
# This script only reads build artifacts; it never executes panel binaries.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-x86_64-unknown-linux-gnu}"
case "$TARGET" in
    x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
    *) echo "Unsupported release target: $TARGET" >&2; exit 1 ;;
esac
VERSION="$(awk '
    /^\[workspace[.]package\]$/ { package=1; next }
    /^\[/ { package=0 }
    package && /^version[[:space:]]*=/ { split($0, parts, "\""); print parts[2]; exit }
' "$REPO/Cargo.toml")"
TAG="${2:-v$VERSION}"
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ && "$TAG" == "v$VERSION" ]] || {
    printf 'Release tag must match Cargo.toml: expected v%s, received %s\n' "$VERSION" "$TAG" >&2
    exit 1
}
[[ $# -le 3 ]] || { echo 'Too many packaging arguments' >&2; exit 1; }
case "${3:-}" in
    --check-version) printf 'Release version validated: %s (%s)\n' "$TAG" "$TARGET"; exit 0 ;;
    '') ;;
    *) echo 'Unknown packaging option' >&2; exit 1 ;;
esac
cd "$REPO"
BIN_DIR="${CARGO_TARGET_DIR:-$REPO/target/release-native/$TARGET}/$TARGET/release"
OUTPUT_DIR="$REPO/release"
STAGE_NAME="qinpanel-$TAG-$TARGET"
ARCHIVE_NAME="$STAGE_NAME.tar.gz"

# One explicit payload list for every release path. Missing files fail the build.
sources=(
    "$BIN_DIR/panel-api" "$BIN_DIR/panel-agent" "$BIN_DIR/panelctl"
    install.sh uninstall.sh UPGRADE_FROM
    scripts/bootstrap-install.sh scripts/panel-update-worker.sh
    scripts/platform-selinux.py
    scripts/object-storage-worker.py
    scripts/mail-auth.py
    scripts/server-backup-worker.py templates/systemd/qonpanel-server-backup.service
    templates/systemd/panel-update.service
    templates/systemd/panel-api.service
    templates/systemd/panel-agent.service
    templates/systemd/panel-proc-hardening.service
    templates/systemd/panel-runtime-order.conf
    templates/nginx/nusa-panel.conf
    templates/nginx/nusa-reject-unknown.conf
    templates/nginx/almalinux.conf
    web/dist/index.html
)
destinations=(
    bin/panel-api bin/panel-agent bin/panelctl
    install.sh uninstall.sh UPGRADE_FROM
    scripts/bootstrap-install.sh scripts/panel-update-worker.sh
    scripts/platform-selinux.py
    scripts/object-storage-worker.py
    scripts/mail-auth.py
    scripts/server-backup-worker.py templates/systemd/qonpanel-server-backup.service
    templates/systemd/panel-update.service
    templates/systemd/panel-api.service
    templates/systemd/panel-agent.service
    templates/systemd/panel-proc-hardening.service
    templates/systemd/panel-runtime-order.conf
    templates/nginx/nusa-panel.conf
    templates/nginx/nusa-reject-unknown.conf
    templates/nginx/almalinux.conf
    web/dist/index.html
)
missing=0
for source in "${sources[@]}"; do
    if [[ ! -f "$source" || ! -r "$source" || ! -s "$source" ]]; then
        printf 'Required release file missing or empty: %s\n' "$source" >&2
        missing=1
    fi
done
if ! compgen -G 'web/dist/assets/*.js' >/dev/null; then
    echo 'Required release assets missing: web/dist/assets/*.js (run the frontend build)' >&2
    missing=1
fi
[[ "$missing" == 0 ]] || exit 1

# Stage in a private, unique directory so failures cannot publish a partial archive.
mkdir -p "$OUTPUT_DIR"
temporary="$(mktemp -d "$OUTPUT_DIR/.package.XXXXXXXX")"
trap 'rm -rf "$temporary"' EXIT
stage="$temporary/$STAGE_NAME"
for index in "${!sources[@]}"; do
    destination="$stage/${destinations[$index]}"
    mkdir -p "$(dirname "$destination")"
    cp "${sources[$index]}" "$destination"
done
cp -R web/dist/. "$stage/web/dist/"
if [[ -n "${QONPANEL_RELEASE_BASE_URL:-}" ]]; then
    (source "$REPO/scripts/bootstrap-install.sh"; validate_release_base "$QONPANEL_RELEASE_BASE_URL") || { echo 'Invalid HTTPS release distribution URL' >&2; exit 1; }
    python3 - "$stage/scripts/bootstrap-install.sh" "$QONPANEL_RELEASE_BASE_URL" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
marker = "PACKAGED_RELEASE_BASE_URL=''"
assert text.count(marker) == 1
with path.open('w', newline='\n') as output:
    output.write(text.replace(marker, "PACKAGED_RELEASE_BASE_URL='" + sys.argv[2] + "'"))
PY
fi
chmod 0755 "$stage/bin/"* "$stage/install.sh" "$stage/uninstall.sh"
# Local documentation must never leak through copied web assets.
tar --exclude='*.[mM][dD]' -czf "$temporary/$ARCHIVE_NAME" -C "$temporary" "$STAGE_NAME"
# Verify every required member in the completed archive, then generate its checksum.
listing="$(tar -tzf "$temporary/$ARCHIVE_NAME")"
for destination in "${destinations[@]}"; do
    grep -Fxq "$STAGE_NAME/$destination" <<< "$listing" || {
        printf 'Incomplete release archive: %s\n' "$destination" >&2
        exit 1
    }
done
(cd "$temporary" && sha256sum "$ARCHIVE_NAME" > "$ARCHIVE_NAME.sha256")
# Compatibility downloads keep their own roots for already-installed bootstraps.
# They are published to Storage, not advertised as new GitHub Release assets.
for brand in qonpanel nusa-panel; do
    legacy_name="$brand-$TAG-$TARGET"
    cp -R "$stage" "$temporary/$legacy_name"
    tar --exclude='*.[mM][dD]' -czf "$temporary/$legacy_name.tar.gz" -C "$temporary" "$legacy_name"
    legacy_listing="$(tar -tzf "$temporary/$legacy_name.tar.gz")"
    for destination in "${destinations[@]}"; do
        grep -Fxq "$legacy_name/$destination" <<< "$legacy_listing" || exit 1
    done
    (cd "$temporary" && sha256sum "$legacy_name.tar.gz" > "$legacy_name.tar.gz.sha256")
done
mv "$temporary/"*.tar.gz "$temporary/"*.tar.gz.sha256 "$OUTPUT_DIR/"
printf '[SUKSES] Paket rilis: %s/%s\nChecksum: %s/%s.sha256\n' \
    "$OUTPUT_DIR" "$ARCHIVE_NAME" "$OUTPUT_DIR" "$ARCHIVE_NAME"
