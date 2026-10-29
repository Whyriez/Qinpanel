#!/usr/bin/env bash
# Mocked release compilation/packaging: never execute cargo, containers, or agents.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
MOCK_REPO="$fixture/repo"
TRACE="$fixture/build.trace"
export MOCK_REPO TRACE
mkdir -p "$MOCK_REPO/scripts" "$MOCK_REPO/templates" "$MOCK_REPO/target/release/build"
cp "$REPO/scripts/compile-release.sh" "$REPO/scripts/build-release.sh" "$REPO/scripts/check-glibc.sh" "$REPO/scripts/package-release.sh" "$MOCK_REPO/scripts/"
cp "$REPO/scripts/server-backup-worker.py" "$REPO/scripts/platform-selinux.py" "$REPO/scripts/mail-auth.py" "$REPO/scripts/object-storage-worker.py" "$REPO/scripts/bootstrap-install.sh" "$REPO/scripts/panel-update-worker.sh" "$MOCK_REPO/scripts/"
cp "$REPO/install.sh" "$REPO/uninstall.sh" "$MOCK_REPO/"
cp "$REPO/UPGRADE_FROM" "$MOCK_REPO/"
cp -r "$REPO/templates/systemd" "$REPO/templates/nginx" "$MOCK_REPO/templates/"
mkdir -p "$MOCK_REPO/web/dist/assets/nested"
printf 'fixture index\n' > "$MOCK_REPO/web/dist/index.html"
printf 'fixture JavaScript\n' > "$MOCK_REPO/web/dist/assets/index-fixture.js"
printf 'private frontend notes\n' > "$MOCK_REPO/web/dist/README.md"
printf 'nested private notes\n' > "$MOCK_REPO/web/dist/assets/nested/NOTES.MD"
printf '{"scripts":{"build":"fixture-only"}}\n' > "$MOCK_REPO/web/package.json"
printf '[workspace.package]\nversion = "0.1.0"\n' > "$MOCK_REPO/Cargo.toml"
printf 'incompatible GLIBC_2.39 cached host build script\n' > "$MOCK_REPO/target/release/build/legacy-build-script"

rustc() { [[ "$*" == -vV ]] || return 99; printf 'host: %s\n' "$FAKE_HOST"; }
getconf() { [[ "$*" == GNU_LIBC_VERSION ]] || return 99; printf '%s\n' "$FAKE_LIBC"; }
cargo() {
    printf 'cargo:%s:%s\n' "$CARGO_TARGET_DIR" "$*" >> "$TRACE"
    [[ "$CARGO_INCREMENTAL" == 0 ]] || return 99
    [[ "$*" == "build --locked --release --target $FAKE_HOST -p panel-agent -p panel-api -p panel-cli" ]] || return 99
    [[ "$PWD" == "$MOCK_REPO" && "$CARGO_TARGET_DIR" != "$MOCK_REPO/target" ]] || return 99
    [[ "${FAKE_CARGO_FAIL:-0}" == 0 ]] || return 9
    local output="$CARGO_TARGET_DIR/$FAKE_HOST/release" binary
    mkdir -p "$output"
    for binary in panel-agent panel-api panelctl; do
        printf 'fresh native %s %s\n' "$FAKE_HOST" "$binary" > "$output/$binary"
    done
}
readelf() {
    printf 'readelf:%s\n' "$*" >> "$TRACE"
    [[ "$1" == --version-info && -f "$2" ]] || return 99
    printf '%s\n' "${FAKE_SYMBOLS:-GLIBC_2.34}"
}
npm() {
    [[ "$PWD" == "$MOCK_REPO/web" && "$*" == 'run build' ]] || return 99
    printf 'npm:%s\n' "$*" >> "$TRACE"
}
export -f rustc getconf cargo readelf npm
FAKE_LIBC='glibc 2.34'
FAKE_CARGO_FAIL=0
FAKE_SYMBOLS=GLIBC_2.34
export FAKE_HOST FAKE_LIBC FAKE_CARGO_FAIL FAKE_SYMBOLS
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }
component_label() {
    case "$1" in
        templates/nginx/nusa-panel.conf) printf 'Qinpanel Nginx proxy template' ;;
        templates/nginx/nusa-reject-unknown.conf) printf 'Qinpanel unknown-host guard template' ;;
        *) printf '%s' "$1" ;;
    esac
}

for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    bash "$REPO/scripts/package-release.sh" "$target" v0.1.0 --check-version >/dev/null
    pass "real workspace accepts development tag v0.1.0 for $target"

    FAKE_HOST="$target"
    : > "$TRACE"
    # An incompatible old target tree must not be used for compilation or packaging.
    mkdir -p "$MOCK_REPO/target/$target/release"
    for binary in panel-agent panel-api panelctl; do
        printf 'stale incompatible cached binary\n' > "$MOCK_REPO/target/$target/release/$binary"
    done
    output="$(unset CARGO_TARGET_DIR; bash "$MOCK_REPO/scripts/compile-release.sh" "$target" 2>&1)"
    grep -Fq "cargo:$MOCK_REPO/target/release-native/$target:" "$TRACE"
    [[ -f "$MOCK_REPO/target/release-native/$target/$target/release/panel-api" ]]
    pass "native $target uses isolated build directory and locked dependencies"

    : > "$TRACE"
    output="$(unset CARGO_TARGET_DIR; bash "$MOCK_REPO/scripts/build-release.sh" "$target" 2>&1)"
    archive="$MOCK_REPO/release/qinpanel-v0.1.0-$target.tar.gz"
    # Checksums contain archive basenames and must be verified from the release directory.
    (cd "$MOCK_REPO/release" && sha256sum -c "$(basename "$archive").sha256" --status)
    [[ "$(grep -c '^readelf:' "$TRACE")" == 3 ]]
    extract="$fixture/extracted-$target"
    mkdir -p "$extract"
    tar -xzf "$archive" -C "$extract"
    for binary in panel-agent panel-api panelctl; do
        [[ "$(cat "$extract/qinpanel-v0.1.0-$target/bin/$binary")" == "fresh native $target $binary" ]]
    done
    [[ -f "$extract/qinpanel-v0.1.0-$target/templates/nginx/nusa-panel.conf" ]]
    cmp -s "$REPO/templates/nginx/nusa-reject-unknown.conf" "$extract/qinpanel-v0.1.0-$target/templates/nginx/nusa-reject-unknown.conf"
    ! tar -tzf "$archive" | grep -Ei '\.[mM][dD]$'
    pass "release $target excludes local Markdown even from web assets"
    [[ -f "$extract/qinpanel-v0.1.0-$target/templates/systemd/panel-api.service" ]]
    cmp -s "$REPO/templates/systemd/panel-proc-hardening.service" "$extract/qinpanel-v0.1.0-$target/templates/systemd/panel-proc-hardening.service"
    cmp -s "$REPO/templates/systemd/panel-runtime-order.conf" "$extract/qinpanel-v0.1.0-$target/templates/systemd/panel-runtime-order.conf"
    cmp -s "$REPO/templates/systemd/nginx-panel-startup.conf" "$extract/qinpanel-v0.1.0-$target/templates/systemd/nginx-panel-startup.conf"
    [[ "$output" == *'[SUKSES]'* ]]
    primary="$archive"
    (cd "$MOCK_REPO/release" && sha256sum -c "$(basename "$primary").sha256" --status)
    ! compgen -G "$MOCK_REPO/release/nusa-panel-*" >/dev/null
    ! compgen -G "$MOCK_REPO/release/qonpanel-*" >/dev/null
    pass "only the Qinpanel $target archive and checksum are generated"
    pass "native $target GLIBC gate and tarball read freshly compiled artifacts"

    (export QONPANEL_RELEASE_BASE_URL=https://download.arelqon.com/qonpanel; unset CARGO_TARGET_DIR; bash "$MOCK_REPO/scripts/package-release.sh" "$target" v0.1.0 >/dev/null)
    tar -xzf "$primary" -C "$extract"
    grep -Fq "PACKAGED_RELEASE_BASE_URL='https://download.arelqon.com/qonpanel'" "$extract/qinpanel-v0.1.0-$target/scripts/bootstrap-install.sh"
    (cd "$MOCK_REPO/release" && sha256sum -c "$(basename "$primary").sha256" --status)
    pass "verified $target package pins future worker downloads to the built distribution source"
done

python3 - "$REPO" "$MOCK_REPO/release" "$fixture/verified-dist" <<'PY'
import importlib.util
from pathlib import Path
import shutil
import sys

sys.dont_write_bytecode = True
repo, packaged, distribution = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location("publisher", repo / "scripts/publish-distribution.py")
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)
distribution.mkdir()
for archive in packaged.glob("qinpanel-*.tar.gz"):
    shutil.copyfile(archive, distribution / archive.name)
checksum_parts = []
for path in sorted(packaged.glob("qinpanel-*.tar.gz.sha256")):
    data = path.read_bytes()
    # MSYS sha256sum marks binary mode with '*'; emulate Linux release-runner
    # checksum syntax for this local fixture without changing any archive bytes.
    if sys.platform == "win32":
        data = data.replace(b" *qinpanel-", b"  qinpanel-", 1)
    checksum_parts.append(data)
(distribution / "SHA256SUMS").write_bytes(b"".join(checksum_parts))
manifest, _ = publisher.prepare(
    distribution, "v0.1.0", "https://download.arelqon.com/qonpanel", repo / "scripts/bootstrap-install.sh"
)
github = distribution.parent / "verified-github"
publisher.prepare_github_assets(distribution, github, manifest)
expected = {f"qinpanel-v0.1.0-{target}.tar.gz" for target in publisher.TARGETS} | {"SHA256SUMS"}
assert {path.name for path in distribution.iterdir()} == expected
assert {path.name for path in github.iterdir()} == expected
for name in expected:
    assert (distribution / name).read_bytes() == (github / name).read_bytes()
PY
pass 'actual packaged Qinpanel archives and combined checksums pass publisher and GitHub staging verification'

FAKE_HOST=x86_64-unknown-linux-gnu
# Execute the packaging run block from the real CI workflow with mocked artifacts.
workflow="$REPO/.github/workflows/release.yml"
ci_package_step="$(awk '
    /^      - name: Package tarball and checksum$/ { found=1; next }
    found && /^      - name:/ { exit }
    found && /^        run: \|$/ { commands=1; next }
    found && commands { sub(/^          /, ""); print }
' "$workflow")"
[[ -n "$ci_package_step" ]]
# Resolve old expressions too, so this regression also reproduces the old bug.
ci_package_step="${ci_package_step//'${{ inputs.tag }}'/'$RELEASE_VERSION'}"
ci_package_step="${ci_package_step//'${{ matrix.target }}'/'$RELEASE_TARGET'}"
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    output="$(cd "$MOCK_REPO"; export CARGO_TARGET_DIR="target/release-native/$target"
        export RELEASE_TARGET="$target" RELEASE_VERSION=v0.1.0
        bash -euo pipefail -c "$ci_package_step" 2>&1)"
    archive="$MOCK_REPO/release/qinpanel-v0.1.0-$target.tar.gz"
    (cd "$MOCK_REPO/release" && sha256sum -c "$(basename "$archive").sha256" --status)
    listing="$(tar -tzf "$archive")"
    for required in templates/nginx/nusa-reject-unknown.conf web/dist/index.html; do
        if ! grep -Fxq "qinpanel-v0.1.0-$target/$required" <<< "$listing"; then
            printf 'CI package missing required file: %s\n' "$required" >&2
            exit 1
        fi
    done
    pass "actual CI packaging block includes guard and web assets without local docs for $target"
done

for custom_dir in "$fixture/custom build output" 'target/custom output'; do
    : > "$TRACE"
    output="$(export CARGO_TARGET_DIR="$custom_dir"; bash "$MOCK_REPO/scripts/build-release.sh" "$FAKE_HOST" 2>&1)"
    grep -Fq "cargo:$custom_dir:" "$TRACE"
    grep -Fq "readelf:--version-info $custom_dir/$FAKE_HOST/release/panel-api" "$TRACE"
    [[ "$output" == *'[SUKSES]'* ]]
    pass "packaging honors explicit CARGO_TARGET_DIR: $custom_dir"
done

for invalid in wrong-host new-glibc old-glibc unsupported-target; do
    FAKE_HOST=x86_64-unknown-linux-gnu
    FAKE_LIBC='glibc 2.34'
    target="$FAKE_HOST"
    case "$invalid" in
        wrong-host) target=aarch64-unknown-linux-gnu ;;
        new-glibc) FAKE_LIBC='glibc 2.39' ;;
        old-glibc) FAKE_LIBC='glibc 2.23' ;;
        unsupported-target) target=wasm32-unknown-unknown ;;
    esac
    : > "$TRACE"
    status=0
    output="$(unset CARGO_TARGET_DIR; bash "$MOCK_REPO/scripts/compile-release.sh" "$target" 2>&1)" || status=$?
    [[ "$status" != 0 && ! -s "$TRACE" ]]
    case "$invalid" in
        wrong-host) [[ "$output" == *'Native release build required'* ]] ;;
        new-glibc|old-glibc) [[ "$output" == *'Release build requires GLIBC 2.34'* ]] ;;
        unsupported-target) [[ "$output" == *'Unsupported release target'* ]] ;;
    esac
    pass "preflight rejects $invalid before cargo starts"
done

FAKE_LIBC='glibc 2.34'
for failure in compiler glibc-gate; do
    FAKE_CARGO_FAIL=0
    FAKE_SYMBOLS=GLIBC_2.34
    if [[ "$failure" == compiler ]]; then FAKE_CARGO_FAIL=1; else FAKE_SYMBOLS=GLIBC_2.39; fi
    archive="$MOCK_REPO/release/qinpanel-v0.1.0-$FAKE_HOST.tar.gz"
    rm -f "$archive" "$archive.sha256"
    : > "$TRACE"
    status=0
    output="$(unset CARGO_TARGET_DIR; bash "$MOCK_REPO/scripts/build-release.sh" "$FAKE_HOST" 2>&1)" || status=$?
    [[ "$status" != 0 && ! -f "$archive" && "$output" != *'[SUKSES]'* ]]
    if [[ "$failure" == compiler ]]; then
        [[ "$status" == 9 ]]
        ! grep -q '^readelf:' "$TRACE"
    else
        [[ "$output" == *'Unsupported GLIBC baseline'* ]]
    fi
    pass "$failure failure prevents packaging and success output"
done

# Reject incomplete CI payloads before creating or advertising an archive.
FAKE_CARGO_FAIL=0
FAKE_SYMBOLS=GLIBC_2.34
export RELEASE_TARGET="$FAKE_HOST" RELEASE_VERSION=v0.1.0
export CARGO_TARGET_DIR="$MOCK_REPO/target/release-native/$FAKE_HOST"
archive="$MOCK_REPO/release/qinpanel-v0.1.0-$FAKE_HOST.tar.gz"
for missing in \
    "${CARGO_TARGET_DIR#"$MOCK_REPO/"}/$FAKE_HOST/release/panel-agent" \
    templates/systemd/panel-proc-hardening.service \
    templates/systemd/panel-runtime-order.conf \
    templates/systemd/nginx-panel-startup.conf \
    templates/systemd/panel-update.service scripts/bootstrap-install.sh scripts/panel-update-worker.sh \
    templates/nginx/nusa-panel.conf templates/nginx/nusa-reject-unknown.conf templates/nginx/almalinux.conf scripts/platform-selinux.py scripts/mail-auth.py scripts/object-storage-worker.py \
    install.sh uninstall.sh UPGRADE_FROM web/dist/index.html \
    web/dist/assets/index-fixture.js; do
    mv "$MOCK_REPO/$missing" "$fixture/withheld"
    rm -f "$archive" "$archive.sha256"
    status=0
    output="$(cd "$MOCK_REPO"; bash -euo pipefail -c "$ci_package_step" 2>&1)" || status=$?
    mv "$fixture/withheld" "$MOCK_REPO/$missing"
    [[ "$status" != 0 && ! -f "$archive" && ! -f "$archive.sha256" && "$output" != *'[SUKSES]'* ]]
    [[ "$output" == *'Required release'* ]]
    pass "CI packaging rejects missing $(component_label "$missing")"
done

cp "$MOCK_REPO/templates/nginx/nusa-reject-unknown.conf" "$fixture/guard"
: > "$MOCK_REPO/templates/nginx/nusa-reject-unknown.conf"
status=0
output="$(cd "$MOCK_REPO"; bash -euo pipefail -c "$ci_package_step" 2>&1)" || status=$?
cp "$fixture/guard" "$MOCK_REPO/templates/nginx/nusa-reject-unknown.conf"
[[ "$status" != 0 && ! -f "$archive" && "$output" == *'missing or empty'* ]]
pass 'CI packaging rejects empty mandatory files'

for invalid_tag in v0.1.1 v1.0.0 '../v0.1.0' 'v0.1.0;touch unexpected'; do
    status=0
    output="$(cd "$MOCK_REPO"; export RELEASE_VERSION="$invalid_tag"
        bash -euo pipefail -c "$ci_package_step" 2>&1)" || status=$?
    [[ "$status" != 0 && ! -f "$archive" && "$output" == *'Release tag must match Cargo.toml'* ]]
    pass "CI packaging rejects mismatched or unsafe tag: $invalid_tag"
done
: > "$TRACE"
status=0
output="$(RELEASE_VERSION=v0.1.1 bash "$MOCK_REPO/scripts/build-release.sh" "$FAKE_HOST" 2>&1)" || status=$?
[[ "$status" != 0 && ! -s "$TRACE" ]]
pass 'local release rejects version mismatch before building'

status=0
output="$(bash "$MOCK_REPO/scripts/package-release.sh" wasm32-unknown-unknown v0.1.0 2>&1)" || status=$?
[[ "$status" != 0 && "$output" == *'Unsupported release target'* ]]
pass 'packager rejects unsupported architecture'

tar() {
    local arg
    for arg in "$@"; do
        if [[ "$arg" == -czf ]]; then return 8; fi
    done
    command tar "$@"
}
export -f tar
status=0
output="$(cd "$MOCK_REPO"; bash -euo pipefail -c "$ci_package_step" 2>&1)" || status=$?
unset -f tar
[[ "$status" == 8 && ! -f "$archive" && ! -f "$archive.sha256" && "$output" != *'[SUKSES]'* ]]
[[ -z "$(find "$MOCK_REPO/release" -maxdepth 1 -name '.package.*' -print)" ]]
pass 'failed tar creation cleans temporary payload and publishes no partial artifacts'

# Run the real installer preflight ONLY, with stub binaries in a temporary root.
INSTALL_PREFLIGHT_TRACE="$fixture/preflight.trace"
export INSTALL_PREFLIGHT_TRACE
mkdir -p "$MOCK_REPO/bin"
for binary in panel-api panel-agent panelctl; do
    cat > "$MOCK_REPO/bin/$binary" <<'STUB'
#!/usr/bin/env bash
[[ "$*" == --version ]] || exit 99
printf 'stub version check\n' >> "$INSTALL_PREFLIGHT_TRACE"
printf 'panel fixture 0.1.0\n'
STUB
    chmod 0755 "$MOCK_REPO/bin/$binary"
done
for missing in UPGRADE_FROM templates/systemd/panel-api.service \
    templates/systemd/panel-agent.service templates/systemd/panel-proc-hardening.service \
    templates/systemd/panel-runtime-order.conf \
    templates/systemd/nginx-panel-startup.conf \
    templates/systemd/panel-update.service scripts/bootstrap-install.sh scripts/panel-update-worker.sh \
    templates/nginx/nusa-panel.conf templates/nginx/nusa-reject-unknown.conf templates/nginx/almalinux.conf scripts/platform-selinux.py scripts/mail-auth.py scripts/object-storage-worker.py; do
    mv "$MOCK_REPO/$missing" "$fixture/withheld"
    : > "$INSTALL_PREFLIGHT_TRACE"
    status=0
    output="$(bash -c 'source "$1/install.sh"; find_release' test "$MOCK_REPO" 2>&1)" || status=$?
    mv "$fixture/withheld" "$MOCK_REPO/$missing"
    [[ "$status" != 0 && "$output" == *"$missing"* && ! -s "$INSTALL_PREFLIGHT_TRACE" ]]
    pass "installer detects missing $(component_label "$missing") before executing any binaries"
done
: > "$INSTALL_PREFLIGHT_TRACE"
bash -c 'source "$1/install.sh"; find_release' test "$MOCK_REPO"
[[ "$(wc -l < "$INSTALL_PREFLIGHT_TRACE")" == 3 ]]
pass 'complete package passes installer preflight using stub versions only'

[[ "$(cat "$MOCK_REPO/target/release/build/legacy-build-script")" == 'incompatible GLIBC_2.39 cached host build script' ]]
pass 'legacy incompatible build cache remains unused and untouched'

workflow="$REPO/.github/workflows/release.yml"
grep -Fq 'runs-on: ${{ matrix.runner }}' "$workflow"
grep -q 'runner: ubuntu-22.04$' "$workflow"
grep -q 'runner: ubuntu-22.04-arm$' "$workflow"
! grep -Eq 'cross build|cargo install cross|use_cross' "$workflow"
grep -Fq 'image: almalinux:9' "$workflow"
grep -Fq "prefix-key: release-native-glibc234-v2" "$workflow"
grep -Fq "cache-targets: 'false'" "$workflow"
grep -Fq "cache-bin: 'false'" "$workflow"
grep -Fq 'CARGO_TARGET_DIR: target/release-native/${{ matrix.target }}' "$workflow"
grep -Fq 'scripts/compile-release.sh' "$workflow"
grep -Fq 'scripts/package-release.sh "$RELEASE_TARGET" "$RELEASE_VERSION"' "$workflow"
! grep -q 'cp templates/' "$workflow"
! grep -Fq 'cp target/${{ matrix.target }}' "$workflow"
pass 'release workflow uses native runners, source-only cache and isolated artifact paths'
grep -Fq 'name: qinpanel-${{ matrix.target }}' "$workflow"
grep -Fq -- '--github-directory github-dist' "$workflow"
grep -Fq '            github-dist/*' "$workflow"
! grep -Fq '            dist/*' "$workflow"
grep -Fq '            release/qinpanel-*.tar.gz' "$workflow"
grep -Fq '            release/qinpanel-*.tar.gz.sha256' "$workflow"
! grep -Fq '            release/*.tar.gz' "$workflow"
pass 'artifact upload and GitHub release select only Qinpanel archives and checksums'
grep -Fq "default: 'v0.1.0'" "$workflow"
grep -Fq 'default: false' "$workflow"
grep -Fq "github.event_name == 'workflow_dispatch' && inputs.publish" "$workflow"
grep -Fq "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/')" "$workflow"
grep -Fq 'tag_name: ${{ inputs.tag || github.ref_name }}' "$workflow"
grep -Fq 'overwrite_files: true' "$workflow"
pass 'manual development republish is opt-in and uses the requested release tag'

printf '%s release build checks passed (mock compiler/readelf, no real release build).\n' "$count"
