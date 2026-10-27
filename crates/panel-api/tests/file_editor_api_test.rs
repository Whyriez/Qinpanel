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
use panel_api::files::{FileContentApiResponse, FileSaveApiResponse};
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
async fn test_read_and_save_file_content_workflow() {
    let ctx = setup_test_context().await;

    // 1. Create a text file in Alice's home
    let file_path = ctx.alice_home.join("hello.txt");
    fs::write(&file_path, "Hello Initial Content\nLine 2").expect("write initial file");

    // 2. Read file content via GET /api/v1/files/content?path=hello.txt
    let read_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/content?path=hello.txt")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build read req");

    let read_res = ctx
        .app
        .clone()
        .oneshot(read_req)
        .await
        .expect("read oneshot");
    assert_eq!(read_res.status(), StatusCode::OK);

    let read_body = read_res
        .into_body()
        .collect()
        .await
        .expect("read bytes")
        .to_bytes();
    let read_data: FileContentApiResponse =
        serde_json::from_slice(&read_body).expect("parse read response");

    assert_eq!(read_data.content, "Hello Initial Content\nLine 2");
    assert!(!read_data.read_only);
    assert!(!read_data.is_binary);
    assert!(read_data.modified_at.is_some());
    assert!(!read_data.etag.is_empty());

    let initial_mtime = read_data.modified_at;
    let initial_etag = read_data.etag;

    // 3. Save modified content via PUT /api/v1/files/content
    let save_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/files/content")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "hello.txt",
                "content": "Updated Hello World!",
                "expected_mtime": initial_mtime,
                "expected_etag": initial_etag,
                "force": false
            })
            .to_string(),
        ))
        .expect("build save req");

    let save_res = ctx
        .app
        .clone()
        .oneshot(save_req)
        .await
        .expect("save oneshot");
    assert_eq!(save_res.status(), StatusCode::OK);

    let save_body = save_res
        .into_body()
        .collect()
        .await
        .expect("save bytes")
        .to_bytes();
    let save_data: FileSaveApiResponse =
        serde_json::from_slice(&save_body).expect("parse save response");
    assert_eq!(save_data.size_bytes, 20);
    assert_eq!(save_data.message, "Berkas berhasil disimpan");

    // Verify disk content
    let disk_content = fs::read_to_string(&file_path).expect("read from disk");
    assert_eq!(disk_content, "Updated Hello World!");
}

#[tokio::test]
async fn test_save_conflict_detection_and_force_overwrite() {
    let ctx = setup_test_context().await;

    // 1. Create file
    let file_path = ctx.alice_home.join("config.json");
    fs::write(&file_path, "{\"version\": 1}").expect("write config.json");

    // 2. Read file to get initial mtime and etag
    let read_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/content?path=config.json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build read req");

    let read_res = ctx
        .app
        .clone()
        .oneshot(read_req)
        .await
        .expect("read oneshot");
    assert_eq!(read_res.status(), StatusCode::OK);

    let read_body = read_res
        .into_body()
        .collect()
        .await
        .expect("read bytes")
        .to_bytes();
    let read_data: FileContentApiResponse =
        serde_json::from_slice(&read_body).expect("parse read response");
    let old_mtime = read_data.modified_at;
    let old_etag = read_data.etag;

    // 3. Simulate another process modifying the file externally
    tokio::time::sleep(Duration::from_millis(50)).await;
    fs::write(&file_path, "{\"version\": 2, \"external\": true}").expect("external write");

    // 4. The stale content revision must cause 409 regardless of mtime resolution.
    let conflict_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/files/content")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "config.json",
                "content": "{\"version\": 3, \"alice\": true}",
                "expected_mtime": old_mtime,
                "expected_etag": old_etag,
                "force": false
            })
            .to_string(),
        ))
        .expect("build conflict req");

    let conflict_res = ctx
        .app
        .clone()
        .oneshot(conflict_req)
        .await
        .expect("conflict oneshot");
    assert_eq!(conflict_res.status(), StatusCode::CONFLICT);

    let conflict_body = conflict_res
        .into_body()
        .collect()
        .await
        .expect("bytes")
        .to_bytes();
    let err_json: serde_json::Value =
        serde_json::from_slice(&conflict_body).expect("parse error json");
    assert!(err_json["error"].as_str().unwrap().contains("Konflik"));
    assert!(err_json["conflict"].as_bool().unwrap());
    assert!(err_json["current_mtime"].as_i64().is_some());

    // Disk content should still be external version
    let current_disk = fs::read_to_string(&file_path).expect("read disk");
    assert_eq!(current_disk, "{\"version\": 2, \"external\": true}");

    // 5. Save with force: true -> should succeed despite mismatch
    let force_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/files/content")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "config.json",
                "content": "{\"version\": 3, \"forced\": true}",
                "expected_mtime": old_mtime,
                "expected_etag": old_etag,
                "force": true
            })
            .to_string(),
        ))
        .expect("build force req");

    let force_res = ctx
        .app
        .clone()
        .oneshot(force_req)
        .await
        .expect("force oneshot");
    assert_eq!(force_res.status(), StatusCode::OK);

    let final_disk = fs::read_to_string(&file_path).expect("read final disk");
    assert_eq!(final_disk, "{\"version\": 3, \"forced\": true}");
}

#[tokio::test]
async fn test_path_traversal_forbidden() {
    let ctx = setup_test_context().await;

    // Alice attempts to read /etc/passwd or escape home directory
    let read_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/content?path=../../etc/passwd")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build req");

    let res = ctx.app.clone().oneshot(read_req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

async fn editor_save(
    ctx: &TestContext,
    payload: serde_json::Value,
    csrf: bool,
) -> axum::response::Response {
    editor_save_with_csrf(ctx, payload, csrf.then_some(ctx.alice_csrf.as_str())).await
}

async fn editor_save_with_csrf(
    ctx: &TestContext,
    payload: serde_json::Value,
    csrf: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::builder()
        .method("PUT")
        .uri("/api/v1/files/content")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        );
    if let Some(token) = csrf {
        request = request.header(CSRF_HEADER_NAME, token);
    }
    ctx.app
        .clone()
        .oneshot(request.body(Body::from(payload.to_string())).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn editor_requires_revision_and_rejects_legacy_tokens_without_writing() {
    let ctx = setup_test_context().await;
    fs::write(ctx.alice_home.join("secret"), b"original").unwrap();
    for token in [serde_json::Value::Null, serde_json::json!("\"abcdef-8\"")] {
        let response = editor_save(
            &ctx,
            serde_json::json!({"path":"secret","content":"overwritten","expected_etag":token}),
            true,
        )
        .await;
        assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
        assert_eq!(
            fs::read(ctx.alice_home.join("secret")).unwrap(),
            b"original"
        );
    }
}

#[tokio::test]
async fn editor_detects_same_size_same_mtime_changes() {
    let ctx = setup_test_context().await;
    let path = ctx.alice_home.join("file");
    fs::write(&path, b"initial").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let old_etag = panel_core::file_revision::content_etag(b"initial");
    fs::write(&path, b"changed").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let response = editor_save(
        &ctx,
        serde_json::json!({"path":"file","content":"lost update","expected_etag":old_etag}),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(fs::read(&path).unwrap(), b"changed");
}

#[tokio::test]
async fn editor_cannot_recreate_a_deleted_file_or_replace_a_large_file_with_force() {
    let ctx = setup_test_context().await;
    let token = panel_core::file_revision::content_etag(b"initial");
    let response = editor_save(&ctx, serde_json::json!({"path":"deleted","content":"resurrected","expected_etag":token,"force":true}), true).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(!ctx.alice_home.join("deleted").exists());
    let large = ctx.alice_home.join("large");
    fs::File::create(&large)
        .unwrap()
        .set_len(panel_api::files::MAX_EDITOR_FILE_SIZE + 1)
        .unwrap();
    let response = editor_save(
        &ctx,
        serde_json::json!({"path":"large","content":"truncate","force":true}),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        fs::metadata(&large).unwrap().len(),
        panel_api::files::MAX_EDITOR_FILE_SIZE + 1
    );
}

#[tokio::test]
async fn editor_enforces_auth_csrf_and_tenant_scope_even_for_forced_writes() {
    let ctx = setup_test_context().await;
    fs::write(ctx.alice_home.join("file"), b"untouched").unwrap();
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/content?path=file")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/files/content")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"path":"file","content":"anonymous","force":true})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // A valid token from another session of the same user is still invalid here.
    let (_, other_csrf) = login_and_get_csrf(&ctx.app, "alice", "AlicePass123!").await;
    assert_ne!(other_csrf, ctx.alice_csrf);
    let revision = panel_core::file_revision::content_etag(b"untouched");
    for force in [false, true] {
        for token in [None, Some("invalid-token"), Some(other_csrf.as_str())] {
            let response = editor_save_with_csrf(
                &ctx,
                serde_json::json!({"path":"file","content":"csrf","expected_etag":revision,"force":force}),
                token,
            )
            .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(fs::read(ctx.alice_home.join("file")).unwrap(), b"untouched");
        }
        let response = editor_save(
            &ctx,
            serde_json::json!({"path":"file","content":"other tenant","tenant":"p_bob","expected_etag":revision,"force":force}),
            true,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(fs::read(ctx.alice_home.join("file")).unwrap(), b"untouched");
    }
    let response = editor_save(
        &ctx,
        serde_json::json!({"path":"file","content":"authorized","expected_etag":revision}),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        fs::read(ctx.alice_home.join("file")).unwrap(),
        b"authorized"
    );
}

#[tokio::test]
async fn editor_responses_are_not_cacheable_and_revision_covers_actual_content() {
    let ctx = setup_test_context().await;
    fs::write(ctx.alice_home.join("file"), b"private source code").unwrap();
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/content?path=file")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let data: FileContentApiResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        data.etag,
        panel_core::file_revision::content_etag(data.content.as_bytes())
    );
}
