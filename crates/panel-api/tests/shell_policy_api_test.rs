#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::ssh::SshManager;
use panel_agent::system::MockSystemRunner;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreatePlan, CreateUser};
use panel_db::pool::{DbPool, init_pool, run_migrations};
use panel_db::repositories::{SqlitePlanRepository, SqliteUserRepository};
use serde_json::Value;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    _pool: DbPool,
    alice_id: i64,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    bob_csrf: String,
    admin_session: String,
    admin_csrf: String,
    _temp: tempfile::TempDir,
    _agent_shutdown_tx: tokio::sync::broadcast::Sender<()>,
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
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");
    let db_path = tmp.path().join("panel_test.db");
    let ssh_keys_dir = tmp.path().join("etc_panel_ssh_authorized_keys");
    let sshd_dropin_path = tmp.path().join("50-panel.conf");
    std::fs::create_dir_all(&ssh_keys_dir).expect("create ssh keys dir");

    // 1. Setup agent server with custom ssh_keys_dir & dropin path
    let runner = Arc::new(MockSystemRunner::new());
    let ssh_manager = Arc::new(SshManager::with_paths(
        runner,
        ssh_keys_dir.clone(),
        sshd_dropin_path,
    ));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };
    let agent_server = Arc::new(AgentServer::new(agent_cfg).with_ssh_manager(ssh_manager));
    let (agent_shutdown_tx, agent_shutdown_rx) = tokio::sync::broadcast::channel(1);

    tokio::spawn(async move {
        let _ = agent_server.run(agent_shutdown_rx).await;
    });

    let start = std::time::Instant::now();
    while !sock_path.exists() && start.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 2. Setup database
    let pool = init_pool(&format!(
        "sqlite://{}",
        db_path.to_str().expect("db_path").replace('\\', "/")
    ))
    .await
    .expect("init db pool");
    run_migrations(&pool).await.expect("migrations");

    // 3. Create plan & users
    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let default_plan = plan_repo
        .create(&CreatePlan {
            name: "Standard".to_string(),
            description: None,
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(50),
            io_weight: Some(100),
            disk_limit_bytes: Some(5 * 1024 * 1024 * 1024),
            inode_limit: Some(500_000),
            max_sites: Some(5),
            max_domains: Some(5),
            max_databases: Some(2),
            max_cron_jobs: Some(5),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(50 * 1024 * 1024),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create plan");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let pwd_hash = hash_password("Password123!").expect("hash pwd");

    user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: pwd_hash.clone(),
            role: "admin".to_string(),
            email: Some("admin@nusa.local".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create admin");

    let alice = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: pwd_hash.clone(),
            role: "user".to_string(),
            email: Some("alice@nusa.local".to_string()),
            linux_username: Some("p_alice".to_string()),
            linux_uid: Some(10001),
            plan_id: Some(default_plan.id),
        })
        .await
        .expect("create alice");

    user_repo
        .create(&CreateUser {
            username: "bob".to_string(),
            password_hash: pwd_hash.clone(),
            role: "user".to_string(),
            email: Some("bob@nusa.local".to_string()),
            linux_username: Some("p_bob".to_string()),
            linux_uid: Some(10002),
            plan_id: Some(default_plan.id),
        })
        .await
        .expect("create bob");

    let agent_client = Arc::new(AgentClient::new(sock_path));
    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };
    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let state = AppState::new(
        agent_client,
        pool.clone(),
        "testing".to_string(),
        auth_config,
        lockout_manager,
    );
    let app = create_router(state);

    async fn login(app: &axum::Router, user: &str) -> (String, String) {
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "username": user,
                    "password": "Password123!"
                })
                .to_string(),
            ))
            .expect("login req");

        let res = app.clone().oneshot(req).await.expect("login res");
        assert_eq!(res.status(), StatusCode::OK);

        let cookie_hdr = res
            .headers()
            .get(header::SET_COOKIE)
            .expect("set cookie")
            .to_str()
            .expect("to_str");
        let session = extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).expect("session");

        let body_bytes = res.into_body().collect().await.expect("collect").to_bytes();
        let val: Value = serde_json::from_slice(&body_bytes).expect("json");
        let csrf = val["csrf_token"].as_str().expect("csrf").to_string();

        (session, csrf)
    }

    let (admin_session, admin_csrf) = login(&app, "admin").await;
    let (alice_session, alice_csrf) = login(&app, "alice").await;
    let (bob_session, bob_csrf) = login(&app, "bob").await;

    TestContext {
        app,
        _pool: pool,
        alice_id: alice.id,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        admin_session,
        admin_csrf,
        _temp: tmp,
        _agent_shutdown_tx: agent_shutdown_tx,
    }
}

#[tokio::test]
async fn test_tenant_shell_policy_inheritance_and_override() {
    let ctx = setup_test_context().await;

    // 1. Alice initially inherits "full" from plan
    let req = Request::builder()
        .uri(format!("/api/v1/tenants/{}/shell-policy", ctx.alice_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(json["effective_policy"], "full");
    assert_eq!(json["inherited_from_plan"], true);
    assert!(json["shell_policy"].is_null());

    // 2. Admin overrides Alice policy to "sftp"
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/tenants/{}/shell-policy", ctx.alice_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "shell_policy": "sftp"
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(json["effective_policy"], "sftp");
    assert_eq!(json["inherited_from_plan"], false);
    assert_eq!(json["shell_policy"], "sftp");

    // 3. Admin resets override back to None (inherits from plan again)
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/tenants/{}/shell-policy", ctx.alice_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "shell_policy": null
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(json["effective_policy"], "full");
    assert_eq!(json["inherited_from_plan"], true);
    assert!(json["shell_policy"].is_null());

    // 4. Reject invalid shell policy
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/tenants/{}/shell-policy", ctx.alice_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "shell_policy": "invalid_policy"
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 5. Tenant Bob cannot modify Alice's shell policy (403 Forbidden)
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/tenants/{}/shell-policy", ctx.alice_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "shell_policy": "full"
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_terminal_blocked_for_sftp_and_none_policy() {
    let ctx = setup_test_context().await;

    // 1. Admin sets Alice policy to "sftp"
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/tenants/{}/shell-policy", ctx.alice_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "shell_policy": "sftp"
            }))
            .expect("json"),
        ))
        .expect("req");
    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. Alice tries to open terminal session -> 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/terminal/sessions")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "cols": 80,
                "rows": 24
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let body = resp.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        json["error"],
        "Kebijakan shell tenant tidak mengizinkan terminal interaktif."
    );

    // 3. Admin sets Alice policy to "none"
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/tenants/{}/shell-policy", ctx.alice_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "shell_policy": "none"
            }))
            .expect("json"),
        ))
        .expect("req");
    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Alice tries to open terminal session again -> 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/terminal/sessions")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "cols": 80,
                "rows": 24
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let body = resp.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        json["error"],
        "Kebijakan shell tenant tidak mengizinkan terminal interaktif."
    );
}

#[tokio::test]
async fn test_ssh_dropin_apply_permissions() {
    let ctx = setup_test_context().await;

    // 1. Regular tenant Bob cannot apply dropin (403 Forbidden)
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh/dropin/apply")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "custom_content": null
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 2. Admin can trigger dropin apply
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh/dropin/apply")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "custom_content": null
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["success"], true);
    assert_eq!(json["validated"], true);
}
