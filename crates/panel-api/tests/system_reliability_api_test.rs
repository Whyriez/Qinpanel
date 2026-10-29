#![allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::hardening::HardeningManager;
use panel_agent::nftables::NftablesManager;
use panel_agent::quota::QuotaManager;
use panel_agent::reliability::ReliabilityManager;
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
use panel_core::auth::hash_password;
use panel_core::doctor::DoctorReport;
use panel_core::reconcile::ReconcileReport;
use panel_core::system_backup::SystemBackupSummary;
use panel_db::models::{CreateJob, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use panel_db::repositories::job_repo::SqliteJobRepository;
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
    pool: panel_db::pool::DbPool,
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

    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("systemctl", 0, b"active (running)", b"");
    runner.set_response(
        "nginx",
        0,
        b"nginx: the configuration file syntax is ok",
        b"",
    );
    runner.set_response("tar", 0, b"tar (GNU tar) 1.34", b"");

    let nft_rules_file = tmp.path().join("panel-loopback-guard.nft");
    let user_mgr = Arc::new(UserLifecycleManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let slice_mgr = Arc::new(SliceManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let quota_mgr = Arc::new(QuotaManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let resource_reader = Arc::new(CgroupReader::default());
    let hardening_mgr = Arc::new(HardeningManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let webserver_mgr = Arc::new(NginxManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let nftables_mgr = Arc::new(NftablesManager::with_path(
        runner.clone() as Arc<dyn SystemRunner>,
        &nft_rules_file,
    ));
    let reliability_mgr = Arc::new(ReliabilityManager::new(
        runner.clone() as Arc<dyn SystemRunner>,
        tmp.path().join("agent_base"),
        nftables_mgr.clone(),
    ));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let server = Arc::new(
        AgentServer::with_all_managers(
            agent_cfg,
            user_mgr,
            slice_mgr,
            quota_mgr,
            resource_reader,
            hardening_mgr,
            webserver_mgr,
        )
        .with_nftables_manager(nftables_mgr)
        .with_reliability_manager(reliability_mgr),
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let srv_clone = server.clone();
    tokio::spawn(async move {
        let _ = srv_clone.run(shutdown_rx).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init_pool");
    run_migrations(&pool).await.expect("run_migrations");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_hash = hash_password("SuperAdminPassword123!").expect("hash");
    let _admin_user = user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            password_hash: admin_hash,
            role: "admin".to_string(),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create admin");

    let agent_client = Arc::new(AgentClient::new(sock_path));
    let lockout_mgr = Arc::new(LockoutManager::new(5, 300));
    let auth_cfg = AuthConfig {
        session_duration_secs: 3600,
        ..Default::default()
    };

    let state = AppState::new(
        agent_client,
        pool.clone(),
        "test".to_string(),
        auth_cfg,
        lockout_mgr,
    );

    let app = create_router(state);

    // Login admin
    let login_body = serde_json::to_string(&serde_json::json!({
        "username": "admin",
        "password": "SuperAdminPassword123!"
    }))
    .unwrap();

    let login_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(login_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(login_res.status(), StatusCode::OK, "admin login failed");
    let mut admin_session = String::new();

    for (k, v) in login_res.headers() {
        if k == header::SET_COOKIE {
            let cookie_str = v.to_str().unwrap();
            if let Some(s) = extract_cookie_value(cookie_str, SESSION_COOKIE_NAME) {
                admin_session = s;
            }
        }
    }
    assert!(
        !admin_session.is_empty(),
        "login omitted the session cookie"
    );

    // Login returns the CSRF token in JSON; x-csrf-token is a request header.
    let login_body = login_res.into_body().collect().await.unwrap().to_bytes();
    let login_json: serde_json::Value = serde_json::from_slice(&login_body).unwrap();
    let admin_csrf = login_json["csrf_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .expect("login omitted the CSRF token")
        .to_owned();

    TestContext {
        app,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        pool,
        runner,
    }
}

#[tokio::test]
async fn test_doctor_api_check_and_fix() {
    let ctx = setup_test_context().await;

    // Insert an interrupted job to verify detection
    let job_repo = SqliteJobRepository::new(ctx.pool.clone());
    job_repo
        .create(&CreateJob {
            id: "stuck-job-1".to_string(),
            user_id: None,
            name: "test-task".to_string(),
            payload_json: "{}".to_string(),
            max_retries: 3,
        })
        .await
        .unwrap();
    job_repo
        .update_status("stuck-job-1", "running", None, None)
        .await
        .unwrap();

    // 1. GET /api/v1/system/doctor
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/doctor")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let report: DoctorReport = serde_json::from_slice(&body).unwrap();

    // Verify checks contain SQLite check and interrupted job
    assert!(report.checks.iter().any(|c| c.id == "database_integrity"));
    let job_check = report
        .checks
        .iter()
        .find(|c| c.id == "interrupted_jobs")
        .unwrap();
    assert_eq!(
        job_check.status,
        panel_core::doctor::DoctorCheckStatus::Warn
    );
    assert!(job_check.fixable);

    // 2. POST /api/v1/system/doctor/fix
    let fix_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/doctor/fix")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let fix_res = ctx.app.clone().oneshot(fix_req).await.unwrap();
    assert_eq!(fix_res.status(), StatusCode::OK);

    let fix_body = fix_res.into_body().collect().await.unwrap().to_bytes();
    let fixed_report: DoctorReport = serde_json::from_slice(&fix_body).unwrap();

    let fixed_job_check = fixed_report
        .checks
        .iter()
        .find(|c| c.id == "interrupted_jobs")
        .unwrap();
    assert_eq!(
        fixed_job_check.status,
        panel_core::doctor::DoctorCheckStatus::Pass
    );

    // Verify the job is actually failed in DB
    let job = job_repo.find_by_id("stuck-job-1").await.unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(
        !ctx.runner
            .get_commands()
            .iter()
            .any(|command| command == "systemctl restart nginx"
                || command == "systemctl start nginx")
    );
}

#[tokio::test]
async fn doctor_fix_rejects_missing_csrf_before_any_host_mutation() {
    let ctx = setup_test_context().await;
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/system/doctor/fix")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(ctx.runner.get_commands().is_empty());
}

#[tokio::test]
async fn doctor_fix_returns_actionable_agent_failure_instead_of_a_success_report() {
    let ctx = setup_test_context().await;
    ctx.runner
        .set_response("nft", 1, b"", b"Unable to initialize Netlink socket");
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/system/doctor/fix")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(body["code"], "nftables_netlink_unavailable");
    assert!(
        !ctx.runner
            .get_commands()
            .iter()
            .any(|command| command == "systemctl restart nginx"
                || command == "systemctl start nginx")
    );
}

#[tokio::test]
async fn doctor_fix_does_not_claim_success_when_agent_is_offline() {
    let ctx = setup_test_context().await;
    ctx._shutdown_tx.send(()).expect("shutdown mock agent");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/system/doctor/fix")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert!(
        body["error"]
            .as_str()
            .expect("error")
            .contains("Diagnosis agent belum tersedia")
    );
    assert!(ctx.runner.get_commands().is_empty());
}

#[tokio::test]
async fn doctor_fix_enables_nginx_boot_without_restarting_a_running_proxy() {
    let ctx = setup_test_context().await;
    ctx.runner.set_response(
        "systemctl show --property=UnitFileState --value nginx.service",
        0,
        b"disabled",
        b"",
    );
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/system/doctor/fix")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let commands = ctx.runner.get_commands();
    assert!(
        commands
            .iter()
            .any(|command| command == "systemctl enable nginx.service")
    );
    assert!(
        !commands
            .iter()
            .any(|command| command == "systemctl restart nginx"
                || command == "systemctl start nginx")
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let report: DoctorReport = serde_json::from_slice(&bytes).expect("report");
    // This fixture deliberately keeps returning disabled. The refreshed report
    // must retain the warning rather than assume enablement from a successful RPC.
    assert_eq!(
        report
            .checks
            .iter()
            .find(|check| check.id == "nginx_boot_enabled")
            .expect("boot check")
            .status,
        panel_core::doctor::DoctorCheckStatus::Warn
    );
}

#[tokio::test]
async fn doctor_fix_syncs_authoritative_tenant_ports_without_restarting_nginx() {
    let ctx = setup_test_context().await;
    let users = SqliteUserRepository::new(ctx.pool.clone());
    let owner = users
        .create(&CreateUser {
            username: "doctortenant".into(),
            password_hash: "fixture".into(),
            email: None,
            role: "user".into(),
            linux_username: Some("p_doctortenant".into()),
            linux_uid: Some(10002),
            plan_id: None,
        })
        .await
        .expect("owner");
    sqlx::query("INSERT INTO sites(user_id,kind,docroot,status,settings_json,created_at,updated_at) VALUES(?,'nodejs','/fixture','active',?,1,1)")
        .bind(owner.id).bind(r#"{"port":20051}"#).execute(&ctx.pool).await.expect("site");
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/system/doctor/fix")
                .header(
                    header::COOKIE,
                    format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(ctx.runner.recorded_actions().iter().any(|action| matches!(action,
        panel_agent::system::MockAction::WriteFile { content, .. }
        if String::from_utf8_lossy(content).contains("20051") && String::from_utf8_lossy(content).contains("10002"))));
    assert!(
        !ctx.runner
            .get_commands()
            .iter()
            .any(|command| command == "systemctl restart nginx"
                || command == "systemctl start nginx")
    );
}

#[tokio::test]
async fn test_reconcile_api_get_and_apply() {
    let ctx = setup_test_context().await;

    // 1. GET /api/v1/system/reconcile
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/reconcile")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let report: ReconcileReport = serde_json::from_slice(&body).unwrap();
    assert!(report.is_clean);

    // 2. POST /api/v1/system/reconcile
    let apply_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/reconcile")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let apply_res = ctx.app.clone().oneshot(apply_req).await.unwrap();
    assert_eq!(apply_res.status(), StatusCode::OK);

    let apply_body = apply_res.into_body().collect().await.unwrap().to_bytes();
    let applied_report: ReconcileReport = serde_json::from_slice(&apply_body).unwrap();
    assert!(applied_report.is_clean);
}

#[tokio::test]
async fn test_system_backup_api_lifecycle() {
    let ctx = setup_test_context().await;

    // 1. POST /api/v1/system/backups
    let create_payload = serde_json::to_string(&serde_json::json!({
        "filename": "panel-backup-api-test.tar.gz"
    }))
    .unwrap();

    let create_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/backups")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(create_payload))
        .unwrap();

    let create_res = ctx.app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(create_res.status(), StatusCode::CREATED);

    let create_body = create_res.into_body().collect().await.unwrap().to_bytes();
    let summary: SystemBackupSummary = serde_json::from_slice(&create_body).unwrap();
    assert_eq!(summary.filename, "panel-backup-api-test.tar.gz");

    // 2. GET /api/v1/system/backups
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/backups")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let list_res = ctx.app.clone().oneshot(list_req).await.unwrap();
    assert_eq!(list_res.status(), StatusCode::OK);

    let list_body = list_res.into_body().collect().await.unwrap().to_bytes();
    let backups: Vec<SystemBackupSummary> = serde_json::from_slice(&list_body).unwrap();
    assert!(!backups.is_empty());
    assert!(
        backups
            .iter()
            .any(|b| b.filename == "panel-backup-api-test.tar.gz")
    );
}
