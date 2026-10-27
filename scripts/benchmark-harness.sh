#!/usr/bin/env bash
# scripts/benchmark-harness.sh: Automated Benchmark Harness for Qinpanel (P10-T1)
# Measures HTTP throughput, latency percentiles (p50/p95/p99), tuning comparisons,
# and verifies PRD Section 9 Non-Functional Targets.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${REPO_DIR}/docs/benchmarks"
RESULTS_DIR="${OUTPUT_DIR}/results"
mkdir -p "${OUTPUT_DIR}" "${RESULTS_DIR}"

SCENARIO="all"
TUNING="baseline"
TOOL="auto"
CONCURRENCY="50"
DURATION="10"
WARMUP="2"
URL_BASE="http://127.0.0.1:8080"
VALIDATE_TARGETS=false

print_help() {
    echo "Qinpanel Benchmark Harness Runner (P10-T1)"
    echo ""
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  --scenario <all|static|php|wordpress|node|proxy>   Workload scenario to run (default: all)"
    echo "  --tuning <baseline|opcache|fastcgi-cache|gzip|brotli|http2|http3> Tuning profile (default: baseline)"
    echo "  --tool <auto|oha|wrk|k6|builtin>                   Tool to generate load (default: auto)"
    echo "  --concurrency <N>                                  Concurrent workers (default: 50)"
    echo "  --duration <N>                                     Duration in seconds (default: 10)"
    echo "  --warmup <N>                                       Warmup time in seconds (default: 2)"
    echo "  --url <URL>                                        Base URL of test server (default: http://127.0.0.1:8080)"
    echo "  --output-dir <DIR>                                 Output directory for reports (default: docs/benchmarks)"
    echo "  --validate-targets                                 Run PRD Section 9 target validations"
    echo "  --help                                             Show this help message"
    echo ""
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --scenario)
            SCENARIO="$2"
            shift 2
            ;;
        --tuning)
            TUNING="$2"
            shift 2
            ;;
        --tool)
            TOOL="$2"
            shift 2
            ;;
        --concurrency)
            CONCURRENCY="$2"
            shift 2
            ;;
        --duration)
            DURATION="$2"
            shift 2
            ;;
        --warmup)
            WARMUP="$2"
            shift 2
            ;;
        --url)
            URL_BASE="$2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            RESULTS_DIR="${OUTPUT_DIR}/results"
            mkdir -p "${OUTPUT_DIR}" "${RESULTS_DIR}"
            shift 2
            ;;
        --validate-targets)
            VALIDATE_TARGETS=true
            shift
            ;;
        --help|-h)
            print_help
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            print_help
            exit 1
            ;;
    esac
done

echo "=========================================================="
echo " Qinpanel Performance Benchmark Harness (Phase 10 / P10-T1)"
echo "=========================================================="
echo "Repo Dir:     ${REPO_DIR}"
echo "Scenario:     ${SCENARIO}"
echo "Tuning:       ${TUNING}"
echo "Concurrency:  ${CONCURRENCY} workers"
echo "Duration:     ${DURATION}s (Warmup: ${WARMUP}s)"
echo "Base URL:     ${URL_BASE}"
echo "Output Dir:   ${OUTPUT_DIR}"
echo "=========================================================="

# Check tool availability
cd "${REPO_DIR}"
echo "==> Detecting load testing tools..."
cargo run -p panel-cli -- benchmark tools

if [ "$VALIDATE_TARGETS" = true ]; then
    echo ""
    echo "==> Validating PRD Section 9 Non-Functional Performance Targets..."
    cargo run -p panel-cli -- benchmark validate-targets --output "${OUTPUT_DIR}/targets-validation.md"
fi

run_single_benchmark() {
    local sc="$1"
    local tn="$2"
    local endpoint="$3"
    local target_url="${URL_BASE}${endpoint}"
    local json_out="${RESULTS_DIR}/${sc}-${tn}.json"

    echo ""
    echo "----------------------------------------------------------"
    echo "Running Workload: ${sc} | Tuning: ${tn}"
    echo "Target URL:       ${target_url}"
    echo "----------------------------------------------------------"

    cargo run -p panel-cli -- benchmark run \
        --url "${target_url}" \
        --scenario "${sc}" \
        --tuning "${tn}" \
        --concurrency "${CONCURRENCY}" \
        --duration "${DURATION}" \
        --warmup "${WARMUP}" \
        --output "${json_out}"
}

if [ "$SCENARIO" = "all" ] || [ "$SCENARIO" = "static" ]; then
    run_single_benchmark "static" "baseline" "/index.html"
    run_single_benchmark "static" "gzip" "/index.html"
fi

if [ "$SCENARIO" = "all" ] || [ "$SCENARIO" = "php" ]; then
    run_single_benchmark "php" "baseline" "/hello.php"
    run_single_benchmark "php" "opcache" "/hello.php"
    run_single_benchmark "php" "fastcgi-cache" "/hello.php"
fi

if [ "$SCENARIO" = "all" ] || [ "$SCENARIO" = "wordpress" ]; then
    run_single_benchmark "wordpress" "baseline" "/index.php"
    run_single_benchmark "wordpress" "fastcgi-cache" "/index.php"
fi

if [ "$SCENARIO" = "all" ] || [ "$SCENARIO" = "node" ]; then
    run_single_benchmark "node" "baseline" "/api/hello"
    run_single_benchmark "node" "http2" "/api/hello"
fi

if [ "$SCENARIO" = "all" ] || [ "$SCENARIO" = "proxy" ]; then
    run_single_benchmark "proxy" "baseline" "/proxy-test"
fi

# Compare PHP baseline vs FastCGI Cache
if [ -f "${RESULTS_DIR}/php-baseline.json" ] && [ -f "${RESULTS_DIR}/php-fastcgi-cache.json" ]; then
    echo ""
    echo "==> Computing Tuning Comparison (PHP Baseline vs FastCGI Microcache)..."
    cargo run -p panel-cli -- benchmark compare \
        --baseline "${RESULTS_DIR}/php-baseline.json" \
        --tuned "${RESULTS_DIR}/php-fastcgi-cache.json" \
        --output "${OUTPUT_DIR}/php-cache-comparison.md"
fi

echo ""
echo "=========================================================="
echo " Benchmark Harness Execution Complete."
echo " Reports and raw metrics exported to: ${OUTPUT_DIR}/"
echo "=========================================================="
