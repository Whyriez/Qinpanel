#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use flate2::Compression;
use flate2::write::GzEncoder;
use http_body_util::BodyExt;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateDomain, CreatePlan, CreateSite, CreateUser};
use panel_db::pool::{DbPool, init_pool, run_migrations};
use panel_db::repositories::{
    SqliteDomainRepository, SqliteJobRepository, SqlitePlanRepository, SqliteSiteRepository,
    SqliteUserRepository,
};
use panel_runtime::node::compute_sha256;
use serde_json::Value;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    job_runner: Arc<panel_api::jobs::JobRunner>,
    pool: DbPool,
    admin_session: String,
    user_session: String,
    _runtimes_base_dir: std::path::PathBuf,
    _temp: tempfile::TempDir,
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

fn build_mock_node_tarball(ver: &str) -> Vec<u8> {
    use std::io::Write;
    let mut tar_builder = tar::Builder::new(Vec::new());

    let node_script = format!("#!/usr/bin/env node\nconsole.log('{}');\n", ver);
    let mut header_node = tar::Header::new_gnu();
    header_node
        .set_path(format!("node-v{}-linux-x64/bin/node", ver))
        .unwrap();
    header_node.set_size(node_script.len() as u64);
    header_node.set_mode(0o755);
    header_node.set_cksum();
    tar_builder
        .append(&header_node, node_script.as_bytes())
        .unwrap();

    let cp_script = b"#!/usr/bin/env node\n// corepack\n";
    let mut header_cp = tar::Header::new_gnu();
    header_cp
        .set_path(format!("node-v{}-linux-x64/bin/corepack", ver))
        .unwrap();
    header_cp.set_size(cp_script.len() as u64);
    header_cp.set_mode(0o755);
    header_cp.set_cksum();
    tar_builder.append(&header_cp, &cp_script[..]).unwrap();

    let tar_data = tar_builder.into_inner().unwrap();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&tar_data).unwrap();
    encoder.finish().unwrap()
}

async fn setup_test_context() -> TestContext {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");
    let db_path = tmp.path().join("panel_test.db");
    let runtimes_base_dir = tmp.path().join("opt_panel_runtimes_node");
    std::fs::create_dir_all(&runtimes_base_dir).expect("create runtimes dir");

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };
    let agent_server = Arc::new(AgentServer::new(agent_cfg));
    let (agent_shutdown_tx, agent_shutdown_rx) = tokio::sync::broadcast::channel(1);

    tokio::spawn(async move {
        let _ = agent_server.run(agent_shutdown_rx).await;
    });

    let start = std::time::Instant::now();
    while !sock_path.exists() && start.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_path.to_str().expect("db_path").replace('\\', "/")
    ))
    .await
    .expect("init db pool");
    run_migrations(&pool).await.expect("run db migrations");
    // Hermetic official-metadata fixture; runtime policy never fetches the network in tests.
    let catalog: panel_api::runtime_catalog::NodeCatalog =
        serde_json::from_value(serde_json::json!({
            "releases":[
                {"version":"v22.14.0","date":"2025-02-11","lts":"Jod","files":[]},
                {"version":"v22.14.1","date":"2025-02-12","lts":"Jod","files":[]}
            ],
            "schedule":{"v22":{"start":"2024-04-24","lts":"2024-10-29","end":"2099-04-30"}}
        }))
        .unwrap();
    panel_api::runtime_catalog::store_refresh(
        &pool,
        "runtime.catalog.node",
        Ok(catalog),
        chrono::Utc::now().timestamp(),
    )
    .await
    .unwrap();

    let user_repo = SqliteUserRepository::new(pool.clone());
    let plan_repo = SqlitePlanRepository::new(pool.clone());

    let plan = plan_repo
        .create(&CreatePlan {
            name: "Default Plan".to_string(),
            description: None,
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(50),
            io_weight: Some(100),
            disk_limit_bytes: Some(5 * 1024 * 1024 * 1024),
            inode_limit: Some(500_000),
            max_sites: Some(5),
            max_domains: Some(5),
            max_databases: Some(2),
            max_cron_jobs: Some(5),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(50 * 1024 * 1024),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create plan");

    let _admin = user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            password_hash: hash_password("AdminPass123!").expect("hash"),
            role: "admin".to_string(),
            plan_id: None,
            linux_username: None,
            linux_uid: None,
        })
        .await
        .expect("create admin");

    let _user = user_repo
        .create(&CreateUser {
            username: "p_user".to_string(),
            email: Some("user@example.com".to_string()),
            password_hash: hash_password("UserPass123!").expect("hash"),
            role: "user".to_string(),
            plan_id: Some(plan.id),
            linux_username: Some("p_user".to_string()),
            linux_uid: Some(10001),
        })
        .await
        .expect("create user");

    let agent_client = Arc::new(AgentClient::new(&sock_path));
    let state = AppState::new(
        agent_client,
        pool.clone(),
        "test".to_string(),
        AuthConfig::default(),
        Arc::new(LockoutManager::new(5, 900)),
    );
    let job_runner = Arc::clone(&state.job_runner);
    let app = create_router(state);

    // Login Admin
    let admin_login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "username": "admin",
                "password": "AdminPass123!"
            }))
            .expect("json"),
        ))
        .expect("request");
    let admin_res = app.clone().oneshot(admin_login_req).await.expect("res");
    let admin_cookie_hdr = admin_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("admin set-cookie")
        .to_str()
        .expect("to_str");
    let admin_session =
        extract_cookie_value(admin_cookie_hdr, SESSION_COOKIE_NAME).expect("admin session");

    // Login User
    let user_login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "username": "p_user",
                "password": "UserPass123!"
            }))
            .expect("json"),
        ))
        .expect("request");
    let user_res = app.clone().oneshot(user_login_req).await.expect("res");
    let user_cookie_hdr = user_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("user set-cookie")
        .to_str()
        .expect("to_str");
    let user_session =
        extract_cookie_value(user_cookie_hdr, SESSION_COOKIE_NAME).expect("user session");

    TestContext {
        app,
        job_runner,
        pool,
        admin_session,
        user_session,
        _runtimes_base_dir: runtimes_base_dir,
        _temp: tmp,
        _agent_shutdown_tx: agent_shutdown_tx,
    }
}

async fn get_csrf_token(app: &axum::Router, session_token: &str) -> String {
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session_token),
        )
        .body(Body::empty())
        .expect("req");
    let res = app.clone().oneshot(req).await.expect("res");
    let body = res.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&body).expect("json");
    json["csrf_token"].as_str().expect("token").to_string()
}

#[tokio::test]
async fn test_get_node_versions_auth_and_structure() {
    let ctx = setup_test_context().await;

    // 1. Unauthenticated request rejected
    let unauth_req = Request::builder()
        .method("GET")
        .uri("/api/v1/runtimes/node/versions")
        .body(Body::empty())
        .unwrap();
    let unauth_res = ctx.app.clone().oneshot(unauth_req).await.unwrap();
    assert_eq!(unauth_res.status(), StatusCode::UNAUTHORIZED);

    // 2. Authenticated user request succeeds
    let auth_req = Request::builder()
        .method("GET")
        .uri("/api/v1/runtimes/node/versions")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.user_session),
        )
        .body(Body::empty())
        .unwrap();
    let auth_res = ctx.app.clone().oneshot(auth_req).await.unwrap();
    assert_eq!(auth_res.status(), StatusCode::OK);

    let body = auth_res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert!(json["versions"].is_array());
    let versions = json["versions"].as_array().unwrap();
    assert!(!versions.is_empty());

    // Verify presence of Node 22 LTS in the returned catalog
    let v22 = versions
        .iter()
        .find(|v| v["version"].as_str() == Some("22.14.0"))
        .expect("v22.14.0 should be present");
    assert_eq!(v22["is_lts"].as_bool(), Some(true));
    assert_eq!(v22["lts_codename"].as_str(), Some("Jod"));
}

#[tokio::test]
async fn test_install_node_version_rbac_and_flow() {
    let ctx = setup_test_context().await;

    // 1. Regular user trying to install should get 403 Forbidden
    let user_csrf = get_csrf_token(&ctx.app, &ctx.user_session).await;
    let user_install_req = Request::builder()
        .method("POST")
        .uri("/api/v1/runtimes/node/install")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.user_session),
        )
        .header("x-csrf-token", user_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "version": "22.14.0"
            }))
            .unwrap(),
        ))
        .unwrap();
    let user_install_res = ctx.app.clone().oneshot(user_install_req).await.unwrap();
    assert_eq!(user_install_res.status(), StatusCode::FORBIDDEN);

    // 2. Prepare mock offline tarball for fast and isolated test
    let tarball_bytes = build_mock_node_tarball("22.14.0");
    let checksum = compute_sha256(&tarball_bytes);
    let offline_tar_file = ctx._temp.path().join("node-v22.14.0-linux-x64.tar.gz");
    std::fs::write(&offline_tar_file, &tarball_bytes).unwrap();

    // 3. Admin installs Node.js 22.14.0 using offline tarball
    let admin_csrf = get_csrf_token(&ctx.app, &ctx.admin_session).await;
    let install_req = Request::builder()
        .method("POST")
        .uri("/api/v1/runtimes/node/install")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header("x-csrf-token", admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "version": "22.14.0",
                "offline_tarball_path": offline_tar_file.to_string_lossy(),
                "offline_checksum": checksum,
                "enable_corepack": true,
                "base_dir": ctx._runtimes_base_dir.to_string_lossy()
            }))
            .unwrap(),
        ))
        .unwrap();
    let install_res = ctx.app.clone().oneshot(install_req).await.unwrap();
    assert_eq!(install_res.status(), StatusCode::ACCEPTED);
    let body = install_res.into_body().collect().await.unwrap().to_bytes();
    let job: panel_api::jobs::JobItem = serde_json::from_slice(&body).unwrap();
    assert_eq!(job.name, "node.install");
    assert_eq!(job.status, "queued");
    assert_eq!(job.user_id, Some(1));
    let target = ctx._runtimes_base_dir.join("22.14.0/bin/node");
    assert!(
        !target.exists(),
        "HTTP request must not install the runtime"
    );
    assert!(
        panel_api::jobs::process_one_job(&ctx.job_runner)
            .await
            .unwrap()
    );
    let finished = SqliteJobRepository::new(ctx.pool.clone())
        .find_by_id(&job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.status, "succeeded", "{:?}", finished.error_message);
    assert_eq!(finished.progress, 100);
    let result: Value = serde_json::from_str(finished.result_json.as_deref().unwrap()).unwrap();
    assert_eq!(result["success"], true);
    assert_eq!(result["version"], "22.14.0");
    assert!(target.exists());

    // A failed background installation persists the full error for the monitor.
    let bad_payload = serde_json::json!({
        "version": "22.14.1", "enable_corepack": false,
        "mirror_url": null, "offline_tarball_path": offline_tar_file.to_string_lossy(),
        "offline_checksum": "0".repeat(64), "base_dir": ctx._runtimes_base_dir.to_string_lossy()
    });
    let (bad_job, _) = ctx
        .job_runner
        .enqueue(
            Some(1),
            None,
            "node.install".into(),
            bad_payload.to_string(),
            Some(0),
        )
        .await
        .unwrap();
    assert!(
        panel_api::jobs::process_one_job(&ctx.job_runner)
            .await
            .unwrap()
    );
    let failed = SqliteJobRepository::new(ctx.pool.clone())
        .find_by_id(&bad_job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert!(
        failed
            .error_message
            .unwrap()
            .contains("Instalasi Node.js v22.14.1 gagal")
    );
    assert!(!ctx._runtimes_base_dir.join("22.14.1/bin/node").exists());

    // Recheck privileges when executing work queued before a role change.
    let (denied_job, _) = ctx
        .job_runner
        .enqueue(
            Some(2),
            None,
            "node.install".into(),
            bad_payload.to_string(),
            Some(0),
        )
        .await
        .unwrap();
    assert!(
        panel_api::jobs::process_one_job(&ctx.job_runner)
            .await
            .unwrap()
    );
    let denied = SqliteJobRepository::new(ctx.pool.clone())
        .find_by_id(&denied_job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(denied.status, "failed");
    assert!(
        denied
            .error_message
            .unwrap()
            .contains("administrator aktif")
    );
}

#[tokio::test]
async fn test_delete_node_version_blocked_when_used_by_site() {
    let ctx = setup_test_context().await;
    let admin_csrf = get_csrf_token(&ctx.app, &ctx.admin_session).await;

    // 1. Create a site in DB that uses Node.js version 22.14.0
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let domain_repo = SqliteDomainRepository::new(ctx.pool.clone());

    let site = site_repo
        .create(&CreateSite {
            user_id: 2,
            kind: "nodejs".to_string(),
            docroot: "/home/p_user/sites/api.acme.test/app".to_string(),
            runtime: Some("nodejs".to_string()),
            version: Some("22.14.0".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .unwrap();

    domain_repo
        .create(&CreateDomain {
            site_id: site.id,
            name: "api.acme.test".to_string(),
            unicode_name: "api.acme.test".to_string(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .unwrap();

    // 2. Admin attempts to delete Node.js 22.14.0
    let del_req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/runtimes/node/22.14.0")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header("x-csrf-token", admin_csrf)
        .body(Body::empty())
        .unwrap();

    let del_res = ctx.app.clone().oneshot(del_req).await.unwrap();

    // 3. Must be rejected with 409 Conflict per PRD specifications!
    assert_eq!(del_res.status(), StatusCode::CONFLICT);

    let body = del_res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let error_msg = json["error"].as_str().expect("error message");
    assert!(error_msg.contains("api.acme.test"));
    assert!(error_msg.contains("sedang digunakan oleh"));
}

#[tokio::test]
async fn test_delete_node_version_succeeds_when_not_in_use() {
    let ctx = setup_test_context().await;
    let admin_csrf = get_csrf_token(&ctx.app, &ctx.admin_session).await;

    // Admin attempts to delete an unused version (e.g. 20.18.3)
    let del_req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/runtimes/node/20.18.3")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header("x-csrf-token", admin_csrf)
        .body(Body::empty())
        .unwrap();

    let del_res = ctx.app.clone().oneshot(del_req).await.unwrap();
    assert_eq!(del_res.status(), StatusCode::OK);

    let body = del_res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["success"].as_bool(), Some(true));
}
