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
    CreateStaticSiteRequest, SiteOverviewResponse, SiteQuickActionRequest, SiteQuickActionResponse,
    SiteResponse,
};
use panel_core::auth::hash_password;
use panel_db::models::{CreateCertificate, CreateSiteDeployment, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{
    SqliteCertificateRepository, SqliteGitDeployRepository, SqliteUserRepository,
};
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
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");
    runner.add_existing_path("/run/panel/php/p_alice-8.3/fpm.sock");

    // The overview fixture creates a PHP site through the provisioning API.
    runner.add_existing_path("/usr/sbin/php-fpm8.3");
    runner.add_existing_path("/usr/bin/php8.3");
    runner.add_existing_path("/run/panel/php/p_alice-8.3/fpm.sock");
    runner.set_response("php-fpm8.3", 0, b"test is successful", b"");
    runner.set_response(
        "systemctl is-enabled php8.3-fpm.service",
        0,
        b"masked\n",
        b"",
    );

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
    let s_tx = shutdown_tx.clone();
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

    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"admin","password":"AdminPass123!"}"#,
        ))
        .unwrap();

    let resp = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let cookies = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect::<Vec<_>>();

    let mut session_token = String::new();
    let mut csrf_token = String::new();
    for c in &cookies {
        if let Some(s) = extract_cookie_value(c, SESSION_COOKIE_NAME) {
            session_token = s;
        }
        if let Some(cs) = extract_cookie_value(c, "csrf_token") {
            csrf_token = cs;
        }
    }

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: s_tx,
        admin_session: session_token,
        admin_csrf: csrf_token,
        alice_id: alice.id,
    }
}

#[tokio::test]
async fn test_site_overview_and_quick_actions() {
    let ctx = setup_test_context().await;

    // 1. Create a site for Alice
    let create_site_payload = CreateStaticSiteRequest {
        domain: "overview-demo.com".to_string(),
        user_id: Some(ctx.alice_id),
        spa_fallback: Some(false),
        force_https: Some(false),
        docroot: None,
        kind: Some("php".to_string()),
        php_version: Some("8.3".to_string()),
        preset: Some("standard".to_string()),
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&create_site_payload).unwrap(),
        ))
        .unwrap();

    let resp = ctx.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "Overview site creation response: {}",
        String::from_utf8_lossy(&body)
    );
    let site: SiteResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(site.primary_domain, "overview-demo.com");

    // Add a deployment record
    let git_repo = SqliteGitDeployRepository::new(ctx.pool.clone());
    let _ = git_repo
        .create_deployment(&CreateSiteDeployment {
            site_id: site.id,
            release_id: "rel_20261010_001".to_string(),
            commit_hash: Some("a1b2c3d".to_string()),
            commit_message: Some("Initial release".to_string()),
            commit_author: Some("Alice <alice@example.com>".to_string()),
            branch: "main".to_string(),
            trigger_type: "manual".to_string(),
            status: "success".to_string(),
            log_output: Some("Build succeeded".to_string()),
            duration_ms: Some(1250),
            created_at: 1770000000,
            finished_at: Some(1770000001),
        })
        .await
        .unwrap();

    // Add an active SSL cert record
    let cert_repo = SqliteCertificateRepository::new(ctx.pool.clone());
    let _ = cert_repo
        .create(&CreateCertificate {
            site_id: site.id,
            primary_domain: "overview-demo.com".to_string(),
            san_domains_json: "[]".to_string(),
            issuer: "letsencrypt".to_string(),
            cert_path: "/tmp/cert.pem".to_string(),
            key_path: "/tmp/key.pem".to_string(),
            fullchain_pem: "CERT".to_string(),
            privkey_pem: "KEY".to_string(),
            not_before: 1770000000,
            not_after: 1770000000 + (60 * 86400),
            auto_renew: true,
            ocsp_stapling: true,
            status: "active".to_string(),
        })
        .await
        .unwrap();

    // 2. Query Overview Endpoint: GET /api/v1/sites/{id}/overview
    let overview_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/overview", site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let resp = ctx.app.clone().oneshot(overview_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let overview: SiteOverviewResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(overview.site_id, site.id);
    assert_eq!(overview.domain, "overview-demo.com");
    assert_eq!(overview.status.webserver_status, "active");
    assert!(overview.status.healthy);
    assert!(overview.ssl.has_certificate);
    assert_eq!(overview.ssl.issuer.as_deref(), Some("letsencrypt"));
    assert_eq!(overview.runtime.kind, "php");
    assert!(
        !overview
            .quick_actions_available
            .contains(&"restart".to_string())
    );
    assert!(overview.latest_deployment.is_some());
    assert_eq!(
        overview.latest_deployment.as_ref().unwrap().commit_hash,
        "a1b2c3d"
    );

    // 3. Execute Quick Action: restart
    let restart_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/action", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&SiteQuickActionRequest {
                action: "restart".to_string(),
            })
            .unwrap(),
        ))
        .unwrap();

    let resp = ctx.app.clone().oneshot(restart_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let act_resp: SiteQuickActionResponse = serde_json::from_slice(&body).unwrap();
    assert!(act_resp.success);
    assert_eq!(act_resp.action, "restart");

    // 4. Execute Quick Action: reload
    let reload_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/action", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&SiteQuickActionRequest {
                action: "reload".to_string(),
            })
            .unwrap(),
        ))
        .unwrap();

    let resp = ctx.app.clone().oneshot(reload_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. Execute Quick Action: invalid action -> 400 Bad Request
    let bad_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/action", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&SiteQuickActionRequest {
                action: "invalid_action_xyz".to_string(),
            })
            .unwrap(),
        ))
        .unwrap();

    let resp = ctx.app.clone().oneshot(bad_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
