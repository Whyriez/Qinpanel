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
use panel_analytics::{AnalyticsStorage, SecurityEvent, SecurityEventType, ServerMetricsPoint};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::retention::{
    AnalyticsRetentionResponse, TriggerMaintenanceRequest, UpdateRetentionRequest,
};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
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
    tenant_session: String,
    tenant_csrf: String,
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

    let analytics_storage = Arc::new(
        AnalyticsStorage::connect(&analytics_db_file)
            .await
            .expect("init analytics storage"),
    );

    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_hash = hash_password("AdminPass123!").expect("hash pass");
    let _admin = user_repo
        .create(&CreateUser {
            username: "admin_user".to_string(),
            password_hash: admin_hash,
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            plan_id: None,
            linux_username: Some("p_admin".to_string()),
            linux_uid: Some(10001),
        })
        .await
        .expect("create admin");

    let tenant_hash = hash_password("TenantPass123!").expect("hash pass");
    let _tenant = user_repo
        .create(&CreateUser {
            username: "tenant_bob".to_string(),
            password_hash: tenant_hash,
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            plan_id: None,
            linux_username: Some("p_bob".to_string()),
            linux_uid: Some(10002),
        })
        .await
        .expect("create tenant");

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };

    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(sock_path));
    let app_state = AppState::new(
        agent_client,
        pool.clone(),
        "test".to_string(),
        auth_config,
        lockout_manager,
    )
    .with_analytics(Arc::clone(&analytics_storage));

    let app = create_router(app_state);

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

        let cookie_header = login_resp
            .headers()
            .get(header::SET_COOKIE)
            .expect("set cookie header")
            .to_str()
            .expect("cookie str");
        let session_token =
            extract_cookie_value(cookie_header, SESSION_COOKIE_NAME).expect("session cookie");

        let csrf_req = Request::builder()
            .method("GET")
            .uri("/api/v1/auth/csrf")
            .header(
                header::COOKIE,
                format!("{SESSION_COOKIE_NAME}={session_token}"),
            )
            .body(Body::empty())
            .expect("csrf req");

        let csrf_resp = app_router
            .clone()
            .oneshot(csrf_req)
            .await
            .expect("csrf resp");
        let csrf_body = csrf_resp
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let csrf_json: serde_json::Value =
            serde_json::from_slice(&csrf_body).expect("parse csrf json");
        let csrf_token = csrf_json["csrf_token"]
            .as_str()
            .expect("csrf token")
            .to_string();

        (session_token, csrf_token)
    }

    let (admin_session, admin_csrf) = login_helper(&app, "admin_user", "AdminPass123!").await;
    let (tenant_session, tenant_csrf) = login_helper(&app, "tenant_bob", "TenantPass123!").await;

    TestContext {
        app,
        analytics: analytics_storage,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        tenant_session,
        tenant_csrf,
    }
}

#[tokio::test]
async fn test_get_retention_auth_and_overview() {
    let ctx = setup_test_context().await;

    // 1. Unauthenticated request -> 401
    let unauth_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/analytics/retention")
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(unauth_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. Tenant non-admin request -> 403 Forbidden
    let tenant_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/analytics/retention")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.tenant_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.tenant_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(tenant_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 3. Admin request -> 200 OK
    let admin_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/analytics/retention")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(admin_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let data: AnalyticsRetentionResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(data.retention.raw_retention_secs, 7200);
    assert_eq!(data.retention.one_minute_retention_secs, 14 * 86400);
    assert!(data.stats.is_healthy);
    assert_eq!(data.stats.integrity_message, "ok");
    assert_eq!(data.stats.table_stats.len(), 23);
}

#[tokio::test]
async fn test_update_retention_validation_and_persistence() {
    let ctx = setup_test_context().await;

    // 1. Invalid parameter (raw_retention_hours = 0) -> 400
    let invalid_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/system/analytics/retention")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&UpdateRetentionRequest {
                raw_retention_hours: Some(0),
                one_minute_retention_days: None,
                one_hour_retention_days: None,
                one_day_retention_days: None,
                security_events_retention_days: None,
                max_db_size_mb: None,
                auto_vacuum: None,
            })
            .unwrap(),
        ))
        .unwrap();
    let res = ctx.app.clone().oneshot(invalid_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 2. Valid update: 6h raw, 7d 1m, 90d 1h, 3d sec, max 500 MB, auto_vacuum true
    let valid_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/system/analytics/retention")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&UpdateRetentionRequest {
                raw_retention_hours: Some(6),
                one_minute_retention_days: Some(7),
                one_hour_retention_days: Some(90),
                one_day_retention_days: Some(Some(365)),
                security_events_retention_days: Some(3),
                max_db_size_mb: Some(Some(500)),
                auto_vacuum: Some(true),
            })
            .unwrap(),
        ))
        .unwrap();
    let res = ctx.app.clone().oneshot(valid_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. Verify GET returns updated configuration
    let get_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/analytics/retention")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let data: AnalyticsRetentionResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(data.retention.raw_retention_secs, 6 * 3600);
    assert_eq!(data.retention.one_minute_retention_secs, 7 * 86400);
    assert_eq!(data.retention.one_hour_retention_secs, 90 * 86400);
    assert_eq!(data.retention.one_day_retention_secs, Some(365 * 86400));
    assert_eq!(data.retention.security_events_retention_secs, 3 * 86400);
    assert_eq!(data.retention.max_db_size_bytes, Some(500 * 1024 * 1024));
    assert!(data.retention.auto_vacuum);
}

#[tokio::test]
async fn test_trigger_maintenance_and_integrity_check() {
    let ctx = setup_test_context().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Seed old records
    let pt = ServerMetricsPoint {
        timestamp: now - 50_000,
        cpu_percent: 10.0,
        cpu_cores: 2,
        memory_used_bytes: 1000,
        memory_total_bytes: 2000,
        swap_used_bytes: 0,
        swap_total_bytes: 1000,
        load1: 0.1,
        load5: 0.1,
        load15: 0.1,
        disk_used_bytes: 1000,
        disk_total_bytes: 10000,
        inodes_used: 10,
        inodes_total: 100,
        net_rx_bytes_sec: 10.0,
        net_tx_bytes_sec: 10.0,
        io_read_bytes_sec: 1.0,
        io_write_bytes_sec: 1.0,
    };
    ctx.analytics.insert_server_point(&pt).await.unwrap();

    let sec = SecurityEvent {
        id: None,
        site_id: 1,
        timestamp: now - 800_000,
        client_ip: "198.51.100.1".to_string(),
        method: "GET".to_string(),
        uri: "/.git/config".to_string(),
        status: 404,
        user_agent: None,
        country: None,
        event_type: SecurityEventType::Probing,
        threat_score: 90,
    };
    ctx.analytics.insert_security_events(&[sec]).await.unwrap();

    // 1. Trigger maintenance with force_vacuum
    let maint_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/analytics/maintenance")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&TriggerMaintenanceRequest {
                force_vacuum: Some(true),
            })
            .unwrap(),
        ))
        .unwrap();

    let res = ctx.app.clone().oneshot(maint_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let rep: panel_api::retention::RetentionMaintenanceReportDto =
        serde_json::from_slice(&body).unwrap();
    assert!(rep.raw_records_pruned >= 1);
    assert_eq!(rep.security_events_pruned, 1);
    assert!(rep.wal_checkpoint_executed);
    assert!(rep.vacuum_executed);

    // 2. Check integrity check endpoint
    let integ_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/analytics/integrity-check")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let res2 = ctx.app.clone().oneshot(integ_req).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);

    let body2 = res2.into_body().collect().await.unwrap().to_bytes();
    let integ: panel_api::retention::IntegrityCheckReportDto =
        serde_json::from_slice(&body2).unwrap();
    assert!(integ.is_healthy);
    assert_eq!(integ.status, "ok");
}
