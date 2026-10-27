#!/usr/bin/env bash
# Check the real CI copy step using a disposable Git fixture, never Cargo/daemons.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
workflow="$REPO/.github/workflows/ci.yml"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
count=0
pass() { count=$((count + 1)); printf 'PASS %s\n' "$1"; }

# Keep this fixture independent from the real project's branch, index and history.
export GITHUB_WORKSPACE="$fixture/source"
export QINPANEL_FIXTURE_WORKDIR="$fixture/build/Qinpanel"
mkdir -p "$GITHUB_WORKSPACE" "$fixture/build"
git -C "$GITHUB_WORKSPACE" init -q
git -C "$GITHUB_WORKSPACE" config core.autocrlf false
printf '[workspace]\nmembers = []\n' > "$GITHUB_WORKSPACE/Cargo.toml"
git -C "$GITHUB_WORKSPACE" add Cargo.toml
git -C "$GITHUB_WORKSPACE" -c user.name='Qinpanel fixture' -c user.email=fixture@example.invalid \
    -c commit.gpgsign=false commit -qm fixture
export GITHUB_SHA
GITHUB_SHA="$(git -C "$GITHUB_WORKSPACE" rev-parse HEAD)"

copy_step="$(awk '
    /- name: Prepare Qinpanel build directory/ { in_step=1; next }
    in_step && /- name:/ { exit }
    in_step && /run: \|/ { in_run=1; next }
    in_run && /^          / { sub(/^          /, ""); print }
' "$workflow")"
[[ "$copy_step" == *'cp -a '* && "$copy_step" == *'rev-parse HEAD'* ]]
# Redirect the literal CI destination to a task-owned fixture, preserving quotes.
copy_step="${copy_step//\/tmp\/Qinpanel/\"\$QINPANEL_FIXTURE_WORKDIR\"}"
bash -euo pipefail -c "$copy_step"
[[ ! -L "$QINPANEL_FIXTURE_WORKDIR" ]]
[[ "$(cd "$QINPANEL_FIXTURE_WORKDIR" && pwd -P)" == "$QINPANEL_FIXTURE_WORKDIR" ]]
cmp "$GITHUB_WORKSPACE/Cargo.toml" "$QINPANEL_FIXTURE_WORKDIR/Cargo.toml"
[[ "$(git -C "$QINPANEL_FIXTURE_WORKDIR" rev-parse HEAD)" == "$GITHUB_SHA" ]]
pass 'Qinpanel build directory contains a physical copy of the checked-out commit'

printf 'keep existing build\n' > "$QINPANEL_FIXTURE_WORKDIR/sentinel"
if bash -euo pipefail -c "$copy_step" >/dev/null 2>&1; then exit 1; fi
grep -Fxq 'keep existing build' "$QINPANEL_FIXTURE_WORKDIR/sentinel"
pass 'CI copy refuses to overwrite an existing build directory'

grep -Fxq '        working-directory: /tmp/Qinpanel' "$workflow"
grep -Fxq '          workspaces: /tmp/Qinpanel -> target' "$workflow"
grep -Fxq '          prefix-key: qinpanel-backend-v1' "$workflow"
grep -Fxq '        run: cargo test --workspace' "$workflow"
grep -Fq 'cargo clippy --workspace --all-targets -- -D warnings' "$workflow"
pass 'Cargo checks and dependency cache use the Qinpanel build workspace'

if grep -RniE --include='*.rs' \
    '^[[:space:]]*(async )?fn [a-z0-9_]*(qonpanel|qpanel|nusa_panel|nusa_brand|vpanel)[a-z0-9_]*\(' \
    "$REPO/crates"; then exit 1; fi
pass 'Rust test names use current branding or describe historical compatibility generically'

printf '%s Qinpanel CI branding checks passed (temporary Git fixture only).\n' "$count"
