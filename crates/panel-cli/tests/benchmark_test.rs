#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_cli::{
    BenchmarkComparison, BenchmarkOptions, BenchmarkRunResult, BenchmarkTarget, BenchmarkTool,
    LatencyDistribution, MeasuredSystemMetrics, TuningProfile, export_results_to_json,
    generate_markdown_report, run_builtin_benchmark, validate_non_functional_targets,
};
use std::str::FromStr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[test]
fn test_benchmark_targets_and_tuning_profiles_parsing() {
    assert_eq!(
        BenchmarkTarget::from_str("static").unwrap(),
        BenchmarkTarget::Static
    );
    assert_eq!(
        BenchmarkTarget::from_str("php-hello").unwrap(),
        BenchmarkTarget::PhpHelloworld
    );
    assert_eq!(
        BenchmarkTarget::from_str("wordpress").unwrap(),
        BenchmarkTarget::WordPressDynamic
    );
    assert_eq!(
        BenchmarkTarget::from_str("node").unwrap(),
        BenchmarkTarget::NodeHello
    );
    assert_eq!(
        BenchmarkTarget::from_str("proxy").unwrap(),
        BenchmarkTarget::ProxyUpstream
    );
    assert!(BenchmarkTarget::from_str("invalid_scenario").is_err());

    assert_eq!(
        TuningProfile::from_str("baseline").unwrap(),
        TuningProfile::Baseline
    );
    assert_eq!(
        TuningProfile::from_str("opcache").unwrap(),
        TuningProfile::OpcacheEnabled
    );
    assert_eq!(
        TuningProfile::from_str("fastcgi-cache").unwrap(),
        TuningProfile::FastcgiCacheHit
    );
    assert_eq!(
        TuningProfile::from_str("gzip").unwrap(),
        TuningProfile::CompressionGzip
    );
    assert_eq!(
        TuningProfile::from_str("brotli").unwrap(),
        TuningProfile::CompressionBrotli
    );
    assert_eq!(
        TuningProfile::from_str("http2").unwrap(),
        TuningProfile::Http2
    );
    assert_eq!(
        TuningProfile::from_str("http3").unwrap(),
        TuningProfile::Http3Quic
    );
}

#[test]
fn test_latency_distribution_percentiles() {
    let empty = LatencyDistribution::from_sorted_samples(vec![]);
    assert_eq!(empty.avg_ms, 0.0);
    assert_eq!(empty.p50_ms, 0.0);

    // 100 sample latensi terurut: 1.0 ms .. 100.0 ms
    let samples: Vec<f64> = (1..=100).map(|i| i as f64).collect();
    let dist = LatencyDistribution::from_sorted_samples(samples);

    assert_eq!(dist.min_ms, 1.0);
    assert_eq!(dist.max_ms, 100.0);
    assert!((dist.avg_ms - 50.5).abs() < 0.01);
    assert!((dist.p50_ms - 50.0).abs() <= 1.0);
    assert!((dist.p90_ms - 90.0).abs() <= 1.0);
    assert!((dist.p95_ms - 95.0).abs() <= 1.0);
    assert!((dist.p99_ms - 99.0).abs() <= 1.0);
}

#[test]
fn test_benchmark_comparison_computation() {
    let baseline = BenchmarkRunResult {
        scenario: BenchmarkTarget::PhpHelloworld,
        target_url: "http://127.0.0.1:8080/hello.php".to_string(),
        tuning_profile: TuningProfile::Baseline,
        tool_used: "builtin-tokio-http".to_string(),
        concurrency: 50,
        duration_secs: 10.0,
        total_requests: 10000,
        successful_requests: 10000,
        failed_requests: 0,
        requests_per_sec: 1000.0,
        transfer_rate_bytes_sec: 500_000.0,
        latency: LatencyDistribution {
            min_ms: 1.0,
            avg_ms: 12.0,
            p50_ms: 10.0,
            p90_ms: 18.0,
            p95_ms: 22.0,
            p99_ms: 35.0,
            max_ms: 50.0,
        },
        cpu_usage_percent: Some(45.0),
        memory_rss_mb: Some(40.0),
        executed_at: 1700000000,
    };

    let tuned = BenchmarkRunResult {
        scenario: BenchmarkTarget::PhpHelloworld,
        target_url: "http://127.0.0.1:8080/hello.php".to_string(),
        tuning_profile: TuningProfile::FastcgiCacheHit,
        tool_used: "builtin-tokio-http".to_string(),
        concurrency: 50,
        duration_secs: 10.0,
        total_requests: 45000,
        successful_requests: 45000,
        failed_requests: 0,
        requests_per_sec: 4500.0, // 4.5x faster (+350%)
        transfer_rate_bytes_sec: 2_250_000.0,
        latency: LatencyDistribution {
            min_ms: 0.2,
            avg_ms: 2.1,
            p50_ms: 1.5,
            p90_ms: 3.0,
            p95_ms: 4.4, // 5x lower latency (-80%)
            p99_ms: 6.0,
            max_ms: 12.0,
        },
        cpu_usage_percent: Some(25.0),
        memory_rss_mb: Some(42.0),
        executed_at: 1700000010,
    };

    let comp = BenchmarkComparison::compute(&baseline, &tuned);
    assert_eq!(comp.scenario, BenchmarkTarget::PhpHelloworld);
    assert_eq!(comp.baseline_profile, TuningProfile::Baseline);
    assert_eq!(comp.tuned_profile, TuningProfile::FastcgiCacheHit);
    assert!((comp.rps_delta_percent - 350.0).abs() < 0.1);
    assert!((comp.p95_latency_delta_percent - (-80.0)).abs() < 0.1);
}

#[test]
fn test_validate_non_functional_targets_all_pass() {
    let metrics = MeasuredSystemMetrics {
        ram_idle_api_mb: Some(34.2),                       // <= 40
        ram_idle_agent_mb: Some(16.5),                     // <= 20
        ram_panel_medium_load_mb: Some(118.0),             // <= 150
        cold_start_api_ms: Some(420),                      // < 1000
        api_read_latency_p95_ms: Some(14.8),               // < 50
        site_create_static_secs: Some(1.85),               // < 5
        site_create_php_secs: Some(4.10),                  // < 10
        reload_nginx_drops_count: Some(0),                 // == 0
        config_rollback_success_rate_percent: Some(100.0), // >= 100
        log_ingest_rate_lines_sec: Some(6850.0),           // >= 5000
    };

    let report = validate_non_functional_targets(&metrics);
    assert!(report.all_passed);
    assert_eq!(report.results.len(), 10);
    for r in &report.results {
        assert!(r.passed, "Target {} should pass", r.name);
    }
}

#[test]
fn test_validate_non_functional_targets_with_failures() {
    let metrics = MeasuredSystemMetrics {
        ram_idle_api_mb: Some(55.0), // Fails: > 40
        ram_idle_agent_mb: Some(15.0),
        cold_start_api_ms: Some(1500), // Fails: > 1000
        ..Default::default()
    };

    let report = validate_non_functional_targets(&metrics);
    assert!(!report.all_passed);

    let api_ram = report
        .results
        .iter()
        .find(|r| r.target_id == "G6-RAM-API")
        .unwrap();
    assert!(!api_ram.passed);

    let cold_start = report
        .results
        .iter()
        .find(|r| r.target_id == "G6-START-COLD")
        .unwrap();
    assert!(!cold_start.passed);

    let agent_ram = report
        .results
        .iter()
        .find(|r| r.target_id == "G6-RAM-AGENT")
        .unwrap();
    assert!(agent_ram.passed);
}

#[test]
fn test_markdown_and_json_report_generation() {
    let run = BenchmarkRunResult {
        scenario: BenchmarkTarget::Static,
        target_url: "http://127.0.0.1:8080/index.html".to_string(),
        tuning_profile: TuningProfile::Baseline,
        tool_used: "builtin-tokio-http".to_string(),
        concurrency: 50,
        duration_secs: 5.0,
        total_requests: 5000,
        successful_requests: 5000,
        failed_requests: 0,
        requests_per_sec: 1000.0,
        transfer_rate_bytes_sec: 800_000.0,
        latency: LatencyDistribution {
            min_ms: 0.5,
            avg_ms: 3.5,
            p50_ms: 2.5,
            p90_ms: 6.0,
            p95_ms: 8.0,
            p99_ms: 12.0,
            max_ms: 20.0,
        },
        cpu_usage_percent: Some(20.0),
        memory_rss_mb: Some(35.0),
        executed_at: 1700000000,
    };

    let metrics = MeasuredSystemMetrics {
        ram_idle_api_mb: Some(34.0),
        ..Default::default()
    };
    let target_rep = validate_non_functional_targets(&metrics);

    let md = generate_markdown_report(std::slice::from_ref(&run), &[], Some(&target_rep));
    assert!(
        md.contains("# Laporan Benchmark &amp; Validasi Kinerja Qinpanel")
            || md.contains("# Laporan Benchmark")
    );
    assert!(md.contains("Static Assets / HTML"));
    assert!(md.contains("1000.0"));
    assert!(md.contains("G6-RAM-API"));

    let temp_dir = tempfile::tempdir().unwrap();
    let json_path = temp_dir.path().join("results.json");
    export_results_to_json(&[run], &[], Some(&target_rep), &json_path).unwrap();

    let json_content = std::fs::read_to_string(&json_path).unwrap();
    assert!(json_content.contains("builtin-tokio-http"));
    assert!(json_content.contains("static"));
}

#[tokio::test]
async fn test_builtin_http_benchmark_mock_server() {
    // Spawn local TCP mock HTTP server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                if let Ok(n) = socket.read(&mut buf).await
                    && n > 0
                {
                    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\nConnection: close\r\n\r\nHello, World!";
                    let _ = socket.write_all(response.as_bytes()).await;
                }
            });
        }
    });

    let options = BenchmarkOptions {
        scenario: BenchmarkTarget::Static,
        target_url: format!("http://{}/index.html", local_addr),
        tuning_profile: TuningProfile::Baseline,
        concurrency: 5,
        duration_secs: 1,
        total_requests: None,
        warmup_secs: 0,
        tool: BenchmarkTool::Builtin,
        headers: vec![("User-Agent".to_string(), "nusa-test/1.0".to_string())],
    };

    let result = run_builtin_benchmark(&options).await.unwrap();
    assert!(result.total_requests >= 5);
    assert_eq!(result.failed_requests, 0);
    assert!(result.requests_per_sec > 0.0);
    assert!(result.latency.p50_ms > 0.0);
}
