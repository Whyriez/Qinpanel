#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::hardening::HardeningManager;
use panel_agent::quota::QuotaManager;
use panel_agent::resource::CgroupReader;
use panel_agent::slice::SliceManager;
use panel_agent::system::{MockSystemRunner, SystemRunner};
use panel_agent::user::UserLifecycleManager;
use panel_agent::webserver::NginxManager;
use panel_agent::{AgentConfig, AgentServer};
use panel_analytics::{
    AnalyticsStorage, AppProcessMetricPoint, AppType, ErrorUrlItem, LatencyHistogram,
    PhpFpmPoolMetricPoint, PhpSlowlogEntry, PhpSlowlogFrame, ServerMetricsPoint, SiteMetricsPoint,
    SlowUrlItem, TopItem,
};
use panel_api::agent_client::AgentClient;
use panel_api::analytics::{
    SiteAnalyticsOverviewResponse, SiteBreakdownsResponse, SiteTimeseriesResponse,
};
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqliteSiteRepository, SqliteUserRepository};
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    analytics: Arc<AnalyticsStorage>,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    admin_session: String,
    admin_csrf: String,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    bob_csrf: String,
    alice_site_id: i64,
    node_site_id: i64,
}

fn extract_cookie_value(cookie_header: &str, cookie_name: &str) -> Option<String> {
    for pair in cookie_header.split(';') {
        let (key, val) = pair.trim().split_once('=')?;
        if key == cookie_name {
            return Some(val.to_string());
        }
    }
    None
}

async fn setup_test_context() -> TestContext {
    // Match the provisioner's tenant UID range (10000..=59999).
    setup_test_context_with_alice_uid(10001).await
}

async fn setup_test_context_with_alice_uid(alice_uid: i64) -> TestContext {
    let tmp = tempdir().expect("tempdir");
    let db_file = tmp.path().join("panel.db");
    let analytics_db_file = tmp.path().join("analytics.db");

    #[cfg(unix)]
    let sock_path = tmp.path().join("agent.sock");
    #[cfg(not(unix))]
    let sock_path = tmp.path().join("agent.port");

    let cgroup_dir = tmp.path().join("cgroup");
    fs::create_dir_all(&cgroup_dir).expect("create cgroup dir");
    fs::write(
        cgroup_dir.join("cgroup.controllers"),
        "cpu memory io pids\n",
    )
    .expect("write controllers");

    let mounts_path = tmp.path().join("mounts");
    fs::write(
        &mounts_path,
        "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n",
    )
    .expect("write mounts");

    let group_path = tmp.path().join("group");
    fs::write(&group_path, "root:x:0:\n").expect("write group");

    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");

    let user_manager = Arc::new(UserLifecycleManager::new(
        Arc::clone(&runner) as Arc<dyn SystemRunner>
    ));
    let slice_manager = Arc::new(SliceManager::new(
        Arc::clone(&runner) as Arc<dyn SystemRunner>
    ));
    let quota_manager = Arc::new(QuotaManager::new(
        Arc::clone(&runner) as Arc<dyn SystemRunner>
    ));
    let resource_reader = Arc::new(CgroupReader::new(&cgroup_dir));
    let hardening_manager = Arc::new(HardeningManager::with_paths(
        Arc::clone(&runner) as Arc<dyn SystemRunner>,
        &cgroup_dir,
        &mounts_path,
        &group_path,
    ));
    let webserver_manager = Arc::new(NginxManager::new(
        Arc::clone(&runner) as Arc<dyn SystemRunner>
    ));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::with_all_managers(
        agent_cfg,
        user_manager,
        slice_manager,
        quota_manager,
        resource_reader,
        hardening_manager,
        webserver_manager,
    ));

    tokio::spawn(async move {
        let _ = server.run(rx).await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let analytics = Arc::new(
        AnalyticsStorage::connect(&analytics_db_file)
            .await
            .expect("init analytics storage"),
    );

    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_hash = hash_password("AdminPass123!").expect("hash pass");
    let admin = user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: admin_hash,
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            plan_id: None,
            linux_username: None,
            linux_uid: None,
        })
        .await
        .expect("create admin");

    let alice_hash = hash_password("AlicePass123!").expect("hash pass");
    let alice = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            plan_id: None,
            linux_username: Some("p_alice".to_string()),
            linux_uid: Some(alice_uid),
        })
        .await
        .expect("create alice");

    let bob_hash = hash_password("BobPass123!").expect("hash pass");
    let bob = user_repo
        .create(&CreateUser {
            username: "bob".to_string(),
            password_hash: bob_hash,
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            plan_id: None,
            linux_username: Some("p_bob".to_string()),
            linux_uid: Some(10002),
        })
        .await
        .expect("create bob");

    let site_repo = SqliteSiteRepository::new(pool.clone());
    let alice_site = site_repo
        .create(&CreateSite {
            user_id: alice.id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/public_html".to_string(),
            runtime: Some("php:8.3".to_string()),
            version: None,
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create alice site");

    let node_site = site_repo
        .create(&CreateSite {
            user_id: alice.id,
            kind: "nodejs".to_string(),
            docroot: "/home/p_alice/node_app".to_string(),
            runtime: Some("node:20".to_string()),
            version: None,
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create node site");

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };

    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(sock_path));
    let state = AppState::new(
        agent_client,
        pool.clone(),
        "test".to_string(),
        auth_config,
        lockout_manager,
    )
    .with_analytics(Arc::clone(&analytics));

    let app = create_router(state);

    async fn login_helper(app_router: &axum::Router, user: &str, pass: &str) -> (String, String) {
        let login_body = serde_json::json!({
            "username": user,
            "password": pass,
        })
        .to_string();

        let login_req = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(login_body))
            .expect("login req");

        let login_resp = app_router
            .clone()
            .oneshot(login_req)
            .await
            .expect("login resp");
        assert_eq!(login_resp.status(), StatusCode::OK);

        let cookies = login_resp
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().expect("to str").to_string())
            .collect::<Vec<_>>();

        let mut session_token = String::new();
        let mut csrf_token = String::new();
        for c in &cookies {
            if let Some(s) = extract_cookie_value(c, SESSION_COOKIE_NAME) {
                session_token = s;
            }
            if let Some(cs) = extract_cookie_value(c, "csrf_token") {
                csrf_token = cs;
            }
        }

        (session_token, csrf_token)
    }

    let (admin_session, admin_csrf) = login_helper(&app, &admin.username, "AdminPass123!").await;
    let (alice_session, alice_csrf) = login_helper(&app, &alice.username, "AlicePass123!").await;
    let (bob_session, bob_csrf) = login_helper(&app, &bob.username, "BobPass123!").await;

    TestContext {
        app,
        analytics,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        alice_site_id: alice_site.id,
        node_site_id: node_site.id,
    }
}

fn sample_site_point(site_id: i64, bucket_ts: i64, reqs: u64, bytes: u64) -> SiteMetricsPoint {
    let mut hist = LatencyHistogram::default();
    hist.record_ms(15.0);
    hist.record_ms(45.0);
    hist.record_ms(120.0);
    hist.record_ms(500.0);

    SiteMetricsPoint {
        site_id,
        bucket_ts,
        total_requests: reqs,
        total_bytes: bytes,
        status_2xx: reqs * 8 / 10,
        status_3xx: reqs / 10,
        status_4xx: reqs / 20,
        status_5xx: reqs / 20,
        cache_hit: reqs * 6 / 10,
        cache_miss: reqs * 4 / 10,
        cache_bypass: 0,
        cache_other: 0,
        latency_p50_ms: hist.percentile_ms(50.0),
        latency_p95_ms: hist.percentile_ms(95.0),
        latency_p99_ms: hist.percentile_ms(99.0),
        latency_avg_ms: 65.5,
        upstream_time_avg_ms: None,
        unique_visitors: (reqs / 5).max(1),
        human_requests: reqs,
        bot_requests: 0,
        http_v1: 0,
        http_v2: reqs,
        http_v3: 0,
        tls_v12: 0,
        tls_v13: reqs,
        tls_other: 0,
        top_paths: vec![
            TopItem {
                name: "/".to_string(),
                count: reqs / 2,
            },
            TopItem {
                name: "/api/items".to_string(),
                count: reqs / 3,
            },
        ],
        top_referrers: vec![TopItem {
            name: "google.com".to_string(),
            count: reqs / 2,
        }],
        top_countries: vec![
            TopItem {
                name: "ID".to_string(),
                count: reqs * 7 / 10,
            },
            TopItem {
                name: "SG".to_string(),
                count: reqs * 3 / 10,
            },
        ],
        top_browsers: vec![TopItem {
            name: "Chrome".to_string(),
            count: reqs * 8 / 10,
        }],
        top_os: vec![TopItem {
            name: "Windows".to_string(),
            count: reqs * 6 / 10,
        }],
        top_devices: vec![TopItem {
            name: "Desktop".to_string(),
            count: reqs * 7 / 10,
        }],
        top_slow_urls: vec![SlowUrlItem {
            uri: "/heavy-task".to_string(),
            avg_time_ms: 1250.0,
            count: 12,
        }],
        top_error_urls: vec![ErrorUrlItem {
            uri: "/not-found".to_string(),
            status: 404,
            count: 25,
        }],
        latency_buckets: hist.buckets.to_vec(),
    }
}

#[tokio::test]
async fn test_site_analytics_overview_and_histogram_interpolation() {
    let ctx = setup_test_context().await;
    let base_ts = 1_700_000_000;

    // Seed 1m points for alice site
    let p1 = sample_site_point(ctx.alice_site_id, base_ts, 1000, 500_000);
    let p2 = sample_site_point(ctx.alice_site_id, base_ts + 60, 2000, 1_000_000);
    ctx.analytics
        .insert_site_metrics_1m(&[p1, p2])
        .await
        .expect("insert points");

    let req = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/overview?from={}&to={}&interval=1m",
            ctx.alice_site_id,
            base_ts - 10,
            base_ts + 120
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let body: SiteAnalyticsOverviewResponse = serde_json::from_slice(&bytes).expect("parse json");

    assert_eq!(body.site_id, ctx.alice_site_id);
    assert_eq!(body.current.total_requests, 3000);
    assert_eq!(body.current.total_bytes, 1_500_000);
    assert!(body.current.latency_p50_ms > 0.0);
    assert!(body.current.latency_p95_ms >= body.current.latency_p50_ms);
    assert!(body.current.latency_p99_ms >= body.current.latency_p95_ms);
}

#[tokio::test]
async fn test_site_analytics_auto_resolution_selection() {
    let ctx = setup_test_context().await;
    let now = 1_700_000_000;

    // Case 1: <= 6 hours -> auto resolves to 1m
    let req1 = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/timeseries?from={}&to={}&interval=auto",
            ctx.alice_site_id,
            now - 3600 * 4,
            now
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req1");
    let resp1 = ctx.app.clone().oneshot(req1).await.expect("resp1");
    assert_eq!(resp1.status(), StatusCode::OK);
    let bytes1 = resp1
        .into_body()
        .collect()
        .await
        .expect("bytes1")
        .to_bytes();
    let body1: SiteTimeseriesResponse = serde_json::from_slice(&bytes1).expect("body1");
    assert_eq!(body1.interval, "1m");

    // Case 2: 6 hours to 30 days -> auto resolves to 1h
    let req2 = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/timeseries?from={}&to={}&interval=auto",
            ctx.alice_site_id,
            now - 86400 * 7,
            now
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req2");
    let resp2 = ctx.app.clone().oneshot(req2).await.expect("resp2");
    assert_eq!(resp2.status(), StatusCode::OK);
    let bytes2 = resp2
        .into_body()
        .collect()
        .await
        .expect("bytes2")
        .to_bytes();
    let body2: SiteTimeseriesResponse = serde_json::from_slice(&bytes2).expect("body2");
    assert_eq!(body2.interval, "1h");

    // Case 3: > 30 days -> auto resolves to 1d
    let req3 = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/timeseries?from={}&to={}&interval=auto",
            ctx.alice_site_id,
            now - 86400 * 60,
            now
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req3");
    let resp3 = ctx.app.clone().oneshot(req3).await.expect("resp3");
    assert_eq!(resp3.status(), StatusCode::OK);
    let bytes3 = resp3
        .into_body()
        .collect()
        .await
        .expect("bytes3")
        .to_bytes();
    let body3: SiteTimeseriesResponse = serde_json::from_slice(&bytes3).expect("body3");
    assert_eq!(body3.interval, "1d");
}

#[tokio::test]
async fn test_query_cost_limits_and_validation_guardrails() {
    let ctx = setup_test_context().await;
    let now = 1_700_000_000;

    // 1. from > to -> 400 Bad Request
    let req_invalid = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/overview?from={}&to={}",
            ctx.alice_site_id,
            now + 100,
            now
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let resp_invalid = ctx.app.clone().oneshot(req_invalid).await.expect("resp");
    assert_eq!(resp_invalid.status(), StatusCode::BAD_REQUEST);

    // 2. 1m interval requesting > 7 days -> 400 Bad Request
    let req_1m_over = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/overview?from={}&to={}&interval=1m",
            ctx.alice_site_id,
            now - 86400 * 8,
            now
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let resp_1m_over = ctx.app.clone().oneshot(req_1m_over).await.expect("resp");
    assert_eq!(resp_1m_over.status(), StatusCode::BAD_REQUEST);

    // 3. 1h interval requesting > 90 days -> 400 Bad Request
    let req_1h_over = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/overview?from={}&to={}&interval=1h",
            ctx.alice_site_id,
            now - 86400 * 95,
            now
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let resp_1h_over = ctx.app.clone().oneshot(req_1h_over).await.expect("resp");
    assert_eq!(resp_1h_over.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_period_comparison_delta_calculation() {
    let ctx = setup_test_context().await;
    let base_ts = 1_700_000_000;

    // Previous period: 100 requests
    let prev = sample_site_point(ctx.alice_site_id, base_ts - 3600, 100, 50_000);
    // Current period: 150 requests (50% increase)
    let curr = sample_site_point(ctx.alice_site_id, base_ts, 150, 75_000);

    ctx.analytics
        .insert_site_metrics_1m(&[prev, curr])
        .await
        .expect("insert");

    let req = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/compare?from={}&to={}&prev_from={}&prev_to={}&interval=1m",
            ctx.alice_site_id,
            base_ts - 60,
            base_ts + 60,
            base_ts - 3660,
            base_ts - 3540
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let body: SiteAnalyticsOverviewResponse = serde_json::from_slice(&bytes).expect("parse");

    assert!(body.comparison.is_some());
    let comp = body.comparison.unwrap();
    // Delta percentage for 100 -> 150 requests is +50.0%
    assert!((comp.requests_change_percent - 50.0).abs() < 0.1);
}

#[tokio::test]
async fn test_breakdown_aggregations() {
    let ctx = setup_test_context().await;
    let base_ts = 1_700_000_000;

    let p = sample_site_point(ctx.alice_site_id, base_ts, 1000, 500_000);
    ctx.analytics
        .insert_site_metrics_1m(&[p])
        .await
        .expect("insert");

    let req = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/breakdowns?from={}&to={}&limit=5",
            ctx.alice_site_id,
            base_ts - 60,
            base_ts + 60
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let body: SiteBreakdownsResponse = serde_json::from_slice(&bytes).expect("parse");

    assert_eq!(body.site_id, ctx.alice_site_id);
    assert!(!body.paths.is_empty());
    assert_eq!(body.paths[0].name, "/");
    assert!(!body.countries.is_empty());
    assert_eq!(body.countries[0].name, "ID");
}

#[tokio::test]
async fn test_csv_export_headers_and_content() {
    let ctx = setup_test_context().await;
    let base_ts = 1_700_000_000;

    let p = sample_site_point(ctx.alice_site_id, base_ts, 500, 250_000);
    ctx.analytics
        .insert_site_metrics_1m(&[p])
        .await
        .expect("insert");

    let req = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/export?from={}&to={}&interval=1m",
            ctx.alice_site_id,
            base_ts - 60,
            base_ts + 60
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/csv; charset=utf-8"
    );
    let disposition = resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(disposition.starts_with("attachment; filename=\"site-"));

    let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let csv_str = String::from_utf8(body_bytes.to_vec()).expect("utf8");
    assert!(csv_str.starts_with("Timestamp,DateTime,Requests,BytesSent"));
    assert!(csv_str.contains("500,250000"));
}

#[tokio::test]
async fn test_runtime_app_metrics_php_and_node() {
    let ctx = setup_test_context().await;
    let base_ts = 1_700_000_000;

    // 1. PHP-FPM pool metrics for alice
    let pool_name = "p_alice-8.3";
    for i in 0..6 {
        let pt = PhpFpmPoolMetricPoint {
            timestamp: base_ts + i * 10,
            pool_name: pool_name.to_string(),
            user: Some("p_alice".to_string()),
            php_version: Some("8.3".to_string()),
            active_processes: 5,
            idle_processes: 10,
            total_processes: 15,
            max_active_processes: 8,
            max_children_reached: 0,
            listen_queue: 1,
            max_listen_queue: 2,
            listen_queue_len: 128,
            slow_requests: 3,
            accepted_conn: 100 + i as u64 * 10,
        };
        ctx.analytics
            .insert_php_fpm_point(&pt)
            .await
            .expect("fpm pt");
    }
    ctx.analytics
        .rollup_php_fpm_1m(base_ts + 70)
        .await
        .expect("rollup fpm");

    let req_fpm = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/app?from={}&to={}",
            ctx.alice_site_id,
            base_ts - 60,
            base_ts + 120
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let resp_fpm = ctx.app.clone().oneshot(req_fpm).await.expect("resp");
    let status_fpm = resp_fpm.status();

    let bytes = resp_fpm
        .into_body()
        .collect()
        .await
        .expect("bytes")
        .to_bytes();
    assert_eq!(
        status_fpm,
        StatusCode::OK,
        "PHP metrics response: {}",
        String::from_utf8_lossy(&bytes)
    );
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("parse");
    assert_eq!(body["site_id"], ctx.alice_site_id);
    assert_eq!(body["runtime"], "php:8.3");
    assert_eq!(body["php_fpm"]["pool_name"], pool_name);
    assert_eq!(body["php_fpm"]["active_avg"], 5.0);

    // 2. Node.js process metrics
    for i in 0..6 {
        let pt = AppProcessMetricPoint {
            timestamp: base_ts + i * 10,
            site_id: ctx.node_site_id,
            app_type: AppType::Node,
            service_name: "node-site-1.service".to_string(),
            cpu_usage_usec: 50_000,
            cpu_percent: 12.5,
            memory_current_bytes: 64 * 1024 * 1024,
            memory_peak_bytes: Some(70 * 1024 * 1024),
            memory_swap_bytes: None,
            pids_current: 4,
            io_read_bytes_sec: 1024.0,
            io_write_bytes_sec: 2048.0,
        };
        ctx.analytics
            .insert_app_process_point(&pt)
            .await
            .expect("node pt");
    }
    ctx.analytics
        .rollup_app_process_1m(base_ts + 70)
        .await
        .expect("rollup node");

    let req_node = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/app?from={}&to={}",
            ctx.node_site_id,
            base_ts - 60,
            base_ts + 120
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let resp_node = ctx.app.clone().oneshot(req_node).await.expect("resp");
    let status_node = resp_node.status();

    let bytes_node = resp_node
        .into_body()
        .collect()
        .await
        .expect("bytes")
        .to_bytes();
    assert_eq!(
        status_node,
        StatusCode::OK,
        "Node metrics response: {}",
        String::from_utf8_lossy(&bytes_node)
    );
    let body_node: serde_json::Value = serde_json::from_slice(&bytes_node).expect("parse");
    assert_eq!(body_node["site_id"], ctx.node_site_id);
    assert_eq!(body_node["app_process"]["app_type"], "node");
    assert_eq!(body_node["app_process"]["cpu_avg"], 12.5);
}

#[tokio::test]
async fn test_php_app_metrics_reject_invalid_tenant_uid_for_owner_and_admin() {
    // A regular system UID must stay invalid even when the requester is an admin.
    let ctx = setup_test_context_with_alice_uid(1001).await;
    let base_ts = 1_700_000_000;

    for (session, csrf) in [
        (&ctx.alice_session, &ctx.alice_csrf),
        (&ctx.admin_session, &ctx.admin_csrf),
    ] {
        let req = Request::builder()
            .uri(format!(
                "/api/v1/sites/{}/analytics/app?from={}&to={}",
                ctx.alice_site_id,
                base_ts - 60,
                base_ts + 120
            ))
            .header(header::COOKIE, format!("{SESSION_COOKIE_NAME}={session}"))
            .header(CSRF_HEADER_NAME, csrf)
            .body(Body::empty())
            .expect("req");
        let resp = ctx.app.clone().oneshot(req).await.expect("resp");
        let status = resp.status();
        let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "Invalid tenant UID response: {}",
            String::from_utf8_lossy(&bytes)
        );
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("parse");
        assert_eq!(
            body["error"],
            "Tenant belum memiliki identitas Linux yang valid. Provision tenant terlebih dahulu."
        );
    }
}

#[tokio::test]
async fn test_structured_slowlog_events() {
    let ctx = setup_test_context().await;
    let base_ts = 1_700_000_000;

    let slow_entry = PhpSlowlogEntry {
        id: None,
        timestamp: base_ts,
        pool: "p_alice-8.3".to_string(),
        pid: 24510,
        script_filename: "/home/p_alice/public_html/index.php".to_string(),
        top_function: Some("PDOStatement->execute()".to_string()),
        frames: vec![
            PhpSlowlogFrame {
                frame_index: 0,
                function: "PDOStatement->execute()".to_string(),
                file: "/home/p_alice/public_html/vendor/db.php".to_string(),
                line: 88,
                address: Some("0x7f88a910".to_string()),
            },
            PhpSlowlogFrame {
                frame_index: 1,
                function: "run_query()".to_string(),
                file: "/home/p_alice/public_html/index.php".to_string(),
                line: 25,
                address: None,
            },
        ],
        duration_secs: Some(3.45),
        site_id: Some(ctx.alice_site_id),
        tenant_user: Some("alice".to_string()),
    };

    ctx.analytics
        .insert_slowlog_entry(&slow_entry)
        .await
        .expect("insert slowlog");

    let req = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/slowlog?from={}&to={}",
            ctx.alice_site_id,
            base_ts - 60,
            base_ts + 60
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("parse");
    assert_eq!(body["site_id"], ctx.alice_site_id);
    let events = body["events"].as_array().expect("events array");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["top_function"], "PDOStatement->execute()");
    assert_eq!(events[0]["frames"][0]["line"], 88);
}

#[tokio::test]
async fn test_system_timeseries_admin_guard_and_data() {
    let ctx = setup_test_context().await;
    let base_ts = 1_700_000_000;

    // 1. Non-admin (Alice) gets 403 Forbidden
    let req_alice = Request::builder()
        .uri(format!(
            "/api/v1/system/analytics/timeseries?from={}&to={}",
            base_ts - 60,
            base_ts + 60
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");

    let resp_alice = ctx.app.clone().oneshot(req_alice).await.expect("resp");
    assert_eq!(resp_alice.status(), StatusCode::FORBIDDEN);

    // 2. Insert host metrics and rollup for Admin
    for i in 0..6 {
        let pt = ServerMetricsPoint {
            timestamp: base_ts + i * 10,
            cpu_percent: 25.0,
            cpu_cores: 4,
            memory_used_bytes: 4 * 1024 * 1024 * 1024,
            memory_total_bytes: 8 * 1024 * 1024 * 1024,
            swap_used_bytes: 0,
            swap_total_bytes: 2 * 1024 * 1024 * 1024,
            load1: 1.2,
            load5: 1.0,
            load15: 0.8,
            disk_used_bytes: 50 * 1024 * 1024 * 1024,
            disk_total_bytes: 100 * 1024 * 1024 * 1024,
            inodes_used: 100_000,
            inodes_total: 1_000_000,
            net_rx_bytes_sec: 10_000.0,
            net_tx_bytes_sec: 20_000.0,
            io_read_bytes_sec: 5000.0,
            io_write_bytes_sec: 8000.0,
        };
        ctx.analytics.insert_server_point(&pt).await.expect("pt");
    }
    ctx.analytics.rollup_1m(base_ts + 70).await.expect("rollup");

    let req_admin = Request::builder()
        .uri(format!(
            "/api/v1/system/analytics/timeseries?from={}&to={}&interval=1m",
            base_ts - 60,
            base_ts + 120
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .expect("req");

    let resp_admin = ctx.app.clone().oneshot(req_admin).await.expect("resp");
    assert_eq!(resp_admin.status(), StatusCode::OK);

    let bytes = resp_admin
        .into_body()
        .collect()
        .await
        .expect("bytes")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("parse");
    let points = body["points"].as_array().expect("points");
    assert!(!points.is_empty());
    assert_eq!(points[0]["cpu_avg"], 25.0);
}

#[tokio::test]
async fn test_tenant_isolation_prevents_unauthorized_site_analytics() {
    let ctx = setup_test_context().await;
    let base_ts = 1_700_000_000;

    // Bob tries to access Alice's site analytics
    let req = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/analytics/overview?from={}&to={}",
            ctx.alice_site_id,
            base_ts - 60,
            base_ts + 60
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    // Should be 403 Forbidden or 404 Not Found due to tenant isolation
    assert!(resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND);
}
