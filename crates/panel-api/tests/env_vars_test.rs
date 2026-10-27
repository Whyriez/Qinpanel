#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::bool_assert_comparison
)]

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
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_api::sites::SiteResponse;
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    #[allow(dead_code)]
    admin_session: String,
    #[allow(dead_code)]
    admin_csrf: String,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    bob_csrf: String,
    #[allow(dead_code)]
    alice_id: i64,
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
    let db_file = tmp.path().join("panel_env_test.db");

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

    let nginx_dir = tmp.path().join("etc/nginx");
    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response("systemd-run", 0, b"", b"");
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
    let webserver_manager = Arc::new(NginxManager::with_base_dir(
        Arc::clone(&runner) as Arc<dyn SystemRunner>,
        &nginx_dir,
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
        server.run(rx).await.expect("agent server run");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_hash = hash_password("AdminPass123!").expect("hash pass");
    user_repo
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
    user_repo
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
        "development".to_string(),
        auth_config,
        lockout_manager,
    );
    let app = create_router(state);

    // Login admin
    let login_admin = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"admin","password":"AdminPass123!"}"#,
        ))
        .unwrap();
    let res = app.clone().oneshot(login_admin).await.unwrap();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let admin_session = extract_cookie_value(cookie, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&body_bytes).unwrap();
    let admin_csrf = val["csrf_token"].as_str().unwrap().to_string();

    // Login alice
    let login_alice = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"alice","password":"AlicePass123!"}"#,
        ))
        .unwrap();
    let res = app.clone().oneshot(login_alice).await.unwrap();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let alice_session = extract_cookie_value(cookie, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&body_bytes).unwrap();
    let alice_csrf = val["csrf_token"].as_str().unwrap().to_string();

    // Login bob
    let login_bob = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"username":"bob","password":"BobPass123!"}"#))
        .unwrap();
    let res = app.clone().oneshot(login_bob).await.unwrap();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let bob_session = extract_cookie_value(cookie, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&body_bytes).unwrap();
    let bob_csrf = val["csrf_token"].as_str().unwrap().to_string();

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        alice_id: alice.id,
    }
}

async fn create_test_site(ctx: &TestContext, domain: &str) -> SiteResponse {
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({ "domain": domain }).to_string(),
        ))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_encrypted_env_vars_lifecycle() {
    let ctx = setup_test_context().await;
    let site = create_test_site(&ctx, "env-app.test").await;

    // 1. Initial GET /api/v1/sites/{id}/env should be empty and restart_required = false
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/env", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["variables"].as_array().unwrap().len(), 0);
    assert_eq!(json["restart_required"].as_bool().unwrap(), false);

    // 2. PUT /api/v1/sites/{id}/env with sensitive environment variables
    let secret_db_url = "postgresql://user:super_secret_db_pass_123@10.0.0.5:5432/app_db";
    let secret_api_key = "sk_live_very_secret_token_987654321";
    let mut vars = BTreeMap::new();
    vars.insert("DATABASE_URL".to_string(), secret_db_url.to_string());
    vars.insert("API_KEY".to_string(), secret_api_key.to_string());
    vars.insert("APP_ENV".to_string(), "production".to_string());

    let put_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/env", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({ "variables": vars }).to_string(),
        ))
        .unwrap();

    let res = ctx.app.clone().oneshot(put_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["success"].as_bool().unwrap(), true);
    assert_eq!(json["restart_required"].as_bool().unwrap(), true);

    // 3. CRITICAL ACCEPTANCE CHECK: Verify SQLite database does NOT contain plaintext secrets!
    let row: (String,) = sqlx::query_as("SELECT settings_json FROM sites WHERE id = ?")
        .bind(site.id)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    let settings_str = row.0;

    assert!(
        !settings_str.contains("super_secret_db_pass_123"),
        "SECURITY VIOLATION: Database contained plaintext password!"
    );
    assert!(
        !settings_str.contains("sk_live_very_secret_token_987654321"),
        "SECURITY VIOLATION: Database contained plaintext API key!"
    );
    assert!(
        settings_str.contains("enc:v1:"),
        "Database must contain encrypted ciphertext prefix enc:v1:"
    );

    // 4. GET /api/v1/sites/{id}/env (default masked)
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/env", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["restart_required"].as_bool().unwrap(), true);
    let list = json["variables"].as_array().unwrap();
    assert_eq!(list.len(), 3);
    for item in list {
        assert_eq!(item["is_masked"].as_bool().unwrap(), true);
        assert_eq!(item["value"].as_str().unwrap(), "••••••••");
    }

    // 5. GET /api/v1/sites/{id}/env?reveal=true (decrypted via agent)
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/env?reveal=true", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let list = json["variables"].as_array().unwrap();
    let mut decrypted_map = BTreeMap::new();
    for item in list {
        assert_eq!(item["is_masked"].as_bool().unwrap(), false);
        decrypted_map.insert(
            item["key"].as_str().unwrap().to_string(),
            item["value"].as_str().unwrap().to_string(),
        );
    }
    assert_eq!(decrypted_map.get("DATABASE_URL").unwrap(), secret_db_url);
    assert_eq!(decrypted_map.get("API_KEY").unwrap(), secret_api_key);
    assert_eq!(decrypted_map.get("APP_ENV").unwrap(), "production");

    // 6. POST /api/v1/sites/{id}/env/import with .env content
    let dotenv_raw = r#"
# New Imported Variables
export JWT_SECRET="super-jwt-secret-phrase\nwith-escapes"
REDIS_PORT=6379
SINGLE_RAW='literal $value without interpolation'
"#;
    let imp_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/env/import", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "content": dotenv_raw,
                "merge": true
            })
            .to_string(),
        ))
        .unwrap();
    let res = ctx.app.clone().oneshot(imp_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["success"].as_bool().unwrap(), true);
    assert_eq!(json["restart_required"].as_bool().unwrap(), true);

    // Verify imported variables with reveal
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/env?reveal=true", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let list = json["variables"].as_array().unwrap();
    let mut merged_map = BTreeMap::new();
    for item in list {
        merged_map.insert(
            item["key"].as_str().unwrap().to_string(),
            item["value"].as_str().unwrap().to_string(),
        );
    }
    assert_eq!(
        merged_map.get("JWT_SECRET").unwrap(),
        "super-jwt-secret-phrase\nwith-escapes"
    );
    assert_eq!(merged_map.get("REDIS_PORT").unwrap(), "6379");
    assert_eq!(
        merged_map.get("SINGLE_RAW").unwrap(),
        "literal $value without interpolation"
    );
    // Merged old values still present
    assert_eq!(merged_map.get("DATABASE_URL").unwrap(), secret_db_url);

    // 7. POST /api/v1/sites/{id}/env/restart resets restart_required to false
    let rst_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/env/restart", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(rst_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["success"].as_bool().unwrap(), true);
    assert_eq!(json["restart_required"].as_bool().unwrap(), false);

    // GET /api/v1/sites/{id}/env now shows restart_required = false
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/env", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["restart_required"].as_bool().unwrap(), false);

    // 8. RBAC: Bob attempts to read or write Alice's env vars -> 403 Forbidden
    let bob_get = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/env", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(bob_get).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let bob_put = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/env", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::from(r#"{"variables":{"HACK":"true"}}"#))
        .unwrap();
    let res = ctx.app.clone().oneshot(bob_put).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 9. AUDIT LOG CHECK: verify audit logs exist and contain NO plaintext secrets
    let audit_rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT action, details_json FROM audit_logs WHERE target_resource = ?")
            .bind(format!("site:{}", site.id))
            .fetch_all(&ctx.pool)
            .await
            .unwrap();

    assert!(!audit_rows.is_empty(), "Audit logs must be written");
    for (action, details) in audit_rows {
        assert!(
            action.starts_with("site.env_") || action.starts_with("site.create"),
            "Expected site action, got {}",
            action
        );
        if let Some(detail_str) = details {
            assert!(
                !detail_str.contains("super_secret_db_pass_123"),
                "SECURITY VIOLATION: Audit logs contained plaintext password!"
            );
            assert!(
                !detail_str.contains("sk_live_very_secret_token_987654321"),
                "SECURITY VIOLATION: Audit logs contained plaintext API key!"
            );
        }
    }
}
