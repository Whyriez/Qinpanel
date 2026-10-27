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
use panel_api::notifications::routes::ListNotificationsResponse;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateNotificationInput, CreatePlan, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{
    SqliteCertificateRepository, SqliteNotificationRepository, SqlitePlanRepository,
    SqliteSiteRepository, SqliteUserRepository,
};
use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    admin_session: String,
    admin_csrf: String,
    user_session: String,
    user_csrf: String,
    user_id: i64,
    site_id: i64,
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

    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"", b"");

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

    let (shutdown_tx, rx) = tokio::sync::broadcast::channel(1);
    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

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

    let agent_client = Arc::new(AgentClient::new(&sock_path));
    for _ in 0..50 {
        if agent_client.ping().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let db_url = format!("sqlite://{}", db_file.to_str().expect("db file to str"));
    let pool = init_pool(&db_url).await.expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let plan = plan_repo
        .create(&CreatePlan {
            name: "NotifPlan".to_string(),
            description: Some("Notification Test Plan".to_string()),
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(200),
            io_weight: Some(100),
            disk_limit_bytes: Some(10 * 1024 * 1024 * 1024),
            inode_limit: Some(100000),
            max_sites: Some(5),
            max_domains: Some(10),
            max_databases: Some(5),
            max_cron_jobs: Some(10),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(100 * 1024 * 1024),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create test plan");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let _admin = user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: hash_password("AdminPass123!").expect("hash admin pass"),
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            linux_username: Some("p_admin".to_string()),
            linux_uid: Some(10000),
            plan_id: Some(plan.id),
        })
        .await
        .expect("create admin");

    let tenant = user_repo
        .create(&CreateUser {
            username: "tenant1".to_string(),
            password_hash: hash_password("TenantPass123!").expect("hash tenant pass"),
            role: "user".to_string(),
            email: Some("tenant1@example.com".to_string()),
            linux_username: Some("p_tenant1".to_string()),
            linux_uid: Some(10002),
            plan_id: Some(plan.id),
        })
        .await
        .expect("create tenant");

    let site_repo = SqliteSiteRepository::new(pool.clone());
    let site = site_repo
        .create(&CreateSite {
            user_id: tenant.id,
            kind: "php".to_string(),
            docroot: "/home/p_tenant1/sites/notifsite.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };
    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let audit_writer = Arc::new(panel_api::audit::AuditWriter::new(pool.clone()));
    let job_runner = Arc::new(panel_api::jobs::JobRunner::with_registry(
        pool.clone(),
        panel_api::jobs::JobRegistry::new_with_agent(Arc::clone(&agent_client)),
    ));

    let state = AppState {
        agent_client: Arc::clone(&agent_client),
        operations_pool: pool.clone(),
        operations_read_pool: pool.clone(),
        pool: pool.clone(),
        environment: "testing".to_string(),
        auth_config,
        lockout_manager,
        audit_writer,
        job_runner,
        terminal_hub: Arc::new(panel_api::terminal::TerminalHub::new(Arc::clone(
            &agent_client,
        ))),
        analytics_storage: Arc::new(tokio::sync::RwLock::new(None)),
        analytics_db_path: tmp.path().join("analytics.db"),
        collector_status: Arc::new(tokio::sync::RwLock::new(Default::default())),
    };

    let app = create_router(state);

    // Login admin
    let (admin_session, admin_csrf) = login_user(&app, "admin", "AdminPass123!").await;
    // Login tenant
    let (user_session, user_csrf) = login_user(&app, "tenant1", "TenantPass123!").await;

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        user_session,
        user_csrf,
        user_id: tenant.id,
        site_id: site.id,
    }
}

async fn login_user(app: &axum::Router, username: &str, password: &str) -> (String, String) {
    let login_body = serde_json::json!({
        "username": username,
        "password": password
    });
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&login_body).unwrap()))
        .unwrap();

    let login_res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(login_res.status(), StatusCode::OK);

    let cookie_header = login_res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let session = extract_cookie_value(cookie_header, SESSION_COOKIE_NAME).unwrap();

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(header::COOKIE, format!("{SESSION_COOKIE_NAME}={session}"))
        .body(Body::empty())
        .unwrap();
    let csrf_res = app.clone().oneshot(csrf_req).await.unwrap();
    let csrf_json: serde_json::Value =
        serde_json::from_slice(&csrf_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let csrf = csrf_json["csrf_token"].as_str().unwrap().to_string();

    (session, csrf)
}

#[tokio::test]
async fn test_notifications_rest_api_lifecycle() {
    let ctx = setup_test_context().await;
    let notif_repo = SqliteNotificationRepository::new(ctx.pool.clone());

    // 1. Seed some notifications via repository
    let notif_user = notif_repo
        .create(&CreateNotificationInput {
            id: None,
            user_id: Some(ctx.user_id),
            site_id: Some(ctx.site_id),
            category: "ssl_expiring".to_string(),
            severity: "warning".to_string(),
            title: "Sertifikat SSL segera kedaluwarsa".to_string(),
            message: "Sertifikat untuk notifsite.com tersisa 7 hari".to_string(),
            metadata_json: Some(r#"{"days_left": 7}"#.to_string()),
            action_url: Some(format!("/sites/{}?tab=ssl", ctx.site_id)),
        })
        .await
        .expect("seed notif_user");

    let notif_system = notif_repo
        .create(&CreateNotificationInput {
            id: None,
            user_id: None,
            site_id: None,
            category: "system".to_string(),
            severity: "info".to_string(),
            title: "Pembaruan versi sistem v1.0".to_string(),
            message: "Pembaruan panel telah diterapkan secara otomatis".to_string(),
            metadata_json: None,
            action_url: None,
        })
        .await
        .expect("seed notif_system");

    // 2. GET /api/v1/notifications for tenant
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/notifications")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.user_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: ListNotificationsResponse =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.total, 2);
    assert_eq!(body.unread_count, 2);

    // 3. GET /api/v1/notifications/unread-count
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/notifications/unread-count")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.user_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let count_json: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(count_json["unread_count"], 2);

    // 4. PATCH /api/v1/notifications/{id}/read
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/notifications/{}/read", notif_user.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.user_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.user_csrf)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Unread count should now be 1
    assert_eq!(notif_repo.unread_count(Some(ctx.user_id)).await.unwrap(), 1);

    // 5. POST /api/v1/notifications/mark-all-read
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/notifications/mark-all-read")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.user_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.user_csrf)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(notif_repo.unread_count(Some(ctx.user_id)).await.unwrap(), 0);

    // 6. DELETE /api/v1/notifications/{id}
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/notifications/{}", notif_system.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.user_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.user_csrf)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 7. DELETE /api/v1/notifications (clear all read)
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/notifications?read_only=true")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.user_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.user_csrf)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_notification_settings_and_admin_security() {
    let ctx = setup_test_context().await;

    // 1. GET /api/v1/notifications/settings
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/notifications/settings")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let settings_json: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(settings_json["smtp_port"], 587);

    // 2. Regular user cannot update settings (403 Forbidden)
    let update_body = serde_json::json!({
        "smtp_enabled": true,
        "smtp_host": "smtp.mailgun.org",
        "smtp_port": 587,
        "from_address": "alerts@nusa.local"
    });
    let req_user = Request::builder()
        .method("PUT")
        .uri("/api/v1/notifications/settings")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.user_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.user_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
        .unwrap();

    let res_user = ctx.app.clone().oneshot(req_user).await.unwrap();
    assert_eq!(res_user.status(), StatusCode::FORBIDDEN);

    // 3. Admin successfully updates settings
    let req_admin = Request::builder()
        .method("PUT")
        .uri("/api/v1/notifications/settings")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
        .unwrap();

    let res_admin = ctx.app.clone().oneshot(req_admin).await.unwrap();
    assert_eq!(res_admin.status(), StatusCode::OK);
    let updated_json: serde_json::Value =
        serde_json::from_slice(&res_admin.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(updated_json["smtp_enabled"], true);
    assert_eq!(updated_json["smtp_host"], "smtp.mailgun.org");

    // 4. Input validation: invalid port (0) -> 400 Bad Request
    let bad_body = serde_json::json!({
        "smtp_port": 0
    });
    let bad_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/notifications/settings")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&bad_body).unwrap()))
        .unwrap();

    let bad_res = ctx.app.clone().oneshot(bad_req).await.unwrap();
    assert_eq!(bad_res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_system_alert_evaluation_and_expiring_ssl() {
    let ctx = setup_test_context().await;
    let cert_repo = SqliteCertificateRepository::new(ctx.pool.clone());

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Seed certificate expiring in 5 days (5 * 86400)
    let not_after = now + (5 * 86400);
    cert_repo
        .create(&panel_db::models::CreateCertificate {
            site_id: ctx.site_id,
            primary_domain: "expiring.domain.local".to_string(),
            san_domains_json: "[]".to_string(),
            issuer: "letsencrypt".to_string(),
            cert_path: "/etc/ssl/cert.pem".to_string(),
            key_path: "/etc/ssl/key.pem".to_string(),
            fullchain_pem: "---CERT---".to_string(),
            privkey_pem: "---KEY---".to_string(),
            not_before: now - (80 * 86400),
            not_after,
            auto_renew: true,
            ocsp_stapling: false,
            status: "active".to_string(),
        })
        .await
        .expect("create cert");

    // Trigger evaluation via POST /api/v1/notifications/evaluate
    let eval_req = Request::builder()
        .method("POST")
        .uri("/api/v1/notifications/evaluate")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let eval_res = ctx.app.clone().oneshot(eval_req).await.unwrap();
    assert_eq!(eval_res.status(), StatusCode::OK);
    let eval_json: serde_json::Value =
        serde_json::from_slice(&eval_res.into_body().collect().await.unwrap().to_bytes()).unwrap();

    assert!(eval_json["triggered_count"].as_i64().unwrap() >= 1);
    let notifs = eval_json["notifications"].as_array().unwrap();
    let ssl_notif = notifs
        .iter()
        .find(|n| n["category"] == "ssl_expiring")
        .expect("found ssl notif");
    assert_eq!(ssl_notif["severity"], "critical");
    assert!(
        ssl_notif["title"]
            .as_str()
            .unwrap()
            .contains("expiring.domain.local")
    );
}
