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
use panel_api::jobs::JobItem;
use panel_api::php::PhpRuntimeListApiResponse;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqliteJobRepository, SqliteSiteRepository, SqliteUserRepository};
use panel_runtime::php::PhpVersion;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestContext {
    app: axum::Router,
    pool: panel_db::pool::DbPool,
    #[allow(dead_code)]
    state: AppState,
    runner: Arc<MockSystemRunner>,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    _test_guard: tokio::sync::MutexGuard<'static, ()>,
    admin_session: String,
    admin_csrf: String,
    alice_session: String,
    alice_csrf: String,
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

fn php_runtime_runner() -> Arc<MockSystemRunner> {
    let runner = Arc::new(MockSystemRunner::new());
    runner.set_file_content("/etc/os-release", b"ID=ubuntu\nVERSION_ID=\"24.04\"\n");
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"masked\n", b"");
    runner.set_response("apt-get", 0, b"Reading package lists... Done\n", b"");
    runner.set_response("add-apt-repository", 0, b"Repository added\n", b"");
    runner.set_response("dpkg-query", 0, b"", b"");
    let policy: String = PhpVersion::all_supported()
        .iter()
        .flat_map(PhpVersion::base_packages)
        .map(|package| format!("{package}:\n  Installed: (none)\n  Candidate: 1.0\n"))
        .collect();
    runner.set_response("apt-cache", 0, policy.as_bytes(), b"");
    runner
}

fn assert_supported_php_versions(data: &PhpRuntimeListApiResponse) {
    assert_eq!(data.versions.len(), PhpVersion::all_supported().len());
    let versions: Vec<&str> = data.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(
        versions,
        vec!["7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"]
    );
}

// Exercises this suite's command fixture without starting AgentServer or a daemon.
#[test]
fn php_install_fixture_supports_candidate_checks_and_fpm_masking_without_daemon() {
    let runner = php_runtime_runner();
    let result = panel_agent::php::install_php_runtime(runner.as_ref(), "8.2")
        .expect("PHP API fixture must provide installable package candidates");
    assert!(result.success);
    assert!(result.masked);
    assert!(
        runner
            .get_commands()
            .iter()
            .any(|command| command == "systemctl mask php8.2-fpm.service")
    );
    let inventory = panel_agent::php::list_php_runtimes(runner.as_ref())
        .expect("PHP API fixture must enumerate all supported versions");
    assert_supported_php_versions(&PhpRuntimeListApiResponse::from(inventory));
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

    let runner = php_runtime_runner();

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
        server.run(rx).await.expect("agent server run");
    });

    let start = std::time::Instant::now();
    while !sock_path.exists() && start.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

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

    let alice_hash = hash_password("AlicePass123!").expect("hash alice pass");
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

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        ..Default::default()
    };
    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(sock_path));

    let state = AppState::new(
        agent_client,
        pool.clone(),
        "test".to_string(),
        auth_config,
        lockout_manager,
    );

    let app = create_router(state.clone());

    // Login admin
    let login_payload = serde_json::json!({
        "username": "admin",
        "password": "AdminPass123!",
    });
    let login_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&login_payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("login admin");

    let cookies = login_res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let admin_session =
        extract_cookie_value(&cookies, SESSION_COOKIE_NAME).expect("admin session cookie");

    let csrf_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/csrf")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, admin_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("csrf admin");
    let csrf_body = csrf_res.into_body().collect().await.unwrap().to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&csrf_body).unwrap();
    let admin_csrf = csrf_json["csrf_token"].as_str().unwrap().to_string();

    // Login alice
    let login_alice = serde_json::json!({
        "username": "alice",
        "password": "AlicePass123!",
    });
    let login_alice_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&login_alice).unwrap()))
                .unwrap(),
        )
        .await
        .expect("login alice");

    let alice_cookies = login_alice_res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let alice_session =
        extract_cookie_value(&alice_cookies, SESSION_COOKIE_NAME).expect("alice session cookie");

    let alice_csrf_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/csrf")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, alice_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("csrf alice");
    let alice_csrf_body = alice_csrf_res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let alice_csrf_json: serde_json::Value = serde_json::from_slice(&alice_csrf_body).unwrap();
    let alice_csrf = alice_csrf_json["csrf_token"].as_str().unwrap().to_string();

    TestContext {
        app,
        pool,
        state,
        runner,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        _test_guard: guard,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        alice_id: alice.id,
    }
}

#[tokio::test]
async fn test_php_runtime_list_permissions_and_response() {
    let ctx = setup_test_context().await;

    // 1. Unauthenticated request -> 401 Unauthorized
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/php/versions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. Authenticated user (alice) -> 200 OK with PHP versions
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/php/versions")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let data: PhpRuntimeListApiResponse = serde_json::from_slice(&body).unwrap();
    assert_supported_php_versions(&data);
}

#[tokio::test]
async fn test_php_repo_setup_rbac_and_execution() {
    let ctx = setup_test_context().await;

    // 1. Normal user cannot setup repo -> 403 Forbidden
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/repo/setup")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 2. Admin with CSRF enqueues job -> 202 Accepted
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/repo/setup")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let job_item: JobItem = serde_json::from_slice(&body).unwrap();
    assert_eq!(job_item.name, "php.repo.setup");

    // 3. Process background job
    let processed = panel_api::jobs::worker::process_one_job(&ctx.state.job_runner)
        .await
        .expect("process job");
    assert!(processed);

    let job_repo = SqliteJobRepository::new(ctx.pool.clone());
    let finished_job = job_repo
        .find_by_id(&job_item.id)
        .await
        .unwrap()
        .expect("job exists");
    assert_eq!(
        finished_job.status, "succeeded",
        "Job error: {:?}",
        finished_job.error_message
    );
    assert_eq!(finished_job.progress, 100);

    // Check mock runner executed add-apt-repository
    let commands = ctx.runner.get_commands();
    assert!(
        commands
            .iter()
            .any(|cmd| cmd.contains("add-apt-repository") && cmd.contains("ondrej/php")),
        "Expected add-apt-repository call for ondrej/php"
    );
}

#[tokio::test]
async fn test_php_install_invalid_version_and_masking() {
    let ctx = setup_test_context().await;

    // 1. Invalid version rejected -> 400 Bad Request
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/5.6/install")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 2. Valid version 8.2 enqueues install job -> 202 Accepted
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.2/install")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let job_item: JobItem = serde_json::from_slice(&body).unwrap();
    assert_eq!(job_item.name, "php.install");

    // 3. Process installation and masking job
    let processed = panel_api::jobs::worker::process_one_job(&ctx.state.job_runner)
        .await
        .expect("process job");
    assert!(processed);

    let job_repo = SqliteJobRepository::new(ctx.pool.clone());
    let finished_job = job_repo
        .find_by_id(&job_item.id)
        .await
        .unwrap()
        .expect("job exists");
    assert_eq!(
        finished_job.status, "succeeded",
        "Job error: {:?}",
        finished_job.error_message
    );
    assert_eq!(finished_job.progress, 100);
    assert_eq!(finished_job.retry_count, 0);

    // CRITICAL PRD REQUIREMENT: Verify systemctl mask php8.2-fpm.service was executed!
    let commands = ctx.runner.get_commands();
    assert!(
        commands
            .iter()
            .any(|cmd| cmd.contains("systemctl mask php8.2-fpm.service")),
        "CRITICAL REQUIREMENT: systemctl mask php8.2-fpm.service must be called on install"
    );
}

#[tokio::test]
async fn test_php_remove_prevented_if_in_use_by_site() {
    let ctx = setup_test_context().await;

    // 1. Create a site using PHP 8.2
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/public_html".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.2".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .expect("create site");

    // 2. Attempt to remove PHP 8.2 -> Should be rejected with 400 Bad Request
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.2/remove")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let err_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        err_json["error"]
            .as_str()
            .unwrap()
            .contains("sedang digunakan oleh 1 situs"),
        "Error message must specify that the version is currently used by sites"
    );

    // 3. Remove PHP 8.3 (which is NOT used by any sites) -> 202 Accepted
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.3/remove")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let job_item: JobItem = serde_json::from_slice(&body).unwrap();
    assert_eq!(job_item.name, "php.remove");

    // 4. Process removal job
    let processed = panel_api::jobs::worker::process_one_job(&ctx.state.job_runner)
        .await
        .expect("process job");
    assert!(processed);

    let job_repo = SqliteJobRepository::new(ctx.pool.clone());
    let finished_job = job_repo
        .find_by_id(&job_item.id)
        .await
        .unwrap()
        .expect("job exists");
    assert_eq!(
        finished_job.status, "succeeded",
        "Job error: {:?}",
        finished_job.error_message
    );

    // Check systemctl unmask and apt-get purge executed
    let commands = ctx.runner.get_commands();
    assert!(
        commands
            .iter()
            .any(|cmd| cmd.contains("systemctl unmask php8.3-fpm.service")),
        "Expected systemctl unmask php8.3-fpm.service on remove"
    );
    assert!(
        commands
            .iter()
            .any(|cmd| cmd.contains("apt-get remove -y --purge") && cmd.contains("php8.3")),
        "Expected apt-get remove --purge for php8.3 packages"
    );
}
