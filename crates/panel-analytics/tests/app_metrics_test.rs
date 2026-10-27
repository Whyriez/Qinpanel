use panel_analytics::{
    AppProcessMetricPoint, AppProcessMetricsCollector, AppType, FpmStatusClient,
    PhpFpmPoolMetricPoint, PhpSlowlogParser, RetentionConfig, build_fcgi_status_request,
    parse_fcgi_status_response, parse_pool_identifiers, parse_status_json,
    storage::AnalyticsStorage,
};
use std::fs;
use std::thread::sleep;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_fcgi_request_builder_and_identifiers() {
    let req = build_fcgi_status_request("/fpm-status");
    assert!(!req.is_empty());
    // First byte is FastCGI version 1
    assert_eq!(req[0], 1);
    // Second byte is FCGI_BEGIN_REQUEST (1)
    assert_eq!(req[1], 1);

    // Identifier extraction tests
    let (user1, ver1) = parse_pool_identifiers("p_alice-8.3");
    assert_eq!(user1.as_deref(), Some("p_alice"));
    assert_eq!(ver1.as_deref(), Some("8.3"));

    let (user2, ver2) = parse_pool_identifiers("site-10-8.2");
    assert_eq!(user2.as_deref(), Some("site-10"));
    assert_eq!(ver2.as_deref(), Some("8.2"));

    let (user3, ver3) = parse_pool_identifiers("default");
    assert_eq!(user3, None);
    assert_eq!(ver3, None);
}

#[test]
fn test_fcgi_response_parsing() {
    let json_body = r#"{
        "pool": "p_alice-8.3",
        "process manager": "ondemand",
        "start time": 1770000000,
        "start since": 120,
        "accepted conn": 1500,
        "listen queue": 2,
        "max listen queue": 5,
        "listen queue len": 128,
        "idle processes": 4,
        "active processes": 3,
        "total processes": 7,
        "max active processes": 10,
        "max children reached": 1,
        "slow requests": 12
    }"#;

    // 1. Direct JSON parsing
    let pt = parse_status_json(json_body, 1770000100).expect("parse json");
    assert_eq!(pt.pool_name, "p_alice-8.3");
    assert_eq!(pt.user.as_deref(), Some("p_alice"));
    assert_eq!(pt.php_version.as_deref(), Some("8.3"));
    assert_eq!(pt.active_processes, 3);
    assert_eq!(pt.idle_processes, 4);
    assert_eq!(pt.total_processes, 7);
    assert_eq!(pt.max_active_processes, 10);
    assert_eq!(pt.max_children_reached, 1);
    assert_eq!(pt.listen_queue, 2);
    assert_eq!(pt.slow_requests, 12);
    assert_eq!(pt.accepted_conn, 1500);

    // 2. Synthesize FastCGI STDOUT stream response
    let http_headers = b"Content-type: application/json\r\n\r\n";
    let mut payload = Vec::new();
    payload.extend_from_slice(http_headers);
    payload.extend_from_slice(json_body.as_bytes());

    let mut stream = Vec::new();
    let content_len = payload.len() as u16;
    // FCGI_STDOUT record: ver 1, type 6, req_id 1, content_len, pad 0
    stream.extend_from_slice(&[
        1,
        6,
        0,
        1,
        (content_len >> 8) as u8,
        (content_len & 0xff) as u8,
        0,
        0,
    ]);
    stream.extend_from_slice(&payload);
    // FCGI_END_REQUEST record: ver 1, type 8, req_id 1, len 8, pad 0
    stream.extend_from_slice(&[1, 8, 0, 1, 0, 8, 0, 0]);
    stream.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);

    let parsed_fcgi = parse_fcgi_status_response(&stream, 1770000100).expect("parse fcgi stream");
    assert_eq!(parsed_fcgi.pool_name, "p_alice-8.3");
    assert_eq!(parsed_fcgi.active_processes, 3);
    assert_eq!(parsed_fcgi.slow_requests, 12);
}

#[test]
fn test_fpm_status_client_mock() {
    let tmp = tempdir().expect("tempdir");
    let sock_path = tmp.path().join("p_alice-8.3.sock");
    let mock_json_path = sock_path.with_extension("status.json");

    let sample_json = r#"{
        "pool": "p_alice-8.3",
        "process manager": "dynamic",
        "start time": 1770000000,
        "start since": 300,
        "accepted conn": 880,
        "listen queue": 0,
        "max listen queue": 1,
        "listen queue len": 128,
        "idle processes": 2,
        "active processes": 1,
        "total processes": 3,
        "max active processes": 5,
        "max children reached": 0,
        "slow requests": 2
    }"#;
    fs::write(&mock_json_path, sample_json).expect("write mock json");

    let client = FpmStatusClient::default();
    let point = client
        .query_socket(&sock_path, 1770000200)
        .expect("query mock socket");
    assert_eq!(point.pool_name, "p_alice-8.3");
    assert_eq!(point.active_processes, 1);
    assert_eq!(point.accepted_conn, 880);
    assert_eq!(point.slow_requests, 2);
}

#[test]
fn test_slowlog_parser_multiple_entries() {
    let raw_slowlog = r#"
[11-Oct-2026 14:20:10]  [pool p_alice-8.3] pid 12345
script_filename = /home/p_alice/sites/example.com/public/index.php
[0x00007f3b8c2d1234] sleep() /home/p_alice/sites/example.com/public/heavy.php:14
[0x00007f3b8c2d5678] run() /home/p_alice/sites/example.com/public/index.php:25

[11-Oct-2026 14:22:30]  [pool p_alice-8.3] pid 12349
script_filename = /home/p_alice/sites/example.com/public/api.php
[0x00007f3b8c2d9999] curl_exec() /home/p_alice/sites/example.com/vendor/guzzle/Client.php:80
[0x00007f3b8c2daaaa] request() /home/p_alice/sites/example.com/public/api.php:42
"#;

    let entries = PhpSlowlogParser::parse_str(raw_slowlog);
    assert_eq!(entries.len(), 2);

    let e1 = &entries[0];
    assert_eq!(e1.pool, "p_alice-8.3");
    assert_eq!(e1.pid, 12345);
    assert_eq!(
        e1.script_filename,
        "/home/p_alice/sites/example.com/public/index.php"
    );
    assert_eq!(e1.top_function.as_deref(), Some("sleep()"));
    assert_eq!(e1.tenant_user.as_deref(), Some("p_alice"));
    assert_eq!(e1.frames.len(), 2);
    assert_eq!(e1.frames[0].function, "sleep()");
    assert_eq!(
        e1.frames[0].file,
        "/home/p_alice/sites/example.com/public/heavy.php"
    );
    assert_eq!(e1.frames[0].line, 14);
    assert_eq!(e1.frames[0].address.as_deref(), Some("0x00007f3b8c2d1234"));

    let e2 = &entries[1];
    assert_eq!(e2.pid, 12349);
    assert_eq!(e2.top_function.as_deref(), Some("curl_exec()"));
    assert_eq!(e2.frames.len(), 2);
    assert_eq!(e2.frames[0].line, 80);
}

#[test]
fn test_slowlog_incremental_offset_parsing() {
    let tmp = tempdir().expect("tempdir");
    let log_path = tmp.path().join("php-fpm-8.3-slow.log");

    let part1 = r#"[11-Oct-2026 14:30:00]  [pool p_bob-8.3] pid 901
script_filename = /home/p_bob/sites/bob.com/test.php
[0x00007f3b11112222] usleep() /home/p_bob/sites/bob.com/test.php:5
"#;
    fs::write(&log_path, part1).expect("write part1");

    let (entries1, offset1) =
        PhpSlowlogParser::parse_from_offset(&log_path, 0).expect("read part1");
    assert_eq!(entries1.len(), 1);
    assert_eq!(entries1[0].top_function.as_deref(), Some("usleep()"));
    assert!(offset1 > 0);

    // Read again with offset1: should return empty and same offset
    let (entries_empty, offset2) =
        PhpSlowlogParser::parse_from_offset(&log_path, offset1).expect("read same");
    assert_eq!(entries_empty.len(), 0);
    assert_eq!(offset2, offset1);

    // Append part2
    let part2 = r#"[11-Oct-2026 14:31:00]  [pool p_bob-8.3] pid 905
script_filename = /home/p_bob/sites/bob.com/heavy.php
[0x00007f3b33334444] PDOStatement::execute() /home/p_bob/sites/bob.com/db.php:12
"#;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .expect("open append");
    file.write_all(part2.as_bytes()).expect("append part2");

    let (entries2, offset3) =
        PhpSlowlogParser::parse_from_offset(&log_path, offset1).expect("read part2");
    assert_eq!(entries2.len(), 1);
    assert_eq!(
        entries2[0].top_function.as_deref(),
        Some("PDOStatement::execute()")
    );
    assert!(offset3 > offset1);
}

#[test]
fn test_app_cgroup_metrics_collector() {
    let tmp = tempdir().expect("tempdir");
    let cgroup_root = tmp.path();

    // Create system.slice/site-101.service mock directory
    let svc_dir = cgroup_root.join("system.slice").join("site-101.service");
    fs::create_dir_all(&svc_dir).expect("create svc dir");

    fs::write(
        svc_dir.join("cpu.stat"),
        "usage_usec 1000000\nuser_usec 800000\nsystem_usec 200000\n",
    )
    .expect("write cpu.stat");
    fs::write(svc_dir.join("memory.current"), "52428800\n").expect("write memory.current"); // 50 MB
    fs::write(svc_dir.join("memory.peak"), "62914560\n").expect("write memory.peak"); // 60 MB
    fs::write(svc_dir.join("memory.swap.current"), "0\n").expect("write swap");
    fs::write(svc_dir.join("pids.current"), "4\n").expect("write pids.current");
    fs::write(
        svc_dir.join("io.stat"),
        "8:0 rbytes=1048576 wbytes=2097152 rios=10 wios=20\n",
    )
    .expect("write io.stat");

    let collector = AppProcessMetricsCollector::new(cgroup_root);

    // Initial collection
    let p1 = collector
        .collect_app_metrics(101, AppType::Node, "site-101", 1770001000)
        .expect("collect 1");
    assert_eq!(p1.site_id, 101);
    assert_eq!(p1.app_type, AppType::Node);
    assert_eq!(p1.cpu_usage_usec, 1000000);
    assert_eq!(p1.memory_current_bytes, 52428800);
    assert_eq!(p1.memory_peak_bytes, Some(62914560));
    assert_eq!(p1.pids_current, 4);
    assert_eq!(p1.cpu_percent, 0.0); // First reading has no delta

    // Advance usage and collect second reading
    sleep(Duration::from_millis(10));
    fs::write(
        svc_dir.join("cpu.stat"),
        "usage_usec 1050000\nuser_usec 840000\nsystem_usec 210000\n",
    )
    .expect("write updated cpu.stat");

    let p2 = collector
        .collect_app_metrics(101, AppType::Node, "site-101", 1770001010)
        .expect("collect 2");
    assert!(p2.cpu_percent > 0.0);
    assert_eq!(p2.cpu_usage_usec, 1050000);
}

#[tokio::test]
async fn test_storage_php_fpm_and_app_metrics_lifecycle() {
    let storage = AnalyticsStorage::connect_in_memory()
        .await
        .expect("in-memory db");

    let base_ts = 1770000000;

    // 1. PHP-FPM storage and rollup
    let mut fpm_pts = Vec::new();
    for i in 0..6 {
        let ts = base_ts + (i * 10);
        fpm_pts.push(PhpFpmPoolMetricPoint {
            timestamp: ts,
            pool_name: "p_alice-8.3".to_string(),
            user: Some("p_alice".to_string()),
            php_version: Some("8.3".to_string()),
            active_processes: 2 + (i as u32 % 2),
            idle_processes: 5,
            total_processes: 7,
            max_active_processes: 10,
            max_children_reached: 0,
            listen_queue: i as u32,
            max_listen_queue: 5,
            listen_queue_len: 128,
            slow_requests: 10 + i as u64,
            accepted_conn: 100 + (i as u64 * 10),
        });
    }

    storage
        .insert_php_fpm_points(&fpm_pts)
        .await
        .expect("insert fpm points");

    let raw_fpm = storage
        .query_php_fpm_raw("p_alice-8.3", base_ts, base_ts + 60)
        .await
        .expect("query fpm raw");
    assert_eq!(raw_fpm.len(), 6);

    let rollup_cnt = storage
        .rollup_php_fpm_1m(base_ts + 60)
        .await
        .expect("rollup fpm 1m");
    assert_eq!(rollup_cnt, 1);

    let fpm_rollups = storage
        .query_php_fpm_1m("p_alice-8.3", base_ts - 10, base_ts + 100)
        .await
        .expect("query fpm 1m");
    assert_eq!(fpm_rollups.len(), 1);
    let r_fpm = &fpm_rollups[0];
    assert_eq!(r_fpm.pool_name, "p_alice-8.3");
    assert_eq!(r_fpm.sample_count, 6);
    assert_eq!(r_fpm.listen_queue_max, 5);
    assert_eq!(r_fpm.slow_requests_delta, 5);
    assert_eq!(r_fpm.accepted_conn_delta, 50);

    // 2. Slowlog storage
    let slow_entry = panel_analytics::PhpSlowlogEntry {
        id: None,
        timestamp: base_ts + 15,
        pool: "p_alice-8.3".to_string(),
        pid: 3344,
        script_filename: "/home/p_alice/public/index.php".to_string(),
        top_function: Some("sleep()".to_string()),
        frames: vec![panel_analytics::PhpSlowlogFrame {
            frame_index: 0,
            function: "sleep()".to_string(),
            file: "/home/p_alice/public/heavy.php".to_string(),
            line: 42,
            address: Some("0x1234".to_string()),
        }],
        duration_secs: Some(5.0),
        site_id: Some(42),
        tenant_user: Some("p_alice".to_string()),
    };

    let entry_id = storage
        .insert_slowlog_entry(&slow_entry)
        .await
        .expect("insert slowlog");
    assert!(entry_id > 0);

    let queried_slow = storage
        .query_slowlog_events(Some("p_alice-8.3"), None, base_ts, base_ts + 100, 10)
        .await
        .expect("query slowlog");
    assert_eq!(queried_slow.len(), 1);
    assert_eq!(queried_slow[0].top_function.as_deref(), Some("sleep()"));
    assert_eq!(queried_slow[0].frames.len(), 1);

    // 3. Application Process Metrics (Node / Python)
    let mut app_pts = Vec::new();
    for i in 0..6 {
        let ts = base_ts + (i * 10);
        app_pts.push(AppProcessMetricPoint {
            timestamp: ts,
            site_id: 42,
            app_type: AppType::Node,
            service_name: "site-42.service".to_string(),
            cpu_usage_usec: 100000 * (i as u64 + 1),
            cpu_percent: 10.0 + (i as f64 * 2.0),
            memory_current_bytes: 40000000 + (i as u64 * 1000000),
            memory_peak_bytes: Some(50000000),
            memory_swap_bytes: None,
            pids_current: 3,
            io_read_bytes_sec: 1024.0,
            io_write_bytes_sec: 2048.0,
        });
    }

    storage
        .insert_app_process_points(&app_pts)
        .await
        .expect("insert app process pts");

    let raw_app = storage
        .query_app_process_raw(42, AppType::Node, base_ts, base_ts + 60)
        .await
        .expect("query app process raw");
    assert_eq!(raw_app.len(), 6);

    let app_rollup_cnt = storage
        .rollup_app_process_1m(base_ts + 60)
        .await
        .expect("rollup app process 1m");
    assert_eq!(app_rollup_cnt, 1);

    let app_rollups = storage
        .query_app_process_1m(42, AppType::Node, base_ts - 10, base_ts + 100)
        .await
        .expect("query app process 1m");
    assert_eq!(app_rollups.len(), 1);
    let r_app = &app_rollups[0];
    assert_eq!(r_app.site_id, 42);
    assert_eq!(r_app.app_type, AppType::Node);
    assert_eq!(r_app.sample_count, 6);
    assert_eq!(r_app.cpu_min_or_max(), ()); // verify existence
    assert_eq!(r_app.pids_max, 3);
    assert_eq!(r_app.cpu_max, 20.0);

    // 4. Pruning
    let retention = RetentionConfig {
        raw_retention_secs: 10,
        one_minute_retention_secs: 10,
        one_hour_retention_secs: 10,
        one_day_retention_secs: None,
        ..Default::default()
    };
    let pruned = storage
        .prune_expired(base_ts + 200, &retention)
        .await
        .expect("prune expired");
    assert!(pruned > 0);
}

trait HelperExt {
    fn cpu_min_or_max(&self) {}
}
impl HelperExt for panel_analytics::AppProcessMetricRollup {}
