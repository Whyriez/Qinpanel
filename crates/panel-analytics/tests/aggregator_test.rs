#![allow(clippy::unwrap_used, clippy::expect_used, clippy::cognitive_complexity)]

use panel_analytics::{
    AnalyticsStorage, GeoIpResolver, LatencyHistogram, ParsedLogEntry, PathNormalizer,
    RetentionConfig, RollupResolution, SiteMetricsAggregator, SiteMetricsPoint, SpaceSavingCounter,
    UaParser, compute_visitor_hash,
};

#[test]
fn test_normalizer_various_patterns() {
    assert_eq!(
        PathNormalizer::normalize_path("/api/v1/users/99/items/101?sort=asc#top"),
        "/api/v1/users/:id/items/:id"
    );
    assert_eq!(
        PathNormalizer::normalize_path("/checkout/c4ca4238a0b923820dcc509a6f75849b"),
        "/checkout/:hash"
    );
    assert_eq!(
        PathNormalizer::normalize_path("/documents/123e4567-e89b-12d3-a456-426614174000/download"),
        "/documents/:uuid/download"
    );
    assert_eq!(PathNormalizer::normalize_path("/about-us"), "/about-us");
}

#[test]
fn test_space_saving_heavy_hitters() {
    let mut ss = SpaceSavingCounter::new(5);
    for _ in 0..50 {
        ss.increment("/index.html");
    }
    for _ in 0..30 {
        ss.increment("/api/products");
    }
    for _ in 0..20 {
        ss.increment("/login");
    }
    for i in 0..10 {
        ss.increment(format!("/rare-{}", i));
    }

    let top = ss.top_k(3);
    assert_eq!(top.len(), 3);
    assert_eq!(top[0].name, "/index.html");
    assert_eq!(top[0].count, 50);
    assert_eq!(top[1].name, "/api/products");
    assert_eq!(top[1].count, 30);
    assert_eq!(top[2].name, "/login");
    assert_eq!(top[2].count, 20);
}

#[test]
fn test_latency_histogram_percentiles() {
    let mut hist = LatencyHistogram::new();
    // Record 200 items evenly distributed from 1ms to 200ms
    for i in 1..=200 {
        hist.record_ms(i as f64);
    }

    assert_eq!(hist.count, 200);
    assert!((hist.avg_ms() - 100.5).abs() < 1e-4);
    let p50 = hist.p50_ms();
    let p95 = hist.p95_ms();
    let p99 = hist.p99_ms();

    assert!(p50 > 50.0 && p50 < 150.0, "p50={}", p50);
    assert!(p95 > 150.0 && p95 <= 250.0, "p95={}", p95);
    assert!(p99 > 180.0 && p99 <= 250.0, "p99={}", p99);
}

#[test]
fn test_ua_parser_classification() {
    let chrome_desktop = UaParser::parse(Some(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/122.0.0.0 Safari/537.36",
    ));
    assert_eq!(chrome_desktop.browser, "Chrome");
    assert_eq!(chrome_desktop.os, "Windows");
    assert_eq!(chrome_desktop.device, "Desktop");
    assert!(!chrome_desktop.is_bot);

    let mobile_safari = UaParser::parse(Some(
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148 Safari/604.1",
    ));
    assert_eq!(mobile_safari.browser, "Safari");
    assert_eq!(mobile_safari.os, "iOS");
    assert_eq!(mobile_safari.device, "Mobile");
    assert!(!mobile_safari.is_bot);

    let googlebot = UaParser::parse(Some("Googlebot/2.1 (+http://www.google.com/bot.html)"));
    assert!(googlebot.is_bot);
    assert_eq!(googlebot.bot_name.as_deref(), Some("Googlebot"));
    assert_eq!(googlebot.device, "Bot");

    let python = UaParser::parse(Some("python-requests/2.31.0"));
    assert!(python.is_bot);
    assert_eq!(python.bot_name.as_deref(), Some("PythonRequests"));
}

#[test]
fn test_geoip_and_visitor_hashing() {
    let geoip = GeoIpResolver::new_empty();
    geoip.register_override("203.0.113.5", "ID");
    geoip.register_override("1.1.1.1", "US");

    assert_eq!(geoip.lookup_country("203.0.113.5"), "ID");
    assert_eq!(geoip.lookup_country("1.1.1.1"), "US");
    assert_eq!(geoip.lookup_country("127.0.0.1"), "LOCAL");
    assert_eq!(geoip.lookup_country("10.0.0.1"), "LAN");

    let h1 = compute_visitor_hash("203.0.113.5", Some("Chrome"), "salt-2026-10-11");
    let h2 = compute_visitor_hash("203.0.113.5", Some("Chrome"), "salt-2026-10-11");
    let h3 = compute_visitor_hash("203.0.113.5", Some("Chrome"), "salt-2026-10-12");
    assert_eq!(h1, h2);
    assert_ne!(h1, h3);
}

#[test]
fn test_site_metrics_aggregator_end_to_end() {
    let geoip = GeoIpResolver::new_empty();
    geoip.register_override("203.0.113.5", "ID");
    geoip.register_override("1.1.1.1", "US");

    let mut aggregator = SiteMetricsAggregator::new(geoip);
    aggregator.set_daily_salt("test-salt-123");

    let base_ts = 1770000000; // aligned to minute: 1770000000 % 60 == 0

    // Feed 3 entries for site 42
    let entry1 = ParsedLogEntry {
        site_id: 42,
        timestamp: base_ts + 10,
        client_ip: "203.0.113.5".to_string(),
        host: "app.example.com".to_string(),
        method: "GET".to_string(),
        uri: "/posts/101?ref=home".to_string(),
        status: 200,
        body_bytes_sent: 1500,
        request_time_secs: 0.045,
        upstream_response_time_secs: Some(0.040),
        upstream_cache_status: Some("HIT".to_string()),
        referer: Some("https://google.com/search".to_string()),
        user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0".to_string()),
        protocol: Some("HTTP/2.0".to_string()),
        tls_version: Some("TLSv1.3".to_string()),
    };

    let entry2 = ParsedLogEntry {
        site_id: 42,
        timestamp: base_ts + 20,
        client_ip: "1.1.1.1".to_string(),
        host: "app.example.com".to_string(),
        method: "POST".to_string(),
        uri: "/api/v1/auth/login".to_string(),
        status: 401,
        body_bytes_sent: 450,
        request_time_secs: 0.350, // slow > 250ms
        upstream_response_time_secs: Some(0.340),
        upstream_cache_status: Some("MISS".to_string()),
        referer: None,
        user_agent: Some("curl/7.88.1".to_string()),
        protocol: Some("HTTP/1.1".to_string()),
        tls_version: Some("TLSv1.2".to_string()),
    };

    let entry3 = ParsedLogEntry {
        site_id: 42,
        timestamp: base_ts + 30,
        client_ip: "203.0.113.5".to_string(), // same visitor as entry1
        host: "app.example.com".to_string(),
        method: "GET".to_string(),
        uri: "/posts/102".to_string(),
        status: 200,
        body_bytes_sent: 1200,
        request_time_secs: 0.030,
        upstream_response_time_secs: Some(0.025),
        upstream_cache_status: Some("HIT".to_string()),
        referer: Some("https://app.example.com/".to_string()),
        user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0".to_string()),
        protocol: Some("HTTP/2.0".to_string()),
        tls_version: Some("TLSv1.3".to_string()),
    };

    aggregator.record(&entry1);
    aggregator.record(&entry2);
    aggregator.record(&entry3);

    // Flush all points
    let points = aggregator.flush_all();
    assert_eq!(points.len(), 1);
    let p = &points[0];

    assert_eq!(p.site_id, 42);
    assert_eq!(p.bucket_ts, base_ts);
    assert_eq!(p.total_requests, 3);
    assert_eq!(p.total_bytes, 3150);
    assert_eq!(p.status_2xx, 2);
    assert_eq!(p.status_4xx, 1);
    assert_eq!(p.status_5xx, 0);
    assert_eq!(p.cache_hit, 2);
    assert_eq!(p.cache_miss, 1);
    assert_eq!(p.unique_visitors, 2); // 203.0.113.5 (visited twice) and 1.1.1.1
    assert_eq!(p.human_requests, 2);
    assert_eq!(p.bot_requests, 1); // curl is bot
    assert_eq!(p.http_v2, 2);
    assert_eq!(p.http_v1, 1);
    assert_eq!(p.tls_v13, 2);
    assert_eq!(p.tls_v12, 1);

    // Verify top normalized paths: "/posts/:id" should have count 2!
    let post_path = p.top_paths.iter().find(|t| t.name == "/posts/:id");
    assert!(post_path.is_some(), "Path /posts/:id should be normalized");
    assert_eq!(post_path.unwrap().count, 2);

    // Verify referrers: google.com
    assert!(p.top_referrers.iter().any(|r| r.name == "google.com"));

    // Verify countries: ID (2), US (1)
    let id_country = p.top_countries.iter().find(|c| c.name == "ID");
    assert!(id_country.is_some());
    assert_eq!(id_country.unwrap().count, 2);

    // Verify slow URLs: /api/v1/auth/login (350ms)
    assert_eq!(p.top_slow_urls.len(), 1);
    assert_eq!(p.top_slow_urls[0].uri, "/api/v1/auth/login");

    // Verify error URLs: /api/v1/auth/login (status 401)
    assert_eq!(p.top_error_urls.len(), 1);
    assert_eq!(p.top_error_urls[0].uri, "/api/v1/auth/login");
    assert_eq!(p.top_error_urls[0].status, 401);
}

#[tokio::test]
async fn test_storage_site_metrics_lifecycle_and_rollups() {
    let storage = AnalyticsStorage::connect_in_memory()
        .await
        .expect("in-memory db");

    let base_ts = (1770000000 / 86400) * 86400;

    // Create two 1-minute points for site 10 in hour base_ts
    let pt1 = SiteMetricsPoint {
        site_id: 10,
        bucket_ts: base_ts,
        total_requests: 100,
        total_bytes: 50000,
        status_2xx: 90,
        status_3xx: 5,
        status_4xx: 3,
        status_5xx: 2,
        cache_hit: 80,
        cache_miss: 20,
        cache_bypass: 0,
        cache_other: 0,
        latency_p50_ms: 25.0,
        latency_p95_ms: 90.0,
        latency_p99_ms: 180.0,
        latency_avg_ms: 35.0,
        upstream_time_avg_ms: Some(30.0),
        unique_visitors: 45,
        human_requests: 90,
        bot_requests: 10,
        http_v1: 15,
        http_v2: 85,
        http_v3: 0,
        tls_v12: 10,
        tls_v13: 90,
        tls_other: 0,
        top_paths: vec![panel_analytics::TopItem {
            name: "/home".to_string(),
            count: 60,
        }],
        top_referrers: vec![panel_analytics::TopItem {
            name: "google.com".to_string(),
            count: 40,
        }],
        top_countries: vec![panel_analytics::TopItem {
            name: "ID".to_string(),
            count: 80,
        }],
        top_browsers: vec![panel_analytics::TopItem {
            name: "Chrome".to_string(),
            count: 70,
        }],
        top_os: vec![panel_analytics::TopItem {
            name: "Android".to_string(),
            count: 65,
        }],
        top_devices: vec![panel_analytics::TopItem {
            name: "Mobile".to_string(),
            count: 65,
        }],
        top_slow_urls: vec![],
        top_error_urls: vec![],
        latency_buckets: vec![10, 20, 30, 20, 10, 5, 5, 0, 0, 0, 0, 0],
    };

    let pt2 = SiteMetricsPoint {
        site_id: 10,
        bucket_ts: base_ts + 60,
        total_requests: 50,
        total_bytes: 25000,
        status_2xx: 45,
        status_3xx: 2,
        status_4xx: 2,
        status_5xx: 1,
        cache_hit: 40,
        cache_miss: 10,
        cache_bypass: 0,
        cache_other: 0,
        latency_p50_ms: 20.0,
        latency_p95_ms: 70.0,
        latency_p99_ms: 120.0,
        latency_avg_ms: 28.0,
        upstream_time_avg_ms: Some(25.0),
        unique_visitors: 25,
        human_requests: 48,
        bot_requests: 2,
        http_v1: 5,
        http_v2: 45,
        http_v3: 0,
        tls_v12: 5,
        tls_v13: 45,
        tls_other: 0,
        top_paths: vec![panel_analytics::TopItem {
            name: "/home".to_string(),
            count: 30,
        }],
        top_referrers: vec![panel_analytics::TopItem {
            name: "google.com".to_string(),
            count: 20,
        }],
        top_countries: vec![panel_analytics::TopItem {
            name: "ID".to_string(),
            count: 40,
        }],
        top_browsers: vec![panel_analytics::TopItem {
            name: "Chrome".to_string(),
            count: 35,
        }],
        top_os: vec![panel_analytics::TopItem {
            name: "Android".to_string(),
            count: 30,
        }],
        top_devices: vec![panel_analytics::TopItem {
            name: "Mobile".to_string(),
            count: 30,
        }],
        top_slow_urls: vec![],
        top_error_urls: vec![],
        latency_buckets: vec![5, 10, 15, 10, 5, 3, 2, 0, 0, 0, 0, 0],
    };

    // 1. Insert 1m points
    storage
        .insert_site_metrics_1m(&[pt1, pt2])
        .await
        .expect("insert 1m");

    // 2. Query 1m points
    let queried_1m = storage
        .query_site_metrics_1m(10, base_ts, base_ts + 120)
        .await
        .expect("query 1m");
    assert_eq!(queried_1m.len(), 2);
    assert_eq!(queried_1m[0].total_requests, 100);
    assert_eq!(queried_1m[1].total_requests, 50);

    // 3. Execute site rollups for completed hour
    let report = storage
        .execute_site_rollups(base_ts + 3600)
        .await
        .expect("execute rollups");
    assert_eq!(report.site_1h_created, 1);

    // 4. Query 1-hour rollups
    let queried_1h = storage
        .query_site_rollups(10, base_ts, base_ts + 3600, RollupResolution::OneHour)
        .await
        .expect("query 1h");
    assert_eq!(queried_1h.len(), 1);
    assert_eq!(queried_1h[0].total_requests, 150);
    assert_eq!(queried_1h[0].total_bytes, 75000);
    assert_eq!(queried_1h[0].sample_count, 2);
    assert_eq!(queried_1h[0].top_paths[0].name, "/home");
    assert_eq!(queried_1h[0].top_paths[0].count, 90);

    // 5. Rollup 1h to 1d
    let report_day = storage
        .execute_site_rollups(base_ts + 86400)
        .await
        .expect("execute day rollups");
    assert_eq!(report_day.site_1d_created, 1);

    let queried_1d = storage
        .query_site_rollups(10, base_ts, base_ts + 86400, RollupResolution::OneDay)
        .await
        .expect("query 1d");
    assert_eq!(queried_1d.len(), 1);
    assert_eq!(queried_1d[0].total_requests, 150);

    // 6. Test retention pruning
    let test_retention = RetentionConfig {
        raw_retention_secs: 10,
        one_minute_retention_secs: 10,
        one_hour_retention_secs: 10,
        one_day_retention_secs: None,
        ..Default::default()
    };
    let (p_1m, p_1h) = storage
        .prune_expired_site_metrics(base_ts + 100, &test_retention)
        .await
        .expect("prune");
    assert_eq!(p_1m, 2);
    assert_eq!(p_1h, 1);
}
