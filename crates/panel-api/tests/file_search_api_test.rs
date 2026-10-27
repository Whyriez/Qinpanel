#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::fs_manager::FsManager;
use panel_agent::quota::QuotaManager;
use panel_agent::system::MockSystemRunner;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::files::FileSearchApiResponse;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqliteJobRepository, SqliteUserRepository};
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    alice_session: String,
    alice_csrf: String,
    alice_home: std::path::PathBuf,
    job_repo: SqliteJobRepository,
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

async fn login_and_get_csrf(app: &axum::Router, user: &str, pass: &str) -> (String, String) {
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "username": user,
                "password": pass
            })
            .to_string(),
        ))
        .expect("build login req");

    let login_res = app.clone().oneshot(login_req).await.expect("login oneshot");
    assert_eq!(login_res.status(), StatusCode::OK);

    let cookie_hdr = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .expect("cookie str");
    let session = extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).expect("session cookie");

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session),
        )
        .body(Body::empty())
        .expect("build csrf req");

    let csrf_res = app.clone().oneshot(csrf_req).await.expect("csrf oneshot");
    assert_eq!(csrf_res.status(), StatusCode::OK);

    let csrf_body = csrf_res
        .into_body()
        .collect()
        .await
        .expect("collect csrf")
        .to_bytes();
    let csrf_val: serde_json::Value = serde_json::from_slice(&csrf_body).expect("parse csrf");
    let csrf_token = csrf_val["csrf_token"]
        .as_str()
        .expect("csrf_token str")
        .to_string();

    (session, csrf_token)
}

async fn setup_test_context() -> TestContext {
    let tmp = tempdir().expect("tempdir");
    let db_file = tmp.path().join("panel.db");

    #[cfg(unix)]
    let sock_path = tmp.path().join("agent.sock");
    #[cfg(not(unix))]
    let sock_path = tmp.path().join("agent.port");

    let home_base = tmp.path().join("home");
    fs::create_dir_all(&home_base).expect("create home base");

    let alice_home = home_base.join("p_alice");
    fs::create_dir_all(&alice_home).expect("create alice home");

    let mock_runner = Arc::new(MockSystemRunner::new());
    for (name, uid) in [("p_alice", 10001), ("p_bob", 10002)] {
        mock_runner.set_response(
            &format!("getent passwd {name}"),
            0,
            format!(
                "{name}:x:{uid}:{uid}::{}:/usr/sbin/nologin\n",
                home_base.join(name).display()
            )
            .as_bytes(),
            b"",
        );
    }
    let runner: Arc<dyn panel_agent::system::SystemRunner> = mock_runner;
    let fs_manager = Arc::new(FsManager::with_home_base(
        Arc::clone(&runner),
        home_base.clone(),
    ));
    let quota_manager = Arc::new(QuotaManager::with_home_base(Arc::clone(&runner), home_base));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(
        AgentServer::new(agent_cfg)
            .with_fs_manager(fs_manager)
            .with_quota_manager(quota_manager),
    );

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
    let alice_hash = hash_password("AlicePass123!").expect("hash pass");
    user_repo
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
    let audit_writer = Arc::new(panel_api::audit::AuditWriter::new(pool.clone()));
    let job_repo = SqliteJobRepository::new(pool.clone());
    let job_runner = Arc::new(panel_api::jobs::JobRunner::with_registry(
        pool.clone(),
        panel_api::jobs::JobRegistry::new_with_agent(Arc::clone(&agent_client)),
    ));

    let state = AppState {
        agent_client: Arc::clone(&agent_client),
        operations_pool: pool.clone(),
        operations_read_pool: pool.clone(),
        pool,
        environment: "testing".to_string(),
        auth_config,
        lockout_manager,
        audit_writer,
        job_runner: Arc::clone(&job_runner),
        terminal_hub: Arc::new(panel_api::terminal::TerminalHub::new(Arc::clone(
            &agent_client,
        ))),
        analytics_storage: Arc::new(tokio::sync::RwLock::new(None)),
        analytics_db_path: tmp.path().join("analytics.db"),
        collector_status: Arc::new(tokio::sync::RwLock::new(Default::default())),
    };
    let app = create_router(state);

    let (alice_session, alice_csrf) = login_and_get_csrf(&app, "alice", "AlicePass123!").await;

    TestContext {
        app,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        alice_session,
        alice_csrf,
        alice_home,
        job_repo,
    }
}

#[tokio::test]
async fn test_search_files_synchronous_workflow() {
    let ctx = setup_test_context().await;

    // 1. Create file structure in Alice's home
    let public_dir = ctx.alice_home.join("public");
    fs::create_dir_all(&public_dir).expect("create public");
    fs::write(public_dir.join("index.html"), "<h1>Welcome</h1>").expect("write index.html");
    fs::write(
        public_dir.join("index.php"),
        "<?php\n// Database configuration\n$secret = 'ALICE_DB_PASSWORD_XYZ';\n",
    )
    .expect("write index.php");
    fs::write(public_dir.join("style.css"), "body { margin: 0; }").expect("write style.css");

    // 2. Search for filename "index" without content search
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/search")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "public",
                "query": "index",
                "search_content": false
            })
            .to_string(),
        ))
        .expect("build req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.expect("bytes").to_bytes();
    let data: FileSearchApiResponse = serde_json::from_slice(&body).expect("parse response");
    assert_eq!(data.items.len(), 2);
    assert!(!data.limit_reached);
    assert!(!data.timed_out);

    // 3. Search for content "ALICE_DB_PASSWORD"
    let req_content = Request::builder()
        .method("POST")
        .uri("/api/v1/files/search")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "public",
                "query": "ALICE_DB_PASSWORD",
                "search_content": true,
                "case_sensitive": true
            })
            .to_string(),
        ))
        .expect("build req");

    let res_content = ctx.app.clone().oneshot(req_content).await.expect("oneshot");
    assert_eq!(res_content.status(), StatusCode::OK);

    let body_content = res_content
        .into_body()
        .collect()
        .await
        .expect("bytes")
        .to_bytes();
    let data_content: FileSearchApiResponse =
        serde_json::from_slice(&body_content).expect("parse response");
    assert_eq!(data_content.items.len(), 1);
    assert_eq!(data_content.items[0].path, "public/index.php");
    assert_eq!(data_content.items[0].matches.len(), 1);
    assert_eq!(data_content.items[0].matches[0].line_number, 3);
    assert!(
        data_content.items[0].matches[0]
            .line_text
            .contains("$secret = 'ALICE_DB_PASSWORD_XYZ';")
    );
}

#[tokio::test]
async fn test_search_files_as_background_job() {
    let ctx = setup_test_context().await;

    // Create a target file
    let docs_dir = ctx.alice_home.join("docs");
    fs::create_dir_all(&docs_dir).expect("create docs");
    fs::write(docs_dir.join("api.md"), "# API Documentation").expect("write api.md");

    // Request search as background job
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/search")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "docs",
                "query": "api",
                "as_job": true
            })
            .to_string(),
        ))
        .expect("build req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let body = res.into_body().collect().await.expect("bytes").to_bytes();
    let data: FileSearchApiResponse = serde_json::from_slice(&body).expect("parse response");
    assert!(data.job_id.is_some());
    let job_id = data.job_id.unwrap();

    // Verify job record was created in SQLite database with status "queued" and type "files.search"
    let job = ctx
        .job_repo
        .find_by_id(&job_id)
        .await
        .expect("find job")
        .expect("job exists");
    assert_eq!(job.status, "queued");
    assert_eq!(job.name, "files.search");
}

#[tokio::test]
async fn test_search_path_traversal_forbidden_and_empty_query_rejected() {
    let ctx = setup_test_context().await;

    // 1. Path traversal rejected with 403 Forbidden
    let req_traversal = Request::builder()
        .method("POST")
        .uri("/api/v1/files/search")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "../../etc",
                "query": "shadow"
            })
            .to_string(),
        ))
        .expect("build req");

    let res_traversal = ctx
        .app
        .clone()
        .oneshot(req_traversal)
        .await
        .expect("oneshot");
    assert_eq!(res_traversal.status(), StatusCode::FORBIDDEN);

    // 2. Empty query rejected with 400 Bad Request
    let req_empty = Request::builder()
        .method("POST")
        .uri("/api/v1/files/search")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "public",
                "query": "   "
            })
            .to_string(),
        ))
        .expect("build req");

    let res_empty = ctx.app.clone().oneshot(req_empty).await.expect("oneshot");
    assert_eq!(res_empty.status(), StatusCode::BAD_REQUEST);
}
