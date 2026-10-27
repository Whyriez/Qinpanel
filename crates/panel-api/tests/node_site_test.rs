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
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::node_site::allocate_node_site_port;
use panel_api::routes::AppState;
use panel_api::sites::{CreateStaticSiteRequest, SiteResponse};
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
    let db_file = tmp.path().join("panel_test.db");

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
    // Configure mock responses for system binaries
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response(
        "systemctl",
        0,
        b"ActiveState=active\nSubState=running\nMainPID=42\n",
        b"",
    );
    runner.set_response("curl", 0, b"200", b"");
    runner.set_response("runuser", 0, b"200", b"");
    runner.add_existing_path("/opt/panel/runtimes/node/22.14.0/bin/node");
    runner.add_existing_path("/opt/panel/runtimes/python/3.12.9/bin/python3");
    runner.set_response("systemd-run", 0, b"installed 42 packages", b"");
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
    let alice_user = user_repo
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
    assert_eq!(res.status(), StatusCode::OK);
    let cookies = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let admin_session = extract_cookie_value(cookies, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let admin_csrf = body_json["csrf_token"].as_str().unwrap().to_string();

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
    assert_eq!(res.status(), StatusCode::OK);
    let cookies = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let alice_session = extract_cookie_value(cookies, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let alice_csrf = body_json["csrf_token"].as_str().unwrap().to_string();

    // Login bob
    let login_bob = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"username":"bob","password":"BobPass123!"}"#))
        .unwrap();
    let res = app.clone().oneshot(login_bob).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookies = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let bob_session = extract_cookie_value(cookies, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let bob_csrf = body_json["csrf_token"].as_str().unwrap().to_string();

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
        alice_id: alice_user.id,
    }
}

#[tokio::test]
async fn test_node_site_port_allocation() {
    let ctx = setup_test_context().await;

    // Allocate first port
    let port1 = allocate_node_site_port(&ctx.pool)
        .await
        .expect("allocate port 1");
    assert!((20000..=29999).contains(&port1));

    // Insert dummy site using port1
    sqlx::query(
        "INSERT INTO sites (user_id, kind, docroot, status, settings_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(ctx.alice_id)
    .bind("nodejs")
    .bind("/var/www/site1")
    .bind("active")
    .bind(format!(r#"{{"port":{}}}"#, port1))
    .bind(1000)
    .bind(1000)
    .execute(&ctx.pool)
    .await
    .expect("insert site");

    // Allocate second port: must be different
    let port2 = allocate_node_site_port(&ctx.pool)
        .await
        .expect("allocate port 2");
    assert_ne!(port1, port2);
    assert!((20000..=29999).contains(&port2));
}

#[tokio::test]
async fn test_node_site_full_lifecycle_and_security() {
    let ctx = setup_test_context().await;

    // 1. Create a Node.js site under Alice
    let app_dir = "/home/p_alice/sites/node-app.example.com/app";
    let create_payload = CreateStaticSiteRequest {
        domain: "node-app.example.com".to_string(),
        docroot: Some(app_dir.to_string()),
        spa_fallback: Some(false),
        force_https: Some(false),
        user_id: Some(ctx.alice_id),
        kind: Some("nodejs".to_string()),
        node_version: Some("22.14.0".to_string()),
        port: Some(20050),
        app_dir: Some(app_dir.to_string()),
        entrypoint_type: Some("script".to_string()),
        entrypoint_value: Some("server.js".to_string()),
        auto_restart: Some(true),
        env_vars: Some(BTreeMap::from([
            ("NODE_ENV".to_string(), "production".to_string()),
            ("PORT".to_string(), "20050".to_string()),
        ])),
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&create_payload).unwrap()))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "Node site creation failed: {}",
        String::from_utf8_lossy(&body_bytes)
    );
    let site_res: SiteResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(site_res.kind, "nodejs");
    assert_eq!(site_res.docroot, app_dir);
    assert_eq!(site_res.runtime.as_deref(), Some("nodejs"));
    assert_eq!(site_res.version.as_deref(), Some("22.14.0"));

    let site_id = site_res.id;

    // 2. Query status via GET /api/v1/sites/{id}/node/status
    let status_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/node/status", site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();

    let status_res = ctx.app.clone().oneshot(status_req).await.unwrap();
    assert_eq!(status_res.status(), StatusCode::OK);
    let status_bytes = status_res.into_body().collect().await.unwrap().to_bytes();
    let status_json: Value = serde_json::from_slice(&status_bytes).unwrap();
    assert_eq!(status_json["port"], 20050);

    // 3. Process actions: Start, Restart, Redeploy, Stop
    for action in &["start", "restart", "redeploy", "stop"] {
        let action_payload = serde_json::json!({
            "action": action
        });
        let action_req = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/sites/{}/node/action", site_id))
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::COOKIE,
                format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
            )
            .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
            .body(Body::from(serde_json::to_string(&action_payload).unwrap()))
            .unwrap();

        let action_res = ctx.app.clone().oneshot(action_req).await.unwrap();
        assert_eq!(action_res.status(), StatusCode::OK);
    }

    // 4. Run tenant job: npm install
    let job_payload = serde_json::json!({
        "job_type": "npm_install",
        "timeout_secs": 60
    });
    let job_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/node/job", site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&job_payload).unwrap()))
        .unwrap();

    let job_res = ctx.app.clone().oneshot(job_req).await.unwrap();
    assert_eq!(job_res.status(), StatusCode::OK);
    let job_bytes = job_res.into_body().collect().await.unwrap().to_bytes();
    let job_json: Value = serde_json::from_slice(&job_bytes).unwrap();
    assert_eq!(job_json["success"], true);

    // 5. Get and Update settings
    let get_settings_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/node/settings", site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();

    let get_settings_res = ctx.app.clone().oneshot(get_settings_req).await.unwrap();
    assert_eq!(get_settings_res.status(), StatusCode::OK);
    let s_bytes = get_settings_res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let s_json: Value = serde_json::from_slice(&s_bytes).unwrap();
    assert_eq!(s_json["port"], 20050);
    assert_eq!(s_json["entrypoint_type"], "script");
    assert_eq!(s_json["app_dir"], app_dir);

    // Update settings
    let update_settings_payload = serde_json::json!({
        "port": 20088,
        "entrypoint_type": "npm_start",
        "env_vars": {
            "NODE_ENV": "production",
            "CUSTOM_VAR": "hello_vpanel"
        }
    });
    let update_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/node/settings", site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&update_settings_payload).unwrap(),
        ))
        .unwrap();

    let update_res = ctx.app.clone().oneshot(update_req).await.unwrap();
    assert_eq!(update_res.status(), StatusCode::OK);
    let up_bytes = update_res.into_body().collect().await.unwrap().to_bytes();
    let up_json: Value = serde_json::from_slice(&up_bytes).unwrap();
    assert_eq!(up_json["port"], 20088);
    assert_eq!(up_json["entrypoint_type"], "npm_start");

    // 6. Security Isolation Check: Bob attempts to access Alice's node site
    let bob_status_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/node/status", site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::empty())
        .unwrap();

    let bob_res = ctx.app.clone().oneshot(bob_status_req).await.unwrap();
    assert_eq!(bob_res.status(), StatusCode::FORBIDDEN);

    let bob_action_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/node/action", site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::from(r#"{"action":"restart"}"#))
        .unwrap();

    let bob_action_res = ctx.app.clone().oneshot(bob_action_req).await.unwrap();
    assert_eq!(bob_action_res.status(), StatusCode::FORBIDDEN);
}
