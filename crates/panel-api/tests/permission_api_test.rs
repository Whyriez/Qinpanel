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
use panel_api::files::{ChmodApiResponse, ChmodRecursiveApiResponse, ChownApiResponse};
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
    admin_session: String,
    admin_csrf: String,
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
    let admin_hash = hash_password("AdminPass123!").expect("hash pass");
    user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: admin_hash,
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            plan_id: None,
            linux_username: Some("root".to_string()),
            linux_uid: Some(0),
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
    let (admin_session, admin_csrf) = login_and_get_csrf(&app, "admin", "AdminPass123!").await;

    TestContext {
        app,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        alice_session,
        alice_csrf,
        admin_session,
        admin_csrf,
        alice_home,
    }
}

#[tokio::test]
async fn test_chmod_single_file_and_permissive_warning_api() {
    let ctx = setup_test_context().await;

    let file_path = ctx.alice_home.join("hello.txt");
    fs::write(&file_path, "Hello World").expect("write file");

    // 1. Chmod normal 0644
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/chmod")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "hello.txt",
                "mode": "0644"
            })
            .to_string(),
        ))
        .expect("build req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.expect("collect").to_bytes();
    let resp: ChmodApiResponse = serde_json::from_slice(&body).expect("parse json");
    assert_eq!(resp.mode_octal, "0644");
    assert_eq!(resp.mode, 0o644);
    assert!(resp.warning.is_none());

    // 2. Chmod 0777 -> Menghasilkan warning keamanan
    let req_777 = Request::builder()
        .method("POST")
        .uri("/api/v1/files/chmod")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "hello.txt",
                "mode": "0777"
            })
            .to_string(),
        ))
        .expect("build req");

    let res_777 = ctx.app.clone().oneshot(req_777).await.expect("oneshot");
    assert_eq!(res_777.status(), StatusCode::OK);
    let body_777 = res_777
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let resp_777: ChmodApiResponse = serde_json::from_slice(&body_777).expect("parse json");
    assert_eq!(resp_777.mode_octal, "0777");
    assert_eq!(resp_777.mode, 0o777);
    assert!(
        resp_777.warning.is_some(),
        "Mode 777 harus menyertakan warning"
    );
    assert!(resp_777.warning.unwrap().contains("Peringatan keamanan"));

    // 3. Chmod dengan format oktal tidak valid -> 400 Bad Request
    let req_invalid = Request::builder()
        .method("POST")
        .uri("/api/v1/files/chmod")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "hello.txt",
                "mode": "999"
            })
            .to_string(),
        ))
        .expect("build req");

    let res_invalid = ctx.app.clone().oneshot(req_invalid).await.expect("oneshot");
    assert_eq!(res_invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_chmod_recursive_separate_modes_api() {
    let ctx = setup_test_context().await;

    let sub_dir = ctx.alice_home.join("project").join("src");
    fs::create_dir_all(&sub_dir).expect("create sub_dir");
    fs::write(sub_dir.join("index.php"), "<?php echo 'ok';").expect("write file");
    fs::write(sub_dir.join("style.css"), "body {}").expect("write file");

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/chmod-recursive")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "project",
                "file_mode": "0644",
                "dir_mode": "0755"
            })
            .to_string(),
        ))
        .expect("build req");

    let res = ctx.app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.expect("collect").to_bytes();
    let resp: ChmodRecursiveApiResponse = serde_json::from_slice(&body).expect("parse json");

    assert_eq!(resp.file_mode_octal, "0644");
    assert_eq!(resp.dir_mode_octal, "0755");
    assert!(resp.affected_count >= 3);
    assert!(resp.warning.is_none());
}

#[tokio::test]
async fn test_chown_tenant_restriction_and_admin_allowed_api() {
    let ctx = setup_test_context().await;

    let file_path = ctx.alice_home.join("secure.txt");
    fs::write(&file_path, "Secret Data").expect("write file");

    // 1. Tenant (alice) mencoba chown -> Harus 403 Forbidden
    let req_tenant = Request::builder()
        .method("POST")
        .uri("/api/v1/files/chown")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::json!({
                "path": "secure.txt",
                "uid": 1000,
                "gid": 1000
            })
            .to_string(),
        ))
        .expect("build req");

    let res_tenant = ctx.app.clone().oneshot(req_tenant).await.expect("oneshot");
    assert_eq!(res_tenant.status(), StatusCode::FORBIDDEN);
    let body_tenant = res_tenant
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let err_json: serde_json::Value = serde_json::from_slice(&body_tenant).expect("json");
    assert!(
        err_json["error"]
            .as_str()
            .unwrap()
            .contains("chown dibatasi")
    );

    // 2. Admin melakukan chown untuk tenant alice -> Diizinkan (200 OK)
    let req_admin = Request::builder()
        .method("POST")
        .uri("/api/v1/files/chown")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::json!({
                "tenant": "alice",
                "path": "secure.txt",
                "uid": 1001,
                "gid": 1001
            })
            .to_string(),
        ))
        .expect("build req");

    let res_admin = ctx.app.clone().oneshot(req_admin).await.expect("oneshot");
    assert_eq!(res_admin.status(), StatusCode::OK);
    let body_admin = res_admin
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let resp_admin: ChownApiResponse = serde_json::from_slice(&body_admin).expect("json");
    assert_eq!(resp_admin.path, "secure.txt");
    assert_eq!(resp_admin.uid, Some(1001));
    assert_eq!(resp_admin.gid, Some(1001));
}
