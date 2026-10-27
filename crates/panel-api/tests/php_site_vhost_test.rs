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
use panel_api::sites::{
    AddSiteDomainRequest, CreateSiteRequest, SiteMessageResponse, SiteResponse,
};
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
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
    setup_test_context_with_php_sockets(true).await
}

async fn setup_test_context_with_php_sockets(sockets_available: bool) -> TestContext {
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
    runner.set_response("php-fpm8.2", 0, b"test is successful", b"");
    runner.set_response("php-fpm8.4", 0, b"test is successful", b"");

    // Installed runtimes require both binary paths; healthy provisioning also needs a socket.
    for version in ["8.2", "8.3", "8.4"] {
        runner.add_existing_path(format!("/usr/sbin/php-fpm{version}"));
        runner.add_existing_path(format!("/usr/bin/php{version}"));
        runner.set_response(
            &format!("systemctl is-enabled php{version}-fpm.service"),
            0,
            b"masked\n",
            b"",
        );
        if sockets_available {
            runner.add_existing_path(format!("/run/panel/php/p_alice-{version}/fpm.sock"));
        }
    }

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
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "username": "admin",
                "password": "AdminPass123!"
            })
            .to_string(),
        ))
        .expect("build login req");

    let login_res = app.clone().oneshot(login_req).await.expect("login oneshot");
    assert_eq!(login_res.status(), StatusCode::OK);

    let cookie_val = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set cookie")
        .to_str()
        .expect("cookie str");
    let admin_session =
        extract_cookie_value(cookie_val, SESSION_COOKIE_NAME).expect("session cookie");

    // Fetch CSRF
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
    let body_bytes = csrf_res
        .into_body()
        .collect()
        .await
        .expect("csrf body")
        .to_bytes();
    let csrf_json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("parse csrf json");
    let admin_csrf = csrf_json["csrf_token"].as_str().expect("token").to_string();

    TestContext {
        app,
        pool,
        _temp: tmp,
        nginx_dir,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_id: alice_user.id,
    }
}

async fn read_created_site(response: axum::response::Response) -> SiteResponse {
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "PHP site creation response: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).expect("parse created PHP site")
}

#[tokio::test]
async fn test_create_php_site_generic_preset_and_vhost() {
    let ctx = setup_test_context().await;

    let payload = CreateSiteRequest {
        domain: "app-generic.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.3".to_string()),
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
    let site = read_created_site(res).await;
    assert_eq!(site.kind, "php");
    assert_eq!(site.primary_domain, "app-generic.com");

    // Verify vhost file written by NginxManager
    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    assert!(vhost_path.exists());
    let vhost_content = fs::read_to_string(vhost_path).unwrap();

    assert!(vhost_content.contains("server_name app-generic.com;"));
    assert!(vhost_content.contains("client_max_body_size 64M;"));
    assert!(vhost_content.contains("try_files $uri $uri/ /index.php?$args;"));
    assert!(vhost_content.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.3/fpm.sock;"));
    assert!(vhost_content.contains("include /etc/panel/nginx/snippets/security-headers.conf;"));
    assert!(vhost_content.contains("include /etc/panel/nginx/snippets/deny-sensitive.conf;"));
    assert!(vhost_content.contains(r"composer\.(?:json|lock)"));
}

#[tokio::test]
async fn test_create_php_site_wordpress_preset_security_rules() {
    let ctx = setup_test_context().await;

    let payload = CreateSiteRequest {
        domain: "wp-blog.net".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.2".to_string()),
        preset: Some("wordpress".to_string()),
        upload_max_size: Some("128M".to_string()),
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
    let site = read_created_site(res).await;
    assert_eq!(site.kind, "php");

    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    let vhost_content = fs::read_to_string(vhost_path).unwrap();

    // Verify WordPress specific protections
    assert!(vhost_content.contains("client_max_body_size 128M;"));
    assert!(vhost_content.contains("location ~* ^/wp-content/uploads/.*\\.php$ {"));
    assert!(vhost_content.contains("location ~* ^/wp-includes/.*\\.php$ {"));
    assert!(vhost_content.contains("location = /wp-config.php {"));
    assert!(vhost_content.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.2/fpm.sock;"));
}

#[tokio::test]
async fn test_create_php_site_laravel_preset_routing() {
    let ctx = setup_test_context().await;

    let payload = CreateSiteRequest {
        domain: "laravel-api.io".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.3".to_string()),
        preset: Some("laravel".to_string()),
        upload_max_size: Some("32M".to_string()),
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
    let site = read_created_site(res).await;

    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    let vhost_content = fs::read_to_string(vhost_path).unwrap();

    // Verify Laravel try_files query string and storage protection
    assert!(vhost_content.contains("try_files $uri $uri/ /index.php?$query_string;"));
    assert!(vhost_content.contains("location ~* ^/storage/logs/ {"));
}

#[tokio::test]
async fn test_create_php_site_custom_front_controller() {
    let ctx = setup_test_context().await;

    let payload = CreateSiteRequest {
        domain: "custom-site.org".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.4".to_string()),
        preset: Some("custom".to_string()),
        custom_entry_file: Some("main.php".to_string()),
        upload_max_size: None,
        fastcgi_cache: None,
        spa_fallback: None,
        force_https: None,
        docroot: None,
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
    let site = read_created_site(res).await;

    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    let vhost_content = fs::read_to_string(vhost_path).unwrap();

    assert!(vhost_content.contains("try_files $uri $uri/ /main.php?$args;"));
    assert!(vhost_content.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.4/fpm.sock;"));
}

#[tokio::test]
async fn test_create_php_site_with_fastcgi_cache() {
    let ctx = setup_test_context().await;

    let payload = CreateSiteRequest {
        domain: "cached-php.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.3".to_string()),
        preset: Some("generic".to_string()),
        upload_max_size: None,
        fastcgi_cache: Some(true),
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
    let site = read_created_site(res).await;

    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    let vhost_content = fs::read_to_string(vhost_path).unwrap();

    assert!(vhost_content.contains("set $skip_cache 0;"));
    assert!(vhost_content.contains("if ($request_method = POST) {"));
    assert!(vhost_content.contains("fastcgi_cache panel_php_cache;"));
    assert!(vhost_content.contains("fastcgi_cache_bypass $skip_cache;"));
    assert!(vhost_content.contains("fastcgi_no_cache $skip_cache;"));
    assert!(vhost_content.contains("add_header X-FastCGI-Cache $upstream_cache_status;"));
}

#[tokio::test]
async fn test_php_site_domain_redirects_and_recompile() {
    let ctx = setup_test_context().await;

    // 1. Create PHP site
    let payload = CreateSiteRequest {
        domain: "main-php.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.3".to_string()),
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
    let site = read_created_site(res).await;

    // 2. Add an alias domain with redirect to primary
    let add_domain_req = AddSiteDomainRequest {
        domain: "alias-php.com".to_string(),
        redirect_mode: Some("to_primary".to_string()),
        redirect_target: None,
        is_primary: Some(false),
    };

    let req2 = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/domains", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&add_domain_req).unwrap()))
        .unwrap();

    let res2 = ctx.app.clone().oneshot(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::CREATED);

    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    let vhost_content = fs::read_to_string(vhost_path).unwrap();

    // Should contain redirect block for alias-php.com
    assert!(vhost_content.contains("server_name alias-php.com;"));
    assert!(vhost_content.contains("return 301 $scheme://main-php.com$request_uri;"));
    // Primary PHP pass should still be intact
    assert!(vhost_content.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.3/fpm.sock;"));
}

#[tokio::test]
async fn test_delete_php_site_removes_vhost() {
    let ctx = setup_test_context().await;

    let payload = CreateSiteRequest {
        domain: "to-delete.org".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("php".to_string()),
        php_version: Some("8.3".to_string()),
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
    let site = read_created_site(res).await;

    let vhost_path = ctx
        .nginx_dir
        .join("sites-enabled")
        .join(format!("site_{}.conf", site.id));
    assert!(vhost_path.exists());

    // Delete site
    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/sites/{}", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let del_res = ctx.app.clone().oneshot(del_req).await.unwrap();
    assert_eq!(del_res.status(), StatusCode::OK);

    let del_body = del_res.into_body().collect().await.unwrap().to_bytes();
    let msg: SiteMessageResponse = serde_json::from_slice(&del_body).unwrap();
    assert_eq!(msg.status, "deleted");

    assert!(!vhost_path.exists());
}

#[tokio::test]
async fn test_php_site_rejects_missing_runtime_or_socket_without_leaving_records() {
    for (version, sockets_available, expected_status, expected_error) in [
        (
            "8.1",
            true,
            StatusCode::BAD_REQUEST,
            "Runtime PHP belum terpasang. Buka Runtime Manager terlebih dahulu.",
        ),
        (
            "8.3",
            false,
            StatusCode::UNPROCESSABLE_ENTITY,
            "Health check PHP-FPM gagal; pembuatan situs dibatalkan.",
        ),
    ] {
        let ctx = setup_test_context_with_php_sockets(sockets_available).await;
        let payload = CreateSiteRequest {
            domain: "rejected-php.test".to_string(),
            user_id: Some(ctx.alice_id),
            kind: Some("php".to_string()),
            php_version: Some(version.to_string()),
            preset: Some("generic".to_string()),
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
        let response = ctx.app.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            status,
            expected_status,
            "Rejected PHP site response: {}",
            String::from_utf8_lossy(&body)
        );
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(error["error"].as_str().unwrap().starts_with(expected_error));

        let counts: (i64, i64) =
            sqlx::query_as("SELECT (SELECT COUNT(*) FROM sites), (SELECT COUNT(*) FROM domains)")
                .fetch_one(&ctx.pool)
                .await
                .unwrap();
        assert_eq!(counts, (0, 0), "Rejected provisioning left site records");
        assert!(!ctx.nginx_dir.join("sites-enabled/site_1.conf").exists());
    }
}
