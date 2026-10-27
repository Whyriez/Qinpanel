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
use panel_analytics::{AnalyticsStorage, SiteMetricsPoint};
use panel_api::agent_client::AgentClient;
use panel_api::analytics::{SiteInsightsResponse, SystemInsightsResponse};
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateSite, CreateUser};
use panel_db::pool::{DbPool, init_pool, run_migrations};
use panel_db::repositories::{
    SqliteNotificationRepository, SqliteSiteRepository, SqliteUserRepository,
};
use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;
use tower::ServiceExt;

#[allow(dead_code)]
struct TestContext {
    app: axum::Router,
    pool: DbPool,
    analytics: Arc<AnalyticsStorage>,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    admin_session: String,
    admin_csrf: String,
    alice_session: String,
    alice_csrf: String,
    alice_user_id: i64,
    bob_session: String,
    bob_csrf: String,
    alice_site_id: i64,
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
            linux_uid: Some(10001),
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
        pool,
        analytics,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        alice_user_id: alice.id,
        bob_session,
        bob_csrf,
        alice_site_id: alice_site.id,
    }
}

#[tokio::test]
async fn test_get_site_insights_auth_and_rbac() {
    let ctx = setup_test_context().await;

    // 1. Unauthenticated request -> 401 Unauthorized
    let unauth_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/analytics/insights",
            ctx.alice_site_id
        ))
        .body(Body::empty())
        .expect("unauth req");
    let resp = ctx.app.clone().oneshot(unauth_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 2. Bob tries to access Alice's site -> 403 Forbidden
    let bob_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/analytics/insights",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .body(Body::empty())
        .expect("bob req");
    let resp = ctx.app.clone().oneshot(bob_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 3. Alice accesses her own site -> 200 OK
    let alice_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/analytics/insights",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("alice req");
    let resp = ctx.app.clone().oneshot(alice_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let body: SiteInsightsResponse = serde_json::from_slice(&bytes).expect("parse site insights");
    assert_eq!(body.site_id, ctx.alice_site_id);

    // 4. Admin accesses Alice's site -> 200 OK
    let admin_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/analytics/insights",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("admin req");
    let resp = ctx.app.clone().oneshot(admin_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_evaluate_site_insights_and_notification() {
    let ctx = setup_test_context().await;
    let notif_repo = SqliteNotificationRepository::new(ctx.pool.clone());

    // 1. Insert a site metric point with high 5xx error rate (20% > 3% threshold)
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_secs() as i64;

    let point = SiteMetricsPoint {
        site_id: ctx.alice_site_id,
        bucket_ts: now_secs - 120,
        total_requests: 100,
        total_bytes: 50_000,
        status_2xx: 70,
        status_3xx: 0,
        status_4xx: 10,
        status_5xx: 20,
        cache_hit: 50,
        cache_miss: 50,
        cache_bypass: 0,
        cache_other: 0,
        latency_p50_ms: 30.0,
        latency_p95_ms: 120.0,
        latency_p99_ms: 250.0,
        latency_avg_ms: 45.0,
        upstream_time_avg_ms: None,
        unique_visitors: 40,
        human_requests: 95,
        bot_requests: 5,
        http_v1: 0,
        http_v2: 100,
        http_v3: 0,
        tls_v12: 0,
        tls_v13: 100,
        tls_other: 0,
        top_paths: vec![],
        top_referrers: vec![],
        top_countries: vec![],
        top_browsers: vec![],
        top_os: vec![],
        top_devices: vec![],
        top_slow_urls: vec![],
        top_error_urls: vec![],
        latency_buckets: vec![],
    };
    ctx.analytics
        .insert_site_metrics_1m(&[point])
        .await
        .expect("insert site point");

    // 2. Alice triggers POST evaluate
    let eval_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/sites/{}/analytics/insights/evaluate",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("eval req");

    let resp = ctx.app.clone().oneshot(eval_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let body: SiteInsightsResponse = serde_json::from_slice(&bytes).expect("parse evaluate body");
    assert_eq!(body.site_id, ctx.alice_site_id);
    assert!(!body.insights.is_empty(), "Expected at least 1 insight");

    // Verify the five_xx_spike insight exists and has critical severity
    let spike_insight = body
        .insights
        .iter()
        .find(|i| i.insight_type == "five_xx_spike");
    assert!(spike_insight.is_some(), "Expected five_xx_spike insight");
    let spike = spike_insight.expect("spike");
    assert_eq!(spike.severity, "critical");
    assert!(spike.action.is_some());
    assert_eq!(
        spike.action.as_ref().map(|a| a.action_type.as_str()),
        Some("view_logs")
    );

    // 3. Verify notification was dispatched to NotificationService & stored in DB
    let (notifs, _) = notif_repo
        .list(
            Some(ctx.alice_user_id),
            &panel_db::models::NotificationFilter {
                unread_only: false,
                category: None,
                severity: None,
                limit: 10,
                offset: 0,
            },
        )
        .await
        .expect("list notifs");

    let insight_notif = notifs.iter().find(|n| n.category == "analytics_insight");
    assert!(
        insight_notif.is_some(),
        "Expected analytics_insight notification in database"
    );
    let n = insight_notif.expect("insight notif");
    assert_eq!(n.severity, "critical");
    assert!(n.title.contains("Lonjakan Server Error 5xx"));
}

#[tokio::test]
async fn test_system_insights_admin_only() {
    let ctx = setup_test_context().await;

    // 1. Alice (normal user) gets 403 Forbidden
    let alice_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/analytics/insights")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("alice req");
    let resp = ctx.app.clone().oneshot(alice_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 2. Admin gets 200 OK
    let admin_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/analytics/insights")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("admin req");
    let resp = ctx.app.clone().oneshot(admin_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let body: SystemInsightsResponse =
        serde_json::from_slice(&bytes).expect("parse system insights");
    assert!(body.evaluated_at.is_positive());
}
