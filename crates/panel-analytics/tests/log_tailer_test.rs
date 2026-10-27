#![allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]

use panel_analytics::{
    AnalyticsStorage, LogFileTailer, LogTailerEngine, NginxJsonLog, SiteAttributor,
    TailerEngineConfig,
};
use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;

#[test]
fn test_nginx_json_log_parsing() {
    let raw = r#"{"time_local":"2026-10-10T14:32:10Z","remote_addr":"203.0.113.195","request_method":"POST","request_uri":"/api/v1/auth/login?ref=direct","status":200,"body_bytes_sent":1450,"request_time":0.045,"http_referer":"https://app.example.com/login","http_user_agent":"Mozilla/5.0 Chrome/120.0","upstream_response_time":"0.042","host":"app.example.com"}"#;

    let parsed = NginxJsonLog::parse_line(raw).expect("should parse valid JSON");
    assert_eq!(parsed.remote_addr.as_deref(), Some("203.0.113.195"));
    assert_eq!(parsed.request_method.as_deref(), Some("POST"));
    assert_eq!(
        parsed.request_uri.as_deref(),
        Some("/api/v1/auth/login?ref=direct")
    );
    assert_eq!(parsed.status, Some(200));
    assert_eq!(parsed.body_bytes_sent, Some(1450));
    assert!((parsed.request_time.unwrap_or(0.0) - 0.045).abs() < 1e-6);
    assert_eq!(
        parsed.http_referrer.as_deref(),
        Some("https://app.example.com/login")
    );
    assert_eq!(parsed.host.as_deref(), Some("app.example.com"));

    let entry = parsed.into_entry(42);
    assert_eq!(entry.site_id, 42);
    assert_eq!(entry.client_ip, "203.0.113.195");
    assert_eq!(entry.method, "POST");
    assert_eq!(entry.uri, "/api/v1/auth/login?ref=direct");
    assert_eq!(entry.status, 200);
    assert_eq!(entry.body_bytes_sent, 1450);
    assert!((entry.request_time_secs - 0.045).abs() < 1e-6);
    assert_eq!(entry.upstream_response_time_secs, Some(0.042));
}

#[test]
fn test_site_attributor_path_and_host() {
    let attributor = SiteAttributor::new();

    // Directory-based pattern
    assert_eq!(
        attributor.resolve_site_id(
            &PathBuf::from("/var/log/panel/sites/site-101/access.log"),
            None
        ),
        Some(101)
    );
    assert_eq!(
        attributor.resolve_site_id(
            &PathBuf::from("/var/log/panel/sites/102/access.json.log"),
            None
        ),
        Some(102)
    );

    // Vhost filename pattern
    assert_eq!(
        attributor.resolve_site_id(&PathBuf::from("/var/log/nginx/vhost-88.log"), None),
        Some(88)
    );

    // Host header resolution
    attributor.register_host("my-portal.com", 205);
    attributor.register_host("*.internal.net", 206);

    assert_eq!(
        attributor.resolve_site_id(
            &PathBuf::from("/var/log/nginx/access.log"),
            Some("my-portal.com")
        ),
        Some(205)
    );
    assert_eq!(
        attributor.resolve_site_id(
            &PathBuf::from("/var/log/nginx/access.log"),
            Some("api.internal.net")
        ),
        Some(206)
    );
    assert_eq!(
        attributor.resolve_site_id(
            &PathBuf::from("/var/log/nginx/access.log"),
            Some("unknown.com")
        ),
        None
    );
}

#[tokio::test]
async fn test_tailer_rotation_and_recovery() {
    let mut temp = NamedTempFile::new().expect("create temp log file");
    let path = temp.path().to_path_buf();

    let line1 = r#"{"time_local":"2026-10-10T10:00:00Z","remote_addr":"10.0.0.1","request_method":"GET","request_uri":"/page1","status":200,"body_bytes_sent":500,"request_time":0.010,"host":"example.com"}"#;
    let line2 = r#"{"time_local":"2026-10-10T10:00:01Z","remote_addr":"10.0.0.2","request_method":"GET","request_uri":"/page2","status":200,"body_bytes_sent":600,"request_time":0.012,"host":"example.com"}"#;

    writeln!(temp, "{line1}").expect("write line1");
    writeln!(temp, "{line2}").expect("write line2");
    temp.flush().expect("flush");

    let mut tailer = LogFileTailer::new(&path, 1, 10);
    let batch1 = tailer
        .read_next_batch()
        .await
        .expect("read batch")
        .expect("should have batch");
    assert_eq!(batch1.entries.len(), 2);
    let offset_after_batch1 = tailer.current_offset();
    assert!(offset_after_batch1 > 0);

    // Simulate copytruncate rotation (file truncated back to 0)
    let f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .expect("truncate file");
    drop(f);

    // Append new line to rotated file
    let line3 = r#"{"time_local":"2026-10-10T10:05:00Z","remote_addr":"10.0.0.3","request_method":"GET","request_uri":"/page3","status":200,"body_bytes_sent":700,"request_time":0.020,"host":"example.com"}"#;
    let mut f_new = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open file");
    writeln!(f_new, "{line3}").expect("write line3");
    f_new.flush().expect("flush");

    // Tailer should detect truncation and read the new line starting from offset 0
    let batch2 = tailer
        .read_next_batch()
        .await
        .expect("read batch after truncate")
        .expect("batch present");
    assert_eq!(batch2.entries.len(), 1);
    assert_eq!(batch2.entries[0].uri, "/page3");
    assert_eq!(batch2.entries[0].client_ip, "10.0.0.3");
}

#[tokio::test]
async fn test_tailer_checkpoint_persistence_and_resumption() {
    let mut temp = NamedTempFile::new().expect("create temp log file");
    let path = temp.path().to_path_buf();

    let storage = AnalyticsStorage::connect_in_memory()
        .await
        .expect("connect in-memory db");

    let line1 = r#"{"time_local":"2026-10-10T11:00:00Z","remote_addr":"1.2.3.4","request_method":"GET","request_uri":"/a","status":200,"body_bytes_sent":100,"request_time":0.01}"#;
    let line2 = r#"{"time_local":"2026-10-10T11:00:01Z","remote_addr":"1.2.3.5","request_method":"GET","request_uri":"/b","status":200,"body_bytes_sent":200,"request_time":0.02}"#;
    writeln!(temp, "{line1}").expect("write line1");
    writeln!(temp, "{line2}").expect("write line2");
    temp.flush().expect("flush");

    // Session 1: Run engine and consume first batch
    let config = TailerEngineConfig {
        watch_directories: vec![],
        poll_interval: std::time::Duration::from_millis(50),
        batch_size: 10,
        channel_capacity: 5,
    };

    {
        let (engine, mut rx) = LogTailerEngine::new(config.clone(), Some(storage.clone()), None);
        engine
            .register_log_file(10, &path)
            .await
            .expect("register file");

        let processed = engine.process_all_once().await.expect("process once");
        assert_eq!(processed, 1);

        let batch = rx.recv().await.expect("recv batch");
        assert_eq!(batch.entries.len(), 2);
    }

    // Append line 3 while service was offline
    let line3 = r#"{"time_local":"2026-10-10T11:00:02Z","remote_addr":"1.2.3.6","request_method":"GET","request_uri":"/c","status":200,"body_bytes_sent":300,"request_time":0.03}"#;
    writeln!(temp, "{line3}").expect("write line3");
    temp.flush().expect("flush");

    // Session 2: New engine instance created (simulating restart after crash)
    {
        let (engine2, mut rx2) = LogTailerEngine::new(config, Some(storage.clone()), None);
        engine2
            .register_log_file(10, &path)
            .await
            .expect("register file in session 2");

        let processed = engine2
            .process_all_once()
            .await
            .expect("process once in session 2");
        assert_eq!(processed, 1);

        let batch = rx2.recv().await.expect("recv batch");
        // Should only have read line3, NOT line1 and line2 again!
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.entries[0].uri, "/c");
        assert_eq!(batch.entries[0].client_ip, "1.2.3.6");
    }
}

#[tokio::test]
async fn test_tailer_skips_corrupted_lines() {
    let mut temp = NamedTempFile::new().expect("create temp log file");
    let path = temp.path().to_path_buf();

    let valid1 = r#"{"time_local":"2026-10-10T12:00:00Z","remote_addr":"1.1.1.1","request_method":"GET","request_uri":"/ok1","status":200}"#;
    let corrupted = r#"CORRUPTED GARBAGE NOT JSON {foo:"#;
    let valid2 = r#"{"time_local":"2026-10-10T12:00:02Z","remote_addr":"1.1.1.2","request_method":"GET","request_uri":"/ok2","status":200}"#;

    writeln!(temp, "{valid1}").expect("write valid1");
    writeln!(temp, "{corrupted}").expect("write corrupted");
    writeln!(temp, "{valid2}").expect("write valid2");
    temp.flush().expect("flush");

    let mut tailer = LogFileTailer::new(&path, 99, 10);
    let batch = tailer
        .read_next_batch()
        .await
        .expect("read batch")
        .expect("batch present");

    // Corrupted line should have been skipped, both valid lines preserved
    assert_eq!(batch.entries.len(), 2);
    assert_eq!(batch.entries[0].uri, "/ok1");
    assert_eq!(batch.entries[1].uri, "/ok2");
}

#[tokio::test]
async fn test_tailer_rename_rotation_drains_rotated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log_path = dir.path().join("access.log");
    let rotated_path = dir.path().join("access.log.1");

    let line1 = r#"{"time_local":"2026-10-10T10:00:00Z","remote_addr":"10.0.0.1","request_method":"GET","request_uri":"/p1","status":200}"#;
    let line2 = r#"{"time_local":"2026-10-10T10:00:01Z","remote_addr":"10.0.0.2","request_method":"GET","request_uri":"/p2","status":200}"#;
    let line3 = r#"{"time_local":"2026-10-10T10:00:02Z","remote_addr":"10.0.0.3","request_method":"GET","request_uri":"/p3","status":200}"#;

    // Step 1: Write line1 and line2 to access.log
    std::fs::write(&log_path, format!("{line1}\n{line2}\n")).expect("write initial lines");

    // Read with batch size 1 so only line1 is consumed
    let mut tailer = LogFileTailer::new(&log_path, 1, 1);
    let batch1 = tailer
        .read_next_batch()
        .await
        .expect("read batch1")
        .expect("batch1 present");
    assert_eq!(batch1.entries.len(), 1);
    assert_eq!(batch1.entries[0].uri, "/p1");

    // Step 2: Simulate logrotate (rename access.log -> access.log.1, create new access.log with line3)
    std::fs::rename(&log_path, &rotated_path).expect("rename to .1");
    std::fs::write(&log_path, format!("{line3}\n")).expect("write new access.log");

    // Step 3: Next read should detect rotation, find access.log.1, and drain unread line2 from it!
    let batch2 = tailer
        .read_next_batch()
        .await
        .expect("read batch2")
        .expect("batch2 present");
    assert_eq!(batch2.entries.len(), 1);
    assert_eq!(batch2.entries[0].uri, "/p2");

    // Step 4: Next read should switch to the new access.log and read line3!
    let batch3 = tailer
        .read_next_batch()
        .await
        .expect("read batch3")
        .expect("batch3 present");
    assert_eq!(batch3.entries.len(), 1);
    assert_eq!(batch3.entries[0].uri, "/p3");
}

#[tokio::test]
async fn test_tailer_backpressure_with_bounded_channel() {
    let mut temp = NamedTempFile::new().expect("create temp log file");
    let path = temp.path().to_path_buf();

    for i in 1..=5 {
        writeln!(
            temp,
            r#"{{"time_local":"2026-10-10T10:00:0{}Z","remote_addr":"1.1.1.{}","request_method":"GET","request_uri":"/item/{}","status":200}}"#,
            i, i, i
        )
        .expect("write log line");
    }
    temp.flush().expect("flush");

    // Config with batch_size=1 and small bounded channel_capacity=2
    let config = TailerEngineConfig {
        watch_directories: vec![],
        poll_interval: std::time::Duration::from_millis(100),
        batch_size: 1,
        channel_capacity: 2,
    };

    let (engine, mut rx) = LogTailerEngine::new(config, None, None);
    engine
        .register_log_file(42, &path)
        .await
        .expect("register file");

    let engine_arc = std::sync::Arc::new(engine);
    let cancel = tokio_util::sync::CancellationToken::new();

    let runner_engine = engine_arc.clone();
    let runner_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        let _ = runner_engine.run_loop(runner_cancel).await;
    });

    // Receive all 5 batches from the bounded channel
    let mut received_uris = Vec::new();
    for _ in 0..5 {
        let batch = rx.recv().await.expect("recv batch");
        received_uris.push(batch.entries[0].uri.clone());
    }

    assert_eq!(
        received_uris,
        vec!["/item/1", "/item/2", "/item/3", "/item/4", "/item/5"]
    );

    cancel.cancel();
    let _ = handle.await;
}

#[test]
fn test_site_attributor_domain_parent_folder_and_helpers() {
    let attributor = SiteAttributor::new();
    attributor.register_site(70, "my-shop.id", None::<PathBuf>);

    // Path in /var/log/panel/p_alice/my-shop.id/access.log has parent folder 'my-shop.id'
    let path = PathBuf::from("/var/log/panel/p_alice/my-shop.id/access.log");
    assert_eq!(attributor.resolve_site_id(&path, None), Some(70));

    // Wildcard subdomain matching via host header
    assert_eq!(
        attributor.resolve_site_id(
            &PathBuf::from("/var/log/nginx/access.log"),
            Some("checkout.my-shop.id:443")
        ),
        Some(70)
    );
}

#[test]
fn test_multi_upstream_log_parsing() {
    let raw = r#"{"time_local":"2026-10-10T14:32:10Z","remote_addr":"203.0.113.195","request_method":"GET","request_uri":"/test","status":"200","body_bytes_sent":"512","request_time":"0.050","upstream_response_time":"0.020, 0.045","host":"api.example.com"}"#;
    let parsed = NginxJsonLog::parse_line(raw).expect("parse multi-upstream line");
    let entry = parsed.into_entry(88);
    assert_eq!(entry.status, 200);
    assert_eq!(entry.body_bytes_sent, 512);
    assert!((entry.request_time_secs - 0.050).abs() < 1e-6);
    assert_eq!(entry.upstream_response_time_secs, Some(0.045));
}

#[tokio::test]
async fn test_high_throughput_ingest_performance() {
    let mut temp = NamedTempFile::new().expect("create temp log file");
    let path = temp.path().to_path_buf();

    let num_lines = 5000;
    for i in 0..num_lines {
        let line = format!(
            r#"{{"time_local":"2026-10-10T12:00:00Z","remote_addr":"192.168.1.{}","request_method":"GET","request_uri":"/api/products/{}","status":200,"body_bytes_sent":1024,"request_time":0.005,"upstream_response_time":0.003,"host":"benchmark.org"}}"#,
            i % 250,
            i
        );
        writeln!(temp, "{line}").expect("write line");
    }
    temp.flush().expect("flush");

    let mut tailer = LogFileTailer::new(&path, 999, 1000);
    let start = std::time::Instant::now();

    let mut total_ingested = 0;
    while let Some(batch) = tailer.read_next_batch().await.expect("read batch") {
        total_ingested += batch.entries.len();
        if total_ingested >= num_lines {
            break;
        }
    }

    let elapsed = start.elapsed();
    assert_eq!(total_ingested, num_lines);
    // Acceptance criteria: ≥ 5.000 baris/detik dengan lag < 5 s.
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "Ingestion took {:?}, expected < 5s for 5,000 lines",
        elapsed
    );
}

#[tokio::test]
async fn test_tailer_engine_stats_and_dynamic_registration() {
    let mut temp = NamedTempFile::new().expect("create temp log file");
    let path = temp.path().to_path_buf();

    writeln!(
        temp,
        r#"{{"time_local":"2026-10-10T12:00:00Z","remote_addr":"1.1.1.1","request_method":"GET","request_uri":"/home","status":200,"body_bytes_sent":2048}}"#
    )
    .expect("write line");
    temp.flush().expect("flush");

    let config = TailerEngineConfig {
        watch_directories: vec![],
        poll_interval: std::time::Duration::from_millis(50),
        batch_size: 50,
        channel_capacity: 10,
    };

    let (engine, mut rx) = LogTailerEngine::new(config, None, None);
    assert_eq!(engine.active_tailers_count().await, 0);
    assert!(!engine.is_tracking_path(&path).await);

    engine
        .register_log_file(123, &path)
        .await
        .expect("register file");
    assert_eq!(engine.active_tailers_count().await, 1);
    assert!(engine.is_tracking_path(&path).await);

    let tracked = engine.tracked_paths().await;
    assert_eq!(tracked.len(), 1);
    assert_eq!(tracked[0], path);

    let processed = engine.process_all_once().await.expect("process once");
    assert_eq!(processed, 1);

    let _batch = rx.recv().await.expect("recv batch");
    let stats = engine.stats().await;
    assert_eq!(stats.active_tailers, 1);
    assert_eq!(stats.total_lines_processed, 1);
    assert_eq!(stats.total_bytes_processed, 2048);
    assert_eq!(stats.total_batches_processed, 1);

    engine.unregister_log_file(&path).await.expect("unregister");
    assert_eq!(engine.active_tailers_count().await, 0);
    assert!(!engine.is_tracking_path(&path).await);
}
