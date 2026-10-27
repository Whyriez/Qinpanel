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
use panel_api::processes::ProcessListResponse;
use panel_api::routes::AppState;
use panel_api::sites::{CreateStaticSiteRequest, SiteResponse};
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use panel_proto::{NodeSiteStatusRequest, PythonSiteStatusRequest};
use serde_json::Value;
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
    admin_csrf: String,
    alice_session: String,
    #[allow(dead_code)]
    alice_csrf: String,
    bob_session: String,
    #[allow(dead_code)]
    bob_csrf: String,
    alice_id: i64,
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

fn configure_process_runner(runner: &MockSystemRunner) {
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response(
        "systemctl",
        0,
        b"ActiveState=active\nSubState=running\nMainPID=9876\nActiveEnterTimestampMonotonic=1000000\n",
        b"",
    );
    runner.set_response("journalctl", 0, b"2026-10-10 server started\n", b"");
    runner.set_response("curl", 0, b"200", b"");
    runner.set_response("runuser", 0, b"200", b"");
    runner.set_response("systemd-run", 0, b"Environment ready", b"");
    runner.add_existing_path("/opt/panel/runtimes/node/20.18.0/bin/node");
    runner.add_existing_path("/opt/panel/runtimes/python/3.12.9/bin/python3");
    // Node is created first; the Python site receives ID 2 in this context.
    runner.add_existing_path("/run/panel/python/site_2/app.sock");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");
}

#[test]
fn unified_process_fixture_has_ready_node_and_python_services() {
    let runner = MockSystemRunner::new();
    configure_process_runner(&runner);
    let node = panel_agent::node_site::status_node_site(
        &runner,
        &NodeSiteStatusRequest {
            site_id: "site_1".into(),
            port: Some(20050),
            probe_path: Some("/".into()),
        },
    )
    .expect("Node fixture status");
    assert!(node.active && node.healthy, "{node:?}");
    assert_eq!(node.probe_status_code, Some(200));
    assert_eq!(node.pid, Some(9876));

    let python = panel_agent::python_site::status_python_site(
        &runner,
        &PythonSiteStatusRequest {
            site_id: "site_2".into(),
            socket_path: Some("/run/panel/python/site_2/app.sock".into()),
        },
    )
    .expect("Python fixture status");
    assert!(
        python.active && python.socket_exists && python.healthy,
        "{python:?}"
    );
    assert_eq!(python.pid, Some(9876));
}

async fn created_site_response(response: axum::response::Response) -> SiteResponse {
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "Process fixture site creation failed: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).expect("created site response")
}

async fn setup_test_context() -> TestContext {
    let tmp = tempdir().expect("tempdir");
    let db_file = tmp.path().join("panel_test.db");

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
    configure_process_runner(&runner);

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
    let login_admin = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"admin","password":"AdminPass123!"}"#,
        ))
        .unwrap();
    let res = app.clone().oneshot(login_admin).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookies = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let admin_session = extract_cookie_value(cookies, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let admin_csrf = body_json["csrf_token"].as_str().unwrap().to_string();

    // Login alice
    let login_alice = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"alice","password":"AlicePass123!"}"#,
        ))
        .unwrap();
    let res = app.clone().oneshot(login_alice).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookies = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let alice_session = extract_cookie_value(cookies, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let alice_csrf = body_json["csrf_token"].as_str().unwrap().to_string();

    // Login bob
    let login_bob = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"username":"bob","password":"BobPass123!"}"#))
        .unwrap();
    let res = app.clone().oneshot(login_bob).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookies = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let bob_session = extract_cookie_value(cookies, SESSION_COOKIE_NAME).unwrap();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let bob_csrf = body_json["csrf_token"].as_str().unwrap().to_string();

    TestContext {
        app,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        alice_id: alice_user.id,
    }
}

#[tokio::test]
async fn test_unified_process_list_and_app_logs() {
    let ctx = setup_test_context().await;

    // 1. Create a Node.js site for Alice
    let create_node_req = CreateStaticSiteRequest {
        domain: "node-app.example.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("nodejs".to_string()),
        node_version: Some("20.18.0".to_string()),
        port: Some(20050),
        ..Default::default()
    };
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&create_node_req).unwrap()))
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    let node_site = created_site_response(res).await;

    // 2. Create a Python site for Alice
    let create_py_req = CreateStaticSiteRequest {
        domain: "py-app.example.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("python".to_string()),
        python_version: Some("3.12.9".to_string()),
        ..Default::default()
    };
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&create_py_req).unwrap()))
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    let py_site = created_site_response(res).await;

    // 3. Alice lists processes via GET /api/v1/processes
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/processes")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let process_list: ProcessListResponse = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(process_list.processes.len(), 2);
    assert_eq!(process_list.total_active, 2);

    let node_proc = process_list
        .processes
        .iter()
        .find(|p| p.runtime == "nodejs")
        .expect("nodejs process present");
    assert_eq!(node_proc.site_id, node_site.id);
    assert_eq!(node_proc.primary_domain, "node-app.example.com");
    assert!(node_proc.active);
    assert_eq!(node_proc.sub_state, "running");
    assert_eq!(node_proc.pid, Some(9876));

    let py_proc = process_list
        .processes
        .iter()
        .find(|p| p.runtime == "python")
        .expect("python process present");
    assert_eq!(py_proc.site_id, py_site.id);
    assert_eq!(py_proc.primary_domain, "py-app.example.com");
    assert!(py_proc.active);
    assert_eq!(py_proc.sub_state, "running");
    assert_eq!(py_proc.pid, Some(9876));

    // 4. Bob (another tenant) lists processes -> should see 0 processes
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/processes")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let bob_list: ProcessListResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(bob_list.processes.len(), 0);
    assert_eq!(bob_list.total_active, 0);

    // 5. Admin lists processes -> sees both
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/processes")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let admin_list: ProcessListResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(admin_list.processes.len(), 2);

    // 6. Test application log retrieval (type=app)
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/logs?type=app&lines=50",
            node_site.id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let logs_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(logs_json["log_type"], "app");
    assert!(logs_json["lines"].is_array());
}
