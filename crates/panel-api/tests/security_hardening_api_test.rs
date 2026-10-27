#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::hardening::HardeningManager;
use panel_agent::nftables::NftablesManager;
use panel_agent::optimizer::OptimizerManager;
use panel_agent::quota::QuotaManager;
use panel_agent::resource::CgroupReader;
use panel_agent::slice::SliceManager;
use panel_agent::system::{MockSystemRunner, SystemRunner};
use panel_agent::user::UserLifecycleManager;
use panel_agent::webserver::NginxManager;
use panel_agent::{AgentConfig, AgentServer};
use panel_analytics::AnalyticsStorage;
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_api::security_hardening::{
    NftablesSyncPayload, NftablesSyncResponseDto, SecurityHardeningOverviewResponse,
    TenantPortBindingDto,
};
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    admin_session: String,
    admin_csrf: String,
    tenant_session: String,
    _tenant_csrf: String,
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

    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nft", 0, b"", b"");
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("sysctl", 0, b"", b"");

    let nft_rules_file = tmp.path().join("panel-loopback-guard.nft");

    let user_mgr = Arc::new(UserLifecycleManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let slice_mgr = Arc::new(SliceManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let quota_mgr = Arc::new(QuotaManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let resource_reader = Arc::new(CgroupReader::default());
    let hardening_mgr = Arc::new(HardeningManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let webserver_mgr = Arc::new(NginxManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let optimizer_mgr = Arc::new(OptimizerManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let nftables_mgr = Arc::new(NftablesManager::with_path(
        runner.clone() as Arc<dyn SystemRunner>,
        &nft_rules_file,
    ));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let server = Arc::new(
        AgentServer::with_all_managers(
            agent_cfg,
            user_mgr,
            slice_mgr,
            quota_mgr,
            resource_reader,
            hardening_mgr,
            webserver_mgr,
        )
        .with_optimizer_manager(optimizer_mgr)
        .with_nftables_manager(nftables_mgr),
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let srv_clone = server.clone();
    tokio::spawn(async move {
        let _ = srv_clone.run(shutdown_rx).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init db pool");
    run_migrations(&pool).await.expect("run db migrations");

    let analytics = Arc::new(
        AnalyticsStorage::connect(&analytics_db_file)
            .await
            .expect("open analytics db"),
    );

    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_pass_hash = hash_password("AdminSecret123!").expect("hash admin");
    let _admin_user = user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            password_hash: admin_pass_hash,
            role: "admin".to_string(),
            plan_id: None,
            linux_username: Some("p_admin".to_string()),
            linux_uid: Some(10001),
        })
        .await
        .expect("create admin");

    let tenant_pass_hash = hash_password("TenantSecret123!").expect("hash tenant");
    let _tenant_user = user_repo
        .create(&CreateUser {
            username: "tenant1".to_string(),
            email: Some("tenant1@example.com".to_string()),
            password_hash: tenant_pass_hash,
            role: "user".to_string(),
            plan_id: None,
            linux_username: Some("p_tenant1".to_string()),
            linux_uid: Some(10002),
        })
        .await
        .expect("create tenant");

    let auth_cfg = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };
    let agent_client = Arc::new(AgentClient::new(sock_path));
    let lockout = Arc::new(LockoutManager::new(5, 300));
    let state = AppState::new(
        agent_client,
        pool.clone(),
        "test".to_string(),
        auth_cfg,
        lockout,
    )
    .with_analytics(analytics);
    let app = create_router(state);

    // Login admin
    let login_admin_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "username": "admin",
                "password": "AdminSecret123!"
            })
            .to_string(),
        ))
        .unwrap();

    let res = app
        .clone()
        .oneshot(login_admin_req)
        .await
        .expect("login res");
    assert_eq!(res.status(), StatusCode::OK);
    let cookie_val = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .expect("cookie str");
    let admin_session = extract_cookie_value(cookie_val, SESSION_COOKIE_NAME).expect("session");
    let body_bytes = res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let admin_login_body: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("parse login body");
    let admin_csrf = admin_login_body["csrf_token"]
        .as_str()
        .expect("csrf_token")
        .to_string();

    // Login tenant
    let login_tenant_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "username": "tenant1",
                "password": "TenantSecret123!"
            })
            .to_string(),
        ))
        .unwrap();

    let res2 = app
        .clone()
        .oneshot(login_tenant_req)
        .await
        .expect("login res2");
    assert_eq!(res2.status(), StatusCode::OK);
    let cookie_val2 = res2
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie2")
        .to_str()
        .expect("cookie str2");
    let tenant_session = extract_cookie_value(cookie_val2, SESSION_COOKIE_NAME).expect("session2");
    let body_bytes2 = res2
        .into_body()
        .collect()
        .await
        .expect("read body2")
        .to_bytes();
    let tenant_login_body: serde_json::Value =
        serde_json::from_slice(&body_bytes2).expect("parse login body2");
    let tenant_csrf = tenant_login_body["csrf_token"]
        .as_str()
        .expect("csrf_token2")
        .to_string();

    TestContext {
        app,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        tenant_session,
        _tenant_csrf: tenant_csrf,
    }
}

#[tokio::test]
async fn test_security_headers_middleware_injected() {
    let ctx = setup_test_context().await;
    let req = Request::builder()
        .uri("/healthz")
        .method("GET")
        .body(Body::empty())
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let headers = res.headers();
    assert!(headers.get("Content-Security-Policy").is_some());
    let csp = headers
        .get("Content-Security-Policy")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));

    assert_eq!(
        headers
            .get("X-Content-Type-Options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        headers.get("X-Frame-Options").and_then(|v| v.to_str().ok()),
        Some("DENY")
    );
    assert_eq!(
        headers
            .get("Strict-Transport-Security")
            .and_then(|v| v.to_str().ok()),
        Some("max-age=31536000; includeSubDomains; preload")
    );
}

#[tokio::test]
async fn test_get_security_hardening_overview_admin() {
    let ctx = setup_test_context().await;
    let req = Request::builder()
        .uri("/api/v1/system/security-hardening")
        .method("GET")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res.into_body().collect().await.expect("bytes").to_bytes();
    let overview: SecurityHardeningOverviewResponse =
        serde_json::from_slice(&body_bytes).expect("overview");

    assert_eq!(overview.asvs_audit.target_level, "Level 2 (L2)");
    assert_eq!(overview.asvs_audit.compliance_percentage, 100.0);
    assert!(overview.asvs_audit.passed_controls >= 10);
    assert_eq!(overview.asvs_audit.failed_controls, 0);

    assert_eq!(overview.nftables_status.table_name, "panel_loopback_guard");

    assert_eq!(overview.systemd_security.len(), 2);
    assert_eq!(
        overview.systemd_security,
        panel_core::hardening::get_default_service_security_evaluations(),
        "API must report the template assessment without hiding privileged-agent exposure"
    );
    let api = overview
        .systemd_security
        .iter()
        .find(|score| score.service_name == "panel-api.service")
        .expect("API score");
    assert!(!api.is_root);
    assert!(api.exposure_score <= 2.0);
    assert_eq!(api.rating, "OK");
    let agent = overview
        .systemd_security
        .iter()
        .find(|score| score.service_name == "panel-agent.service")
        .expect("agent score");
    assert!(agent.is_root);
    assert!(agent.exposure_score > 2.0 && agent.exposure_score <= 5.0);
    assert_eq!(agent.rating, "MEDIUM");
    assert!(api.sandboxed && agent.sandboxed);
    assert!(overview.secret_vault_active);
}

#[tokio::test]
async fn test_get_security_hardening_overview_unauthorized_and_forbidden() {
    let ctx = setup_test_context().await;

    // 1. Unauthenticated -> 401
    let unauth_req = Request::builder()
        .uri("/api/v1/system/security-hardening")
        .method("GET")
        .body(Body::empty())
        .expect("req");

    let res1 = ctx.app.clone().oneshot(unauth_req).await.expect("res1");
    assert_eq!(res1.status(), StatusCode::UNAUTHORIZED);

    // 2. Non-admin tenant -> 403
    let forbidden_req = Request::builder()
        .uri("/api/v1/system/security-hardening")
        .method("GET")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.tenant_session),
        )
        .body(Body::empty())
        .expect("req");

    let res2 = ctx.app.clone().oneshot(forbidden_req).await.expect("res2");
    assert_eq!(res2.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_sync_nftables_loopback_admin_success() {
    let ctx = setup_test_context().await;

    let payload = NftablesSyncPayload {
        custom_bindings: Some(vec![
            TenantPortBindingDto {
                site_id: 101,
                tenant_uid: 10001,
                port: 20050,
            },
            TenantPortBindingDto {
                site_id: 102,
                tenant_uid: 10002,
                port: 20060,
            },
        ]),
    };

    let req = Request::builder()
        .uri("/api/v1/system/security-hardening/nftables/sync")
        .method("POST")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&payload).expect("json")))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res.into_body().collect().await.expect("bytes").to_bytes();
    let sync_res: NftablesSyncResponseDto = serde_json::from_slice(&body_bytes).expect("sync_res");

    assert!(sync_res.success);
    assert_eq!(sync_res.rules_count, 4); // 2 bindings + 2 base rules
}

#[tokio::test]
async fn test_sync_nftables_invalid_port_rejected() {
    let ctx = setup_test_context().await;

    let payload = NftablesSyncPayload {
        custom_bindings: Some(vec![TenantPortBindingDto {
            site_id: 101,
            tenant_uid: 10001,
            port: 80, // Invalid: must be 20000..=29999
        }]),
    };

    let req = Request::builder()
        .uri("/api/v1/system/security-hardening/nftables/sync")
        .method("POST")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&payload).expect("json")))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn nftables_sync_requires_admin_csrf_before_any_firewall_command() {
    let ctx = setup_test_context().await;
    for token in [None, Some("wrong-token")] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/system/security-hardening/nftables/sync")
            .header(
                header::COOKIE,
                format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
            )
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(CSRF_HEADER_NAME, token);
        }
        let response = ctx
            .app
            .clone()
            .oneshot(request.body(Body::from("{}")).expect("request"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
