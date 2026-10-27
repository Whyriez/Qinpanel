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
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use serde_json::Value;
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
    #[allow(dead_code)]
    admin_id: i64,
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

    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response(
        "findmnt --task 1 -n -o OPTIONS --target /proc",
        0,
        b"rw,nosuid,nodev,noexec,hidepid=2,gid=100\n",
        b"",
    );
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

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::with_managers(
        agent_cfg,
        user_manager,
        slice_manager,
        quota_manager,
        resource_reader,
        hardening_manager,
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
    let admin_user = user_repo
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
            serde_json::to_string(&serde_json::json!({
                "username": "admin",
                "password": "AdminPass123!"
            }))
            .expect("json"),
        ))
        .expect("build");

    let login_res = app.clone().oneshot(login_req).await.expect("res");
    assert_eq!(login_res.status(), StatusCode::OK);

    let session_cookie = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .expect("cookie str");
    let session_val =
        extract_cookie_value(session_cookie, SESSION_COOKIE_NAME).expect("session cookie");

    // CSRF token
    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session_val),
        )
        .body(Body::empty())
        .expect("build");
    let csrf_res = app.clone().oneshot(csrf_req).await.expect("res");
    assert_eq!(csrf_res.status(), StatusCode::OK);
    let csrf_body = csrf_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let csrf_json: Value = serde_json::from_slice(&csrf_body).expect("parse csrf");
    let csrf_token = csrf_json["csrf_token"]
        .as_str()
        .expect("csrf str")
        .to_string();

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session: session_val,
        admin_csrf: csrf_token,
        admin_id: admin_user.id,
    }
}

#[tokio::test]
async fn test_plans_api_crud_lifecycle() {
    let ctx = setup_test_context().await;

    // 1. Create a plan as admin
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/v1/plans")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "StandardPlan",
                "description": "Standard hosting plan",
                "cpu_quota_percent": 150,
                "memory_limit_bytes": 1073741824, // 1GB
                "tasks_max": 200,
                "disk_limit_bytes": 10737418240i64, // 10GB
                "inode_limit": 500000,
                "shell_policy": "sftp"
            }))
            .expect("json"),
        ))
        .expect("build");

    let create_res = ctx.app.clone().oneshot(create_req).await.expect("res");
    assert_eq!(create_res.status(), StatusCode::CREATED);
    let create_body = create_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let create_json: Value = serde_json::from_slice(&create_body).expect("parse json");
    let plan_id = create_json["plan"]["id"].as_i64().expect("plan id");
    assert_eq!(create_json["plan"]["name"], "StandardPlan");
    assert_eq!(create_json["plan"]["cpu_quota_percent"], 150);

    // 2. List plans
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/plans")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build");

    let list_res = ctx.app.clone().oneshot(list_req).await.expect("res");
    assert_eq!(list_res.status(), StatusCode::OK);
    let list_body = list_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let plans_res: Value = serde_json::from_slice(&list_body).expect("parse json");
    let plans = plans_res["plans"].as_array().expect("plans array");
    assert!(plans.iter().any(|p| p["name"] == "StandardPlan"));

    // 3. Get plan by ID
    let get_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/plans/{}", plan_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build");

    let get_res = ctx.app.clone().oneshot(get_req).await.expect("res");
    assert_eq!(get_res.status(), StatusCode::OK);

    // 4. Update plan
    let update_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/plans/{}", plan_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "cpu_quota_percent": 200,
                "tasks_max": 250
            }))
            .expect("json"),
        ))
        .expect("build");

    let update_res = ctx.app.clone().oneshot(update_req).await.expect("res");
    assert_eq!(update_res.status(), StatusCode::OK);
    let update_body = update_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let update_json: Value = serde_json::from_slice(&update_body).expect("parse json");
    assert_eq!(update_json["plan"]["cpu_quota_percent"], 200);
    assert_eq!(update_json["plan"]["tasks_max"], 250);
}

#[tokio::test]
async fn test_tenant_wizard_provisioning_resources_and_lifecycle() {
    let ctx = setup_test_context().await;

    // 1. Create a default plan first
    let plan_req = Request::builder()
        .method("POST")
        .uri("/api/v1/plans")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "WebDeveloper",
                "cpu_quota_percent": 100,
                "memory_limit_bytes": 536870912, // 512MB
                "tasks_max": 100,
                "disk_limit_bytes": 5368709120i64, // 5GB
                "inode_limit": 200000,
                "shell_policy": "sftp"
            }))
            .expect("json"),
        ))
        .expect("build");

    let plan_res = ctx.app.clone().oneshot(plan_req).await.expect("res");
    assert_eq!(plan_res.status(), StatusCode::CREATED);
    let plan_body = plan_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let plan_json: Value = serde_json::from_slice(&plan_body).expect("parse json");
    let plan_id = plan_json["plan"]["id"].as_i64().expect("plan id");

    // 2. Reject reserved slug
    let reject_req = Request::builder()
        .method("POST")
        .uri("/api/v1/tenants")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username_slug": "root",
                "email": "root@fake.com",
                "password": "Password123!",
                "plan_id": plan_id
            }))
            .expect("json"),
        ))
        .expect("build");

    let reject_res = ctx.app.clone().oneshot(reject_req).await.expect("res");
    assert_eq!(reject_res.status(), StatusCode::BAD_REQUEST);

    // 3. Provision tenant via wizard (slug "client1")
    let provision_req = Request::builder()
        .method("POST")
        .uri("/api/v1/tenants")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username_slug": "client1",
                "email": "client1@example.com",
                "password": "ClientPassword123!",
                "plan_id": plan_id
            }))
            .expect("json"),
        ))
        .expect("build");

    let provision_res = ctx.app.clone().oneshot(provision_req).await.expect("res");
    let status = provision_res.status();
    let prov_body = provision_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    if status != StatusCode::CREATED {
        panic!(
            "Provision failed with {}: {}",
            status,
            String::from_utf8_lossy(&prov_body)
        );
    }
    let prov_json: Value = serde_json::from_slice(&prov_body).expect("parse json");
    let tenant_id = prov_json["tenant"]["id"].as_i64().expect("tenant id");
    let linux_uname = prov_json["tenant"]["linux_username"]
        .as_str()
        .expect("uname");
    let linux_uid = prov_json["tenant"]["linux_uid"].as_i64().expect("uid");

    assert_eq!(linux_uname, "p_client1");
    assert!((10000..=59999).contains(&linux_uid));
    assert_eq!(prov_json["tenant"]["status"], "active");
    assert_eq!(prov_json["plan"]["name"], "WebDeveloper");

    // 4. List tenants
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/tenants")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build");

    let list_res = ctx.app.clone().oneshot(list_req).await.expect("res");
    assert_eq!(list_res.status(), StatusCode::OK);
    let list_body = list_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let list_json: Value = serde_json::from_slice(&list_body).expect("parse json");
    let tenants = list_json["tenants"].as_array().expect("tenants array");
    assert!(tenants.iter().any(|t| t["username"] == "client1"));

    // 5. Get tenant details
    let detail_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/tenants/{}", tenant_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build");

    let detail_res = ctx.app.clone().oneshot(detail_req).await.expect("res");
    assert_eq!(detail_res.status(), StatusCode::OK);

    // 6. Get tenant live resources
    let res_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/tenants/{}/resources", tenant_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build");

    let res_resp = ctx.app.clone().oneshot(res_req).await.expect("res");
    assert_eq!(res_resp.status(), StatusCode::OK);
    let res_body = res_resp
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let res_json: Value = serde_json::from_slice(&res_body).expect("parse json");
    assert_eq!(res_json["uid"], linux_uid);
    assert_eq!(res_json["linux_username"], "p_client1");
    assert!(res_json.get("metrics").is_some());

    // 7. Suspend tenant
    let suspend_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/tenants/{}/suspend", tenant_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .expect("build");

    let suspend_res = ctx.app.clone().oneshot(suspend_req).await.expect("res");
    assert_eq!(suspend_res.status(), StatusCode::OK);

    // Verify status is now suspended
    let check_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/tenants/{}", tenant_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build");

    let check_res = ctx.app.clone().oneshot(check_req).await.expect("res");
    let check_body = check_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let check_json: Value = serde_json::from_slice(&check_body).expect("parse json");
    assert_eq!(check_json["tenant"]["status"], "suspended");

    // 8. Unsuspend tenant
    let unsuspend_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/tenants/{}/unsuspend", tenant_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .expect("build");

    let unsuspend_res = ctx.app.clone().oneshot(unsuspend_req).await.expect("res");
    assert_eq!(unsuspend_res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_tenant_impersonation_flow_and_audit() {
    let ctx = setup_test_context().await;

    // 1. Create a plan and provision a tenant
    let plan_req = Request::builder()
        .method("POST")
        .uri("/api/v1/plans")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "ImpersonatePlan",
                "cpu_quota_percent": 100,
                "memory_limit_bytes": 536870912,
                "tasks_max": 50,
                "disk_limit_bytes": 1073741824,
                "inode_limit": 100000,
                "shell_policy": "sftp"
            }))
            .expect("json"),
        ))
        .expect("build");
    let plan_res = ctx.app.clone().oneshot(plan_req).await.expect("res");
    let plan_body = plan_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let plan_json: Value = serde_json::from_slice(&plan_body).expect("parse json");
    let plan_id = plan_json["plan"]["id"].as_i64().expect("plan id");

    let prov_req = Request::builder()
        .method("POST")
        .uri("/api/v1/tenants")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username_slug": "bob",
                "email": "bob@example.com",
                "password": "BobPassword123!",
                "plan_id": plan_id
            }))
            .expect("json"),
        ))
        .expect("build");
    let prov_res = ctx.app.clone().oneshot(prov_req).await.expect("res");
    let prov_body = prov_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let prov_json: Value = serde_json::from_slice(&prov_body).expect("parse json");
    let bob_id = prov_json["tenant"]["id"].as_i64().expect("id");

    // 2. Admin impersonates bob
    let imp_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/tenants/{}/impersonate", bob_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .expect("build");

    let imp_res = ctx.app.clone().oneshot(imp_req).await.expect("res");
    assert_eq!(imp_res.status(), StatusCode::OK);

    let imp_cookie = imp_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set cookie")
        .to_str()
        .expect("cookie str");
    let bob_session_token =
        extract_cookie_value(imp_cookie, SESSION_COOKIE_NAME).expect("bob session token");

    // 3. Request /api/v1/auth/me using Bob's impersonated session
    let me_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, bob_session_token),
        )
        .body(Body::empty())
        .expect("build");

    let me_res = ctx.app.clone().oneshot(me_req).await.expect("res");
    assert_eq!(me_res.status(), StatusCode::OK);
    let me_body = me_res.into_body().collect().await.expect("read").to_bytes();
    let me_json: Value = serde_json::from_slice(&me_body).expect("parse json");

    assert_eq!(me_json["user"]["username"], "bob");
    assert_eq!(me_json["user"]["role"], "user");
    assert_eq!(me_json["user"]["impersonated_by"], "admin");

    // 4. Stop impersonating
    // Get CSRF for bob
    let bob_csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, bob_session_token),
        )
        .body(Body::empty())
        .expect("build");
    let bob_csrf_res = ctx.app.clone().oneshot(bob_csrf_req).await.expect("res");
    let bob_csrf_body = bob_csrf_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let bob_csrf_json: Value = serde_json::from_slice(&bob_csrf_body).expect("json");
    let bob_csrf = bob_csrf_json["csrf_token"].as_str().expect("csrf");

    let stop_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/stop-impersonating")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, bob_session_token),
        )
        .header(CSRF_HEADER_NAME, bob_csrf)
        .body(Body::empty())
        .expect("build");

    let stop_res = ctx.app.clone().oneshot(stop_req).await.expect("res");
    assert_eq!(stop_res.status(), StatusCode::OK);

    // 5. Verify audit logs record impersonate and stop
    let mut logs = Vec::new();
    for _ in 0..20 {
        let audit_req = Request::builder()
            .method("GET")
            .uri("/api/v1/audit/logs")
            .header(
                header::COOKIE,
                format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
            )
            .body(Body::empty())
            .expect("build");

        let audit_res = ctx.app.clone().oneshot(audit_req).await.expect("res");
        assert_eq!(audit_res.status(), StatusCode::OK);
        let audit_body = audit_res
            .into_body()
            .collect()
            .await
            .expect("read")
            .to_bytes();
        let audit_json: Value = serde_json::from_slice(&audit_body).expect("parse json");
        logs = audit_json["items"].as_array().cloned().unwrap_or_default();

        if logs.iter().any(|l| l["action"] == "tenant.impersonate")
            && logs
                .iter()
                .any(|l| l["action"] == "tenant.impersonate.stop")
        {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    assert!(logs.iter().any(|l| l["action"] == "tenant.impersonate"));
    assert!(
        logs.iter()
            .any(|l| l["action"] == "tenant.impersonate.stop")
    );
}

#[tokio::test]
async fn test_hardening_api_diagnostics_and_apply() {
    let ctx = setup_test_context().await;

    // 1. Get hardening status
    let check_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/hardening")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("build");

    let check_res = ctx.app.clone().oneshot(check_req).await.expect("res");
    let check_status = check_res.status();
    let check_body = check_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    if check_status != StatusCode::OK {
        panic!(
            "Check hardening failed with {}: {}",
            check_status,
            String::from_utf8_lossy(&check_body)
        );
    }
    let check_json: Value = serde_json::from_slice(&check_body).expect("parse json");

    assert!(check_json["cgroup_v2"]["ok"].as_bool().unwrap_or(false));
    assert!(!check_json["overall_ok"].as_bool().unwrap_or(true));

    // 2. Apply hardening via API
    let apply_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/hardening/apply")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "enable_hidepid": true,
                "ensure_tenant_group": true
            }))
            .expect("json"),
        ))
        .expect("build");

    let apply_res = ctx.app.clone().oneshot(apply_req).await.expect("res");
    assert_eq!(apply_res.status(), StatusCode::OK);
    let apply_body = apply_res
        .into_body()
        .collect()
        .await
        .expect("read")
        .to_bytes();
    let apply_json: Value = serde_json::from_slice(&apply_body).expect("parse json");
    assert!(
        apply_json["tenant_group_created"]
            .as_bool()
            .unwrap_or(false)
    );
}
