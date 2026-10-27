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
use panel_api::agent_client::AgentClient;
use panel_api::auth::{LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_core::memory_profile::PanelMemoryProfileReport;
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
    tenant_session: String,
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

    #[cfg(unix)]
    let sock_path = tmp.path().join("agent.sock");
    #[cfg(not(unix))]
    let sock_path = tmp.path().join("agent.port");

    let runner = Arc::new(MockSystemRunner::new());
    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let user_mgr = Arc::new(UserLifecycleManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let slice_mgr = Arc::new(SliceManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let quota_mgr = Arc::new(QuotaManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let cgroup = Arc::new(CgroupReader::default());
    let hard_mgr = Arc::new(HardeningManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let web_mgr = Arc::new(NginxManager::new(runner.clone() as Arc<dyn SystemRunner>));

    let agent_server = Arc::new(AgentServer::with_all_managers(
        agent_cfg, user_mgr, slice_mgr, quota_mgr, cgroup, hard_mgr, web_mgr,
    ));

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let srv_clone = agent_server.clone();
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

    let lockout_mgr = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(sock_path));
    let app_state = AppState::new(
        agent_client,
        pool,
        "test".to_string(),
        auth_cfg,
        lockout_mgr,
    );

    let app = create_router(app_state);

    async fn login_helper(app_router: &axum::Router, user: &str, pass: &str) -> String {
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
            .expect("login response");

        assert_eq!(login_resp.status(), StatusCode::OK);
        let set_cookie = login_resp
            .headers()
            .get(header::SET_COOKIE)
            .expect("set cookie")
            .to_str()
            .expect("cookie str");

        extract_cookie_value(set_cookie, SESSION_COOKIE_NAME).expect("session cookie")
    }

    let admin_session = login_helper(&app, "admin", "AdminSecret123!").await;
    let tenant_session = login_helper(&app, "tenant1", "TenantSecret123!").await;

    TestContext {
        app,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        tenant_session,
    }
}

#[tokio::test]
async fn test_get_memory_profile_admin_success() {
    let ctx = setup_test_context().await;

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/memory-profile")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build request");

    let resp = ctx.app.clone().oneshot(req).await.expect("get response");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let report: PanelMemoryProfileReport =
        serde_json::from_slice(&body).expect("parse json report");

    assert_eq!(report.allocator, "mimalloc");
    assert_eq!(report.release_profile.opt_level, "3");
    assert_eq!(report.release_profile.lto, "fat");
    assert_eq!(report.release_profile.codegen_units, 1);
    assert_eq!(report.release_profile.panic_strategy, "abort");
    assert!(report.release_profile.strip);
    assert!(
        report.all_passed,
        "Semua target memory profile seharusnya lolos, target yang gagal: {:#?}",
        report
            .targets
            .iter()
            .filter(|t| !t.passed)
            .collect::<Vec<_>>()
    );
    assert_eq!(report.targets.len(), 6);

    // Verify all targets match PRD Section 9
    let target_ids: Vec<String> = report.targets.into_iter().map(|t| t.target_id).collect();
    assert!(target_ids.contains(&"BAB9-RAM-API".to_string()));
    assert!(target_ids.contains(&"BAB9-RAM-AGENT".to_string()));
    assert!(target_ids.contains(&"BAB9-RAM-TOTAL".to_string()));
    assert!(target_ids.contains(&"BAB9-COLD-START".to_string()));
    assert!(target_ids.contains(&"BAB9-API-LATENCY".to_string()));
    assert!(target_ids.contains(&"BAB9-WEB-BUNDLE".to_string()));
}

#[tokio::test]
async fn test_get_memory_profile_tenant_forbidden() {
    let ctx = setup_test_context().await;

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/memory-profile")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.tenant_session),
        )
        .body(Body::empty())
        .expect("build request");

    let resp = ctx.app.clone().oneshot(req).await.expect("get response");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_get_memory_profile_unauthenticated() {
    let ctx = setup_test_context().await;

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/memory-profile")
        .body(Body::empty())
        .expect("build request");

    let resp = ctx.app.clone().oneshot(req).await.expect("get response");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
