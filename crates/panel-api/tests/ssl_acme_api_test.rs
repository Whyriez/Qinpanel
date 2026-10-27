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
use panel_api::sites::{CreateStaticSiteRequest, SiteResponse};
use panel_api::ssl::{
    CertificateDetailResponse, CustomCertRequest, IssueSslRequest, SslActionResponse,
    SslStatusResponse, generate_self_signed_cert, inspect_x509_cert, renew_expiring_certificates,
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

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestContext {
    app: axum::Router,
    #[allow(dead_code)]
    pool: panel_db::pool::DbPool,
    #[allow(dead_code)]
    state: AppState,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    _test_guard: tokio::sync::MutexGuard<'static, ()>,
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
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");

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
    let app = create_router(state.clone());

    // Login as admin
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "admin",
                "password": "AdminPass123!"
            }))
            .unwrap(),
        ))
        .unwrap();

    let login_res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(login_res.status(), StatusCode::OK);

    let cookie_val = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set cookie")
        .to_str()
        .expect("cookie str");
    let admin_session =
        extract_cookie_value(cookie_val, SESSION_COOKIE_NAME).expect("admin session cookie");

    // CSRF
    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={admin_session}"),
        )
        .body(Body::empty())
        .unwrap();
    let csrf_res = app.clone().oneshot(csrf_req).await.unwrap();
    let csrf_body = csrf_res.into_body().collect().await.unwrap().to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&csrf_body).unwrap();
    let admin_csrf = csrf_json["csrf_token"].as_str().unwrap().to_string();

    TestContext {
        app,
        pool,
        state,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        _test_guard: guard,
        admin_session,
        admin_csrf,
        alice_id: alice.id,
    }
}

async fn create_test_site(ctx: &TestContext, domain: &str) -> SiteResponse {
    let site_req = CreateStaticSiteRequest {
        domain: domain.to_string(),
        user_id: Some(ctx.alice_id),
        spa_fallback: Some(false),
        force_https: Some(false),
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
        .body(Body::from(serde_json::to_string(&site_req).unwrap()))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create_test_site failed: {text}"
    );
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn test_ssl_lifecycle_and_self_signed_issuance() {
    let ctx = setup_test_context().await;
    let site = create_test_site(&ctx, "ssl-test.com").await;

    // 1. Initial check: has_ssl should be false
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/ssl", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let status_res: SslStatusResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(!status_res.has_ssl);
    assert!(status_res.certificate.is_none());

    // 2. Issue self-signed certificate directly
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/ssl/self-signed", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(status, StatusCode::OK, "Body: {text}");
    let cert_detail: CertificateDetailResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(cert_detail.primary_domain, "ssl-test.com");
    assert_eq!(cert_detail.issuer, "self_signed");
    assert_eq!(cert_detail.status, "active");
    assert!(cert_detail.days_remaining >= 88);

    let vhost_path = ctx
        ._temp
        .path()
        .join(format!("etc/nginx/sites-available/site_{}.conf", site.id));
    let conf = fs::read_to_string(&vhost_path).expect("applied SSL vhost");
    assert!(conf.contains("listen 443 ssl http2;"));
    assert!(conf.contains("listen [::]:443 ssl http2;"));
    assert!(!conf.lines().any(|line| line.trim().starts_with("http2 ")));
    assert!(conf.contains("ssl_certificate "));
    assert!(conf.contains("ssl_certificate_key "));

    // 3. Verify SSL status is now true
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/ssl", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let status_res: SslStatusResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(status_res.has_ssl);
    assert!(status_res.certificate.is_some());

    // 4. Renew self-signed certificate
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/ssl/renew", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let renewed_conf = fs::read_to_string(&vhost_path).expect("renewed SSL vhost");
    assert!(renewed_conf.contains("listen 443 ssl http2;"));
    assert!(renewed_conf.contains("listen [::]:443 ssl http2;"));
    assert!(
        !renewed_conf
            .lines()
            .any(|line| line.trim().starts_with("http2 "))
    );

    // 5. Delete / Disable SSL
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/sites/{}/ssl", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let action_res: SslActionResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(action_res.message.contains("reverted to HTTP"));
    let conf = fs::read_to_string(&vhost_path).expect("HTTP vhost after disabling SSL");
    assert!(!conf.contains("http2"));
    assert!(!conf.contains("ssl_certificate"));

    // 6. Verify SSL status after disable
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/ssl", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let status_res: SslStatusResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(!status_res.has_ssl);
}

#[tokio::test]
async fn test_custom_certificate_upload_and_validation() {
    let ctx = setup_test_context().await;
    let site = create_test_site(&ctx, "custom-cert.org").await;

    // 1. Generate valid certificate and key
    let (valid_chain, valid_key, _, _) =
        generate_self_signed_cert("custom-cert.org", &[]).expect("generate test cert");

    // 2. Upload valid custom certificate
    let custom_req = CustomCertRequest {
        fullchain_pem: valid_chain.clone(),
        privkey_pem: valid_key.clone(),
        auto_renew: Some(false),
    };
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/ssl/custom", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&custom_req).unwrap()))
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let cert_detail: CertificateDetailResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(cert_detail.issuer, "custom");
    assert_eq!(cert_detail.primary_domain, "custom-cert.org");

    // 3. Upload invalid PEM data -> should fail with 400 Bad Request
    let bad_req = CustomCertRequest {
        fullchain_pem: "NOT_A_VALID_PEM".to_string(),
        privkey_pem: valid_key,
        auto_renew: None,
    };
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/ssl/custom", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&bad_req).unwrap()))
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_acme_issue_with_fallback_self_signed() {
    let ctx = setup_test_context().await;
    let site = create_test_site(&ctx, "fallback-acme.com").await;

    // Issue ACME with fallback_self_signed = true (since no real Let's Encrypt endpoint is reachable in test, it should fallback to self-signed)
    let issue_req = IssueSslRequest {
        email: Some("admin@fallback-acme.com".to_string()),
        staging: Some(true),
        fallback_self_signed: Some(true),
        ocsp_stapling: Some(false),
    };

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/ssl/issue", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&issue_req).unwrap()))
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let cert_detail: CertificateDetailResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(cert_detail.primary_domain, "fallback-acme.com");
    assert_eq!(cert_detail.issuer, "self_signed");
    assert_eq!(cert_detail.status, "active");

    // Test with fallback_self_signed = false -> should return 400 Bad Request
    let site_no_fallback = create_test_site(&ctx, "no-fallback.com").await;
    let no_fallback_req = IssueSslRequest {
        email: Some("admin@no-fallback.com".to_string()),
        staging: Some(true),
        fallback_self_signed: Some(false),
        ocsp_stapling: Some(false),
    };
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/ssl/issue", site_no_fallback.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&no_fallback_req).unwrap()))
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_x509_parser_and_self_signed_cert_generation() {
    let primary = "example.id";
    let sans = vec!["www.example.id".to_string(), "api.example.id".to_string()];

    let (chain, key, not_before, not_after) =
        generate_self_signed_cert(primary, &sans).expect("generate cert");

    assert!(!chain.is_empty());
    assert!(!key.is_empty());
    assert!(not_after > not_before);

    let (parsed_nb, parsed_na, issuer, parsed_sans) =
        inspect_x509_cert(&chain).expect("parse cert");

    assert_eq!(parsed_nb, not_before);
    assert_eq!(parsed_na, not_after);
    assert!(issuer.contains("Qinpanel Self-Signed"));
    assert!(parsed_sans.contains(&"example.id".to_string()));
    assert!(parsed_sans.contains(&"www.example.id".to_string()));
    assert!(parsed_sans.contains(&"api.example.id".to_string()));
}

#[tokio::test]
async fn test_renewal_scheduler_expiring_certificates() {
    let ctx = setup_test_context().await;
    let site = create_test_site(&ctx, "scheduler-test.com").await;

    // Issue self-signed cert
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/ssl/self-signed", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Run scheduler check
    renew_expiring_certificates(&ctx.state).await;
}
