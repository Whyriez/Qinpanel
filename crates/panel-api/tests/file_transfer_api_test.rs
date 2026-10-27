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
use panel_api::files::UploadMetadataApiResponse;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use std::fs;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    admin_session: String,
    admin_csrf: String,
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

    // Setup home dirs for alice
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
    let state = AppState::new(
        agent_client,
        pool.clone(),
        "development".to_string(),
        auth_config,
        lockout_manager,
    );

    let app = create_router(state);

    let (admin_session, admin_csrf) = login_and_get_csrf(&app, "admin", "AdminPass123!").await;
    let (alice_session, alice_csrf) = login_and_get_csrf(&app, "alice", "AlicePass123!").await;

    TestContext {
        app,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        alice_home,
    }
}

#[tokio::test]
async fn test_chunked_upload_and_atomic_finalize() {
    let ctx = setup_test_context().await;

    // 1. Init upload session for a 15-byte file: "hello world!!!!"
    let total_size = 15u64;
    let init_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/upload/init")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "target_path": "uploads/data.bin",
                "file_name": "data.bin",
                "total_size": total_size
            })
            .to_string(),
        ))
        .unwrap();

    let res = ctx.app.clone().oneshot(init_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let init_meta: UploadMetadataApiResponse = serde_json::from_slice(&body_bytes).unwrap();
    let upload_id = init_meta.upload_id;
    assert_eq!(init_meta.current_offset, 0);
    assert!(!init_meta.completed);

    // 2. Query session status
    let status_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/files/upload/{}", upload_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(status_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let meta: UploadMetadataApiResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(meta.current_offset, 0);

    // 3. Append Chunk 1: bytes 0..5 ("hello")
    let chunk1_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/files/upload/{}/chunk?offset=0", upload_id))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(&b"hello"[..]))
        .unwrap();

    let res = ctx.app.clone().oneshot(chunk1_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let meta: UploadMetadataApiResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(meta.current_offset, 5);
    assert!(!meta.completed);

    // 4. Test Offset Mismatch Conflict: send chunk with wrong offset (e.g. 2 instead of 5)
    let bad_chunk_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/files/upload/{}/chunk?offset=2", upload_id))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(&b"invalid"[..]))
        .unwrap();

    let res = ctx.app.clone().oneshot(bad_chunk_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // 5. Append Chunk 2: bytes 5..10 (" worl")
    let chunk2_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/files/upload/{}/chunk?offset=5", upload_id))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(&b" worl"[..]))
        .unwrap();

    let res = ctx.app.clone().oneshot(chunk2_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let meta: UploadMetadataApiResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(meta.current_offset, 10);
    assert!(!meta.completed);

    // 6. Append Chunk 3 (final): bytes 10..15 ("d!!!!")
    let chunk3_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/files/upload/{}/chunk?offset=10",
            upload_id
        ))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(&b"d!!!!"[..]))
        .unwrap();

    let res = ctx.app.clone().oneshot(chunk3_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let meta: UploadMetadataApiResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(meta.current_offset, 15);
    assert!(meta.completed);

    // 7. Verify file was finalized into uploads/data.bin
    let stat_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/stat?path=uploads/data.bin")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(stat_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let stat_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(stat_json["size_bytes"], 15);
    assert_eq!(stat_json["name"], "data.bin");
}

#[tokio::test]
async fn test_download_range_etag_and_zip() {
    let ctx = setup_test_context().await;

    // 1. Create a file for download using chunked upload
    let payload = b"0123456789ABCDEF"; // 16 bytes
    let init_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/upload/init")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "target_path": "docs/numbers.txt",
                "file_name": "numbers.txt",
                "total_size": 16
            })
            .to_string(),
        ))
        .unwrap();

    let res = ctx.app.clone().oneshot(init_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let meta: UploadMetadataApiResponse = serde_json::from_slice(&body_bytes).unwrap();

    let chunk_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/files/upload/{}/chunk?offset=0",
            meta.upload_id
        ))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(&payload[..]))
        .unwrap();

    let res = ctx.app.clone().oneshot(chunk_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 2. Full download: GET /api/v1/files/download?path=docs/numbers.txt
    let dl_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/download?path=docs/numbers.txt")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(dl_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::ACCEPT_RANGES).unwrap(), "bytes");
    let etag = res
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body_bytes[..], payload);

    // 3. Conditional download: If-None-Match matches ETag -> 304 Not Modified
    let not_mod_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/download?path=docs/numbers.txt")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(not_mod_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);

    // 4. HTTP Range download: bytes 4-9 ("456789")
    let range_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/download?path=docs/numbers.txt")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(header::RANGE, "bytes=4-9")
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(range_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers().get(header::CONTENT_RANGE).unwrap(),
        "bytes 4-9/16"
    );
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body_bytes[..], b"456789");

    // 5. Download folder as ZIP: GET /api/v1/files/download-zip?path=docs
    let zip_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/download-zip?path=docs")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(zip_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/zip"
    );
    let zip_bytes = res.into_body().collect().await.unwrap().to_bytes();

    // Verify ZIP content using zip crate
    let reader = std::io::Cursor::new(zip_bytes);
    let mut zip_archive = zip::ZipArchive::new(reader).expect("valid zip archive");
    assert!(!zip_archive.is_empty());
    let mut file = zip_archive
        .by_name("numbers.txt")
        .expect("numbers.txt in zip");
    let mut extracted = Vec::new();
    file.read_to_end(&mut extracted).expect("read zip entry");
    assert_eq!(extracted, payload);
}

#[tokio::test]
async fn test_preflight_quota_check() {
    let ctx = setup_test_context().await;

    // Set quota for alice via agent client: limit 50 bytes
    let quota_set_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/upload/init")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::json!({
                "target_path": "bigfile.iso",
                "file_name": "bigfile.iso",
                "total_size": 100, // 100 bytes
                "tenant": "alice"
            })
            .to_string(),
        ))
        .unwrap();

    // Now set quota in agent for alice to 50 bytes
    let agent_client = AgentClient::new(ctx._temp.path().join(
        #[cfg(unix)]
        "agent.sock",
        #[cfg(not(unix))]
        "agent.port",
    ));
    agent_client
        .set_quota(panel_proto::QuotaSetRequest {
            uid: 10001,
            username: "p_alice".to_string(),
            disk_limit_bytes: 50,
            inode_limit: 100,
            mount_point: None,
        })
        .await
        .expect("set quota");

    // Attempt to init upload of 100 bytes (which exceeds 50 bytes limit)
    let res = ctx.app.clone().oneshot(quota_set_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let err_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let err_msg = err_json["error"].as_str().unwrap();
    assert!(
        err_msg.contains("Pre-flight quota exceeded"),
        "Expected quota error message, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn large_download_is_backpressured_in_bounded_frames() {
    let ctx = setup_test_context().await;
    let size = panel_api::files::FILE_DOWNLOAD_CHUNK as usize * 4 + 17;
    fs::write(ctx.alice_home.join("large.bin"), vec![b'x'; size]).unwrap();
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/download?path=large.bin")
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
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(
        response.headers()[header::CONTENT_LENGTH].to_str().unwrap(),
        size.to_string()
    );
    let mut body = response.into_body();
    let mut total = 0;
    while let Some(frame) = body.frame().await {
        let data = frame.unwrap().into_data().unwrap();
        assert!(data.len() as u64 <= panel_api::files::FILE_DOWNLOAD_CHUNK);
        assert!(data.iter().all(|byte| *byte == b'x'));
        total += data.len();
    }
    assert_eq!(total, size);
}

#[tokio::test]
async fn invalid_ranges_are_rejected_and_suffix_ranges_are_supported() {
    let ctx = setup_test_context().await;
    fs::write(ctx.alice_home.join("range.bin"), b"0123456789").unwrap();
    for spec in [
        "bytes=9-1",
        "bytes=0-1,4-5",
        "bytes=18446744073709551616-",
        "bytes=-0",
    ] {
        let response = ctx
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/files/download?path=range.bin")
                    .header(
                        header::COOKIE,
                        format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                    )
                    .header(header::RANGE, spec)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::RANGE_NOT_SATISFIABLE,
            "{spec}"
        );
    }
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/download?path=range.bin")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .header(header::RANGE, "bytes=-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        b"789"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn special_files_cannot_block_the_download_worker() {
    let ctx = setup_test_context().await;
    let _socket =
        std::os::unix::net::UnixListener::bind(ctx.alice_home.join("special.sock")).unwrap();
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/download?path=special.sock")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
