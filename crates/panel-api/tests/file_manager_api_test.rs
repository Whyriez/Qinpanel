#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::fs_manager::FsManager;
use panel_agent::system::MockSystemRunner;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::files::{FsEntryItem, FsListApiResponse, TrashedItemApiResponse};
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
    admin_session: String,
    #[allow(dead_code)]
    admin_csrf: String,
    alice_session: String,
    alice_csrf: String,
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
    // A Windows drive colon cannot appear in a seven-field POSIX passwd
    // record. Keep this mock home relative to a temporary directory in cwd;
    // the production NSS validator remains strict on every platform.
    let tmp = if cfg!(windows) {
        tempfile::tempdir_in(".")
    } else {
        tempdir()
    }
    .expect("tempdir");
    let db_file = tmp.path().join("panel.db");

    #[cfg(unix)]
    let sock_path = tmp.path().join("agent.sock");
    #[cfg(not(unix))]
    let sock_path = tmp.path().join("agent.port");

    let home_base = if cfg!(windows) {
        std::path::PathBuf::from(".")
            .join(tmp.path().file_name().expect("temporary fixture name"))
            .join("home")
    } else {
        tmp.path().join("home")
    };
    fs::create_dir_all(&home_base).expect("create home base");

    // Setup home dirs for alice and bob
    let alice_home = home_base.join("p_alice");
    let bob_home = home_base.join("p_bob");
    fs::create_dir_all(&alice_home).expect("create alice home");
    fs::create_dir_all(&bob_home).expect("create bob home");

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
    let runner = mock_runner;
    let fs_manager = Arc::new(FsManager::with_home_base(runner, home_base));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::new(agent_cfg).with_fs_manager(fs_manager));

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

    // Login admin
    let (admin_session, admin_csrf) = login_and_get_csrf(&app, "admin", "AdminPass123!").await;
    // Login alice
    let (alice_session, alice_csrf) = login_and_get_csrf(&app, "alice", "AlicePass123!").await;

    TestContext {
        app,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
    }
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

#[tokio::test]
async fn test_file_manager_crud_lifecycle() {
    let ctx = setup_test_context().await;

    // 1. Mkdir: alice creates public_html
    let mkdir_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/mkdir")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "path": "public_html",
                "recursive": true
            })
            .to_string(),
        ))
        .expect("build mkdir");

    let mkdir_res = ctx
        .app
        .clone()
        .oneshot(mkdir_req)
        .await
        .expect("mkdir oneshot");
    let status = mkdir_res.status();
    let body = mkdir_res
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    if status != StatusCode::CREATED {
        panic!(
            "mkdir failed with status {}: {}",
            status,
            String::from_utf8_lossy(&body)
        );
    }

    // 2. List: alice lists root
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/list?path=")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build list");

    let list_res = ctx
        .app
        .clone()
        .oneshot(list_req)
        .await
        .expect("list oneshot");
    assert_eq!(list_res.status(), StatusCode::OK);
    let body = list_res
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let list_data: FsListApiResponse = serde_json::from_slice(&body).expect("parse list");
    assert_eq!(list_data.entries.len(), 1);
    assert_eq!(list_data.entries[0].name, "public_html");
    assert!(list_data.entries[0].is_dir);

    // 3. Stat: alice gets stat of public_html
    let stat_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/stat?path=public_html")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build stat");

    let stat_res = ctx
        .app
        .clone()
        .oneshot(stat_req)
        .await
        .expect("stat oneshot");
    assert_eq!(stat_res.status(), StatusCode::OK);
    let stat_body = stat_res
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let stat_data: FsEntryItem = serde_json::from_slice(&stat_body).expect("parse stat");
    assert_eq!(stat_data.name, "public_html");
    assert!(stat_data.is_dir);

    // 4. Rename / Move: alice renames public_html to www
    let rename_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/rename")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "source": "public_html",
                "destination": "www"
            })
            .to_string(),
        ))
        .expect("build rename");

    let rename_res = ctx
        .app
        .clone()
        .oneshot(rename_req)
        .await
        .expect("rename oneshot");
    assert_eq!(rename_res.status(), StatusCode::OK);

    // 5. Copy: alice copies www to www_backup
    let copy_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/copy")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "source": "www",
                "destination": "www_backup",
                "recursive": true
            })
            .to_string(),
        ))
        .expect("build copy");

    let copy_res = ctx
        .app
        .clone()
        .oneshot(copy_req)
        .await
        .expect("copy oneshot");
    assert_eq!(copy_res.status(), StatusCode::OK);

    // 6. Trash: move www_backup to trash
    let trash_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/trash")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "path": "www_backup"
            })
            .to_string(),
        ))
        .expect("build trash");

    let trash_res = ctx
        .app
        .clone()
        .oneshot(trash_req)
        .await
        .expect("trash oneshot");
    assert_eq!(trash_res.status(), StatusCode::OK);
    let trash_body = trash_res
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let trashed: TrashedItemApiResponse =
        serde_json::from_slice(&trash_body).expect("parse trashed item");
    assert_eq!(trashed.original_path, "www_backup");

    // 7. List trash: verify item is listed
    let list_trash_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/trash")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build list trash");

    let list_trash_res = ctx
        .app
        .clone()
        .oneshot(list_trash_req)
        .await
        .expect("list trash oneshot");
    assert_eq!(list_trash_res.status(), StatusCode::OK);
    let list_trash_body = list_trash_res
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let trash_list: Vec<TrashedItemApiResponse> =
        serde_json::from_slice(&list_trash_body).expect("parse trash list");
    assert_eq!(trash_list.len(), 1);
    assert_eq!(trash_list[0].trash_id, trashed.trash_id);

    // 8. Restore from trash
    let restore_req = Request::builder()
        .method("POST")
        .uri("/api/v1/files/trash/restore")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "trash_id": trashed.trash_id
            })
            .to_string(),
        ))
        .expect("build restore");

    let restore_res = ctx
        .app
        .clone()
        .oneshot(restore_req)
        .await
        .expect("restore oneshot");
    assert_eq!(restore_res.status(), StatusCode::OK);

    // 9. Permanent delete restored www_backup
    let del_req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/files/permanent")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "path": "www_backup",
                "recursive": true
            })
            .to_string(),
        ))
        .expect("build delete permanent");

    let del_res = ctx.app.clone().oneshot(del_req).await.expect("del oneshot");
    assert_eq!(del_res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_multi_tenant_isolation_and_rbac() {
    let ctx = setup_test_context().await;

    // Alice attempts to access Bob's files directly -> 403 Forbidden
    let bad_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/list?path=&tenant=bob")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build bad list");

    let bad_res = ctx
        .app
        .clone()
        .oneshot(bad_req)
        .await
        .expect("bad list oneshot");
    assert_eq!(bad_res.status(), StatusCode::FORBIDDEN);

    // Admin can access Bob's files -> 200 OK
    let admin_req = Request::builder()
        .method("GET")
        .uri("/api/v1/files/list?path=&tenant=bob")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build admin list");

    let admin_res = ctx
        .app
        .clone()
        .oneshot(admin_req)
        .await
        .expect("admin list oneshot");
    let admin_status = admin_res.status();
    let admin_body = admin_res
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    if admin_status != StatusCode::OK {
        panic!(
            "admin list failed with status {}: {}",
            admin_status,
            String::from_utf8_lossy(&admin_body)
        );
    }
}
