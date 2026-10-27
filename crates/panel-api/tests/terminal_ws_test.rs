#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::pty::{MockPtyBackend, PtyManagerConfig, PtySessionManager};
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_api::terminal::{
    CreateTerminalSessionRequest, TerminalCloseApiResponse, TerminalResizeRequest,
    TerminalResizeResponse, TerminalSessionListResponse, TerminalSessionResponse,
};
use panel_core::auth::hash_password;
use panel_db::models::{CreatePlan, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqlitePlanRepository, SqliteUserRepository};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    addr: String,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    bob_csrf: String,
    admin_session: String,
    #[allow(dead_code)]
    admin_csrf: String,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    _agent_shutdown_tx: tokio::sync::broadcast::Sender<()>,
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
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");
    let db_path = tmp.path().join("panel_test.db");

    // Prepare home directories
    let home_base = tmp.path().join("home");
    std::fs::create_dir_all(home_base.join("p_alice/sites")).expect("mkdir p_alice");
    std::fs::create_dir_all(home_base.join("p_bob")).expect("mkdir p_bob");

    // 1. Setup mock agent server with PTY support
    let pty_cfg = PtyManagerConfig {
        home_base: home_base.clone(),
        default_shell: "/bin/bash".to_string(),
        default_term: "xterm-256color".to_string(),
        max_sessions_per_tenant: 2,
        idle_timeout: Duration::from_secs(60),
        cgroup_base: tmp.path().join("cgroup"),
    };
    let mock_pty = Arc::new(MockPtyBackend::new());
    let pty_manager = Arc::new(PtySessionManager::new(pty_cfg, mock_pty));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };
    let agent_server = Arc::new(AgentServer::new(agent_cfg).with_pty_manager(pty_manager));
    let (agent_shutdown_tx, agent_shutdown_rx) = tokio::sync::broadcast::channel(1);

    tokio::spawn(async move {
        agent_server
            .run(agent_shutdown_rx)
            .await
            .expect("agent server run");
    });

    let start = std::time::Instant::now();
    while !sock_path.exists() && start.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 2. Setup SQLite database & migrations
    let pool = init_pool(&format!(
        "sqlite://{}",
        db_path.to_str().expect("db_path").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let user_repo = SqliteUserRepository::new(pool.clone());

    // Create a plan with a limit of 2 terminal sessions
    let test_plan = plan_repo
        .create(&CreatePlan {
            name: "Basic Plan".to_string(),
            description: None,
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(512 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(50),
            io_weight: Some(100),
            disk_limit_bytes: Some(1_000_000_000),
            inode_limit: Some(100_000),
            max_sites: Some(5),
            max_domains: Some(5),
            max_databases: Some(2),
            max_cron_jobs: Some(2),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(50 * 1024 * 1024),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create test plan");

    // Admin user
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

    // Alice user (with 2 terminal sessions limit)
    let alice_hash = hash_password("AlicePass123!").expect("hash pass");
    user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            plan_id: Some(test_plan.id),
            linux_username: Some("p_alice".to_string()),
            linux_uid: Some(10001),
        })
        .await
        .expect("create alice");

    // Bob user
    let bob_hash = hash_password("BobPass123!").expect("hash pass");
    user_repo
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

    // 3. Setup panel-api AppState & server
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

    // Bind real TCP listener for WebSocket live tests
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr").to_string();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server_app = app.clone();
    tokio::spawn(async move {
        axum::serve(listener, server_app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("serve app");
    });

    // Helper to login via app.clone().oneshot(...)
    let (admin_session, admin_csrf) = login_helper(&app, "admin", "AdminPass123!").await;
    let (alice_session, alice_csrf) = login_helper(&app, "alice", "AlicePass123!").await;
    let (bob_session, bob_csrf) = login_helper(&app, "bob", "BobPass123!").await;

    TestContext {
        app,
        addr,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        admin_session,
        admin_csrf,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        _agent_shutdown_tx: agent_shutdown_tx,
    }
}

async fn login_helper(app: &axum::Router, username: &str, pass: &str) -> (String, String) {
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": username,
                "password": pass,
            }))
            .expect("json"),
        ))
        .expect("req");

    let res = app.clone().oneshot(login_req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::OK);
    let set_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set cookie")
        .to_str()
        .expect("cookie str");
    let session = extract_cookie_value(set_cookie, SESSION_COOKIE_NAME).expect("session cookie");

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(header::COOKIE, format!("{SESSION_COOKIE_NAME}={session}"))
        .body(Body::empty())
        .expect("csrf req");

    let res = app.clone().oneshot(csrf_req).await.expect("oneshot csrf");
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let val: serde_json::Value = serde_json::from_slice(&bytes).expect("parse json");
    let csrf = val["csrf_token"].as_str().expect("csrf").to_string();

    (session, csrf)
}

#[tokio::test]
async fn test_terminal_rest_lifecycle_and_quota_enforcement() {
    let ctx = setup_test_context().await;

    // 1. Alice creates session #1 -> 201 Created
    let create_payload = serde_json::to_vec(&CreateTerminalSessionRequest {
        cwd: Some("sites".to_string()),
        cols: Some(80),
        rows: Some(24),
        tenant: None,
    })
    .expect("json");

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/terminal/sessions")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(create_payload))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);
    assert_eq!(status, StatusCode::CREATED, "Error body: {}", body_str);
    let session1: TerminalSessionResponse = serde_json::from_slice(&bytes).expect("parse");
    assert_eq!(session1.username, "p_alice");
    assert_eq!(session1.cols, 80);
    assert_eq!(session1.rows, 24);
    assert!(session1.cwd.contains("sites"));
    assert!(!session1.closed);

    // 2. Alice lists sessions -> Sees session #1
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/terminal/sessions")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");
    let res = ctx.app.clone().oneshot(list_req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let list_data: TerminalSessionListResponse = serde_json::from_slice(&bytes).expect("parse");
    assert_eq!(list_data.sessions.len(), 1);
    assert_eq!(list_data.sessions[0].session_id, session1.session_id);

    // 3. Alice resizes session #1 -> 200 OK
    let resize_payload = serde_json::to_vec(&TerminalResizeRequest {
        cols: 120,
        rows: 40,
    })
    .expect("json");
    let resize_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/terminal/sessions/{}/resize",
            session1.session_id
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(resize_payload))
        .expect("req");
    let res = ctx.app.clone().oneshot(resize_req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let resize_data: TerminalResizeResponse = serde_json::from_slice(&bytes).expect("parse");
    assert_eq!(resize_data.cols, 120);
    assert_eq!(resize_data.rows, 40);

    // 4. Bob tries to access Alice's session -> 403 Forbidden
    let bob_get_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/terminal/sessions/{}", session1.session_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .body(Body::empty())
        .expect("req");
    let res = ctx.app.clone().oneshot(bob_get_req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Bob tries to delete Alice's session -> 403 Forbidden
    let bob_del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/terminal/sessions/{}", session1.session_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::empty())
        .expect("req");
    let res = ctx.app.clone().oneshot(bob_del_req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Admin can inspect Alice's session -> 200 OK
    let admin_get_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/terminal/sessions/{}", session1.session_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .expect("req");
    let res = ctx
        .app
        .clone()
        .oneshot(admin_get_req)
        .await
        .expect("oneshot");
    assert_eq!(res.status(), StatusCode::OK);

    // 5. Alice creates session #2 -> 201 Created (reaches quota 2/2)
    let create_payload2 = serde_json::to_vec(&CreateTerminalSessionRequest {
        cwd: None,
        cols: Some(80),
        rows: Some(24),
        tenant: None,
    })
    .expect("json");
    let req2 = Request::builder()
        .method("POST")
        .uri("/api/v1/terminal/sessions")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(create_payload2))
        .expect("req");
    let res2 = ctx.app.clone().oneshot(req2).await.expect("oneshot");
    assert_eq!(res2.status(), StatusCode::CREATED);
    let bytes2 = res2.into_body().collect().await.expect("body").to_bytes();
    let session2: TerminalSessionResponse = serde_json::from_slice(&bytes2).expect("parse");

    // 6. Alice creates session #3 -> 429 Too Many Requests (Plan quota exceeded)
    let create_payload3 = serde_json::to_vec(&CreateTerminalSessionRequest {
        cwd: None,
        cols: Some(80),
        rows: Some(24),
        tenant: None,
    })
    .expect("json");
    let req3 = Request::builder()
        .method("POST")
        .uri("/api/v1/terminal/sessions")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(create_payload3))
        .expect("req");
    let res3 = ctx.app.clone().oneshot(req3).await.expect("oneshot");
    assert_eq!(res3.status(), StatusCode::TOO_MANY_REQUESTS);

    // 7. Alice closes session #1 -> 200 OK
    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/terminal/sessions/{}", session1.session_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let res_del = ctx.app.clone().oneshot(del_req).await.expect("oneshot");
    assert_eq!(res_del.status(), StatusCode::OK);
    let bytes_del = res_del
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let close_res: TerminalCloseApiResponse = serde_json::from_slice(&bytes_del).expect("parse");
    assert!(close_res.closed);

    // Clean up session 2
    let del_req2 = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/terminal/sessions/{}", session2.session_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let _ = ctx.app.clone().oneshot(del_req2).await.expect("oneshot");
}

#[tokio::test]
async fn test_terminal_websocket_streaming_and_reconnect() {
    let ctx = setup_test_context().await;
    // Browser Origin must match the listener's Host, including its assigned port.
    let origin = format!("http://{}", ctx.addr);

    // 1. Create a session for Alice via oneshot
    let create_payload = serde_json::to_vec(&CreateTerminalSessionRequest {
        cwd: Some("sites".to_string()),
        cols: Some(80),
        rows: Some(24),
        tenant: None,
    })
    .expect("json");

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/terminal/sessions")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(create_payload))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);
    assert_eq!(status, StatusCode::CREATED, "Error body: {}", body_str);
    let session: TerminalSessionResponse = serde_json::from_slice(&bytes).expect("parse");

    // 2. Unauthenticated WebSocket connection -> 401 Unauthorized
    let mut stream = TcpStream::connect(&ctx.addr).await.expect("tcp connect");
    let unauth_ws_req = format!(
        "GET /ws/terminal/{} HTTP/1.1\r\n\
         Host: {}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Origin: {origin}\r\n\r\n",
        session.session_id, ctx.addr
    );
    stream
        .write_all(unauth_ws_req.as_bytes())
        .await
        .expect("write unauth");
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).await.expect("read unauth");
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("HTTP/1.1 401 Unauthorized"),
        "Expected 401 Unauthorized, got: {resp}"
    );

    // 3. Malicious Origin Header -> 403 Forbidden (CSWSH Protection)
    let mut stream = TcpStream::connect(&ctx.addr).await.expect("tcp connect");
    let evil_origin_req = format!(
        "GET /ws/terminal/{} HTTP/1.1\r\n\
         Host: {}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Origin: https://evil-attacker.example.org\r\n\
         Cookie: {SESSION_COOKIE_NAME}={}\r\n\r\n",
        session.session_id, ctx.addr, ctx.alice_session
    );
    stream
        .write_all(evil_origin_req.as_bytes())
        .await
        .expect("write evil");
    let n = stream.read(&mut buf).await.expect("read evil");
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("HTTP/1.1 403 Forbidden"),
        "Expected 403 Forbidden for bad origin, got: {resp}"
    );

    // 4. Bob tries to access Alice's session over WebSocket -> 403 Forbidden
    let mut stream = TcpStream::connect(&ctx.addr).await.expect("tcp connect");
    let bob_ws_req = format!(
        "GET /ws/terminal/{} HTTP/1.1\r\n\
         Host: {}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Origin: {origin}\r\n\
         Cookie: {SESSION_COOKIE_NAME}={}\r\n\r\n",
        session.session_id, ctx.addr, ctx.bob_session
    );
    stream
        .write_all(bob_ws_req.as_bytes())
        .await
        .expect("write bob");
    let n = stream.read(&mut buf).await.expect("read bob");
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("HTTP/1.1 403 Forbidden"),
        "Expected 403 Forbidden for cross-tenant WS, got: {resp}"
    );

    // 5. Alice connects with valid Origin and cookie -> 101 Switching Protocols
    let mut stream = TcpStream::connect(&ctx.addr).await.expect("tcp connect");
    let valid_ws_req = format!(
        "GET /ws/terminal/{} HTTP/1.1\r\n\
         Host: {}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Origin: {origin}\r\n\
         Cookie: {SESSION_COOKIE_NAME}={}\r\n\r\n",
        session.session_id, ctx.addr, ctx.alice_session
    );
    stream
        .write_all(valid_ws_req.as_bytes())
        .await
        .expect("write valid");
    let n = stream.read(&mut buf).await.expect("read valid");
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("HTTP/1.1 101 Switching Protocols"),
        "Expected 101 Switching Protocols, got: {resp}"
    );

    // Disconnect stream 1 to simulate client refresh / network reconnect
    drop(stream);

    // 6. Alice reconnects to the same session_id -> 101 Switching Protocols & Scrollback delivered
    let mut stream2 = TcpStream::connect(&ctx.addr).await.expect("tcp connect");
    stream2
        .write_all(valid_ws_req.as_bytes())
        .await
        .expect("write reconnect");
    let n2 = stream2.read(&mut buf).await.expect("read reconnect");
    let resp2 = String::from_utf8_lossy(&buf[..n2]);
    assert!(
        resp2.starts_with("HTTP/1.1 101 Switching Protocols"),
        "Expected 101 Switching Protocols upon reconnect, got: {resp2}"
    );

    drop(stream2);
}
