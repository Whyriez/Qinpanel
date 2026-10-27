//! Integration tests for PHP-FPM tenant master and pool management (P4-T3).

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
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreatePlan, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{
    plan_repo::SqlitePlanRepository, site_repo::SqliteSiteRepository,
    user_repo::SqliteUserRepository,
};
use panel_proto::PhpFpmPoolApplyRequest;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::sync::{Mutex, broadcast};
use tower::ServiceExt;

static TEST_MUTEX: Mutex<()> = Mutex::const_new(());

struct TestContext {
    app: axum::Router,
    agent_client: Arc<AgentClient>,
    #[allow(dead_code)]
    runner: Arc<MockSystemRunner>,
    admin_session: String,
    admin_csrf: String,
    tenant_a_session: String,
    tenant_a_csrf: String,
    _temp_dir: tempfile::TempDir,
    _shutdown_tx: broadcast::Sender<()>,
    _guard: tokio::sync::MutexGuard<'static, ()>,
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
    let guard = TEST_MUTEX.lock().await;
    let tmp = tempdir().expect("tempdir");
    let db_file = tmp.path().join("panel.db");

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
    runner.set_response("systemctl", 0, b"active\n", b"");
    runner.set_response("apt-get", 0, b"Reading package lists... Done\n", b"");
    runner.set_response("add-apt-repository", 0, b"Repository added\n", b"");
    runner.set_response("dpkg-query", 0, b"", b"");
    runner.set_response("phpenmod", 0, b"", b"");
    runner.set_response("phpdismod", 0, b"", b"");

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

    let agent_config = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };
    let agent_server = Arc::new(AgentServer::with_all_managers(
        agent_config,
        user_manager,
        slice_manager,
        quota_manager,
        resource_reader,
        hardening_manager,
        webserver_manager,
    ));

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let agent_server_clone = Arc::clone(&agent_server);
    tokio::spawn(async move {
        let _ = agent_server_clone.run(shutdown_rx).await;
    });

    let start = std::time::Instant::now();
    while !sock_path.exists() && start.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let db_url = format!(
        "sqlite://{}?mode=rwc",
        db_file.to_str().expect("db_file").replace('\\', "/")
    );
    let pool = init_pool(&db_url).await.expect("init_pool");
    run_migrations(&pool).await.expect("run migrations");

    let agent_client = Arc::new(AgentClient::new(sock_path));
    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        ..Default::default()
    };
    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let state = AppState::new(
        Arc::clone(&agent_client),
        pool.clone(),
        "test".to_string(),
        auth_config,
        lockout_manager,
    );
    let app = create_router(state);

    // Create Admin user
    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_hash = hash_password("AdminPass123!").expect("hash_password");
    user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: admin_hash,
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create admin");

    // Create default plan
    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let plan = plan_repo
        .create(&CreatePlan {
            name: "Basic Plan".to_string(),
            description: Some("Basic plan for test".to_string()),
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(512 * 1024 * 1024),
            memory_high_bytes: Some(400 * 1024 * 1024),
            tasks_max: Some(30),
            io_weight: Some(100),
            disk_limit_bytes: Some(1024 * 1024 * 1024),
            inode_limit: Some(100000),
            max_sites: Some(5),
            max_domains: Some(10),
            max_databases: Some(2),
            max_cron_jobs: Some(5),
            max_terminal_sessions: Some(1),
            max_upload_bytes: Some(50 * 1024 * 1024),
            shell_policy: Some("none".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create plan");

    // Create Tenant A (alice)
    let alice_hash = hash_password("AlicePass123!").expect("hash_password");
    let alice = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            linux_username: Some("p_alice".to_string()),
            linux_uid: Some(10001),
            plan_id: Some(plan.id),
        })
        .await
        .expect("create alice");

    // Create Site for Alice
    let site_repo = SqliteSiteRepository::new(pool.clone());
    site_repo
        .create(&CreateSite {
            user_id: alice.id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/alice.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some(r#"{"php":{"extensions":["curl","mbstring"]}}"#.to_string()),
        })
        .await
        .expect("create alice site");

    // Login Admin
    let (admin_session, admin_csrf) = login_and_get_session(&app, "admin", "AdminPass123!").await;

    // Login Tenant Alice
    let (tenant_a_session, tenant_a_csrf) =
        login_and_get_session(&app, "alice", "AlicePass123!").await;

    TestContext {
        app,
        agent_client,
        runner,
        admin_session,
        admin_csrf,
        tenant_a_session,
        tenant_a_csrf,
        _temp_dir: tmp,
        _shutdown_tx: shutdown_tx,
        _guard: guard,
    }
}

async fn login_and_get_session(
    app: &axum::Router,
    username: &str,
    password: &str,
) -> (String, String) {
    let login_payload = serde_json::json!({
        "username": username,
        "password": password
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&login_payload).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let cookies = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let session_val = extract_cookie_value(&cookies, SESSION_COOKIE_NAME).unwrap();

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session_val),
        )
        .body(Body::empty())
        .unwrap();
    let csrf_res = app.clone().oneshot(csrf_req).await.unwrap();
    assert_eq!(csrf_res.status(), StatusCode::OK);
    let csrf_body = csrf_res.into_body().collect().await.unwrap().to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&csrf_body).unwrap();
    let csrf_token = csrf_json["csrf_token"].as_str().unwrap().to_string();

    (session_val, csrf_token)
}

#[tokio::test]
async fn test_php_fpm_agent_client_direct_lifecycle() {
    let ctx = setup_test_context().await;

    // 1. Ensure PHP-FPM Master
    let ensure_res = ctx
        .agent_client
        .ensure_php_fpm_master("p_alice", 10001, "8.3")
        .await
        .unwrap();
    assert_eq!(ensure_res.service_name, "panel-fpm@p_alice-8.3.service");
    assert_eq!(
        ensure_res.default_socket_path,
        "/run/panel/php/p_alice-8.3/fpm.sock"
    );

    // 2. Apply PHP-FPM Pool
    let pool_res = ctx
        .agent_client
        .apply_php_fpm_pool(PhpFpmPoolApplyRequest {
            username: "p_alice".to_string(),
            uid: 10001,
            version: "8.3".to_string(),
            pool_name: "site-1".to_string(),
            docroot: "/home/p_alice/sites/alice.com/public".to_string(),
            socket_path: None,
            max_children: Some(20),
            process_idle_timeout_secs: Some(10),
            max_requests: Some(500),
            open_basedir: None,
            disable_functions: None,
            settings: None,
            admin_ini: None,
            reload: true,
        })
        .await
        .unwrap();
    assert_eq!(pool_res.pool_name, "site-1");
    assert!(pool_res.reloaded);

    // 3. Status Query
    let status = ctx
        .agent_client
        .get_php_fpm_master_status("p_alice", "8.3")
        .await
        .unwrap();
    assert_eq!(status.service_name, "panel-fpm@p_alice-8.3.service");
    assert_eq!(status.pools_count, 1);

    // 4. Reload and Restart
    let reload = ctx
        .agent_client
        .reload_php_fpm_master("p_alice", "8.3")
        .await
        .unwrap();
    assert!(reload.success);
    assert_eq!(reload.action, "reload");

    let restart = ctx
        .agent_client
        .restart_php_fpm_master("p_alice", "8.3")
        .await
        .unwrap();
    assert!(restart.success);
    assert_eq!(restart.action, "restart");

    // 5. Remove Pool
    let rem = ctx
        .agent_client
        .remove_php_fpm_pool("p_alice", "8.3", "site-1", true)
        .await
        .unwrap();
    assert!(rem.removed);
    assert_eq!(rem.remaining_pools, 0);
    assert!(rem.master_stopped);
}

#[tokio::test]
async fn test_php_fpm_api_endpoints() {
    let ctx = setup_test_context().await;

    // Apply master and pool first via agent
    ctx.agent_client
        .ensure_php_fpm_master("p_alice", 10001, "8.3")
        .await
        .unwrap();
    ctx.agent_client
        .apply_php_fpm_pool(PhpFpmPoolApplyRequest {
            username: "p_alice".to_string(),
            uid: 10001,
            version: "8.3".to_string(),
            pool_name: "site-1".to_string(),
            docroot: "/home/p_alice/sites/alice.com/public".to_string(),
            socket_path: None,
            max_children: Some(10),
            process_idle_timeout_secs: Some(10),
            max_requests: Some(500),
            open_basedir: None,
            disable_functions: None,
            settings: None,
            admin_ini: None,
            reload: true,
        })
        .await
        .unwrap();

    // 1. GET /api/v1/php/masters (Admin)
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/php/masters")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let masters: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(masters.is_array());
    assert!(
        masters
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["username"] == "p_alice" && m["version"] == "8.3")
    );

    // 2. GET /api/v1/php/masters/p_alice/8.3/status
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/php/masters/p_alice/8.3/status")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status["service_name"], "panel-fpm@p_alice-8.3.service");
    assert_eq!(status["pools_count"], 1);

    // 3. POST /api/v1/php/masters/p_alice/8.3/reload
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/php/masters/p_alice/8.3/reload")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let action_res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(action_res["success"], true);
    assert_eq!(action_res["action"], "reload");

    // 4. POST /api/v1/php/masters/p_alice/8.3/restart
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/php/masters/p_alice/8.3/restart")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let action_res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(action_res["success"], true);
    assert_eq!(action_res["action"], "restart");
}

#[tokio::test]
async fn test_php_fpm_tenant_rbac_isolation() {
    let ctx = setup_test_context().await;

    // Alice is p_alice. Let's verify Alice CAN access her own master:
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/php/masters/p_alice/8.3/status")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.tenant_a_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Alice can reload her own master:
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/php/masters/p_alice/8.3/reload")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.tenant_a_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.tenant_a_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Alice CANNOT access another tenant's master (e.g. p_bob):
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/php/masters/p_bob/8.3/status")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.tenant_a_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Alice CANNOT reload another tenant's master (e.g. p_bob):
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/php/masters/p_bob/8.3/reload")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.tenant_a_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.tenant_a_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
