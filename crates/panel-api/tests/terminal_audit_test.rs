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
    CreateTerminalSessionRequest, TerminalAuditListResponse, TerminalCloseApiResponse,
    TerminalSessionResponse,
};
use panel_core::auth::hash_password;
use panel_db::models::{CreatePlan, CreateUser};
use panel_db::pool::{DbPool, init_pool, run_migrations};
use panel_db::repositories::{SqlitePlanRepository, SqliteUserRepository};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    #[allow(dead_code)]
    addr: String,
    pool: DbPool,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    #[allow(dead_code)]
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

    run_migrations(&pool).await.expect("migrations");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let plan_repo = SqlitePlanRepository::new(pool.clone());

    // Plan with 2 terminal sessions
    let test_plan = plan_repo
        .create(&CreatePlan {
            name: "Audit Plan".to_string(),
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

    // Alice user
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
            plan_id: Some(test_plan.id),
            linux_username: Some("p_bob".to_string()),
            linux_uid: Some(10002),
        })
        .await
        .expect("create bob");

    // 3. Setup AppState and Router
    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };
    let lockout_mgr = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(sock_path));

    let state = AppState::new(
        agent_client,
        pool.clone(),
        "development".to_string(),
        auth_config,
        lockout_mgr,
    );

    let app = create_router(state);

    // Bind real TCP listener for live HTTP & WebSocket test calls
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind tcp");
    let addr = listener.local_addr().expect("local addr").to_string();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
    let app_clone = app.clone();
    tokio::spawn(async move {
        axum::serve(listener, app_clone)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("serve");
    });

    // Login Alice
    let (alice_session, alice_csrf) = login_helper(&addr, "alice", "AlicePass123!").await;
    // Login Bob
    let (bob_session, bob_csrf) = login_helper(&addr, "bob", "BobPass123!").await;
    // Login Admin
    let (admin_session, admin_csrf) = login_helper(&addr, "admin", "AdminPass123!").await;

    TestContext {
        app,
        addr,
        pool,
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

async fn login_helper(addr: &str, username: &str, password: &str) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.expect("connect");

    let body = serde_json::to_string(&serde_json::json!({
        "username": username,
        "password": password
    }))
    .expect("json");

    let req = format!(
        "POST /api/v1/auth/login HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n\
         {}",
        addr,
        body.len(),
        body
    );

    stream.write_all(req.as_bytes()).await.expect("write");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).await.expect("read");

    let mut session_id = String::new();
    let mut csrf_token = String::new();

    for line in resp.lines() {
        if line.to_lowercase().starts_with("set-cookie:")
            && let Some(s) = extract_cookie_value(&line[11..], SESSION_COOKIE_NAME)
        {
            session_id = s;
        }
    }

    if let Some(idx) = resp.find("\r\n\r\n") {
        let json_part = &resp[idx + 4..];
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_part)
            && let Some(csrf) = val.get("csrf_token").and_then(|v| v.as_str())
        {
            csrf_token = csrf.to_string();
        }
    }

    (session_id, csrf_token)
}

#[tokio::test]
async fn test_terminal_audit_start_and_end_records() {
    let ctx = setup_test_context().await;

    // 1. Alice initiates terminal session with specific client IP header
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
        .header("x-forwarded-for", "198.51.100.77")
        .body(Body::from(create_payload))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let session: TerminalSessionResponse = serde_json::from_slice(&bytes).expect("parse");

    // Allow background audit log writer task to persist
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify session_start audit log was written in SQLite
    let audit_rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT action, target_resource, details_json, ip_address FROM audit_logs WHERE action = 'terminal.session_start' AND target_resource = ?"
    )
    .bind(&session.session_id)
    .fetch_all(&ctx.pool)
    .await
    .expect("fetch audit");

    assert_eq!(audit_rows.len(), 1);
    let (action, target, details_json, ip_addr) = &audit_rows[0];
    assert_eq!(action, "terminal.session_start");
    assert_eq!(target, &session.session_id);
    assert_eq!(ip_addr.as_deref(), Some("198.51.100.77"));

    let details: serde_json::Value =
        serde_json::from_str(details_json.as_deref().expect("details")).expect("parse details");
    assert_eq!(details["tenant"], "p_alice");
    assert_eq!(details["keystroke_recording"], false);
    assert!(details["folder"].as_str().unwrap().contains("sites"));

    // Wait a brief moment to produce non-zero duration
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Alice closes the terminal session with X-Forwarded-For
    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/terminal/sessions/{}", session.session_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header("x-forwarded-for", "198.51.100.77")
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

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify session_end audit log was written in SQLite
    let end_rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT action, target_resource, details_json, ip_address FROM audit_logs WHERE action = 'terminal.session_end' AND target_resource = ?"
    )
    .bind(&session.session_id)
    .fetch_all(&ctx.pool)
    .await
    .expect("fetch audit");

    assert_eq!(end_rows.len(), 1);
    let (end_action, end_target, end_details_json, end_ip) = &end_rows[0];
    assert_eq!(end_action, "terminal.session_end");
    assert_eq!(end_target, &session.session_id);
    assert_eq!(end_ip.as_deref(), Some("198.51.100.77"));

    let end_details: serde_json::Value =
        serde_json::from_str(end_details_json.as_deref().expect("details")).expect("parse details");
    assert_eq!(end_details["tenant"], "p_alice");
    assert_eq!(end_details["reason"], "manual_close");
    assert!(end_details.get("duration_secs").is_some());
    assert!(end_details["folder"].as_str().unwrap().contains("sites"));

    // 3. Query GET /api/v1/terminal/audit as Alice
    let audit_req = Request::builder()
        .method("GET")
        .uri("/api/v1/terminal/audit")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let audit_res = ctx.app.clone().oneshot(audit_req).await.expect("oneshot");
    assert_eq!(audit_res.status(), StatusCode::OK);
    let audit_bytes = audit_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let audit_list: TerminalAuditListResponse =
        serde_json::from_slice(&audit_bytes).expect("parse");

    assert!(audit_list.total >= 2);
    let start_item = audit_list
        .items
        .iter()
        .find(|i| i.action == "terminal.session_start")
        .expect("found start item");
    assert_eq!(start_item.actor_username, "alice");
    assert_eq!(start_item.ip_address.as_deref(), Some("198.51.100.77"));
    assert!(!start_item.keystroke_recording);

    let end_item = audit_list
        .items
        .iter()
        .find(|i| i.action == "terminal.session_end")
        .expect("found end item");
    assert_eq!(end_item.actor_username, "alice");
    assert!(end_item.duration_secs.is_some());
}

#[tokio::test]
async fn test_terminal_audit_no_keystroke_leakage_and_rbac() {
    let ctx = setup_test_context().await;

    // 1. Alice creates session
    let create_payload = serde_json::to_vec(&CreateTerminalSessionRequest {
        cwd: None,
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
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let session: TerminalSessionResponse = serde_json::from_slice(&bytes).expect("parse");

    // Close session
    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/terminal/sessions/{}", session.session_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");
    let _ = ctx.app.clone().oneshot(del_req).await.expect("oneshot");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // 2. Verify all audit logs in DB strictly do not contain raw keystrokes
    let all_logs: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT action, details_json FROM audit_logs")
            .fetch_all(&ctx.pool)
            .await
            .expect("fetch all");

    for (act, details) in all_logs {
        if act.starts_with("terminal.") {
            let d = details.unwrap_or_default();
            assert!(!d.contains("keystrokes"));
            assert!(!d.contains("input_bytes"));
            assert!(!d.contains("stdin"));
        }
    }

    // 3. RBAC isolation test: Bob queries terminal audit -> sees 0 logs
    let bob_req = Request::builder()
        .method("GET")
        .uri("/api/v1/terminal/audit")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .body(Body::empty())
        .expect("req");

    let bob_res = ctx.app.clone().oneshot(bob_req).await.expect("oneshot");
    assert_eq!(bob_res.status(), StatusCode::OK);
    let bob_bytes = bob_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let bob_list: TerminalAuditListResponse = serde_json::from_slice(&bob_bytes).expect("parse");
    assert_eq!(bob_list.total, 0);
    assert!(bob_list.items.is_empty());

    // 4. Admin queries terminal audit -> sees Alice's logs
    let admin_req = Request::builder()
        .method("GET")
        .uri("/api/v1/terminal/audit")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .expect("req");

    let admin_res = ctx.app.clone().oneshot(admin_req).await.expect("oneshot");
    assert_eq!(admin_res.status(), StatusCode::OK);
    let admin_bytes = admin_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let admin_list: TerminalAuditListResponse =
        serde_json::from_slice(&admin_bytes).expect("parse");
    assert!(admin_list.total >= 2);
}
