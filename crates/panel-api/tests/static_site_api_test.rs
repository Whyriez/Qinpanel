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
use panel_api::sites::{CreateStaticSiteRequest, SiteMessageResponse, SiteResponse};
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
    #[allow(dead_code)]
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    admin_session: String,
    admin_csrf: String,
    alice_id: i64,
    runner: Arc<MockSystemRunner>,
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
    // Configure mock responses for system binaries
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

    // Create active tenant alice
    let alice_hash = hash_password("AlicePass123!").expect("hash pass");
    let alice_user = user_repo
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

    // Login admin
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "username": "admin",
                "password": "AdminPass123!"
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
    let admin_session =
        extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).expect("session cookie");

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={admin_session}"),
        )
        .body(Body::empty())
        .expect("build csrf req");

    let csrf_res = app.clone().oneshot(csrf_req).await.expect("csrf oneshot");
    let csrf_body = csrf_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&csrf_body).expect("parse json");
    let admin_csrf = csrf_json["csrf_token"]
        .as_str()
        .expect("csrf_token")
        .to_string();

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_id: alice_user.id,
        runner,
    }
}

#[tokio::test]
async fn create_site_log_failure_rolls_back_records_and_preserves_docroot() {
    let ctx = setup_test_context().await;
    ctx.runner.set_response(
        &format!(
            "setfacl -m u:p_alice:r-x {}",
            std::path::Path::new("/var/log/panel")
                .join("p_alice")
                .display()
        ),
        1,
        b"",
        b"ACL denied",
    );
    let payload = CreateStaticSiteRequest {
        domain: "failed-log.test".into(),
        user_id: Some(ctx.alice_id),
        ..Default::default()
    };
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&payload).unwrap()))
        .unwrap();
    let response = ctx.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("Persiapan direktori log situs gagal"));
    let sites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sites")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    let domains: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domains")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!((sites, domains), (0, 0));
    let placeholder = ctx
        .runner
        .recorded_actions()
        .iter()
        .find_map(|action| match action {
            panel_agent::system::MockAction::WriteFile { path, .. }
                if path.ends_with("index.html") =>
            {
                Some(path.clone())
            }
            _ => None,
        })
        .expect("docroot was provisioned before log preparation");
    assert!(
        !ctx.runner.read_file(&placeholder).unwrap().is_empty(),
        "rollback must preserve tenant data"
    );
}

#[tokio::test]
async fn create_site_web_acl_failure_stops_before_apply_and_preserves_existing_data() {
    let ctx = setup_test_context().await;
    let root = std::path::Path::new("/home/p_alice/sites/acl-failure.test/public");
    let index = root.join("index.html");
    ctx.runner
        .set_file_content(&index, b"existing tenant content");
    ctx.runner.set_response(
        &format!(
            "setfacl -P -m u:www-data:--x -- {}",
            std::path::Path::new("/home/p_alice")
                .join("sites")
                .display()
        ),
        1,
        b"",
        b"ACL unsupported",
    );
    let payload = CreateStaticSiteRequest {
        domain: "acl-failure.test".into(),
        user_id: Some(ctx.alice_id),
        ..Default::default()
    };
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&payload).unwrap()))
        .unwrap();
    let response = ctx.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("ACL Nginx gagal"));
    assert!(String::from_utf8_lossy(&body).contains("ACL unsupported"));
    let sites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sites")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    let domains: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domains")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!((sites, domains), (0, 0));
    assert_eq!(
        ctx.runner.read_file(&index).unwrap(),
        b"existing tenant content"
    );
    assert!(
        !ctx.runner
            .recorded_actions()
            .iter()
            .any(|action| matches!(action,
                panel_agent::system::MockAction::Command { binary, .. } if binary == "nginx"
            ))
    );
}

#[tokio::test]
async fn test_static_site_crud_and_lifecycle() {
    let ctx = setup_test_context().await;

    // 1. Create static site
    let create_payload = CreateStaticSiteRequest {
        domain: "hello-static.test".to_string(),
        user_id: Some(ctx.alice_id),
        spa_fallback: Some(true),
        force_https: Some(false),
        docroot: None,
        ..Default::default()
    };

    let create_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&create_payload).unwrap()))
        .unwrap();

    let create_res = ctx.app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(create_res.status(), StatusCode::CREATED);

    let body = create_res.into_body().collect().await.unwrap().to_bytes();
    let created_site: SiteResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(created_site.primary_domain, "hello-static.test");
    assert_eq!(created_site.user_id, ctx.alice_id);
    assert_eq!(created_site.kind, "static");
    assert_eq!(created_site.status, "active");
    assert!(created_site.spa_fallback);
    assert_eq!(created_site.domains.len(), 1);
    assert_eq!(created_site.domains[0].name, "hello-static.test");
    assert!(created_site.domains[0].is_primary);

    // The router must pass authoritative tenant/domain context through RPC.
    // Successful mock syntax alone used to hide missing log directories.
    let log_dir = std::path::Path::new("/var/log/panel/p_alice/hello-static.test");
    assert!(ctx.runner.path_exists(log_dir));
    let actions = ctx.runner.recorded_actions();
    let log_prepared = actions
        .iter()
        .position(|action| {
            matches!(action,
        panel_agent::system::MockAction::CreateDir { path, uid: 0, gid: 0, mode: 0o750 }
        if path == log_dir)
        })
        .expect("root-owned site log directory");
    let nginx_test = actions
        .iter()
        .position(|action| {
            matches!(action,
        panel_agent::system::MockAction::Command { binary, args }
        if binary == "nginx" && args.first().is_some_and(|arg| arg == "-t"))
        })
        .expect("syntax check");
    assert!(log_prepared < nginx_test, "prepare logs before nginx -t");

    let site_id = created_site.id;

    // 2. Reject duplicate domain
    let dup_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&create_payload).unwrap()))
        .unwrap();

    let dup_res = ctx.app.clone().oneshot(dup_req).await.unwrap();
    assert_eq!(dup_res.status(), StatusCode::CONFLICT);

    // 3. Reject invalid domain format
    let bad_payload = CreateStaticSiteRequest {
        domain: "bad_domain;rm -rf".to_string(),
        user_id: Some(ctx.alice_id),
        spa_fallback: None,
        force_https: None,
        docroot: None,
        ..Default::default()
    };
    let bad_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&bad_payload).unwrap()))
        .unwrap();

    let bad_res = ctx.app.clone().oneshot(bad_req).await.unwrap();
    assert_eq!(bad_res.status(), StatusCode::BAD_REQUEST);

    // 4. List sites
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/sites")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let list_res = ctx.app.clone().oneshot(list_req).await.unwrap();
    assert_eq!(list_res.status(), StatusCode::OK);

    let list_body = list_res.into_body().collect().await.unwrap().to_bytes();
    let sites_list: Vec<SiteResponse> = serde_json::from_slice(&list_body).unwrap();
    assert_eq!(sites_list.len(), 1);
    assert_eq!(sites_list[0].id, site_id);

    // 5. Get site by ID
    let get_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{site_id}"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let get_res = ctx.app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(get_res.status(), StatusCode::OK);

    let get_body = get_res.into_body().collect().await.unwrap().to_bytes();
    let site_detail: SiteResponse = serde_json::from_slice(&get_body).unwrap();
    assert_eq!(site_detail.id, site_id);
    assert_eq!(site_detail.status, "active");

    // 6. Disable site
    let disable_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/disable"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let disable_res = ctx.app.clone().oneshot(disable_req).await.unwrap();
    assert_eq!(disable_res.status(), StatusCode::OK);

    let dis_body = disable_res.into_body().collect().await.unwrap().to_bytes();
    let dis_msg: SiteMessageResponse = serde_json::from_slice(&dis_body).unwrap();
    assert_eq!(dis_msg.status, "suspended");

    // 7. Enable site
    let enable_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/enable"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let enable_res = ctx.app.clone().oneshot(enable_req).await.unwrap();
    assert_eq!(enable_res.status(), StatusCode::OK);

    let en_body = enable_res.into_body().collect().await.unwrap().to_bytes();
    let en_msg: SiteMessageResponse = serde_json::from_slice(&en_body).unwrap();
    assert_eq!(en_msg.status, "active");

    // 8. Delete site
    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/sites/{site_id}"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let del_res = ctx.app.clone().oneshot(del_req).await.unwrap();
    assert_eq!(del_res.status(), StatusCode::OK);

    // Verify deleted
    let verify_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{site_id}"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let verify_res = ctx.app.clone().oneshot(verify_req).await.unwrap();
    assert_eq!(verify_res.status(), StatusCode::NOT_FOUND);
}
