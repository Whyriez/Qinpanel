#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
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
use panel_api::logs::{SiteLogsResponse, validate_ws_origin};
use panel_api::routes::AppState;
use panel_api::sites::{CreateStaticSiteRequest, SiteResponse};
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
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
    state: AppState,
    runner: Arc<MockSystemRunner>,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    _test_guard: tokio::sync::MutexGuard<'static, ()>,
    admin_session: String,
    admin_csrf: String,
    alice_session: String,
    #[allow(dead_code)]
    alice_csrf: String,
    bob_session: String,
    #[allow(dead_code)]
    bob_csrf: String,
    alice_id: i64,
    #[allow(dead_code)]
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

    let bob_hash = hash_password("BobPass123!").expect("hash bob pass");
    let bob = user_repo
        .create(&CreateUser {
            username: "bob".to_string(),
            password_hash: bob_hash,
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            plan_id: None,
            linux_username: Some("p_bob".to_string()),
            linux_uid: Some(10002),
        })
        .await
        .expect("create bob");

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
    let login_admin = Request::builder()
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
    let res = app.clone().oneshot(login_admin).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie_hdr = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let admin_session = extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).unwrap();

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={admin_session}"),
        )
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(csrf_req).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let csrf_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let admin_csrf = csrf_res["csrf_token"].as_str().unwrap().to_string();

    // Login as alice
    let login_alice = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "alice",
                "password": "AlicePass123!"
            }))
            .unwrap(),
        ))
        .unwrap();
    let res = app.clone().oneshot(login_alice).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie_hdr = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let alice_session = extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).unwrap();

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={alice_session}"),
        )
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(csrf_req).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let csrf_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let alice_csrf = csrf_res["csrf_token"].as_str().unwrap().to_string();

    // Login as bob
    let login_bob = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "bob",
                "password": "BobPass123!"
            }))
            .unwrap(),
        ))
        .unwrap();
    let res = app.clone().oneshot(login_bob).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie_hdr = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let bob_session = extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).unwrap();

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={bob_session}"),
        )
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(csrf_req).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let csrf_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let bob_csrf = csrf_res["csrf_token"].as_str().unwrap().to_string();

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
        bob_session,
        bob_csrf,
        alice_id: alice.id,
        bob_id: bob.id,
    }
}

async fn create_test_site(ctx: &TestContext, domain: &str, user_id: i64) -> SiteResponse {
    let site_req = CreateStaticSiteRequest {
        domain: domain.to_string(),
        user_id: Some(user_id),
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
async fn test_get_site_logs_tail_and_query_filter() {
    let ctx = setup_test_context().await;
    let site = create_test_site(&ctx, "logsite.com", ctx.alice_id).await;

    // 1. Initially when no log file exists, endpoint returns empty lines
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/logs?type=access", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let log_res: SiteLogsResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(log_res.site_id, site.id);
    assert_eq!(log_res.primary_domain, "logsite.com");
    assert_eq!(log_res.log_type, "access");
    assert!(log_res.lines.is_empty());
    assert_eq!(log_res.total_matched, 0);

    // 2. Populate mock access log file
    let log_path = PathBuf::from("/var/log/panel/p_alice/logsite.com/access.log");
    let access_lines = "\
{\"time_local\":\"2026-10-03T12:00:00Z\",\"remote_addr\":\"1.1.1.1\",\"request_uri\":\"/home\",\"status\":200}\n\
{\"time_local\":\"2026-10-03T12:01:00Z\",\"remote_addr\":\"1.1.1.2\",\"request_uri\":\"/missing\",\"status\":404}\n\
{\"time_local\":\"2026-10-03T12:02:00Z\",\"remote_addr\":\"1.1.1.3\",\"request_uri\":\"/api/data\",\"status\":200}\n\
{\"time_local\":\"2026-10-03T12:03:00Z\",\"remote_addr\":\"1.1.1.4\",\"request_uri\":\"/forbidden\",\"status\":403}\n\
{\"time_local\":\"2026-10-03T12:04:00Z\",\"remote_addr\":\"1.1.1.5\",\"request_uri\":\"/about\",\"status\":200}\n";
    ctx.runner
        .set_file_content(&log_path, access_lines.as_bytes());

    // 3. Tail last 2 lines
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/logs?type=access&lines=2",
            site.id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let log_res: SiteLogsResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(log_res.lines.len(), 2);
    assert_eq!(log_res.total_matched, 5);
    assert!(log_res.lines[0].contains("/forbidden"));
    assert!(log_res.lines[1].contains("/about"));

    // 4. Query filter (search for 404)
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/logs?type=access&q=404", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let log_res: SiteLogsResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(log_res.lines.len(), 1);
    assert_eq!(log_res.total_matched, 1);
    assert!(log_res.lines[0].contains("/missing"));

    // 5. Populate error log file
    let error_log_path = PathBuf::from("/var/log/panel/p_alice/logsite.com/error.log");
    let error_lines = "\
2026/10/03 12:01:00 [error] 1234#0: *1 open() \"/var/www/missing\" failed (2: No such file or directory)\n\
2026/10/03 12:03:00 [warn] 1234#0: *2 directory index of \"/var/www/\" is forbidden\n";
    ctx.runner
        .set_file_content(&error_log_path, error_lines.as_bytes());

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/logs?type=error", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let log_res: SiteLogsResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(log_res.log_type, "error");
    assert_eq!(log_res.lines.len(), 2);
    assert!(log_res.lines[0].contains("No such file"));

    // 6. Invalid log type rejected with 400 Bad Request
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/logs?type=unknown", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_site_logs_tenant_isolation() {
    let ctx = setup_test_context().await;
    let alice_site = create_test_site(&ctx, "alice-log.com", ctx.alice_id).await;

    // 1. Bob attempts to view Alice's site logs -> 403 Forbidden!
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/logs", alice_site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 2. Alice views her own site logs -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/logs", alice_site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. Admin views Alice's site logs -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/logs", alice_site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_websocket_origin_security_and_auth() {
    // 1. Unit test Origin check function
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://localhost:3000"),
    );
    headers.insert(header::HOST, HeaderValue::from_static("localhost:3000"));
    assert!(validate_ws_origin(&headers, false));

    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://127.0.0.1:8443"),
    );
    headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8443"));
    assert!(validate_ws_origin(&headers, true));

    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://evil-attacker.com"),
    );
    headers.insert(header::HOST, HeaderValue::from_static("panel.example.com"));
    assert!(
        !validate_ws_origin(&headers, true),
        "Cross-site Origin MUST be rejected"
    );

    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://panel.example.com:8443"),
    );
    headers.insert(
        header::HOST,
        HeaderValue::from_static("panel.example.com:8443"),
    );
    assert!(
        validate_ws_origin(&headers, true),
        "Matching panel host Origin is allowed"
    );

    // 2. Integration test WS endpoint handshake with missing session -> 401 Unauthorized
    let ctx = setup_test_context().await;
    let site = create_test_site(&ctx, "ws-sec.com", ctx.alice_id).await;

    // Populate mock log so live tail has backlog
    let log_path = PathBuf::from("/var/log/panel/p_alice/ws-sec.com/access.log");
    ctx.runner.set_file_content(
        &log_path,
        b"{\"time_local\":\"2026-10-03T12:00:00Z\",\"status\":200,\"request_uri\":\"/initial\"}\n",
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_server_tx, shutdown_server_rx) = tokio::sync::oneshot::channel::<()>();

    let app = ctx.app.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_server_rx.await;
            })
            .await;
    });

    // 2. Unauthenticated request without session cookie -> 401 Unauthorized
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let no_auth_req = format!(
        "GET /ws/logs/{} HTTP/1.1\r\n\
Host: {}\r\n\
Connection: Upgrade\r\n\
Upgrade: websocket\r\n\
Sec-WebSocket-Version: 13\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Origin: http://{addr}\r\n\r\n",
        site.id, addr
    );
    tokio::io::AsyncWriteExt::write_all(&mut stream, no_auth_req.as_bytes())
        .await
        .unwrap();
    let mut buf = [0u8; 2048];
    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
        .await
        .unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("HTTP/1.1 401 Unauthorized"),
        "Expected 401 Unauthorized, got: {resp}"
    );

    // 3. WS handshake with malicious Origin -> 403 Forbidden
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let evil_origin_req = format!(
        "GET /ws/logs/{} HTTP/1.1\r\n\
Host: {}\r\n\
Connection: Upgrade\r\n\
Upgrade: websocket\r\n\
Sec-WebSocket-Version: 13\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Origin: https://evil-attacker.com\r\n\
Cookie: {SESSION_COOKIE_NAME}={}\r\n\r\n",
        site.id, addr, ctx.alice_session
    );
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_origin_req.as_bytes())
        .await
        .unwrap();
    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
        .await
        .unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("HTTP/1.1 403 Forbidden"),
        "Expected 403 Forbidden for malicious origin, got: {resp}"
    );

    // 4. Bob tries to connect to Alice's site live tail -> 403 Forbidden
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let bob_req = format!(
        "GET /ws/logs/{} HTTP/1.1\r\n\
Host: {}\r\n\
Connection: Upgrade\r\n\
Upgrade: websocket\r\n\
Sec-WebSocket-Version: 13\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Origin: http://{addr}\r\n\
Cookie: {SESSION_COOKIE_NAME}={}\r\n\r\n",
        site.id, addr, ctx.bob_session
    );
    tokio::io::AsyncWriteExt::write_all(&mut stream, bob_req.as_bytes())
        .await
        .unwrap();
    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
        .await
        .unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("HTTP/1.1 403 Forbidden"),
        "Expected 403 Forbidden for cross-tenant access, got: {resp}"
    );

    // 5. Alice connects with valid Origin and session -> 101 Switching Protocols
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let valid_req = format!(
        "GET /ws/logs/{} HTTP/1.1\r\n\
Host: {}\r\n\
Connection: Upgrade\r\n\
Upgrade: websocket\r\n\
Sec-WebSocket-Version: 13\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Origin: http://{addr}\r\n\
Cookie: {SESSION_COOKIE_NAME}={}\r\n\r\n",
        site.id, addr, ctx.alice_session
    );
    tokio::io::AsyncWriteExt::write_all(&mut stream, valid_req.as_bytes())
        .await
        .unwrap();
    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
        .await
        .unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("HTTP/1.1 101 Switching Protocols"),
        "Expected 101 Switching Protocols, got: {resp}"
    );

    let _ = shutdown_server_tx.send(());
}
