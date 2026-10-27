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
use panel_api::files::ArchiveActionResponse;
use panel_api::routes::AppState;
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
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    alice_session: String,
    alice_csrf: String,
    alice_home: std::path::PathBuf,
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
        .header(header::COOKIE, format!("{SESSION_COOKIE_NAME}={session}"))
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
        job_runner,
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
    }
}

#[tokio::test]
async fn test_archive_compress_and_extract_zip_api() {
    let ctx = setup_test_context().await;

    // 1. Prepare files in alice home
    let src_dir = ctx.alice_home.join("mysite");
    fs::create_dir_all(&src_dir).expect("create mysite");
    fs::write(src_dir.join("index.html"), "<h1>Qinpanel Site</h1>").expect("write index");
    fs::write(src_dir.join("style.css"), "h1 { color: red; }").expect("write css");

    // 2. Compress via API POST /api/v1/files/archive/compress
    let compress_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/archive/compress")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "source_paths": ["mysite"],
                "destination": "backup.zip",
                "format": "zip"
            })
            .to_string(),
        ))
        .expect("compress req");

    let compress_res = ctx
        .app
        .clone()
        .oneshot(compress_req)
        .await
        .expect("compress oneshot");
    assert_eq!(compress_res.status(), StatusCode::OK);

    let compress_body = compress_res
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let compress_json: ArchiveActionResponse =
        serde_json::from_slice(&compress_body).expect("parse json");
    assert!(compress_json.success);
    assert!(compress_json.result.is_some());

    // Verify backup.zip file exists in alice home
    assert!(ctx.alice_home.join("backup.zip").exists());

    // 3. Extract via API POST /api/v1/files/archive/extract
    let extract_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/archive/extract")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "archive_path": "backup.zip",
                "destination_dir": "restored_site"
            })
            .to_string(),
        ))
        .expect("extract req");

    let extract_res = ctx
        .app
        .clone()
        .oneshot(extract_req)
        .await
        .expect("extract oneshot");
    assert_eq!(extract_res.status(), StatusCode::OK);

    let extract_body = extract_res
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let extract_json: ArchiveActionResponse =
        serde_json::from_slice(&extract_body).expect("parse json");
    assert!(extract_json.success);

    // Verify extracted content
    let extracted_index = ctx.alice_home.join("restored_site/mysite/index.html");
    assert!(extracted_index.exists());
    assert_eq!(
        fs::read_to_string(extracted_index).expect("read"),
        "<h1>Qinpanel Site</h1>"
    );
}

#[tokio::test]
async fn test_archive_compress_and_extract_tar_gz_api() {
    let ctx = setup_test_context().await;

    // 1. Prepare files
    let src_dir = ctx.alice_home.join("app_data");
    fs::create_dir_all(&src_dir).expect("create app_data");
    fs::write(src_dir.join("config.env"), "PORT=8080").expect("write env");

    // 2. Compress TAR.GZ
    let compress_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/archive/compress")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "source_paths": ["app_data"],
                "destination": "bundle.tar.gz"
            })
            .to_string(),
        ))
        .expect("compress req");

    let compress_res = ctx
        .app
        .clone()
        .oneshot(compress_req)
        .await
        .expect("compress oneshot");
    assert_eq!(compress_res.status(), StatusCode::OK);

    assert!(ctx.alice_home.join("bundle.tar.gz").exists());

    // 3. Extract TAR.GZ
    let extract_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/archive/extract")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "archive_path": "bundle.tar.gz",
                "destination_dir": "unpacked_data"
            })
            .to_string(),
        ))
        .expect("extract req");

    let extract_res = ctx
        .app
        .clone()
        .oneshot(extract_req)
        .await
        .expect("extract oneshot");
    assert_eq!(extract_res.status(), StatusCode::OK);

    let extracted_env = ctx.alice_home.join("unpacked_data/app_data/config.env");
    assert!(extracted_env.exists());
    assert_eq!(
        fs::read_to_string(extracted_env).expect("read"),
        "PORT=8080"
    );
}

#[tokio::test]
async fn test_archive_background_job_enqueued_api() {
    let ctx = setup_test_context().await;

    let src_dir = ctx.alice_home.join("heavy_dir");
    fs::create_dir_all(&src_dir).expect("create heavy_dir");
    fs::write(src_dir.join("file.bin"), vec![0u8; 1024]).expect("write bin");

    // Compress with background: true -> 202 ACCEPTED
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/archive/compress")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "source_paths": ["heavy_dir"],
                "destination": "heavy.zip",
                "background": true
            })
            .to_string(),
        ))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let body = res.into_body().collect().await.expect("collect").to_bytes();
    let json: ArchiveActionResponse = serde_json::from_slice(&body).expect("parse json");
    assert!(json.success);
    assert!(json.job_id.is_some());
}

#[tokio::test]
async fn test_archive_extract_zip_slip_rejected_api() {
    let ctx = setup_test_context().await;

    // Build malicious archive
    let evil_path = ctx.alice_home.join("evil.zip");
    {
        use std::io::Write;
        let file = fs::File::create(&evil_path).expect("create evil");
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer
            .start_file("../../hacked.txt", options)
            .expect("start file");
        zip_writer.write_all(b"malicious").expect("write");
        zip_writer.finish().expect("finish");
    }

    // Try to extract via API -> 400 Bad Request
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/archive/extract")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "archive_path": "evil.zip",
                "destination_dir": "target"
            })
            .to_string(),
        ))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Verify hacked.txt never created outside jail
    assert!(!ctx.alice_home.parent().unwrap().join("hacked.txt").exists());
}
