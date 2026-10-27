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
use panel_api::ssh::{SshKeyResponse, ValidateSshKeyResponse};
use panel_core::auth::hash_password;
use panel_db::models::{CreatePlan, CreateUser};
use panel_db::pool::{DbPool, init_pool, run_migrations};
use panel_db::repositories::{SqlitePlanRepository, SqliteUserRepository};
use tower::ServiceExt;

const VALID_ED25519_KEY_1: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl alice@workstation";
const VALID_ED25519_KEY_2: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICc1y10Zt92G7+e58x1sD7hP5qJ3q6X3G+Y4H7Q6zK2b bob@macbook";

struct TestContext {
    app: axum::Router,
    pool: DbPool,
    ssh_keys_dir: std::path::PathBuf,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    bob_csrf: String,
    admin_session: String,
    #[allow(dead_code)]
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
    std::fs::create_dir_all(&ssh_keys_dir).expect("create ssh keys dir");

    // 1. Setup agent server with custom ssh_keys_dir
    let runner = Arc::new(MockSystemRunner::new());
    let ssh_manager = Arc::new(SshManager::with_base_dir(runner, ssh_keys_dir.clone()));

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

    user_repo
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

    // 5. Login admin, alice, bob
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
        let val: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        let csrf = val["csrf_token"].as_str().expect("csrf").to_string();

        (session, csrf)
    }

    let (admin_session, admin_csrf) = login(&app, "admin").await;
    let (alice_session, alice_csrf) = login(&app, "alice").await;
    let (bob_session, bob_csrf) = login(&app, "bob").await;

    TestContext {
        app,
        pool,
        ssh_keys_dir,
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
async fn test_ssh_key_crud_lifecycle_and_agent_sync() {
    let ctx = setup_test_context().await;

    // 1. Alice adds an SSH key
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh-keys")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "name": "Alice Work Laptop",
                "public_key": VALID_ED25519_KEY_1
            })
            .to_string(),
        ))
        .expect("create req");

    let res = ctx
        .app
        .clone()
        .oneshot(create_req)
        .await
        .expect("create res");
    assert_eq!(res.status(), StatusCode::CREATED);

    let body = res.into_body().collect().await.expect("body").to_bytes();
    let created: SshKeyResponse = serde_json::from_slice(&body).expect("json");
    assert_eq!(created.name, "Alice Work Laptop");
    assert_eq!(created.key_type, "ssh-ed25519");
    assert!(created.fingerprint_sha256.starts_with("SHA256:"));
    assert!(created.fingerprint_md5.is_some());
    assert_eq!(created.public_key, VALID_ED25519_KEY_1);

    // 2. Verify agent root-owned file was created and contains the public key
    let alice_file = ctx.ssh_keys_dir.join("p_alice");
    assert!(alice_file.exists());
    let file_content = std::fs::read_to_string(&alice_file).expect("read auth keys");
    assert!(file_content.contains(VALID_ED25519_KEY_1));

    // 3. Alice lists her keys
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/ssh-keys")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("list req");

    let res = ctx.app.clone().oneshot(list_req).await.expect("list res");
    assert_eq!(res.status(), StatusCode::OK);
    let list_body = res.into_body().collect().await.expect("body").to_bytes();
    let keys: Vec<SshKeyResponse> = serde_json::from_slice(&list_body).expect("json list");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].id, created.id);

    // 4. Alice attempts to register the exact same key again -> 409 Conflict
    let dup_req = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh-keys")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "name": "Alice Duplicate Key",
                "public_key": VALID_ED25519_KEY_1
            })
            .to_string(),
        ))
        .expect("dup req");

    let res = ctx.app.clone().oneshot(dup_req).await.expect("dup res");
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // 5. Alice deletes her key
    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/ssh-keys/{}", created.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("del req");

    let res = ctx.app.clone().oneshot(del_req).await.expect("del res");
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Verify agent file was cleared
    let file_content_after = std::fs::read_to_string(&alice_file).expect("read auth keys");
    assert!(file_content_after.trim().is_empty());

    // 7. Verify audit logs recorded create and delete actions
    tokio::time::sleep(Duration::from_millis(100)).await;
    let audit_repo = panel_db::repositories::SqliteAuditLogRepository::new(ctx.pool.clone());
    let logs = audit_repo
        .list_filtered(&panel_db::repositories::AuditLogFilter::default(), 50, 0)
        .await
        .expect("audit logs");

    assert!(logs.iter().any(|l| l.action == "ssh_key.create"));
    assert!(logs.iter().any(|l| l.action == "ssh_key.delete"));
}

#[tokio::test]
async fn test_ssh_key_validation_and_rejections() {
    let ctx = setup_test_context().await;

    // 1. Reject multiline injection in POST /api/v1/ssh-keys
    let malicious = format!("{}\nssh-ed25519 AAAAC3NzaC... bad", VALID_ED25519_KEY_1);
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh-keys")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "name": "Malicious Key",
                "public_key": malicious
            })
            .to_string(),
        ))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 2. Reject empty or invalid format
    let req_empty = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh-keys")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "name": "Empty Key",
                "public_key": "not-a-valid-key"
            })
            .to_string(),
        ))
        .expect("req");

    let res = ctx.app.clone().oneshot(req_empty).await.expect("res");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 3. Test live validation endpoint POST /api/v1/ssh-keys/validate
    let valid_val_req = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh-keys/validate")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({ "public_key": VALID_ED25519_KEY_1 }).to_string(),
        ))
        .expect("val req");

    let res = ctx
        .app
        .clone()
        .oneshot(valid_val_req)
        .await
        .expect("val res");
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.expect("body").to_bytes();
    let val_res: ValidateSshKeyResponse = serde_json::from_slice(&body).expect("json");
    assert!(val_res.valid);
    assert_eq!(val_res.key_type.as_deref(), Some("ssh-ed25519"));
    assert!(val_res.fingerprint_sha256.is_some());
    assert_eq!(val_res.comment.as_deref(), Some("alice@workstation"));
    assert_eq!(val_res.bit_length, Some(256));

    // 4. Test live validation with invalid key
    let invalid_val_req = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh-keys/validate")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({ "public_key": "ssh-dss AAAAB3NzaC1kc3MAAACB... bad" }).to_string(),
        ))
        .expect("invalid val req");

    let res = ctx.app.clone().oneshot(invalid_val_req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.expect("body").to_bytes();
    let val_res: ValidateSshKeyResponse = serde_json::from_slice(&body).expect("json");
    assert!(!val_res.valid);
    assert!(val_res.error.is_some());
}

#[tokio::test]
async fn test_ssh_key_multi_tenant_rbac_isolation() {
    let ctx = setup_test_context().await;

    // 1. Alice adds Key A
    let alice_add = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh-keys")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "name": "Alice Key",
                "public_key": VALID_ED25519_KEY_1
            })
            .to_string(),
        ))
        .expect("req");
    let res = ctx.app.clone().oneshot(alice_add).await.expect("res");
    assert_eq!(res.status(), StatusCode::CREATED);

    // 2. Bob adds Key B
    let bob_add = Request::builder()
        .method("POST")
        .uri("/api/v1/ssh-keys")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::from(
            serde_json::json!({
                "name": "Bob Key",
                "public_key": VALID_ED25519_KEY_2
            })
            .to_string(),
        ))
        .expect("req");
    let res = ctx.app.clone().oneshot(bob_add).await.expect("res");
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = res.into_body().collect().await.expect("body").to_bytes();
    let bob_created: SshKeyResponse = serde_json::from_slice(&body).expect("json");

    // 3. Alice lists keys -> must ONLY see her own key
    let alice_list = Request::builder()
        .method("GET")
        .uri("/api/v1/ssh-keys")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");
    let res = ctx.app.clone().oneshot(alice_list).await.expect("res");
    let body = res.into_body().collect().await.expect("body").to_bytes();
    let alice_keys: Vec<SshKeyResponse> = serde_json::from_slice(&body).expect("json");
    assert_eq!(alice_keys.len(), 1);
    assert_eq!(alice_keys[0].name, "Alice Key");

    // 4. Alice attempts to delete Bob's key -> 403 Forbidden
    let alice_delete_bob = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/ssh-keys/{}", bob_created.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let res = ctx
        .app
        .clone()
        .oneshot(alice_delete_bob)
        .await
        .expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Verify Bob's agent file is intact
    let bob_file = ctx.ssh_keys_dir.join("p_bob");
    assert!(bob_file.exists());
    let bob_content = std::fs::read_to_string(&bob_file).expect("read");
    assert!(bob_content.contains(VALID_ED25519_KEY_2));

    // 5. Admin lists Bob's keys via ?tenant=p_bob
    let admin_list_bob = Request::builder()
        .method("GET")
        .uri("/api/v1/ssh-keys?tenant=p_bob")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("req");
    let res = ctx.app.clone().oneshot(admin_list_bob).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.expect("body").to_bytes();
    let admin_view_bob_keys: Vec<SshKeyResponse> = serde_json::from_slice(&body).expect("json");
    assert_eq!(admin_view_bob_keys.len(), 1);
    assert_eq!(admin_view_bob_keys[0].id, bob_created.id);
}
