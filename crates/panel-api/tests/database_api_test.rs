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
use panel_api::databases::{
    CreateDatabaseRequest, DatabaseActionResponse, DatabaseServerResponse, ListDatabasesResponse,
    RotatePasswordRequest, UpdateRemoteAccessRequest,
};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateDomain, CreatePlan, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{
    SqliteDomainRepository, SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository,
};
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    jobs: Arc<panel_api::jobs::JobRunner>,
    runner: Arc<MockSystemRunner>,
    app: axum::Router,
    #[allow(dead_code)]
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    alice_session: String,
    alice_csrf: String,
    #[allow(dead_code)]
    admin_session: String,
    #[allow(dead_code)]
    admin_csrf: String,
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

    let nginx_dir = tmp.path().join("etc/nginx");
    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"active\n", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");
    runner.set_response(
        "mariadb",
        0,
        b"mariadb  Ver 15.1 Distrib 10.11.8-MariaDB",
        b"",
    );
    runner.set_response("mariadb-dump", 0, b"", b"");
    runner.set_response("psql", 0, b"psql (PostgreSQL) 16.2", b"");
    runner.set_response("pg_dump", 0, b"", b"");
    runner.set_response("du", 0, b"2048\t/var/lib/mysql\n", b"");

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

    for _ in 0..50 {
        if sock_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let plan = plan_repo
        .create(&CreatePlan {
            name: "DatabasePlan".to_string(),
            description: Some("Database Plan".to_string()),
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(200),
            io_weight: Some(100),
            disk_limit_bytes: Some(10 * 1024 * 1024 * 1024),
            inode_limit: Some(100000),
            max_sites: Some(5),
            max_domains: Some(10),
            max_databases: Some(2), // Limit to 2 for quota test
            max_cron_jobs: Some(10),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(100 * 1024 * 1024),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create plan");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_hash = hash_password("AdminPass123!").expect("hash pass");
    let _admin = user_repo
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
    let alice = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            plan_id: Some(plan.id),
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
        pool: pool.clone(),
        environment: "testing".to_string(),
        auth_config,
        lockout_manager,
        audit_writer,
        job_runner: Arc::clone(&job_runner),
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
        "password": "AlicePass123!"
    });
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&login_body).expect("json")))
        .expect("req");

    let login_res = app.clone().oneshot(login_req).await.expect("login");
    assert_eq!(login_res.status(), StatusCode::OK);
    let cookie_val = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set cookie")
        .to_str()
        .expect("cookie str");
    let alice_session =
        extract_cookie_value(cookie_val, SESSION_COOKIE_NAME).expect("alice session");
    let login_json: serde_json::Value =
        serde_json::from_slice(&login_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let alice_csrf = login_json["csrf_token"].as_str().unwrap().to_string();

    // Login admin
    let admin_login_body = serde_json::json!({
        "username": "admin",
        "password": "AdminPass123!"
    });
    let admin_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&admin_login_body).expect("json"),
        ))
        .expect("req");

    let admin_res = app.clone().oneshot(admin_req).await.expect("login admin");
    assert_eq!(admin_res.status(), StatusCode::OK);
    let admin_cookie_val = admin_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set cookie")
        .to_str()
        .expect("cookie str");
    let admin_session =
        extract_cookie_value(admin_cookie_val, SESSION_COOKIE_NAME).expect("admin session");
    let admin_json: serde_json::Value =
        serde_json::from_slice(&admin_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let admin_csrf = admin_json["csrf_token"].as_str().unwrap().to_string();

    // Database tests start with an existing site; PHP provisioning has its own suite.
    let site_id = SqliteSiteRepository::new(pool.clone())
        .create(&CreateSite {
            user_id: alice.id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/example.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("seed existing Alice site for database tests")
        .id;
    SqliteDomainRepository::new(pool.clone())
        .create(&CreateDomain {
            site_id,
            name: "example.com".to_string(),
            unicode_name: "example.com".to_string(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .expect("seed primary domain for database tests");

    TestContext {
        jobs: job_runner,
        runner,
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        alice_session,
        alice_csrf,
        admin_session,
        admin_csrf,
        site_id,
    }
}

#[tokio::test]
async fn database_install_joins_active_job_and_persists_agent_output() {
    let ctx = setup_test_context().await;
    ctx.runner.set_response(
        "apt-get",
        0,
        b"Get:1 mariadb-server\nUnpacking mariadb-server\nSetting up mariadb-server\n",
        b"APT diagnostic\n",
    );
    ctx.runner
        .set_response("systemctl is-active mariadb", 0, b"active\n", b"");
    let make_request = || {
        Request::builder()
            .method("POST")
            .uri("/api/v1/databases/servers/install")
            .header(
                header::COOKIE,
                format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
            )
            .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"engine":"mariadb"}"#))
            .unwrap()
    };
    let (first, second) = tokio::join!(
        ctx.app.clone().oneshot(make_request()),
        ctx.app.clone().oneshot(make_request())
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let first: DatabaseActionResponse =
        serde_json::from_slice(&first.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let second: DatabaseActionResponse =
        serde_json::from_slice(&second.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(first.job_id, second.job_id);
    let id = first.job_id.unwrap();
    assert!(panel_api::jobs::process_one_job(&ctx.jobs).await.unwrap());
    let job = ctx.jobs.job_repo.find_by_id(&id).await.unwrap().unwrap();
    assert_eq!(job.status, "succeeded", "{:?}", job.error_message);
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/jobs/{id}/logs"))
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let events: Vec<panel_api::jobs::JobProgressEvent> =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let text = events
        .iter()
        .filter_map(|e| e.message.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Unpacking mariadb-server"));
    assert!(text.contains("APT diagnostic"));
    assert!(events.windows(2).all(|w| w[0].sequence < w[1].sequence));
    assert_eq!(events.last().unwrap().status, "succeeded");
    assert_eq!(
        ctx.runner
            .get_commands()
            .iter()
            .filter(|command| command.starts_with("apt-get install"))
            .count(),
        1
    );
}

#[tokio::test]
async fn php_install_streams_real_mock_output_and_failure_is_retained() {
    let ctx = setup_test_context().await;
    ctx.runner
        .set_file_content("/etc/os-release", b"ID=ubuntu\nVERSION_ID=24.04\n");
    let policy = panel_runtime::php::PhpVersion::Php84
        .base_packages()
        .iter()
        .map(|pkg| format!("{pkg}:\n  Candidate: 8.4.1-1\n"))
        .collect::<String>();
    ctx.runner
        .set_response("apt-cache", 0, policy.as_bytes(), b"");
    ctx.runner.set_response(
        "apt-get",
        0,
        b"Reading package lists...\nUnpacking PHP runtime\n",
        b"",
    );
    ctx.runner
        .set_response("systemctl is-enabled php8.4-fpm", 1, b"", b"masked\n");
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/php/versions/8.4/install")
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
                )
                .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job: panel_api::jobs::JobItem =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(panel_api::jobs::process_one_job(&ctx.jobs).await.unwrap());
    let finished = ctx
        .jobs
        .job_repo
        .find_by_id(&job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.status, "succeeded", "{:?}", finished.error_message);
    let events = ctx.jobs.job_repo.events_after(&job.id, 0).await.unwrap();
    assert!(events.iter().any(|e| {
        e.message
            .as_deref()
            .is_some_and(|m| m.contains("Unpacking PHP runtime"))
    }));
    ctx.runner
        .set_response("apt-get", 100, b"", b"E: fixture repository unavailable\n");
    let (failed, _) = ctx
        .jobs
        .enqueue_database_install(finished.user_id.unwrap(), "postgresql")
        .await
        .unwrap();
    assert!(panel_api::jobs::process_one_job(&ctx.jobs).await.unwrap());
    let events = ctx.jobs.job_repo.events_after(&failed.id, 0).await.unwrap();
    assert!(events.iter().any(|e| {
        e.message
            .as_deref()
            .is_some_and(|m| m.contains("fixture repository unavailable"))
    }));
    assert!(!events.iter().any(|e| e.status == "succeeded"));
}

#[tokio::test]
async fn test_database_server_list() {
    let ctx = setup_test_context().await;

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/databases/servers")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let servers: Vec<DatabaseServerResponse> = serde_json::from_slice(&body).unwrap();

    assert_eq!(servers.len(), 2);
    let mariadb = servers.iter().find(|s| s.engine == "mariadb").unwrap();
    assert!(mariadb.is_installed);
    assert_eq!(mariadb.port, 3306);

    let pg = servers.iter().find(|s| s.engine == "postgresql").unwrap();
    assert!(pg.is_installed);
    assert_eq!(pg.port, 5432);
}

#[tokio::test]
async fn test_database_site_list_checks_access_and_missing_site() {
    let ctx = setup_test_context().await;
    let admin = SqliteUserRepository::new(ctx.pool.clone())
        .find_by_username("admin")
        .await
        .unwrap()
        .unwrap();
    let foreign_site = SqliteSiteRepository::new(ctx.pool.clone())
        .create(&CreateSite {
            user_id: admin.id,
            kind: "static".to_string(),
            docroot: "/home/fixture/sites/foreign.test/public".to_string(),
            runtime: None,
            version: None,
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .unwrap();
    for (site_id, expected) in [
        (foreign_site.id, StatusCode::FORBIDDEN),
        (999999, StatusCode::NOT_FOUND),
    ] {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/databases?site_id={site_id}"))
            .header(
                header::COOKIE,
                format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
            )
            .body(Body::empty())
            .unwrap();
        let response = ctx.app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn test_database_lifecycle_and_quota() {
    let ctx = setup_test_context().await;

    // 1. Create first database (MariaDB) with auto prefix prepending
    let create_payload = CreateDatabaseRequest {
        engine: "mariadb".to_string(),
        name: "wordpress".to_string(), // should become p_alice_wordpress
        db_user: Some("wpuser".to_string()), // should become p_alice_wpuser
        password: None,                // should auto-generate
        site_id: Some(ctx.site_id),
        remote_access: Some(false),
        allowed_hosts: Some("localhost".to_string()),
        collation: None,
        character_set: None,
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/databases")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&create_payload).unwrap()))
        .unwrap();

    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let val: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(val["database"]["name"], "p_alice_wordpress");
    assert_eq!(val["database"]["db_user"], "p_alice_wpuser");
    assert_eq!(val["database"]["engine"], "mariadb");
    assert_eq!(val["database"]["remote_access"], false);
    assert!(val["generated_password"].is_string());
    let db1_id = val["database"]["id"].as_i64().unwrap();

    // 2. Create second database (PostgreSQL)
    let create_payload2 = CreateDatabaseRequest {
        engine: "postgresql".to_string(),
        name: "analytics".to_string(),
        db_user: None,
        password: Some("Secr3tP@ssword!".to_string()),
        site_id: None,
        remote_access: Some(false),
        allowed_hosts: None,
        collation: None,
        character_set: None,
    };

    let req2 = Request::builder()
        .method("POST")
        .uri("/api/v1/databases")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&create_payload2).unwrap()))
        .unwrap();

    let res2 = ctx.app.clone().oneshot(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::CREATED);
    let val2: serde_json::Value =
        serde_json::from_slice(&res2.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(val2["database"]["name"], "p_alice_analytics");
    assert_eq!(val2["database"]["engine"], "postgresql");

    // 3. Try creating third database -> should hit quota (plan max_databases = 2)
    let create_payload3 = CreateDatabaseRequest {
        engine: "mariadb".to_string(),
        name: "exceed".to_string(),
        db_user: None,
        password: None,
        site_id: None,
        remote_access: None,
        allowed_hosts: None,
        collation: None,
        character_set: None,
    };

    let req3 = Request::builder()
        .method("POST")
        .uri("/api/v1/databases")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&create_payload3).unwrap()))
        .unwrap();

    let res3 = ctx.app.clone().oneshot(req3).await.unwrap();
    assert_eq!(res3.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // 4. List databases and verify quota info
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/databases")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let list_res = ctx.app.clone().oneshot(list_req).await.unwrap();
    assert_eq!(list_res.status(), StatusCode::OK);
    let list_dto: ListDatabasesResponse =
        serde_json::from_slice(&list_res.into_body().collect().await.unwrap().to_bytes()).unwrap();

    assert_eq!(list_dto.total, 2);
    assert_eq!(list_dto.databases.len(), 2);
    assert_eq!(list_dto.plan_max_databases, 2);
    assert_eq!(list_dto.used_databases, 2);

    // Site-filtered admin reads must use the owner's tenant-wide quota, not admin's.
    let site_list_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/databases?site_id={}", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let site_list_res = ctx.app.clone().oneshot(site_list_req).await.unwrap();
    assert_eq!(site_list_res.status(), StatusCode::OK);
    let site_list_json: serde_json::Value = serde_json::from_slice(
        &site_list_res
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(site_list_json["total"], 1);
    assert_eq!(site_list_json["plan_max_databases"], 2);
    assert_eq!(site_list_json["used_databases"], 2);
    assert_eq!(site_list_json["databases"][0]["id"], db1_id);
    assert!(site_list_json.get("quota").is_none());

    // 5. Rotate password for db1
    let rotate_payload = RotatePasswordRequest {
        new_password: Some("NewStrongPassw0rd!".to_string()),
    };
    let rotate_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/databases/{db1_id}/rotate-password"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&rotate_payload).unwrap()))
        .unwrap();

    let rotate_res = ctx.app.clone().oneshot(rotate_req).await.unwrap();
    assert_eq!(rotate_res.status(), StatusCode::OK);

    // 6. Update remote access for db1
    let remote_payload = UpdateRemoteAccessRequest {
        remote_access: true,
        allowed_hosts: Some("192.168.1.100,10.0.0.%".to_string()),
    };
    let remote_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/databases/{db1_id}/remote-access"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&remote_payload).unwrap()))
        .unwrap();

    let remote_res = ctx.app.clone().oneshot(remote_req).await.unwrap();
    assert_eq!(remote_res.status(), StatusCode::OK);
    let val_remote: serde_json::Value =
        serde_json::from_slice(&remote_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(val_remote["remote_access"], true);
    assert_eq!(val_remote["allowed_hosts"], "192.168.1.100,10.0.0.%");

    // 7. Enqueue export job
    let export_payload = serde_json::json!({
        "export_file_path": "/home/p_alice/backups/export.sql"
    });
    let export_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/databases/{db1_id}/export"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&export_payload).unwrap()))
        .unwrap();

    let export_res = ctx.app.clone().oneshot(export_req).await.unwrap();
    assert_eq!(export_res.status(), StatusCode::ACCEPTED);
    let export_action: DatabaseActionResponse =
        serde_json::from_slice(&export_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert!(export_action.success);
    assert!(export_action.job_id.is_some());

    // 8. Delete db1
    let delete_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/databases/{db1_id}"))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();

    let delete_res = ctx.app.clone().oneshot(delete_req).await.unwrap();
    assert_eq!(delete_res.status(), StatusCode::OK);

    // 9. Verify count decreased to 1 and remaining quota is 1
    let list_req2 = Request::builder()
        .method("GET")
        .uri("/api/v1/databases")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .unwrap();

    let list_res2 = ctx.app.clone().oneshot(list_req2).await.unwrap();
    let list_dto2: ListDatabasesResponse =
        serde_json::from_slice(&list_res2.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(list_dto2.total, 1);
    assert_eq!(list_dto2.used_databases, 1);
}
