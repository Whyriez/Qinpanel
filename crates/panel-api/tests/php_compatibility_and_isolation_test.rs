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
use panel_api::php::SwitchPhpVersionResponse;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateDomain, CreatePlan, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{
    SqliteDomainRepository, SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository,
};
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestContext {
    app: axum::Router,
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    _test_guard: tokio::sync::MutexGuard<'static, ()>,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    bob_csrf: String,
    alice_id: i64,
    bob_id: i64,
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
    runner.set_response("php-fpm8.1", 0, b"test is successful", b"");
    runner.set_response("php-fpm8.2", 0, b"test is successful", b"");
    runner.set_response("php-fpm8.3", 0, b"test is successful", b"");

    // Register installed PHP runtime binaries in mock runner
    runner.add_existing_path("/usr/sbin/php-fpm8.1");
    runner.add_existing_path("/usr/bin/php8.1");
    runner.add_existing_path("/usr/sbin/php-fpm8.2");
    runner.add_existing_path("/usr/bin/php8.2");
    runner.add_existing_path("/usr/sbin/php-fpm8.3");
    runner.add_existing_path("/usr/bin/php8.3");

    // Pre-populate socket paths for mock health checks
    runner.add_existing_path("/run/panel/php/p_alice-8.1/fpm.sock");
    runner.add_existing_path("/run/panel/php/p_alice-8.3/fpm.sock");
    runner.add_existing_path("/run/panel/php/p_bob-8.2/fpm.sock");
    runner.add_existing_path("/run/panel/php/p_bob-8.3/fpm.sock");

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

    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let default_plan = plan_repo
        .create(&CreatePlan {
            name: "Standard Plan".to_string(),
            description: Some("Standard PHP hosting plan".to_string()),
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(512 * 1024 * 1024), // 512 MB
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

    // Alice
    let alice = user_repo
        .create(&CreateUser {
            username: "p_alice".to_string(),
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
    let bob = user_repo
        .create(&CreateUser {
            username: "p_bob".to_string(),
            password_hash: pass_hash.clone(),
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            linux_uid: Some(10002),
            linux_username: Some("p_bob".to_string()),
            plan_id: Some(default_plan.id),
        })
        .await
        .expect("create bob");

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

    let (alice_session, alice_csrf) = login_helper(&app, "p_alice", "Secret123!").await;
    let (bob_session, bob_csrf) = login_helper(&app, "p_bob", "Secret123!").await;

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        _test_guard: guard,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        alice_id: alice.id,
        bob_id: bob.id,
    }
}

#[tokio::test]
async fn test_wordpress_and_laravel_site_creation_and_presets() {
    let ctx = setup_test_context().await;
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let domain_repo = SqliteDomainRepository::new(ctx.pool.clone());

    // 1. Create WordPress site for Alice
    let wp_site = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/myblog.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some(
                serde_json::json!({
                    "php": {
                        "preset": "wordpress"
                    }
                })
                .to_string(),
            ),
        })
        .await
        .unwrap();

    domain_repo
        .create(&CreateDomain {
            site_id: wp_site.id,
            name: "myblog.com".to_string(),
            unicode_name: "myblog.com".to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .unwrap();

    // 2. Create Laravel site for Bob
    let laravel_site = site_repo
        .create(&CreateSite {
            user_id: ctx.bob_id,
            kind: "php".to_string(),
            docroot: "/home/p_bob/sites/laravelapp.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.2".to_string()),
            status: Some("active".to_string()),
            settings_json: Some(
                serde_json::json!({
                    "php": {
                        "preset": "laravel"
                    }
                })
                .to_string(),
            ),
        })
        .await
        .unwrap();

    domain_repo
        .create(&CreateDomain {
            site_id: laravel_site.id,
            name: "laravelapp.com".to_string(),
            unicode_name: "laravelapp.com".to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .unwrap();

    // Verify sites in database
    let fetched_wp = site_repo.find_by_id(wp_site.id).await.unwrap().unwrap();
    let fetched_laravel = site_repo
        .find_by_id(laravel_site.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched_wp.kind, "php");
    assert_eq!(fetched_wp.version.as_deref(), Some("8.3"));
    assert!(fetched_wp.settings_json.contains("wordpress"));

    assert_eq!(fetched_laravel.kind, "php");
    assert_eq!(fetched_laravel.version.as_deref(), Some("8.2"));
    assert!(fetched_laravel.settings_json.contains("laravel"));
}

#[tokio::test]
async fn test_multi_tenant_multi_version_concurrency() {
    let ctx = setup_test_context().await;
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let domain_repo = SqliteDomainRepository::new(ctx.pool.clone());

    // Alice on PHP 8.1
    let site_alice = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/alice-81.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.1".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .unwrap();

    domain_repo
        .create(&CreateDomain {
            site_id: site_alice.id,
            name: "alice-81.com".to_string(),
            unicode_name: "alice-81.com".to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .unwrap();

    // Bob on PHP 8.3
    let site_bob = site_repo
        .create(&CreateSite {
            user_id: ctx.bob_id,
            kind: "php".to_string(),
            docroot: "/home/p_bob/sites/bob-83.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .unwrap();

    domain_repo
        .create(&CreateDomain {
            site_id: site_bob.id,
            name: "bob-83.com".to_string(),
            unicode_name: "bob-83.com".to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .unwrap();

    // Cross-tenant access: Alice cannot inspect Bob's PHP settings
    let req_cross = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/php/settings", site_bob.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();

    let res_cross = ctx.app.clone().oneshot(req_cross).await.unwrap();
    assert_eq!(res_cross.status(), StatusCode::FORBIDDEN);

    // Legitimate access: Bob inspects his own PHP settings
    let req_own = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/php/settings", site_bob.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::empty())
        .unwrap();

    let res_own = ctx.app.clone().oneshot(req_own).await.unwrap();
    assert_eq!(res_own.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_php_version_switch_zero_downtime_flow() {
    let ctx = setup_test_context().await;
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let domain_repo = SqliteDomainRepository::new(ctx.pool.clone());

    // Alice site initially on PHP 8.1
    let site = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/switch-test.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.1".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .unwrap();

    domain_repo
        .create(&CreateDomain {
            site_id: site.id,
            name: "switch-test.com".to_string(),
            unicode_name: "switch-test.com".to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .unwrap();

    // Alice switches version from 8.1 to 8.3
    let switch_payload = serde_json::json!({
        "version": "8.3"
    });

    let req_switch = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/php/version", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&switch_payload).unwrap()))
        .unwrap();

    let res_switch = ctx.app.clone().oneshot(req_switch).await.unwrap();
    assert_eq!(res_switch.status(), StatusCode::OK);

    let body = res_switch.into_body().collect().await.unwrap().to_bytes();
    let resp: SwitchPhpVersionResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(resp.site_id, site.id);
    assert_eq!(resp.previous_version, "8.1");
    assert_eq!(resp.current_version, "8.3");

    // Verify database record updated
    let updated_site = site_repo.find_by_id(site.id).await.unwrap().unwrap();
    assert_eq!(updated_site.version.as_deref(), Some("8.3"));
}
