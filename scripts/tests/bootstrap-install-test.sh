#!/usr/bin/env bash
# ==============================================================================
# Qinpanel - Bootstrap Installer Test Suite
# Tests all requirements specified in BOOTSTRAP_INSTALLER.md Section 24
# ==============================================================================

set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BOOTSTRAP_SCRIPT="${SCRIPT_DIR}/scripts/bootstrap-install.sh"

TEST_COUNT=0
PASS_COUNT=0
FAIL_COUNT=0

# Formatting
if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
    C_GREEN='\033[0;32m'
    C_RED='\033[0;31m'
    C_CYAN='\033[0;36m'
    C_BOLD='\033[1m'
    C_RESET='\033[0m'
else
    C_GREEN=''
    C_RED=''
    C_CYAN=''
    C_BOLD=''
    C_RESET=''
fi

report_test() {
    local name="$1"
    local status="$2"
    local details="${3:-}"
    TEST_COUNT=$((TEST_COUNT + 1))
    if [[ "${status}" == "PASS" ]]; then
        PASS_COUNT=$((PASS_COUNT + 1))
        echo -e "  [${C_GREEN}PASS${C_RESET}] ${name}"
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
        echo -e "  [${C_RED}FAIL${C_RESET}] ${name}"
        if [[ -n "${details}" ]]; then
            echo -e "         ${details}"
        fi
    fi
}

echo -e "${C_BOLD}Running Qinpanel Bootstrap Installer Tests${C_RESET}"
echo "================================================="

# Source the bootstrap script in test mode
export NUSA_PANEL_SKIP_ROOT_CHECK=1
export NUSA_PANEL_SKIP_OS_CHECK=1
# shellcheck source=scripts/bootstrap-install.sh
source "${BOOTSTRAP_SCRIPT}"

# ------------------------------------------------------------------------------
# Test 1 — Architecture Mapping
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 1: Architecture Mapping${C_RESET}"

t1_x86="$(detect_arch "x86_64")"
if [[ "${t1_x86}" == "x86_64-unknown-linux-gnu" ]]; then
    report_test "x86_64 maps to x86_64-unknown-linux-gnu" "PASS"
else
    report_test "x86_64 maps to x86_64-unknown-linux-gnu" "FAIL" "Got: ${t1_x86}"
fi

t1_amd64="$(detect_arch "amd64")"
if [[ "${t1_amd64}" == "x86_64-unknown-linux-gnu" ]]; then
    report_test "amd64 maps to x86_64-unknown-linux-gnu" "PASS"
else
    report_test "amd64 maps to x86_64-unknown-linux-gnu" "FAIL" "Got: ${t1_amd64}"
fi

t1_aarch64="$(detect_arch "aarch64")"
if [[ "${t1_aarch64}" == "aarch64-unknown-linux-gnu" ]]; then
    report_test "aarch64 maps to aarch64-unknown-linux-gnu" "PASS"
else
    report_test "aarch64 maps to aarch64-unknown-linux-gnu" "FAIL" "Got: ${t1_aarch64}"
fi

t1_arm64="$(detect_arch "arm64")"
if [[ "${t1_arm64}" == "aarch64-unknown-linux-gnu" ]]; then
    report_test "arm64 maps to aarch64-unknown-linux-gnu" "PASS"
else
    report_test "arm64 maps to aarch64-unknown-linux-gnu" "FAIL" "Got: ${t1_arm64}"
fi

if detect_arch "mips" >/dev/null 2>&1; then
    report_test "unsupported arch (mips) is rejected" "FAIL" "Expected non-zero exit code"
else
    report_test "unsupported arch (mips) is rejected" "PASS"
fi

# ------------------------------------------------------------------------------
# Test 2 — Release Tag Validation
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 2: Release Tag Validation${C_RESET}"

for valid_tag in "v0.1.0" "v1.0.0" "v1.20.3" "v12.4.7" "v1.0.0-beta.1"; do
    if validate_version "${valid_tag}" >/dev/null 2>&1; then
        report_test "valid tag accepted: ${valid_tag}" "PASS"
    else
        report_test "valid tag accepted: ${valid_tag}" "FAIL"
    fi
done

for invalid_tag in "main" "latest" "../../tmp" 'v1.0.0;rm -rf /' '$(whoami)' 'v1.0' '1.0.0'; do
    if validate_version "${invalid_tag}" >/dev/null 2>&1; then
        report_test "invalid tag rejected: ${invalid_tag}" "FAIL" "Expected non-zero exit code"
    else
        report_test "invalid tag rejected: ${invalid_tag}" "PASS"
    fi
done

# ------------------------------------------------------------------------------
# Test 3 — Artifact Naming
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 3: Artifact Naming${C_RESET}"

t3_name_x86="$(build_asset_name "v0.1.0" "x86_64-unknown-linux-gnu")"
if [[ "${t3_name_x86}" == "qinpanel-v0.1.0-x86_64-unknown-linux-gnu.tar.gz" ]]; then
    report_test "v0.1.0 + x86_64 artifact name" "PASS"
else
    report_test "v0.1.0 + x86_64 artifact name" "FAIL" "Got: ${t3_name_x86}"
fi

t3_name_arm="$(build_asset_name "v1.0.0" "aarch64-unknown-linux-gnu")"
if [[ "${t3_name_arm}" == "qinpanel-v1.0.0-aarch64-unknown-linux-gnu.tar.gz" ]]; then
    report_test "v1.0.0 + aarch64 artifact name" "PASS"
else
    report_test "v1.0.0 + aarch64 artifact name" "FAIL" "Got: ${t3_name_arm}"
fi

# ------------------------------------------------------------------------------
# Test 4 — Checksum Valid
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 4: Checksum Valid${C_RESET}"

TEST_TMP="$(mktemp -d)"
cleanup_test_tmp() {
    rm -rf "${TEST_TMP}"
}
trap cleanup_test_tmp EXIT

dummy_asset="nusa-panel-v0.1.0-x86_64-unknown-linux-gnu.tar.gz"
echo "Dummy content for testing" > "${TEST_TMP}/${dummy_asset}"
(cd "${TEST_TMP}" && sha256sum "${dummy_asset}" > SHA256SUMS)

if verify_checksum "${TEST_TMP}/SHA256SUMS" "${TEST_TMP}/${dummy_asset}" "${dummy_asset}" >/dev/null 2>&1; then
    report_test "valid artifact matches SHA256SUMS" "PASS"
else
    report_test "valid artifact matches SHA256SUMS" "FAIL"
fi

# ------------------------------------------------------------------------------
# Test 5 — Checksum Mismatch
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 5: Checksum Mismatch${C_RESET}"

echo "Tampered content" > "${TEST_TMP}/${dummy_asset}"
if verify_checksum "${TEST_TMP}/SHA256SUMS" "${TEST_TMP}/${dummy_asset}" "${dummy_asset}" >/dev/null 2>&1; then
    report_test "tampered artifact is rejected" "FAIL" "Expected non-zero exit code"
else
    report_test "tampered artifact is rejected" "PASS"
fi

# ------------------------------------------------------------------------------
# Test 6 — Unsafe Tar Paths
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 6: Unsafe Tar Paths${C_RESET}"

# Create archive with traversal entry ../../etc/passwd
mkdir -p "${TEST_TMP}/safe_dir"
echo "ok" > "${TEST_TMP}/safe_dir/file.txt"
tar -czf "${TEST_TMP}/safe.tar.gz" -C "${TEST_TMP}" safe_dir

if validate_archive "${TEST_TMP}/safe.tar.gz" >/dev/null 2>&1; then
    report_test "safe tar archive accepted" "PASS"
else
    report_test "safe tar archive accepted" "FAIL"
fi

# Create archive with ../ traversal
mkdir -p "${TEST_TMP}/traversal_test"
echo "evil" > "${TEST_TMP}/traversal_test/evil.txt"
PYTHON_BIN=""
if command -v python3 >/dev/null 2>&1 && python3 --version >/dev/null 2>&1; then
    PYTHON_BIN="python3"
elif command -v python >/dev/null 2>&1; then
    PYTHON_BIN="python"
fi

if [[ -n "${PYTHON_BIN}" ]]; then
    "${PYTHON_BIN}" -c "
import tarfile
t = tarfile.open('${TEST_TMP}/traversal.tar.gz', 'w:gz')
t.add('${TEST_TMP}/traversal_test/evil.txt', arcname='../../etc/passwd')
t.close()
" 2>/dev/null || true
fi

if validate_archive "${TEST_TMP}/traversal.tar.gz" >/dev/null 2>&1; then
    report_test "traversal tar archive rejected" "FAIL" "Expected archive with ../ to be rejected"
else
    report_test "traversal tar archive rejected" "PASS"
fi

# Test individual path entries
for bad_entry in "/etc/passwd" "/bin/sh" "../../etc/shadow" "foo/../../bar" "foo/.." ".."; do
    if validate_entry_path "${bad_entry}" >/dev/null 2>&1; then
        report_test "unsafe entry rejected: ${bad_entry}" "FAIL" "Expected non-zero exit code"
    else
        report_test "unsafe entry rejected: ${bad_entry}" "PASS"
    fi
done

for ok_entry in "nusa-panel-v0.1.0/install.sh" "./nusa-panel-v0.1.0/bin/panelctl" "templates/systemd/panel.service"; do
    if validate_entry_path "${ok_entry}" >/dev/null 2>&1; then
        report_test "safe entry accepted: ${ok_entry}" "PASS"
    else
        report_test "safe entry accepted: ${ok_entry}" "FAIL"
    fi
done

# ------------------------------------------------------------------------------
# Test 7 — Missing install.sh
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 7: Missing install.sh${C_RESET}"

mkdir -p "${TEST_TMP}/no_installer/nusa-panel-v0.1.0-x86_64-unknown-linux-gnu"
echo "dummy binary" > "${TEST_TMP}/no_installer/nusa-panel-v0.1.0-x86_64-unknown-linux-gnu/panel-api"
tar -czf "${TEST_TMP}/no_installer.tar.gz" -C "${TEST_TMP}/no_installer" nusa-panel-v0.1.0-x86_64-unknown-linux-gnu

EXTRACT_DIR="${TEST_TMP}/extracted_missing"
mkdir -p "${EXTRACT_DIR}"
if extract_and_locate_installer "${TEST_TMP}/no_installer.tar.gz" "${EXTRACT_DIR}" "v0.1.0" "x86_64-unknown-linux-gnu" >/dev/null 2>&1; then
    report_test "archive without install.sh rejected" "FAIL" "Expected failure when install.sh is absent"
else
    report_test "archive without install.sh rejected" "PASS"
fi

# Test archive with valid install.sh
mkdir -p "${TEST_TMP}/with_installer/nusa-panel-v0.1.0-x86_64-unknown-linux-gnu"
cat << 'EOF' > "${TEST_TMP}/with_installer/nusa-panel-v0.1.0-x86_64-unknown-linux-gnu/install.sh"
#!/usr/bin/env bash
exit 0
EOF
chmod +x "${TEST_TMP}/with_installer/nusa-panel-v0.1.0-x86_64-unknown-linux-gnu/install.sh"
tar -czf "${TEST_TMP}/with_installer.tar.gz" -C "${TEST_TMP}/with_installer" nusa-panel-v0.1.0-x86_64-unknown-linux-gnu

EXTRACT_VALID="${TEST_TMP}/extracted_valid"
mkdir -p "${EXTRACT_VALID}"
if extract_and_locate_installer "${TEST_TMP}/with_installer.tar.gz" "${EXTRACT_VALID}" "v0.1.0" "x86_64-unknown-linux-gnu" >/dev/null 2>&1; then
    report_test "archive with install.sh accepted" "PASS"
else
    report_test "archive with install.sh accepted" "FAIL"
fi

# ------------------------------------------------------------------------------
# Test 8 — Forward Arguments
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 8: Argument Forwarding${C_RESET}"

ARGS_DIR="${TEST_TMP}/args_test"
mkdir -p "${ARGS_DIR}"
cat << 'EOF' > "${ARGS_DIR}/install.sh"
#!/usr/bin/env bash
printf "%s\n" "$@" > recorded_args.txt
exit 0
EOF
chmod +x "${ARGS_DIR}/install.sh"

run_installer "${ARGS_DIR}" --port 8443 --unattended --force-os >/dev/null 2>&1

if [[ -f "${ARGS_DIR}/recorded_args.txt" ]]; then
    expected_args="--port
8443
--unattended
--force-os"
    actual_args="$(cat "${ARGS_DIR}/recorded_args.txt")"
    if [[ "${actual_args}" == "${expected_args}" ]]; then
        report_test "arguments forwarded correctly as separate words" "PASS"
    else
        report_test "arguments forwarded correctly as separate words" "FAIL" "Got: ${actual_args}"
    fi
else
    report_test "arguments forwarded correctly as separate words" "FAIL" "recorded_args.txt not created"
fi

# ------------------------------------------------------------------------------
# Test 9 — End-to-End Live GitHub Resolution
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 9: Live GitHub Release Resolution${C_RESET}"

# Test resolve_latest_version against live Whyriez/Qinpanel
if live_ver="$(resolve_latest_version 2>/dev/null)"; then
    if [[ "${live_ver}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+ ]]; then
        report_test "live GitHub release resolved successfully: ${live_ver}" "PASS"
    else
        report_test "live GitHub release resolved successfully" "FAIL" "Resolved: ${live_ver}"
    fi
else
    echo "  [WARN] Live GitHub API unreachable (skipping live test, offline or rate limited)"
fi

# ------------------------------------------------------------------------------
# Test 10 — Full End-to-End Execution Flow (Mock Release Server)
# ------------------------------------------------------------------------------
echo -e "\n${C_CYAN}Test 10: Full End-to-End Execution Flow${C_RESET}"

if [[ -n "${PYTHON_BIN}" ]]; then
    MOCK_ROOT="$(mktemp -d)"
    SERVER_PORT=18445
    MOCK_TARGET="x86_64-unknown-linux-gnu"
    MOCK_VERSION="v0.1.0"
    MOCK_ASSET="nusa-panel-${MOCK_VERSION}-${MOCK_TARGET}.tar.gz"

    MOCK_REL_DIR="${MOCK_ROOT}/Whyriez/Qinpanel/releases/download/${MOCK_VERSION}"
    mkdir -p "${MOCK_REL_DIR}"

    MOCK_PKG_DIR="${MOCK_ROOT}/pkg/nusa-panel-${MOCK_VERSION}-${MOCK_TARGET}"
    mkdir -p "${MOCK_PKG_DIR}"
    cat << 'EOF' > "${MOCK_PKG_DIR}/install.sh"
#!/usr/bin/env bash
echo "=== Mock Qinpanel Installer ==="
echo "Arguments: $@"
exit 0
EOF
    chmod +x "${MOCK_PKG_DIR}/install.sh"

    tar -czf "${MOCK_REL_DIR}/${MOCK_ASSET}" -C "${MOCK_ROOT}/pkg" "nusa-panel-${MOCK_VERSION}-${MOCK_TARGET}"
    (cd "${MOCK_REL_DIR}" && sha256sum "${MOCK_ASSET}" > SHA256SUMS)

    "${PYTHON_BIN}" -m http.server "${SERVER_PORT}" --directory "${MOCK_ROOT}" >/dev/null 2>&1 &
    MOCK_PID=$!
    sleep 1

    e2e_output="$(
        NUSA_PANEL_GITHUB_DOWNLOAD_URL="http://127.0.0.1:${SERVER_PORT}" \
        NUSA_PANEL_VERSION="${MOCK_VERSION}" \
        NUSA_PANEL_SKIP_ROOT_CHECK=1 \
        NUSA_PANEL_SKIP_OS_CHECK=1 \
        bash "${BOOTSTRAP_SCRIPT}" --port 9090 --unattended 2>&1
    )" || true

    kill "${MOCK_PID}" 2>/dev/null || true
    rm -rf "${MOCK_ROOT}"

    if echo "${e2e_output}" | grep -q "Mock Qinpanel Installer" && echo "${e2e_output}" | grep -q -- "--port 9090 --unattended"; then
        report_test "end-to-end download, verify, extract, and execution with arguments" "PASS"
    else
        report_test "end-to-end download, verify, extract, and execution with arguments" "FAIL" "Output: ${e2e_output}"
    fi
else
    echo "  [WARN] Python binary not available for mock server, skipping Test 10"
fi

# ------------------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------------------
echo "================================================="
echo -e "Total: ${TEST_COUNT} | Passed: ${C_GREEN}${PASS_COUNT}${C_RESET} | Failed: ${C_RED}${FAIL_COUNT}${C_RESET}"

if [[ ${FAIL_COUNT} -gt 0 ]]; then
    exit 1
fi

echo -e "\n${C_GREEN}All bootstrap installer tests passed successfully!${C_RESET}"
exit 0
