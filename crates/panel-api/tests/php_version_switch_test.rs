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
use panel_api::php::{SwitchPhpVersionRequest, SwitchPhpVersionResponse};
use panel_api::routes::AppState;
use panel_api::sites::{CreateSiteRequest, SiteResponse};
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqliteSiteRepository, SqliteUserRepository};
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    nginx_dir: std::path::PathBuf,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    admin_session: String,
    admin_csrf: String,
    alice_id: i64,
    runner: Arc<MockSystemRunner>,
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
    runner.set_response("php-fpm8.2", 0, b"test is successful", b"");
    runner.set_response("php-fpm8.3", 0, b"test is successful", b"");
    runner.set_response("php-fpm8.4", 0, b"test is successful", b"");

    // Register PHP 8.2 and 8.3 as installed
    runner.add_existing_path("/usr/sbin/php-fpm8.2");
    runner.add_existing_path("/usr/bin/php8.2");
    runner.add_existing_path("/usr/sbin/php-fpm8.3");
    runner.add_existing_path("/usr/bin/php8.3");

    // Pre-populate socket paths for mock health checks
    runner.add_existing_path("/run/panel/php/p_alice-8.2/fpm.sock");
    runner.add_existing_path("/run/panel/php/p_alice-8.3/fpm.sock");

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
        .expect("create admin user");

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
        .expect("create alice user");

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

    // Login as admin
    let login_body = serde_json::json!({
        "username": "admin",
        "password": "AdminPass123!"
    });
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&login_body).unwrap()))
        .expect("build login req");

    let login_res = app.clone().oneshot(login_req).await.expect("login oneshot");
    assert_eq!(login_res.status(), StatusCode::OK);

    let cookie_hdr = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .expect("cookie str");
    let admin_session =
        extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).expect("session cookie");

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={admin_session}"),
        )
        .body(Body::empty())
        .expect("build csrf req");

    let csrf_res = app.clone().oneshot(csrf_req).await.expect("csrf oneshot");
    let csrf_body = csrf_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&csrf_body).expect("parse json");
    let admin_csrf = csrf_json["csrf_token"]
        .as_str()
        .expect("csrf_token")
        .to_string();

    TestContext {
        app,
        pool,
        _temp: tmp,
        nginx_dir,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_id: alice_user.id,
        runner,
    }
}

#[tokio::test]
async fn test_switch_php_version_success_without_downtime() {
    let ctx = setup_test_context().await;

    // 1. Create PHP site with PHP 8.2
    let payload = CreateSiteRequest {
        domain: "switch-demo.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.2".to_string()),
        preset: Some("generic".to_string()),
        upload_max_size: Some("64M".to_string()),
        fastcgi_cache: Some(false),
        spa_fallback: None,
        force_https: None,
        docroot: None,
        custom_entry_file: None,
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&payload).unwrap()))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let site: SiteResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(site.version, Some("8.2".to_string()));

    // Verify initial vhost points to PHP 8.2 socket
    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    assert!(vhost_path.exists());
    let vhost_82 = fs::read_to_string(&vhost_path).unwrap();
    assert!(vhost_82.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.2/fpm.sock;"));

    // 2. Perform zero-downtime PHP version switch to PHP 8.3
    let switch_req_payload = SwitchPhpVersionRequest {
        version: "8.3".to_string(),
    };

    let switch_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/php/version", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&switch_req_payload).unwrap(),
        ))
        .unwrap();

    let switch_res = ctx.app.clone().oneshot(switch_req).await.unwrap();
    assert_eq!(switch_res.status(), StatusCode::OK);

    let switch_body = switch_res.into_body().collect().await.unwrap().to_bytes();
    let switch_data: SwitchPhpVersionResponse = serde_json::from_slice(&switch_body).unwrap();

    assert_eq!(switch_data.site_id, site.id);
    assert_eq!(switch_data.previous_version, "8.2");
    assert_eq!(switch_data.current_version, "8.3");
    assert_eq!(switch_data.status, "success");
    assert!(switch_data.message.contains("tanpa downtime"));

    // 3. Verify Nginx vhost now atomically points to PHP 8.3 socket
    let vhost_83 = fs::read_to_string(&vhost_path).unwrap();
    assert!(vhost_83.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.3/fpm.sock;"));
    assert!(!vhost_83.contains("unix:/run/panel/php/p_alice-8.2/fpm.sock;"));

    // 4. Verify DB record was updated to 8.3
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let db_site = site_repo.find_by_id(site.id).await.unwrap().unwrap();
    assert_eq!(db_site.version, Some("8.3".to_string()));
}

#[tokio::test]
async fn test_switch_php_version_automatic_rollback_on_health_check_failure() {
    let ctx = setup_test_context().await;

    // 1. Create PHP site with PHP 8.2
    let payload = CreateSiteRequest {
        domain: "fail-rollback.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.2".to_string()),
        preset: Some("generic".to_string()),
        upload_max_size: None,
        fastcgi_cache: None,
        spa_fallback: None,
        force_https: None,
        docroot: None,
        custom_entry_file: None,
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&payload).unwrap()))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let site: SiteResponse = serde_json::from_slice(&body).unwrap();

    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    let initial_vhost = fs::read_to_string(&vhost_path).unwrap();
    assert!(initial_vhost.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.2/fpm.sock;"));

    // 2. Setup mock failure: PHP 8.3 master service fails health check
    ctx.runner.set_response(
        "systemctl is-active --quiet panel-fpm@p_alice-8.3.service",
        1,
        b"",
        b"inactive\n",
    );

    // 3. Attempt to switch to PHP 8.3
    let switch_req_payload = SwitchPhpVersionRequest {
        version: "8.3".to_string(),
    };

    let switch_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/php/version", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&switch_req_payload).unwrap(),
        ))
        .unwrap();

    let switch_res = ctx.app.clone().oneshot(switch_req).await.unwrap();
    assert_eq!(switch_res.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let switch_body = switch_res.into_body().collect().await.unwrap().to_bytes();
    let err_json: serde_json::Value = serde_json::from_slice(&switch_body).unwrap();

    assert!(
        err_json["error"]
            .as_str()
            .unwrap()
            .contains("Health check pada master PHP 8.3 gagal")
    );
    assert_eq!(err_json["rolled_back_to"], "8.2");

    // 4. Verify vhost in sites-enabled was automatically rolled back to 8.2
    let rolled_back_vhost = fs::read_to_string(&vhost_path).unwrap();
    assert!(rolled_back_vhost.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.2/fpm.sock;"));
    assert!(!rolled_back_vhost.contains("unix:/run/panel/php/p_alice-8.3/fpm.sock;"));

    // 5. Verify database record remains 8.2
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let db_site = site_repo.find_by_id(site.id).await.unwrap().unwrap();
    assert_eq!(db_site.version, Some("8.2".to_string()));
}

#[tokio::test]
async fn test_switch_php_version_validation_errors() {
    let ctx = setup_test_context().await;

    // 1. Create a STATIC site
    let static_payload = CreateSiteRequest {
        domain: "pure-static.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("static".to_string()),
        php_version: None,
        preset: None,
        upload_max_size: None,
        fastcgi_cache: None,
        spa_fallback: None,
        force_https: None,
        docroot: None,
        custom_entry_file: None,
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&static_payload).unwrap()))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let static_site: SiteResponse = serde_json::from_slice(&body).unwrap();

    // 2. Reject switching static site
    let switch_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/php/version", static_site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&SwitchPhpVersionRequest {
                version: "8.3".to_string(),
            })
            .unwrap(),
        ))
        .unwrap();

    let res2 = ctx.app.clone().oneshot(switch_req).await.unwrap();
    assert_eq!(res2.status(), StatusCode::BAD_REQUEST);

    // 3. Create a PHP site
    let php_payload = CreateSiteRequest {
        domain: "valid-php.org".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.2".to_string()),
        preset: Some("generic".to_string()),
        upload_max_size: None,
        fastcgi_cache: None,
        spa_fallback: None,
        force_https: None,
        docroot: None,
        custom_entry_file: None,
        ..Default::default()
    };

    let req_php = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&php_payload).unwrap()))
        .unwrap();

    let res_php = ctx.app.clone().oneshot(req_php).await.unwrap();
    assert_eq!(res_php.status(), StatusCode::CREATED);
    let php_body = res_php.into_body().collect().await.unwrap().to_bytes();
    let php_site: SiteResponse = serde_json::from_slice(&php_body).unwrap();

    // 4. Reject switching to the same version (8.2 -> 8.2)
    let same_ver_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/php/version", php_site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&SwitchPhpVersionRequest {
                version: "8.2".to_string(),
            })
            .unwrap(),
        ))
        .unwrap();

    let same_res = ctx.app.clone().oneshot(same_ver_req).await.unwrap();
    assert_eq!(same_res.status(), StatusCode::BAD_REQUEST);

    // 5. Reject switching to an uninstalled version (PHP 7.4 was not added to mock paths)
    let uninstalled_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/php/version", php_site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&SwitchPhpVersionRequest {
                version: "7.4".to_string(),
            })
            .unwrap(),
        ))
        .unwrap();

    let uninstalled_res = ctx.app.clone().oneshot(uninstalled_req).await.unwrap();
    assert_eq!(uninstalled_res.status(), StatusCode::BAD_REQUEST);

    // 6. Reject invalid version string
    let invalid_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/php/version", php_site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&SwitchPhpVersionRequest {
                version: "9.9".to_string(),
            })
            .unwrap(),
        ))
        .unwrap();

    let invalid_res = ctx.app.clone().oneshot(invalid_req).await.unwrap();
    assert_eq!(invalid_res.status(), StatusCode::BAD_REQUEST);
}
