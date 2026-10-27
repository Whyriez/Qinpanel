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
use panel_api::php::{
    PhpExtensionActionApiResponse, PhpExtensionListApiResponse, SitePhpExtensionsResponse,
};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqliteSiteRepository, SqliteUserRepository};
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
    runner.set_response("systemctl", 0, b"masked\n", b"");
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

    let cookie_header = login_res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let admin_session = extract_cookie_value(cookie_header, SESSION_COOKIE_NAME).unwrap();

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

    let body = csrf_res.into_body().collect().await.unwrap().to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let admin_csrf = csrf_json["csrf_token"].as_str().unwrap().to_string();

    // Login alice
    let login_alice = serde_json::json!({
        "username": "alice",
        "password": "AlicePass123!",
    });
    let alice_res = app
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

    let cookie_header = alice_res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let alice_session = extract_cookie_value(cookie_header, SESSION_COOKIE_NAME).unwrap();

    let csrf_res = app
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

    let body = csrf_res.into_body().collect().await.unwrap().to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let alice_csrf = csrf_json["csrf_token"].as_str().unwrap().to_string();

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
        alice_id: alice.id,
    }
}

#[tokio::test]
async fn test_list_php_extensions_endpoint() {
    let ctx = setup_test_context().await;

    // 1. Success list extensions
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/php/versions/8.3/extensions")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: PhpExtensionListApiResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.version, "8.3");
    assert_eq!(body.extensions.len(), 16);

    let redis = body.extensions.iter().find(|e| e.name == "redis").unwrap();
    assert_eq!(redis.package_name, "php8.3-redis");
    assert_eq!(redis.category, "Cache");

    // 2. Reject unauthenticated
    let unauth = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/php/versions/8.3/extensions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    // 3. Reject invalid version
    let bad_ver = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/php/versions/invalid_ver/extensions")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(bad_ver.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_install_and_remove_extension_permissions_and_jobs() {
    let ctx = setup_test_context().await;

    // 1. Non-admin forbidden
    let non_admin_install = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.3/extensions/redis/install")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(non_admin_install.status(), StatusCode::FORBIDDEN);

    // 2. Admin install valid extension enqueues job
    let admin_install = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.3/extensions/redis/install")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(admin_install.status(), StatusCode::ACCEPTED);
    let bytes = admin_install
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let job: JobItem = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(job.name, "php.extension.install");

    // 3. Admin remove base extension rejected
    let rem_base = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.3/extensions/curl/remove")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(rem_base.status(), StatusCode::BAD_REQUEST);

    // 4. Admin remove valid extension enqueues job
    let admin_remove = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.3/extensions/redis/remove")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(admin_remove.status(), StatusCode::ACCEPTED);
    let bytes = admin_remove.into_body().collect().await.unwrap().to_bytes();
    let job: JobItem = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(job.name, "php.extension.remove");
}

#[tokio::test]
async fn test_toggle_php_extension() {
    let ctx = setup_test_context().await;

    // 1. Admin toggle valid extension
    let toggle_payload = serde_json::json!({ "enabled": true });
    let res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.3/extensions/redis/toggle")
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::from(serde_json::to_vec(&toggle_payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let action_res: PhpExtensionActionApiResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(action_res.success);
    assert_eq!(action_res.extension, "redis");
    assert_eq!(action_res.version, "8.3");

    // 2. Non-admin forbidden
    let non_admin = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.3/extensions/redis/toggle")
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
                .body(Body::from(serde_json::to_vec(&toggle_payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(non_admin.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_per_site_php_extension_overrides() {
    let ctx = setup_test_context().await;
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());

    // Create a site for Alice
    let site = site_repo
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: "/var/www/vhosts/alice/alice-app.test/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    // 1. Initial GET site php extensions -> empty list
    let get_res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/sites/{}/php/extensions", site.id))
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(get_res.status(), StatusCode::OK);
    let bytes = get_res.into_body().collect().await.unwrap().to_bytes();
    let ext_res: SitePhpExtensionsResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(ext_res.site_id, site.id);
    assert_eq!(ext_res.extensions.len(), 0);
    assert_eq!(ext_res.available_extensions.len(), 16);

    // 2. Alice updates extension preferences with valid curated extensions
    let update_payload = serde_json::json!({
        "extensions": ["redis", "imagick", "bcmath"]
    });
    let put_res = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/sites/{}/php/extensions", site.id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
                .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(put_res.status(), StatusCode::OK);
    let bytes = put_res.into_body().collect().await.unwrap().to_bytes();
    let updated: SitePhpExtensionsResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(updated.extensions, vec!["redis", "imagick", "bcmath"]);

    // 3. Reject invalid / malicious extension name
    let bad_payload = serde_json::json!({
        "extensions": ["malicious_ext; rm -rf /"]
    });
    let bad_put = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/sites/{}/php/extensions", site.id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
                .body(Body::from(serde_json::to_vec(&bad_payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("request");
    assert_eq!(bad_put.status(), StatusCode::BAD_REQUEST);

    // 4. Verify persisted in database settings_json
    let reloaded = site_repo.find_by_id(site.id).await.unwrap().unwrap();
    assert!(reloaded.settings_json.contains("redis"));
    assert!(reloaded.settings_json.contains("imagick"));
}
