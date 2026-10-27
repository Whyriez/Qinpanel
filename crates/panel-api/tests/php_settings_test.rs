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
use panel_api::php::{SitePhpSettingsResponse, UpdateSitePhpSettingsInput};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_core::php::PhpSettings;
use panel_db::models::{CreatePlan, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestContext {
    app: axum::Router,
    pool: panel_db::pool::DbPool,
    runner: Arc<MockSystemRunner>,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    _test_guard: tokio::sync::MutexGuard<'static, ()>,
    admin_session: String,
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
    runner.set_response("php-fpm8.3", 0, b"syntax is ok\n", b"");

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
    let plan = plan_repo
        .create(&CreatePlan {
            name: "StandardPlan".to_string(),
            description: Some("Plan with 256MB PHP limit".to_string()),
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(256 * 1024 * 1024), // 256 MB
            memory_high_bytes: None,
            tasks_max: Some(50),
            io_weight: Some(100),
            disk_limit_bytes: Some(1024 * 1024 * 1024),
            inode_limit: Some(10000),
            max_sites: Some(5),
            max_domains: Some(5),
            max_databases: Some(2),
            max_cron_jobs: Some(5),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(64 * 1024 * 1024), // 64 MB
            shell_policy: Some("sftp".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create plan");

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
        .expect("create admin user");

    let alice_hash = hash_password("AlicePass123!").expect("hash pass");
    let alice_user = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            plan_id: Some(plan.id),
            linux_username: Some("p_alice".to_string()),
            linux_uid: Some(10001),
        })
        .await
        .expect("create alice user");

    let bob_hash = hash_password("BobPass123!").expect("hash pass");
    user_repo
        .create(&CreateUser {
            username: "bob".to_string(),
            password_hash: bob_hash,
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            plan_id: Some(plan.id),
            linux_username: Some("p_bob".to_string()),
            linux_uid: Some(10002),
        })
        .await
        .expect("create bob user");

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };

    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(&sock_path));
    let state = AppState::new(
        agent_client,
        pool.clone(),
        "development".to_string(),
        auth_config,
        lockout_manager,
    );

    let app = create_router(state);

    let (admin_session, admin_csrf) = login_helper(&app, "admin", "AdminPass123!").await;
    let (alice_session, alice_csrf) = login_helper(&app, "alice", "AlicePass123!").await;
    let (bob_session, bob_csrf) = login_helper(&app, "bob", "BobPass123!").await;

    TestContext {
        app,
        pool,
        runner,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        _test_guard: guard,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        alice_id: alice_user.id,
    }
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

#[tokio::test]
async fn test_get_php_settings_defaults_and_limits() {
    let ctx = setup_test_context().await;

    // Create PHP site for Alice
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let site = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/alice.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .unwrap();

    // 1. Tenant (Alice) calls GET /api/v1/sites/{id}/php/settings
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let resp: SitePhpSettingsResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(resp.site_id, site.id);
    assert_eq!(resp.php_version, "8.3");
    assert!(!resp.is_admin);
    assert!(resp.admin_ini.is_none());
    // Bounded by Alice's plan (256MB memory, 64MB upload)
    assert_eq!(resp.limits.memory_limit_mb, 256);
    assert_eq!(resp.limits.upload_limit_mb, 64);
    assert_eq!(resp.settings.memory_limit_mb, 128); // default for limits
    assert_eq!(resp.settings.date_timezone, "UTC");

    // 2. Admin calls GET /api/v1/sites/{id}/php/settings
    let req_admin = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_admin = ctx.app.clone().oneshot(req_admin).await.unwrap();
    assert_eq!(res_admin.status(), StatusCode::OK);
    let body_admin = res_admin.into_body().collect().await.unwrap().to_bytes();
    let resp_admin: SitePhpSettingsResponse = serde_json::from_slice(&body_admin).unwrap();
    assert!(resp_admin.is_admin);
}

#[tokio::test]
async fn test_update_php_settings_valid_curated_flow() {
    let ctx = setup_test_context().await;

    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let site = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/alice.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .unwrap();

    // Alice updates curated PHP settings
    let new_settings = PhpSettings {
        memory_limit_mb: 200,
        upload_max_filesize_mb: 40,
        post_max_size_mb: 50,
        max_execution_time: 45,
        max_input_vars: 2000,
        display_errors: true,
        date_timezone: "Asia/Jakarta".to_string(),
        opcache_enable: true,
        opcache_memory_consumption_mb: 64,
        opcache_validate_timestamps: true,
        opcache_revalidate_freq: 5,
    };

    let update_input = UpdateSitePhpSettingsInput {
        settings: new_settings.clone(),
        admin_ini: None,
    };

    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&update_input).unwrap()))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let resp: SitePhpSettingsResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(resp.settings.memory_limit_mb, 200);
    assert_eq!(resp.settings.date_timezone, "Asia/Jakarta");
    assert!(resp.settings.display_errors);

    // Verify persisted in DB
    let reloaded = site_repo.find_by_id(site.id).await.unwrap().unwrap();
    assert!(reloaded.settings_json.contains("200"));
    assert!(reloaded.settings_json.contains("Asia/Jakarta"));

    // Verify written pool conf in agent
    let files = ctx.runner.files.lock().unwrap();
    let pool_path = PathBuf::from(format!(
        "/etc/panel/php/p_alice-8.3/pool.d/site_{}.conf",
        site.id
    ));
    assert!(files.contains_key(&pool_path));
    let content = String::from_utf8(files.get(&pool_path).unwrap().clone()).unwrap();
    assert!(content.contains("php_admin_value[memory_limit] = 200M"));
    assert!(content.contains("php_admin_value[date.timezone] = Asia/Jakarta"));
    assert!(content.contains("php_admin_flag[display_errors] = on"));
}

#[tokio::test]
async fn test_update_php_settings_exceeding_plan_limits_rejected() {
    let ctx = setup_test_context().await;

    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let site = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/alice.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .unwrap();

    // 1. memory_limit 512MB exceeds Alice's plan limit of 256MB
    let invalid_mem = PhpSettings {
        memory_limit_mb: 512,
        upload_max_filesize_mb: 32,
        post_max_size_mb: 32,
        max_execution_time: 30,
        max_input_vars: 1000,
        display_errors: false,
        date_timezone: "UTC".to_string(),
        opcache_enable: true,
        opcache_memory_consumption_mb: 64,
        opcache_validate_timestamps: true,
        opcache_revalidate_freq: 2,
    };

    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&UpdateSitePhpSettingsInput {
                settings: invalid_mem,
                admin_ini: None,
            })
            .unwrap(),
        ))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("memory_limit harus antara 1 dan 256"));

    // 2. post_max_size < upload_max_filesize
    let invalid_post = PhpSettings {
        memory_limit_mb: 128,
        upload_max_filesize_mb: 50,
        post_max_size_mb: 30, // smaller than upload_max_filesize
        max_execution_time: 30,
        max_input_vars: 1000,
        display_errors: false,
        date_timezone: "UTC".to_string(),
        opcache_enable: true,
        opcache_memory_consumption_mb: 64,
        opcache_validate_timestamps: true,
        opcache_revalidate_freq: 2,
    };

    let req2 = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&UpdateSitePhpSettingsInput {
                settings: invalid_post,
                admin_ini: None,
            })
            .unwrap(),
        ))
        .unwrap();

    let res2 = ctx.app.clone().oneshot(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::BAD_REQUEST);
    let body2 = res2.into_body().collect().await.unwrap().to_bytes();
    let body2_str = String::from_utf8_lossy(&body2);
    assert!(body2_str.contains("post_max_size harus ≥ upload_max_filesize"));
}

#[tokio::test]
async fn test_update_php_settings_raw_admin_ini_rbac_and_validation() {
    let ctx = setup_test_context().await;

    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let site = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/alice.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .unwrap();

    let valid_settings = PhpSettings {
        memory_limit_mb: 128,
        upload_max_filesize_mb: 32,
        post_max_size_mb: 32,
        max_execution_time: 30,
        max_input_vars: 1000,
        display_errors: false,
        date_timezone: "UTC".to_string(),
        opcache_enable: true,
        opcache_memory_consumption_mb: 64,
        opcache_validate_timestamps: true,
        opcache_revalidate_freq: 2,
    };

    // 1. Regular tenant (Alice) trying to provide admin_ini is FORBIDDEN
    let req_tenant = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&UpdateSitePhpSettingsInput {
                settings: valid_settings.clone(),
                admin_ini: Some("allow_url_fopen = 1".to_string()),
            })
            .unwrap(),
        ))
        .unwrap();

    let res_tenant = ctx.app.clone().oneshot(req_tenant).await.unwrap();
    assert_eq!(res_tenant.status(), StatusCode::FORBIDDEN);

    // 2. Admin successfully provides valid admin_ini
    let req_admin = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&UpdateSitePhpSettingsInput {
                settings: valid_settings.clone(),
                admin_ini: Some("allow_url_fopen = 1\nerror_reporting = 32767".to_string()),
            })
            .unwrap(),
        ))
        .unwrap();

    let res_admin = ctx.app.clone().oneshot(req_admin).await.unwrap();
    assert_eq!(res_admin.status(), StatusCode::OK);

    let body = res_admin.into_body().collect().await.unwrap().to_bytes();
    let resp: SitePhpSettingsResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        resp.admin_ini.as_deref(),
        Some("allow_url_fopen = 1\nerror_reporting = 32767")
    );

    // 3. Admin attempts invalid / forbidden directive (e.g. open_basedir escape attempt)
    let req_bad = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&UpdateSitePhpSettingsInput {
                settings: valid_settings.clone(),
                admin_ini: Some("open_basedir = /".to_string()),
            })
            .unwrap(),
        ))
        .unwrap();

    let res_bad = ctx.app.clone().oneshot(req_bad).await.unwrap();
    assert_eq!(res_bad.status(), StatusCode::BAD_REQUEST);
    let body_bad = res_bad.into_body().collect().await.unwrap().to_bytes();
    let body_bad_str = String::from_utf8_lossy(&body_bad);
    assert!(body_bad_str.contains("open_basedir tidak diizinkan"));
}

#[tokio::test]
async fn test_php_settings_cross_tenant_isolation() {
    let ctx = setup_test_context().await;

    // Site owned by Alice
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let site = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/alice.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .unwrap();

    // Bob tries to GET Alice's PHP settings -> Forbidden
    let req_get = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .body(Body::empty())
        .unwrap();

    let res_get = ctx.app.clone().oneshot(req_get).await.unwrap();
    assert_eq!(res_get.status(), StatusCode::FORBIDDEN);

    // Bob tries to PUT Alice's PHP settings -> Forbidden
    let req_put = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/php/settings", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&UpdateSitePhpSettingsInput {
                settings: PhpSettings::default(),
                admin_ini: None,
            })
            .unwrap(),
        ))
        .unwrap();

    let res_put = ctx.app.clone().oneshot(req_put).await.unwrap();
    assert_eq!(res_put.status(), StatusCode::FORBIDDEN);
}
