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
use panel_api::cron::{CronJobListResponse, ValidateCronResponse};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateDomain, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqliteDomainRepository, SqliteSiteRepository, SqliteUserRepository};
use serde_json::Value;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
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

async fn setup_test_context() -> TestContext {
    let tmp = tempdir().expect("tempdir");
    let db_file = tmp.path().join("panel_cron_test.db");

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
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response(
        "journalctl",
        0,
        b"Cron executed successfully at 02:00\n",
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

    // Alice is assigned plan 1 (Starter plan, max_cron_jobs = 3)
    let alice_hash = hash_password("AlicePass123!").expect("hash pass");
    let alice = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            plan_id: Some(1),
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
            plan_id: Some(1),
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
    let audit_writer = Arc::new(panel_api::audit::AuditWriter::new(pool.clone()));
    let job_runner = Arc::new(panel_api::jobs::JobRunner::with_registry(
        pool.clone(),
        panel_api::jobs::JobRegistry::new_with_agent(Arc::clone(&agent_client)),
    ));

    let state = AppState {
        agent_client: Arc::clone(&agent_client),
        operations_pool: pool.clone(),
        operations_read_pool: pool.clone(),
        pool: pool.clone(),
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

    // Login Alice
    let login_body = serde_json::to_string(&serde_json::json!({
        "username": "alice",
        "password": "AlicePass123!"
    }))
    .unwrap();
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(login_body))
        .unwrap();
    let login_res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(login_res.status(), StatusCode::OK);
    let cookie_hdr = login_res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let alice_session =
        extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).expect("alice session");
    let login_json: Value =
        serde_json::from_slice(&login_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let alice_csrf = login_json["csrf_token"].as_str().unwrap().to_string();

    // Login Bob
    let login_body_b = serde_json::to_string(&serde_json::json!({
        "username": "bob",
        "password": "BobPass123!"
    }))
    .unwrap();
    let login_req_b = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(login_body_b))
        .unwrap();
    let login_res_b = app.clone().oneshot(login_req_b).await.unwrap();
    assert_eq!(login_res_b.status(), StatusCode::OK);
    let cookie_hdr_b = login_res_b
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let bob_session = extract_cookie_value(cookie_hdr_b, SESSION_COOKIE_NAME).expect("bob session");
    let login_json_b: Value =
        serde_json::from_slice(&login_res_b.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    let bob_csrf = login_json_b["csrf_token"].as_str().unwrap().to_string();

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        alice_id: alice.id,
    }
}

async fn create_alice_site(ctx: &TestContext, name: &str) -> i64 {
    // Cron tests start with an existing site; PHP provisioning has its own suite.
    let site = SqliteSiteRepository::new(ctx.pool.clone())
        .create(&CreateSite {
            user_id: ctx.alice_id,
            kind: "php".to_string(),
            docroot: format!("/home/p_alice/sites/{name}/public"),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("seed existing Alice site for cron tests");
    SqliteDomainRepository::new(ctx.pool.clone())
        .create(&CreateDomain {
            site_id: site.id,
            name: name.to_string(),
            unicode_name: name.to_string(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .expect("seed primary domain for cron tests");
    site.id
}

#[tokio::test]
async fn test_validate_cron_expression_api() {
    let ctx = setup_test_context().await;

    // 1. Valid expression
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites/0/cron/validate")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "expression": "*/15 * * * *"
            }))
            .unwrap(),
        ))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let val_res: ValidateCronResponse =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(val_res.valid);
    assert_eq!(val_res.systemd_calendar.unwrap(), "*-*-* *:00/15:00");
    assert_eq!(val_res.human_id.unwrap(), "Setiap 15 menit");
    assert_eq!(val_res.human_en.unwrap(), "Every 15 minutes");

    // 2. Invalid expression
    let req_inv = Request::builder()
        .method("POST")
        .uri("/api/v1/sites/0/cron/validate")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "expression": "60 * * * *"
            }))
            .unwrap(),
        ))
        .unwrap();

    let res_inv = ctx.app.clone().oneshot(req_inv).await.unwrap();
    assert_eq!(res_inv.status(), StatusCode::OK);
    let val_inv: ValidateCronResponse =
        serde_json::from_slice(&res_inv.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(!val_inv.valid);
    assert!(val_inv.error.is_some());
}

#[tokio::test]
async fn test_cron_job_lifecycle_and_tenant_isolation_and_quota() {
    let ctx = setup_test_context().await;
    let site_id = create_alice_site(&ctx, "laravel-cron.test").await;

    // 1. Alice creates Job 1 (Every day at 02:00)
    let req_create = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/cron", site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "Daily Backup",
                "command": "php artisan backup:run",
                "schedule": "0 2 * * *",
                "description": "Backup site database daily",
                "timeout_secs": 1800,
                "enabled": true
            }))
            .unwrap(),
        ))
        .unwrap();

    let res_create = ctx.app.clone().oneshot(req_create).await.unwrap();
    assert_eq!(res_create.status(), StatusCode::CREATED);
    let job_1: Value =
        serde_json::from_slice(&res_create.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    let job_1_id = job_1["id"].as_i64().unwrap();
    assert_eq!(job_1["name"], "Daily Backup");
    assert_eq!(job_1["systemd_calendar"], "*-*-* 02:00:00");
    assert_eq!(job_1["human_description"], "Setiap hari pada 02:00");

    // 2. Bob tries to access Alice's site cron -> 403 Forbidden
    let req_bob_list = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/cron", site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .body(Body::empty())
        .unwrap();
    let res_bob_list = ctx.app.clone().oneshot(req_bob_list).await.unwrap();
    assert_eq!(res_bob_list.status(), StatusCode::FORBIDDEN);

    // 3. Bob tries to create cron on Alice's site -> 403 Forbidden
    let req_bob_create = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/cron", site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "Malicious Job",
                "command": "rm -rf /",
                "schedule": "* * * * *"
            }))
            .unwrap(),
        ))
        .unwrap();
    let res_bob_create = ctx.app.clone().oneshot(req_bob_create).await.unwrap();
    assert_eq!(res_bob_create.status(), StatusCode::FORBIDDEN);

    // 4. Create Job 2 and Job 3 to reach Starter plan quota limit (max_cron_jobs = 3)
    for (i, sched) in [("0 */4 * * *", "Every 4 hours"), ("0 0 * * 0", "Weekly")] {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/sites/{}/cron", site_id))
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::COOKIE,
                format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
            )
            .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "name": sched,
                    "command": "php artisan queue:work --stop-when-empty",
                    "schedule": i
                }))
                .unwrap(),
            ))
            .unwrap();
        let res = ctx.app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }

    // 5. Attempt to create Job 4 -> MUST fail with 403 Forbidden quota exceeded
    let req_quota_exceeded = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/cron", site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "Exceeded Job",
                "command": "php artisan schedule:run",
                "schedule": "* * * * *"
            }))
            .unwrap(),
        ))
        .unwrap();
    let res_quota_exceeded = ctx.app.clone().oneshot(req_quota_exceeded).await.unwrap();
    assert_eq!(res_quota_exceeded.status(), StatusCode::FORBIDDEN);
    let quota_err: Value = serde_json::from_slice(
        &res_quota_exceeded
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    assert!(
        quota_err["error"]
            .as_str()
            .unwrap()
            .contains("Batas kuota cron job tercapai")
    );

    // 6. List site cron jobs -> check quota representation
    let req_list = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/cron", site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res_list = ctx.app.clone().oneshot(req_list).await.unwrap();
    assert_eq!(res_list.status(), StatusCode::OK);
    let list_data: CronJobListResponse =
        serde_json::from_slice(&res_list.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(list_data.jobs.len(), 3);
    assert_eq!(list_data.quota.used, 3);
    assert_eq!(list_data.quota.limit, 3);

    // 7. Update Job 1 (change schedule to every 5 minutes and disable)
    let req_update = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/cron/{}", site_id, job_1_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "schedule": "*/5 * * * *",
                "enabled": false
            }))
            .unwrap(),
        ))
        .unwrap();
    let res_update = ctx.app.clone().oneshot(req_update).await.unwrap();
    assert_eq!(res_update.status(), StatusCode::OK);
    let updated_job: Value =
        serde_json::from_slice(&res_update.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(updated_job["schedule"], "*/5 * * * *");
    assert_eq!(updated_job["systemd_calendar"], "*-*-* *:00/5:00");
    assert_eq!(updated_job["human_description"], "Setiap 5 menit");
    assert_eq!(updated_job["enabled"], false);

    // 8. Run Now on Job 1
    let req_run = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/cron/{}/run", site_id, job_1_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();
    let res_run = ctx.app.clone().oneshot(req_run).await.unwrap();
    assert_eq!(res_run.status(), StatusCode::OK);
    let run_data: Value =
        serde_json::from_slice(&res_run.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(run_data["cron_job_id"], job_1_id);
    assert_eq!(run_data["status"], "success");
    assert_eq!(run_data["exit_code"], 0);

    // 9. Fetch Run History
    let req_runs = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/cron/{}/runs", site_id, job_1_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res_runs = ctx.app.clone().oneshot(req_runs).await.unwrap();
    assert_eq!(res_runs.status(), StatusCode::OK);
    let runs_list: Value =
        serde_json::from_slice(&res_runs.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(runs_list.as_array().unwrap().len(), 1);

    // 10. Fetch Logs
    let req_logs = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/cron/{}/logs", site_id, job_1_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res_logs = ctx.app.clone().oneshot(req_logs).await.unwrap();
    assert_eq!(res_logs.status(), StatusCode::OK);
    let logs_data: Value =
        serde_json::from_slice(&res_logs.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(!logs_data["logs"].as_array().unwrap().is_empty());

    // 11. Delete Job 1
    let req_del = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/sites/{}/cron/{}", site_id, job_1_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();
    let res_del = ctx.app.clone().oneshot(req_del).await.unwrap();
    assert_eq!(res_del.status(), StatusCode::OK);

    // 12. Verify quota is now 2/3
    let req_list_after = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/cron", site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();
    let res_list_after = ctx.app.clone().oneshot(req_list_after).await.unwrap();
    let list_after: CronJobListResponse = serde_json::from_slice(
        &res_list_after
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(list_after.jobs.len(), 2);
    assert_eq!(list_after.quota.used, 2);
    assert_eq!(list_after.quota.limit, 3);
}
