//! Benchmark harness runner, tooling integrations (oha, wrk, k6, builtin),
//! workload scenarios (static, php, wordpress, node, proxy),
//! tuning comparisons (fastcgi_cache, opcache, compression, http2/3),
//! and PRD Section 9 target validations.

use crate::error::CliError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Target workload scenarios required by PRD Section 11 (P10-T1)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkTarget {
    Static,
    PhpHelloworld,
    WordPressDynamic,
    NodeHello,
    ProxyUpstream,
}

impl BenchmarkTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::PhpHelloworld => "php_hello",
            Self::WordPressDynamic => "wordpress_dynamic",
            Self::NodeHello => "node_hello",
            Self::ProxyUpstream => "proxy_upstream",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Static => "Static Assets / HTML",
            Self::PhpHelloworld => "PHP Hello World (FastCGI)",
            Self::WordPressDynamic => "WordPress / Dynamic CMS Emulation",
            Self::NodeHello => "Node.js Native HTTP / API",
            Self::ProxyUpstream => "Reverse Proxy Upstream",
        }
    }

    pub fn default_path(&self) -> &'static str {
        match self {
            Self::Static => "/index.html",
            Self::PhpHelloworld => "/hello.php",
            Self::WordPressDynamic => "/index.php",
            Self::NodeHello => "/api/hello",
            Self::ProxyUpstream => "/proxy-test",
        }
    }
}

impl FromStr for BenchmarkTarget {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "static" | "static_html" => Ok(Self::Static),
            "php" | "php_hello" | "php_helloworld" => Ok(Self::PhpHelloworld),
            "wordpress" | "wp" | "wordpress_dynamic" | "cms" => Ok(Self::WordPressDynamic),
            "node" | "nodejs" | "node_hello" => Ok(Self::NodeHello),
            "proxy" | "proxy_upstream" | "reverse_proxy" => Ok(Self::ProxyUpstream),
            _ => Err(CliError::Validation(format!(
                "Unknown benchmark target: '{}'. Valid values: static, php, wordpress, node, proxy",
                s
            ))),
        }
    }
}

impl fmt::Display for BenchmarkTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Tuning profiles to measure and compare optimizations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningProfile {
    Baseline,
    OpcacheEnabled,
    FastcgiCacheHit,
    FastcgiCacheBypass,
    CompressionGzip,
    CompressionBrotli,
    Http2,
    Http3Quic,
}

impl TuningProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::OpcacheEnabled => "opcache_enabled",
            Self::FastcgiCacheHit => "fastcgi_cache_hit",
            Self::FastcgiCacheBypass => "fastcgi_cache_bypass",
            Self::CompressionGzip => "compression_gzip",
            Self::CompressionBrotli => "compression_brotli",
            Self::Http2 => "http2",
            Self::Http3Quic => "http3_quic",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Baseline => "Baseline (No Tuning)",
            Self::OpcacheEnabled => "With OPcache + JIT",
            Self::FastcgiCacheHit => "With FastCGI Microcache (HIT)",
            Self::FastcgiCacheBypass => "With FastCGI Cache (BYPASS)",
            Self::CompressionGzip => "With Gzip Compression",
            Self::CompressionBrotli => "With Brotli Compression",
            Self::Http2 => "HTTP/2 Multiplexed",
            Self::Http3Quic => "HTTP/3 over QUIC",
        }
    }
}

impl FromStr for TuningProfile {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "baseline" | "default" | "none" => Ok(Self::Baseline),
            "opcache" | "opcache_enabled" => Ok(Self::OpcacheEnabled),
            "fastcgi_cache" | "fastcgi_cache_hit" | "cache_hit" => Ok(Self::FastcgiCacheHit),
            "fastcgi_cache_bypass" | "cache_bypass" => Ok(Self::FastcgiCacheBypass),
            "gzip" | "compression_gzip" => Ok(Self::CompressionGzip),
            "brotli" | "compression_brotli" => Ok(Self::CompressionBrotli),
            "h2" | "http2" => Ok(Self::Http2),
            "h3" | "http3" | "quic" | "http3_quic" => Ok(Self::Http3Quic),
            _ => Err(CliError::Validation(format!(
                "Unknown tuning profile: '{}'. Valid values: baseline, opcache, fastcgi_cache, gzip, brotli, http2, http3",
                s
            ))),
        }
    }
}

impl fmt::Display for TuningProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Benchmark tool selector
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkTool {
    Auto,
    Oha,
    Wrk,
    K6,
    Builtin,
}

impl FromStr for BenchmarkTool {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "oha" => Ok(Self::Oha),
            "wrk" => Ok(Self::Wrk),
            "k6" => Ok(Self::K6),
            "builtin" | "native" => Ok(Self::Builtin),
            _ => Err(CliError::Validation(format!(
                "Unknown benchmark tool: '{}'. Valid values: auto, oha, wrk, k6, builtin",
                s
            ))),
        }
    }
}

/// Detailed percentile latency distribution in milliseconds
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyDistribution {
    pub min_ms: f64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl LatencyDistribution {
    pub fn from_sorted_samples(mut samples: Vec<f64>) -> Self {
        if samples.is_empty() {
            return Self {
                min_ms: 0.0,
                avg_ms: 0.0,
                p50_ms: 0.0,
                p90_ms: 0.0,
                p95_ms: 0.0,
                p99_ms: 0.0,
                max_ms: 0.0,
            };
        }

        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let len = samples.len();
        let sum: f64 = samples.iter().sum();
        let avg_ms = sum / len as f64;
        let min_ms = samples[0];
        let max_ms = samples[len - 1];

        let p50_ms = samples[((len as f64 * 0.50).round() as usize).min(len - 1)];
        let p90_ms = samples[((len as f64 * 0.90).round() as usize).min(len - 1)];
        let p95_ms = samples[((len as f64 * 0.95).round() as usize).min(len - 1)];
        let p99_ms = samples[((len as f64 * 0.99).round() as usize).min(len - 1)];

        Self {
            min_ms,
            avg_ms,
            p50_ms,
            p90_ms,
            p95_ms,
            p99_ms,
            max_ms,
        }
    }
}

/// Comprehensive benchmark execution result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkRunResult {
    pub scenario: BenchmarkTarget,
    pub target_url: String,
    pub tuning_profile: TuningProfile,
    pub tool_used: String,
    pub concurrency: usize,
    pub duration_secs: f64,
    pub total_requests: usize,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub requests_per_sec: f64,
    pub transfer_rate_bytes_sec: f64,
    pub latency: LatencyDistribution,
    pub cpu_usage_percent: Option<f64>,
    pub memory_rss_mb: Option<f64>,
    pub executed_at: i64,
}

/// Comparative evaluation between baseline and tuned runs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkComparison {
    pub scenario: BenchmarkTarget,
    pub baseline_profile: TuningProfile,
    pub tuned_profile: TuningProfile,
    pub baseline_rps: f64,
    pub tuned_rps: f64,
    pub rps_delta_percent: f64,
    pub baseline_p95_ms: f64,
    pub tuned_p95_ms: f64,
    pub p95_latency_delta_percent: f64,
    pub baseline_memory_mb: Option<f64>,
    pub tuned_memory_mb: Option<f64>,
}

impl BenchmarkComparison {
    pub fn compute(baseline: &BenchmarkRunResult, tuned: &BenchmarkRunResult) -> Self {
        let rps_delta_percent = if baseline.requests_per_sec > 0.0 {
            ((tuned.requests_per_sec - baseline.requests_per_sec) / baseline.requests_per_sec)
                * 100.0
        } else {
            0.0
        };

        let p95_latency_delta_percent = if baseline.latency.p95_ms > 0.0 {
            ((tuned.latency.p95_ms - baseline.latency.p95_ms) / baseline.latency.p95_ms) * 100.0
        } else {
            0.0
        };

        Self {
            scenario: baseline.scenario,
            baseline_profile: baseline.tuning_profile,
            tuned_profile: tuned.tuning_profile,
            baseline_rps: baseline.requests_per_sec,
            tuned_rps: tuned.requests_per_sec,
            rps_delta_percent,
            baseline_p95_ms: baseline.latency.p95_ms,
            tuned_p95_ms: tuned.latency.p95_ms,
            p95_latency_delta_percent,
            baseline_memory_mb: baseline.memory_rss_mb,
            tuned_memory_mb: tuned.memory_rss_mb,
        }
    }
}

/// Non-functional performance target verification item (PRD Section 9)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetMetricStatus {
    pub target_id: String,
    pub name: String,
    pub threshold: String,
    pub measured: String,
    pub passed: bool,
}

/// Comprehensive Non-Functional Target Validation Report
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NonFunctionalTargetsReport {
    pub results: Vec<TargetMetricStatus>,
    pub all_passed: bool,
    pub evaluated_at: i64,
}

/// Input parameters for target validation (PRD Section 9)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeasuredSystemMetrics {
    pub ram_idle_api_mb: Option<f64>,
    pub ram_idle_agent_mb: Option<f64>,
    pub ram_panel_medium_load_mb: Option<f64>,
    pub cold_start_api_ms: Option<u64>,
    pub api_read_latency_p95_ms: Option<f64>,
    pub site_create_static_secs: Option<f64>,
    pub site_create_php_secs: Option<f64>,
    pub reload_nginx_drops_count: Option<u64>,
    pub config_rollback_success_rate_percent: Option<f64>,
    pub log_ingest_rate_lines_sec: Option<f64>,
}

/// Options to configure benchmark runs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkOptions {
    pub scenario: BenchmarkTarget,
    pub target_url: String,
    pub tuning_profile: TuningProfile,
    pub concurrency: usize,
    pub duration_secs: u64,
    pub total_requests: Option<usize>,
    pub warmup_secs: u64,
    pub tool: BenchmarkTool,
    pub headers: Vec<(String, String)>,
}

impl Default for BenchmarkOptions {
    fn default() -> Self {
        Self {
            scenario: BenchmarkTarget::Static,
            target_url: "http://127.0.0.1:8080/index.html".to_string(),
            tuning_profile: TuningProfile::Baseline,
            concurrency: 50,
            duration_secs: 10,
            total_requests: None,
            warmup_secs: 1,
            tool: BenchmarkTool::Auto,
            headers: vec![(
                "User-Agent".to_string(),
                "nusa-benchmark-harness/1.0".to_string(),
            )],
        }
    }
}

/// Detect availability of external benchmark tools
pub fn detect_available_tools() -> (bool, bool, bool) {
    let has_oha = which("oha");
    let has_wrk = which("wrk");
    let has_k6 = which("k6");
    (has_oha, has_wrk, has_k6)
}

fn which(cmd: &str) -> bool {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let is_windows = cfg!(target_os = "windows");
    for p in std::env::split_paths(&path_var) {
        let exe_path = p.join(cmd);
        if exe_path.is_file() {
            return true;
        }
        if is_windows {
            let exe_with_ext = p.join(format!("{}.exe", cmd));
            if exe_with_ext.is_file() {
                return true;
            }
        }
    }
    false
}

/// Builtin asynchronous HTTP load generator (zero external tool dependency)
pub async fn run_builtin_benchmark(
    options: &BenchmarkOptions,
) -> Result<BenchmarkRunResult, CliError> {
    let target_url = &options.target_url;
    let (host, port, path) = parse_url(target_url)?;

    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Optional Warmup
    if options.warmup_secs > 0 {
        let warmup_deadline = Instant::now() + Duration::from_secs(options.warmup_secs);
        while Instant::now() < warmup_deadline {
            let _ = make_http_probe(&host, port, &path, &options.headers).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    let start_instant = Instant::now();
    let total_duration = Duration::from_secs(options.duration_secs);
    let deadline = start_instant + total_duration;

    let concurrency = options.concurrency.max(1);
    let (tx_res, mut rx_res) = tokio::sync::mpsc::channel::<(bool, f64, usize)>(concurrency * 100);

    for _ in 0..concurrency {
        let tx = tx_res.clone();
        let host_clone = host.clone();
        let path_clone = path.clone();
        let headers_clone = options.headers.clone();

        tokio::spawn(async move {
            while Instant::now() < deadline {
                let req_start = Instant::now();
                match make_http_probe(&host_clone, port, &path_clone, &headers_clone).await {
                    Ok(bytes_received) => {
                        let latency_ms = req_start.elapsed().as_secs_f64() * 1000.0;
                        if tx.send((true, latency_ms, bytes_received)).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let latency_ms = req_start.elapsed().as_secs_f64() * 1000.0;
                        if tx.send((false, latency_ms, 0)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    drop(tx_res);

    let mut samples = Vec::new();
    let mut total_requests = 0usize;
    let mut successful_requests = 0usize;
    let mut failed_requests = 0usize;
    let mut total_bytes = 0usize;

    while let Some((success, latency_ms, bytes)) = rx_res.recv().await {
        total_requests += 1;
        if success {
            successful_requests += 1;
            total_bytes += bytes;
        } else {
            failed_requests += 1;
        }
        samples.push(latency_ms);
    }

    let actual_duration = start_instant.elapsed().as_secs_f64();
    let requests_per_sec = if actual_duration > 0.0 {
        total_requests as f64 / actual_duration
    } else {
        0.0
    };
    let transfer_rate_bytes_sec = if actual_duration > 0.0 {
        total_bytes as f64 / actual_duration
    } else {
        0.0
    };

    let latency = LatencyDistribution::from_sorted_samples(samples);

    Ok(BenchmarkRunResult {
        scenario: options.scenario,
        target_url: options.target_url.clone(),
        tuning_profile: options.tuning_profile,
        tool_used: "builtin-tokio-http".to_string(),
        concurrency: options.concurrency,
        duration_secs: actual_duration,
        total_requests,
        successful_requests,
        failed_requests,
        requests_per_sec,
        transfer_rate_bytes_sec,
        latency,
        cpu_usage_percent: None,
        memory_rss_mb: read_current_process_rss_mb(),
        executed_at: now_ts,
    })
}

/// Helper function to parse HTTP URL into host, port, path
fn parse_url(url_str: &str) -> Result<(String, u16, String), CliError> {
    let clean = url_str
        .strip_prefix("http://")
        .or_else(|| url_str.strip_prefix("https://"))
        .unwrap_or(url_str);

    let (host_port, path) = match clean.find('/') {
        Some(idx) => (&clean[..idx], &clean[idx..]),
        None => (clean, "/"),
    };

    let (host, port) = match host_port.find(':') {
        Some(idx) => {
            let h = &host_port[..idx];
            let p = host_port[idx + 1..]
                .parse::<u16>()
                .map_err(|_| CliError::Validation(format!("Invalid port in URL: {}", url_str)))?;
            (h.to_string(), p)
        }
        None => (
            host_port.to_string(),
            if url_str.starts_with("https://") {
                443
            } else {
                80
            },
        ),
    };

    Ok((host, port, path.to_string()))
}

/// Asynchronous raw TCP HTTP 1.1 request probe
async fn make_http_probe(
    host: &str,
    port: u16,
    path: &str,
    headers: &[(String, String)],
) -> Result<usize, CliError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let addr = format!("{}:{}", host, port);
    let mut stream = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(&addr))
        .await
        .map_err(|_| CliError::Other("TCP connection timeout".to_string()))?
        .map_err(CliError::Io)?;

    let mut request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        path, host
    );
    for (k, v) in headers {
        request.push_str(&format!("{}: {}\r\n", k, v));
    }
    request.push_str("\r\n");

    stream
        .write_all(request.as_bytes())
        .await
        .map_err(CliError::Io)?;

    let mut buf = [0u8; 4096];
    let mut total_read = 0usize;
    let mut status_ok = false;
    let mut checked_status = false;

    while let Ok(n) = stream.read(&mut buf).await {
        if n == 0 {
            break;
        }
        if !checked_status {
            let head = String::from_utf8_lossy(&buf[..n]);
            if head.starts_with("HTTP/1.1 2")
                || head.starts_with("HTTP/1.0 2")
                || head.starts_with("HTTP/1.1 3")
            {
                status_ok = true;
            }
            checked_status = true;
        }
        total_read += n;
    }

    if !status_ok && checked_status {
        return Err(CliError::Other(
            "HTTP status returned non-2xx/3xx".to_string(),
        ));
    }

    Ok(total_read)
}

/// Read process RSS in MB (Linux /proc or fallback)
fn read_current_process_rss_mb() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let parts: Vec<&str> = statm.split_whitespace().collect();
            if parts.len() >= 2
                && let Ok(resident_pages) = parts[1].parse::<u64>()
            {
                let page_size_kb = 4u64; // typical x86_64/arm page size
                let rss_mb = (resident_pages * page_size_kb) as f64 / 1024.0;
                return Some(rss_mb);
            }
        }
    }
    None
}

/// Evaluate non-functional performance targets as mandated by PRD Section 9
pub fn validate_non_functional_targets(
    metrics: &MeasuredSystemMetrics,
) -> NonFunctionalTargetsReport {
    let mut results = Vec::new();

    // 1. RAM idle panel-api (<= 40 MB RSS)
    if let Some(rss) = metrics.ram_idle_api_mb {
        results.push(TargetMetricStatus {
            target_id: "G6-RAM-API".to_string(),
            name: "RAM Idle panel-api".to_string(),
            threshold: "≤ 40.0 MB RSS".to_string(),
            measured: format!("{:.1} MB", rss),
            passed: rss <= 40.0,
        });
    }

    // 2. RAM idle panel-agent (<= 20 MB RSS)
    if let Some(rss) = metrics.ram_idle_agent_mb {
        results.push(TargetMetricStatus {
            target_id: "G6-RAM-AGENT".to_string(),
            name: "RAM Idle panel-agent".to_string(),
            threshold: "≤ 20.0 MB RSS".to_string(),
            measured: format!("{:.1} MB", rss),
            passed: rss <= 20.0,
        });
    }

    // 3. RAM panel total saat beban sedang (<= 150 MB)
    if let Some(rss) = metrics.ram_panel_medium_load_mb {
        results.push(TargetMetricStatus {
            target_id: "G6-RAM-TOTAL".to_string(),
            name: "RAM Panel Beban Sedang (100 situs, 10 sesi)".to_string(),
            threshold: "≤ 150.0 MB Total".to_string(),
            measured: format!("{:.1} MB", rss),
            passed: rss <= 150.0,
        });
    }

    // 4. Start cold panel-api (< 1 detik / 1000 ms)
    if let Some(ms) = metrics.cold_start_api_ms {
        results.push(TargetMetricStatus {
            target_id: "G6-START-COLD".to_string(),
            name: "Cold Start panel-api".to_string(),
            threshold: "< 1000 ms".to_string(),
            measured: format!("{} ms", ms),
            passed: ms < 1000,
        });
    }

    // 5. Latensi API baca p95 (< 50 ms)
    if let Some(p95) = metrics.api_read_latency_p95_ms {
        results.push(TargetMetricStatus {
            target_id: "G6-API-READ-LATENCY".to_string(),
            name: "Latensi API Baca (p95)".to_string(),
            threshold: "< 50.0 ms".to_string(),
            measured: format!("{:.2} ms", p95),
            passed: p95 < 50.0,
        });
    }

    // 6. Buat site static (< 5 s)
    if let Some(secs) = metrics.site_create_static_secs {
        results.push(TargetMetricStatus {
            target_id: "G6-CREATE-STATIC".to_string(),
            name: "Buat Situs Static".to_string(),
            threshold: "< 5.0 detik".to_string(),
            measured: format!("{:.2} s", secs),
            passed: secs < 5.0,
        });
    }

    // 7. Buat site PHP (< 10 s)
    if let Some(secs) = metrics.site_create_php_secs {
        results.push(TargetMetricStatus {
            target_id: "G6-CREATE-PHP".to_string(),
            name: "Buat Situs PHP (Master & Pool Init)".to_string(),
            threshold: "< 10.0 detik".to_string(),
            measured: format!("{:.2} s", secs),
            passed: secs < 10.0,
        });
    }

    // 8. Reload Nginx koneksi putus (0 koneksi)
    if let Some(drops) = metrics.reload_nginx_drops_count {
        results.push(TargetMetricStatus {
            target_id: "G6-NGINX-RELOAD-DROPS".to_string(),
            name: "Koneksi Putus saat Nginx Reload".to_string(),
            threshold: "0 koneksi terputus".to_string(),
            measured: format!("{} drops", drops),
            passed: drops == 0,
        });
    }

    // 9. Gagal apply config rollback (100% rollback)
    if let Some(rate) = metrics.config_rollback_success_rate_percent {
        results.push(TargetMetricStatus {
            target_id: "G6-CONFIG-ROLLBACK".to_string(),
            name: "Tingkat Keberhasilan Rollback Config Rusak".to_string(),
            threshold: "100.0%".to_string(),
            measured: format!("{:.1}%", rate),
            passed: rate >= 100.0,
        });
    }

    // 10. Ingest log (>= 5.000 baris/detik)
    if let Some(rate) = metrics.log_ingest_rate_lines_sec {
        results.push(TargetMetricStatus {
            target_id: "G6-LOG-INGEST".to_string(),
            name: "Throughput Ingest Log Nginx".to_string(),
            threshold: "≥ 5000 baris/detik".to_string(),
            measured: format!("{:.0} baris/s", rate),
            passed: rate >= 5000.0,
        });
    }

    let all_passed = results.iter().all(|r| r.passed);
    let evaluated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    NonFunctionalTargetsReport {
        results,
        all_passed,
        evaluated_at,
    }
}

/// Generate comprehensive Markdown report for docs/benchmarks/
pub fn generate_markdown_report(
    runs: &[BenchmarkRunResult],
    comparisons: &[BenchmarkComparison],
    target_report: Option<&NonFunctionalTargetsReport>,
) -> String {
    let mut out = String::new();

    out.push_str("# Laporan Benchmark & Validasi Kinerja Qinpanel\n\n");
    out.push_str("> Dokumen ini mencatat baseline performa, throughput (RPS), distribusi latensi p50/p95/p99,\n");
    out.push_str("> perbandingan profil tuning (OPcache, FastCGI Cache, kompresi, HTTP/2/3),\n");
    out.push_str("> dan pembuktian target non-fungsional PRD Bagian 9.\n\n");

    out.push_str("## 1. Ringkasan Baseline Beban Kerja (5 Skenario)\n\n");
    out.push_str("| Skenario | Profil | Tool | Concurrency | RPS (Req/s) | p50 (ms) | p95 (ms) | p99 (ms) | Sukses (%) |\n");
    out.push_str("| :--- | :--- | :--- | :---: | :---: | :---: | :---: | :---: | :---: |\n");

    for r in runs {
        let success_rate = if r.total_requests > 0 {
            (r.successful_requests as f64 / r.total_requests as f64) * 100.0
        } else {
            0.0
        };
        out.push_str(&format!(
            "| **{}** | {} | `{}` | {} | **{:.1}** | {:.2} | {:.2} | {:.2} | {:.1}% |\n",
            r.scenario.display_name(),
            r.tuning_profile.display_name(),
            r.tool_used,
            r.concurrency,
            r.requests_per_sec,
            r.latency.p50_ms,
            r.latency.p95_ms,
            r.latency.p99_ms,
            success_rate
        ));
    }
    out.push('\n');

    if !comparisons.is_empty() {
        out.push_str("## 2. Perbandingan Profil Tuning & Akselerasi\n\n");
        out.push_str("| Skenario | Baseline Profil | Tuned Profil | Baseline RPS | Tuned RPS | Delta RPS (%) | Delta p95 (%) |\n");
        out.push_str("| :--- | :--- | :--- | :---: | :---: | :---: | :---: |\n");

        for c in comparisons {
            let rps_sign = if c.rps_delta_percent >= 0.0 { "+" } else { "" };
            let lat_sign = if c.p95_latency_delta_percent <= 0.0 {
                ""
            } else {
                "+"
            };

            out.push_str(&format!(
                "| **{}** | {} | **{}** | {:.1} | {:.1} | **{}{:.1}%** | {}{:.1}% |\n",
                c.scenario.display_name(),
                c.baseline_profile.display_name(),
                c.tuned_profile.display_name(),
                c.baseline_rps,
                c.tuned_rps,
                rps_sign,
                c.rps_delta_percent,
                lat_sign,
                c.p95_latency_delta_percent
            ));
        }
        out.push('\n');
    }

    if let Some(target_rep) = target_report {
        out.push_str("## 3. Validasi Target Non-Fungsional (PRD Bagian 9)\n\n");
        out.push_str("| ID | Area Target | Target Batas | Hasil Terukur | Status |\n");
        out.push_str("| :--- | :--- | :--- | :--- | :---: |\n");

        for t in &target_rep.results {
            let status = if t.passed { "PASS" } else { "FAIL" };
            out.push_str(&format!(
                "| `{}` | {} | {} | `{}` | **{}** |\n",
                t.target_id, t.name, t.threshold, t.measured, status
            ));
        }
        out.push('\n');

        let summary = if target_rep.all_passed {
            "Semua target performa dan efisiensi memori PRD Bagian 9 TERPENUHI (PASS 100%)."
        } else {
            "Terdapat target performa yang belum memenuhi ambang batas batas (Perlu Optimasi)."
        };
        out.push_str(&format!("**Kesimpulan Validasi:** {}\n\n", summary));
    }

    out
}

/// Save benchmark run results and comparisons to JSON
pub fn export_results_to_json(
    runs: &[BenchmarkRunResult],
    comparisons: &[BenchmarkComparison],
    target_report: Option<&NonFunctionalTargetsReport>,
    path: &Path,
) -> Result<(), CliError> {
    #[derive(Serialize)]
    struct ExportEnvelope<'a> {
        runs: &'a [BenchmarkRunResult],
        comparisons: &'a [BenchmarkComparison],
        target_report: Option<&'a NonFunctionalTargetsReport>,
        exported_at: i64,
    }

    let envelope = ExportEnvelope {
        runs,
        comparisons,
        target_report,
        exported_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
    };

    let json_bytes = serde_json::to_vec_pretty(&envelope)
        .map_err(|e| CliError::Other(format!("Failed to serialize benchmark results: {}", e)))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(CliError::Io)?;
    }
    std::fs::write(path, json_bytes).map_err(CliError::Io)?;

    Ok(())
}
