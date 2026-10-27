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
use panel_api::logs::SiteLogsResponse;
use panel_api::php::{SitePhpLogsResponse, SitePhpStatusResponse};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateDomain, CreatePlan, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{
    SqliteDomainRepository, SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository,
};
use panel_runtime::{fpm::PhpFpmMasterConfig, php::PhpVersion};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestContext {
    app: axum::Router,
    #[allow(dead_code)]
    pool: panel_db::pool::DbPool,
    #[allow(dead_code)]
    runner: Arc<MockSystemRunner>,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    _test_guard: tokio::sync::MutexGuard<'static, ()>,
    admin_session: String,
    alice_session: String,
    bob_session: String,
    alice_php_site_id: i64,
    alice_static_site_id: i64,
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

async fn login_helper(app: &axum::Router, username: &str, pass: &str) -> (String, String) {
    let payload = serde_json::json!({ "username": username, "password": pass });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let session_val = extract_cookie_value(cookie, SESSION_COOKIE_NAME).unwrap();

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let csrf_val = body_json["csrf_token"].as_str().unwrap().to_string();

    (session_val, csrf_val)
}

fn seed_php_status_fixture(runner: &MockSystemRunner) {
    let master = PhpFpmMasterConfig::new("p_alice", 10001, PhpVersion::Php83)
        .expect("valid fixture tenant and PHP version");
    let socket_path = master.default_socket_path();
    let mock_status_file = socket_path.with_extension("status.json");
    runner.set_file_content(&socket_path, b"");
    let status_json = r#"{"pool":"site_1","process manager":"ondemand","start time":1728000000,"start since":100,"accepted conn":120,"listen queue":0,"max listen queue":2,"listen queue len":128,"idle processes":1,"active processes":2,"total processes":3,"max active processes":5,"max children reached":0,"slow requests":1}"#;
    runner.set_file_content(&mock_status_file, status_json.as_bytes());
}

// Exercises the shared fixture and status reader without starting AgentServer.
#[test]
fn php_status_fixture_supplies_metrics_at_the_runtime_socket_path() {
    let runner = MockSystemRunner::new();
    seed_php_status_fixture(&runner);
    let master = PhpFpmMasterConfig::new("p_alice", 10001, PhpVersion::Php83).unwrap();
    let socket_path = master.default_socket_path();
    assert!(runner.path_exists(&socket_path));
    assert!(
        runner
            .read_file(&socket_path.with_extension("status.json"))
            .is_ok(),
        "status JSON must sit beside the actual runtime socket"
    );
    let response = panel_agent::php::get_php_fpm_pool_status(
        &runner,
        panel_proto::PhpFpmPoolStatusRequest {
            username: "p_alice".into(),
            version: "8.3".into(),
            pool_name: "site_1".into(),
        },
    )
    .unwrap();
    assert!(response.is_active);
    assert!(response.socket_exists);
    let metrics = response.metrics.expect("fixture metrics present");
    assert_eq!(metrics.pool, "site_1");
    assert_eq!(metrics.active_processes, 2);
    assert_eq!(metrics.idle_processes, 1);
    assert_eq!(metrics.total_processes, 3);
    assert_eq!(metrics.accepted_conn, 120);
    assert_eq!(metrics.slow_requests, 1);
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

    let nginx_dir = tmp.path().join("etc/nginx");
    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"active\n", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");
    runner.set_response("php-fpm8.3", 0, b"test is successful", b"");

    // Populate mock sockets & status files for Alice
    seed_php_status_fixture(&runner);

    // Populate mock log files for Alice
    let error_log_path = PathBuf::from("/var/log/panel/p_alice/php-fpm-8.3-error.log");
    let slow_log_path = PathBuf::from("/var/log/panel/p_alice/php-fpm-8.3-slow.log");
    let sample_errors = "[01-Oct-2026 10:00:00] NOTICE: fpm is running\n[01-Oct-2026 10:05:00] WARNING: [pool site_1] child 1234 exited on signal 15\n[01-Oct-2026 10:10:00] ERROR: failed to open stream\n";
    runner.set_file_content(&error_log_path, sample_errors.as_bytes());

    let sample_slow = "[01-Oct-2026 10:12:00]  [pool site_1] pid 1234\nscript_filename = /home/p_alice/sites/alice.test/public/slow.php\n[0x00007f] sleep() database query slow\n";
    runner.set_file_content(&slow_log_path, sample_slow.as_bytes());

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
        nginx_dir,
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

    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let default_plan = plan_repo
        .create(&CreatePlan {
            name: "Standard Plan".to_string(),
            description: Some("Standard PHP plan".to_string()),
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(512 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(50),
            io_weight: Some(100),
            disk_limit_bytes: Some(10 * 1024 * 1024 * 1024),
            inode_limit: Some(100_000),
            max_sites: Some(5),
            max_domains: Some(10),
            max_databases: Some(5),
            max_cron_jobs: Some(5),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(64 * 1024 * 1024),
            shell_policy: Some("sftp".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create plan");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let pass_hash = hash_password("Secret123!").expect("hash pass");

    // Admin
    user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: pass_hash.clone(),
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            linux_uid: None,
            linux_username: None,
            plan_id: None,
        })
        .await
        .expect("create admin");

    // Alice
    let alice = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: pass_hash.clone(),
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            linux_uid: Some(10001),
            linux_username: Some("p_alice".to_string()),
            plan_id: Some(default_plan.id),
        })
        .await
        .expect("create alice");

    // Bob
    user_repo
        .create(&CreateUser {
            username: "bob".to_string(),
            password_hash: pass_hash.clone(),
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            linux_uid: Some(10002),
            linux_username: Some("p_bob".to_string()),
            plan_id: Some(default_plan.id),
        })
        .await
        .expect("create bob");

    // Alice PHP Site (ID 1)
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let domain_repo = SqliteDomainRepository::new(pool.clone());

    let alice_php_site = site_repo
        .create(&CreateSite {
            user_id: alice.id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/alice.test/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some(
                serde_json::json!({
                    "php": {
                        "preset": "generic"
                    }
                })
                .to_string(),
            ),
        })
        .await
        .expect("create alice php site");

    domain_repo
        .create(&CreateDomain {
            site_id: alice_php_site.id,
            name: "alice.test".to_string(),
            unicode_name: "alice.test".to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .expect("create alice php domain");

    // Alice Static Site (ID 2)
    let alice_static_site = site_repo
        .create(&CreateSite {
            user_id: alice.id,
            kind: "static".to_string(),
            docroot: "/home/p_alice/sites/alice-static.test/public".to_string(),
            runtime: None,
            version: None,
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .expect("create alice static site");

    domain_repo
        .create(&CreateDomain {
            site_id: alice_static_site.id,
            name: "alice-static.test".to_string(),
            unicode_name: "alice-static.test".to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .expect("create alice static domain");

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };

    let lockout_mgr = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(&sock_path));
    let app_state = AppState::new(
        agent_client,
        pool.clone(),
        "development".to_string(),
        auth_config,
        lockout_mgr,
    );
    let app = create_router(app_state);

    let (admin_session, _) = login_helper(&app, "admin", "Secret123!").await;
    let (alice_session, _) = login_helper(&app, "alice", "Secret123!").await;
    let (bob_session, _) = login_helper(&app, "bob", "Secret123!").await;

    TestContext {
        app,
        pool,
        runner,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        _test_guard: guard,
        admin_session,
        alice_session,
        bob_session,
        alice_php_site_id: alice_php_site.id,
        alice_static_site_id: alice_static_site.id,
    }
}

#[tokio::test]
async fn test_get_site_php_status_metrics() {
    let ctx = setup_test_context().await;

    // 1. Alice queries her PHP site status
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/php/status",
            ctx.alice_php_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let status_res: SitePhpStatusResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(status_res.site_id, ctx.alice_php_site_id);
    assert_eq!(status_res.php_version, "8.3");
    assert!(status_res.is_active);
    assert!(status_res.socket_exists);

    let metrics = status_res.metrics.expect("metrics present");
    assert_eq!(metrics.active_processes, 2);
    assert_eq!(metrics.idle_processes, 1);
    assert_eq!(metrics.total_processes, 3);
    assert_eq!(metrics.accepted_conn, 120);

    // 2. Querying static site returns 400 Bad Request
    let req_static = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/php/status",
            ctx.alice_static_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_static = ctx.app.clone().oneshot(req_static).await.unwrap();
    assert_eq!(res_static.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_get_site_php_logs_read() {
    let ctx = setup_test_context().await;

    // 1. Read PHP error logs
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/php/logs?type=error&lines=10",
            ctx.alice_php_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let logs_res: SitePhpLogsResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(logs_res.site_id, ctx.alice_php_site_id);
    assert_eq!(logs_res.log_type, "error");
    assert_eq!(logs_res.lines.len(), 3);
    assert_eq!(logs_res.total_matched, 3);
    assert!(logs_res.lines[0].contains("NOTICE: fpm is running"));

    // 2. Read PHP slow logs with filter query
    let req_slow = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/php/logs?type=slow&q=database",
            ctx.alice_php_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_slow = ctx.app.clone().oneshot(req_slow).await.unwrap();
    assert_eq!(res_slow.status(), StatusCode::OK);

    let body_slow = res_slow.into_body().collect().await.unwrap().to_bytes();
    let logs_slow: SitePhpLogsResponse = serde_json::from_slice(&body_slow).unwrap();
    assert_eq!(logs_slow.log_type, "slow");
    assert_eq!(logs_slow.lines.len(), 1);
    assert!(logs_slow.lines[0].contains("database query slow"));

    // 3. Static site logs query returns 400 Bad Request
    let req_bad = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/php/logs",
            ctx.alice_static_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_bad = ctx.app.clone().oneshot(req_bad).await.unwrap();
    assert_eq!(res_bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_site_logs_endpoint_php_log_types() {
    let ctx = setup_test_context().await;

    // 1. General logs endpoint with type=php_error
    let req_err = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/logs?type=php_error&lines=20",
            ctx.alice_php_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_err = ctx.app.clone().oneshot(req_err).await.unwrap();
    assert_eq!(res_err.status(), StatusCode::OK);

    let body_err = res_err.into_body().collect().await.unwrap().to_bytes();
    let logs_err: SiteLogsResponse = serde_json::from_slice(&body_err).unwrap();
    assert_eq!(logs_err.site_id, ctx.alice_php_site_id);
    assert_eq!(logs_err.log_type, "php_error");
    assert_eq!(logs_err.lines.len(), 3);

    // 2. General logs endpoint with type=php_slow
    let req_slow = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/logs?type=php_slow&lines=20",
            ctx.alice_php_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_slow = ctx.app.clone().oneshot(req_slow).await.unwrap();
    assert_eq!(res_slow.status(), StatusCode::OK);

    let body_slow = res_slow.into_body().collect().await.unwrap().to_bytes();
    let logs_slow: SiteLogsResponse = serde_json::from_slice(&body_slow).unwrap();
    assert_eq!(logs_slow.log_type, "php_slow");
    assert!(!logs_slow.lines.is_empty());

    // 3. Invalid log type returns 400 Bad Request
    let req_invalid = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/logs?type=unknown_log_type",
            ctx.alice_php_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_invalid = ctx.app.clone().oneshot(req_invalid).await.unwrap();
    assert_eq!(res_invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_php_status_and_logs_cross_tenant_isolation() {
    let ctx = setup_test_context().await;

    // 1. Bob attempts to access Alice's PHP status -> 403 Forbidden
    let req_bob_status = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/php/status",
            ctx.alice_php_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_bob_status = ctx.app.clone().oneshot(req_bob_status).await.unwrap();
    assert_eq!(res_bob_status.status(), StatusCode::FORBIDDEN);

    // 2. Bob attempts to read Alice's PHP logs -> 403 Forbidden
    let req_bob_logs = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/php/logs", ctx.alice_php_site_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_bob_logs = ctx.app.clone().oneshot(req_bob_logs).await.unwrap();
    assert_eq!(res_bob_logs.status(), StatusCode::FORBIDDEN);

    // 3. Admin CAN access Alice's PHP status -> 200 OK
    let req_admin_status = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/php/status",
            ctx.alice_php_site_id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_admin_status = ctx.app.clone().oneshot(req_admin_status).await.unwrap();
    assert_eq!(res_admin_status.status(), StatusCode::OK);

    // 4. Admin CAN read Alice's PHP logs -> 200 OK
    let req_admin_logs = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/php/logs", ctx.alice_php_site_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_admin_logs = ctx.app.clone().oneshot(req_admin_logs).await.unwrap();
    assert_eq!(res_admin_logs.status(), StatusCode::OK);
}
