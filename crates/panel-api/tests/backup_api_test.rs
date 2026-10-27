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
use panel_api::backups::{
    BackupListResponse, CreateBackupRequest, RestoreBackupRequest, UpdateScheduleRequest,
};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreatePlan, CreateSite, CreateUser, SiteBackupScheduleModel};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository};
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
    alice_session: String,
    alice_csrf: String,
    site_id: i64,
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
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");
    runner.set_response("tar", 0, b"", b"");
    runner.set_response("zstd", 0, b"", b"");
    runner.set_response("gzip", 0, b"", b"");
    runner.set_response("mariadb-dump", 0, b"", b"");
    runner.set_response("mysqldump", 0, b"", b"");
    runner.set_response("pg_dump", 0, b"", b"");
    runner.set_response("mariadb", 0, b"", b"");
    runner.set_response("psql", 0, b"", b"");

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
    let webserver_manager = Arc::new(NginxManager::new(
        Arc::clone(&runner) as Arc<dyn SystemRunner>
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

    let agent_client = Arc::new(AgentClient::new(&sock_path));
    for _ in 0..50 {
        if agent_client.ping().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let db_url = format!("sqlite://{}", db_file.to_str().expect("db file to str"));
    let pool = init_pool(&db_url).await.expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let plan = plan_repo
        .create(&CreatePlan {
            name: "BackupPlan".to_string(),
            description: Some("Backup Plan".to_string()),
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(200),
            io_weight: Some(100),
            disk_limit_bytes: Some(10 * 1024 * 1024 * 1024),
            inode_limit: Some(100000),
            max_sites: Some(5),
            max_domains: Some(10),
            max_databases: Some(5),
            max_cron_jobs: Some(10),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(100 * 1024 * 1024),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create test plan");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let alice = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: hash_password("Password123!").expect("hash alice password"),
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            linux_username: Some("p_alice".to_string()),
            linux_uid: Some(10001),
            plan_id: Some(plan.id),
        })
        .await
        .expect("create alice");

    let site_repo = SqliteSiteRepository::new(pool.clone());
    let site = site_repo
        .create(&CreateSite {
            user_id: alice.id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/mysite.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    let domain_repo = panel_db::repositories::SqliteDomainRepository::new(pool.clone());
    let _domain = domain_repo
        .create(&panel_db::models::CreateDomain {
            site_id: site.id,
            name: "mysite.com".to_string(),
            unicode_name: "mysite.com".to_string(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .expect("create primary domain");

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };
    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
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

    // Login alice
    let login_body = serde_json::json!({
        "username": "alice",
        "password": "Password123!"
    });
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&login_body).expect("serialize login body"),
        ))
        .expect("build login request");

    let login_res = app
        .clone()
        .oneshot(login_req)
        .await
        .expect("execute login request");
    assert_eq!(login_res.status(), StatusCode::OK);

    let cookie_header = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set cookie header")
        .to_str()
        .expect("cookie to str");
    let alice_session =
        extract_cookie_value(cookie_header, SESSION_COOKIE_NAME).expect("session cookie");

    // CSRF token
    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_session),
        )
        .body(Body::empty())
        .expect("build csrf request");

    let csrf_res = app
        .clone()
        .oneshot(csrf_req)
        .await
        .expect("execute csrf request");
    assert_eq!(csrf_res.status(), StatusCode::OK);
    let csrf_body = csrf_res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&csrf_body).expect("parse json");
    let alice_csrf = csrf_json["csrf_token"]
        .as_str()
        .expect("csrf string")
        .to_string();

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        alice_session,
        alice_csrf,
        site_id: site.id,
    }
}

#[tokio::test]
async fn test_backup_list_create_and_delete() {
    let ctx = setup_test_context().await;

    // 1. Initially empty backups
    let list_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/backups", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build list req");

    let list_res = ctx
        .app
        .clone()
        .oneshot(list_req)
        .await
        .expect("exec list req");
    assert_eq!(list_res.status(), StatusCode::OK);

    let body_bytes = list_res
        .into_body()
        .collect()
        .await
        .expect("read bytes")
        .to_bytes();
    let parsed: BackupListResponse = serde_json::from_slice(&body_bytes).expect("parse response");
    assert_eq!(parsed.count, 0);
    assert_eq!(parsed.total_bytes_used, 0);

    // 2. Create backup
    let create_payload = CreateBackupRequest {
        label: Some("Manual snapshot before update".to_string()),
        include_files: true,
        include_databases: true,
        format: Some("tar.zst".to_string()),
    };

    let create_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/backups", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&create_payload).unwrap()))
        .expect("build create req");

    let create_res = ctx
        .app
        .clone()
        .oneshot(create_req)
        .await
        .expect("exec create req");
    assert_eq!(create_res.status(), StatusCode::ACCEPTED);

    let create_body = create_res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let create_json: serde_json::Value = serde_json::from_slice(&create_body).expect("parse json");
    assert!(create_json["job_id"].is_string());
    let backup_id = create_json["backup"]["id"].as_i64().expect("backup id");

    // 3. Get single backup
    let get_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/backups/{}",
            ctx.site_id, backup_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build get req");

    let get_res = ctx
        .app
        .clone()
        .oneshot(get_req)
        .await
        .expect("exec get req");
    assert_eq!(get_res.status(), StatusCode::OK);

    // 4. Delete backup
    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/v1/sites/{}/backups/{}",
            ctx.site_id, backup_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("build del req");

    let del_res = ctx
        .app
        .clone()
        .oneshot(del_req)
        .await
        .expect("exec del req");
    assert_eq!(del_res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_backup_schedule_and_restore() {
    let ctx = setup_test_context().await;

    // 1. Initial schedule is null/empty
    let get_sched_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/backups/schedule", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("build get sched req");

    let get_sched_res = ctx
        .app
        .clone()
        .oneshot(get_sched_req)
        .await
        .expect("exec get sched req");
    assert_eq!(get_sched_res.status(), StatusCode::OK);

    // 2. Update schedule
    let sched_payload = UpdateScheduleRequest {
        is_enabled: true,
        frequency: "daily".to_string(),
        time_hour: 3,
        time_minute: 30,
        retention_count: 7,
        include_files: true,
        include_databases: true,
        storage_destination: Some("local".to_string()),
        s3_endpoint: None,
        s3_bucket: None,
        s3_region: None,
        s3_access_key: None,
        s3_secret_key: None,
        s3_prefix: None,
    };

    let put_sched_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/backups/schedule", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&sched_payload).unwrap()))
        .expect("build put sched req");

    let put_sched_res = ctx
        .app
        .clone()
        .oneshot(put_sched_req)
        .await
        .expect("exec put sched req");
    assert_eq!(put_sched_res.status(), StatusCode::OK);

    let put_sched_bytes = put_sched_res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let saved_sched: SiteBackupScheduleModel =
        serde_json::from_slice(&put_sched_bytes).expect("parse sched");
    assert!(saved_sched.is_enabled);
    assert_eq!(saved_sched.frequency, "daily");
    assert_eq!(saved_sched.time_hour, 3);
    assert_eq!(saved_sched.retention_count, 7);

    // 3. Create a backup directly to test restore
    let backup_repo = panel_db::repositories::SqliteBackupRepository::new(ctx.pool.clone());
    let b = backup_repo
        .create_backup(&panel_db::models::CreateBackupInput {
            site_id: ctx.site_id,
            user_id: 1,
            filename: "backup_test.tar.zst".to_string(),
            filepath: "/home/p_alice/backups/backup_test.tar.zst".to_string(),
            size_bytes: 1024,
            label: Some("Snapshot".to_string()),
            includes_files: true,
            includes_databases: true,
            status: "completed".to_string(),
            storage_type: "local".to_string(),
            s3_key: None,
            checksum_sha256: None,
        })
        .await
        .expect("create backup");

    // 4. Trigger restore
    let restore_payload = RestoreBackupRequest {
        target_subpath: None,
        restore_files: true,
        restore_databases: true,
    };

    let restore_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/sites/{}/backups/{}/restore",
            ctx.site_id, b.id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&restore_payload).unwrap()))
        .expect("build restore req");

    let restore_res = ctx
        .app
        .clone()
        .oneshot(restore_req)
        .await
        .expect("exec restore req");
    assert_eq!(restore_res.status(), StatusCode::ACCEPTED);

    let restore_bytes = restore_res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let restore_json: serde_json::Value =
        serde_json::from_slice(&restore_bytes).expect("parse json");
    assert!(restore_json["job_id"].is_string());
}
