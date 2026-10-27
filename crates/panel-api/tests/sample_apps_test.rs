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
use panel_api::sites::{CreateStaticSiteRequest, SiteResponse};
use panel_api::ssl::SslStatusResponse;
use panel_core::auth::hash_password;
use panel_db::models::{CreatePlan, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqlitePlanRepository, SqliteUserRepository};
use panel_proto::{
    NodeSiteApplyRequest, NodeSiteEntrypoint, NodeSiteStatusRequest, PythonSiteApplyRequest,
    PythonSiteEntrypoint, PythonSiteFramework, PythonSiteServer, PythonSiteStatusRequest,
    SiteProvisionRequest, SiteStarter,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestContext {
    app: axum::Router,
    #[allow(dead_code)]
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    _test_guard: tokio::sync::MutexGuard<'static, ()>,
    admin_session: String,
    admin_csrf: String,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    bob_csrf: String,
    alice_id: i64,
    #[allow(dead_code)]
    bob_id: i64,
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

async fn login_helper(app: &axum::Router, username: &str, pass: &str) -> (String, String) {
    let payload = serde_json::json!({ "username": username, "password": pass });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    let session_val = extract_cookie_value(cookie, SESSION_COOKIE_NAME).unwrap();

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body).unwrap();
    let csrf_val = body_json["csrf_token"].as_str().unwrap().to_string();

    (session_val, csrf_val)
}

fn configure_sample_runtime_runner(runner: &MockSystemRunner) {
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response(
        "systemctl",
        0,
        b"ActiveState=active\nSubState=running\nMainPID=42\n",
        b"",
    );
    runner.set_response("curl", 0, b"200", b"");
    runner.set_response("runuser", 0, b"200", b"");
    runner.add_existing_path("/opt/panel/runtimes/node/22.14.0/bin/node");
    runner.add_existing_path("/opt/panel/runtimes/python/3.12.9/bin/python3");
    // Each context has its own database and creates at most five sample sites.
    for id in 1..=5 {
        runner.add_existing_path(format!("/run/panel/python/site_{id}/app.sock"));
    }
    runner.set_response("systemd-run", 0, b"ok", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");
    runner.set_response("uv", 0, b"Installed Python 3.12.9\n", b"");
}

#[test]
fn sample_runtime_fixture_supports_provisioning_and_readiness() {
    let runner = Arc::new(MockSystemRunner::new());
    configure_sample_runtime_runner(&runner);
    let manager = NginxManager::new(Arc::clone(&runner) as Arc<dyn SystemRunner>);
    let provision = |id, domain: &str, starter, content: &str| {
        let app_dir = format!("/home/p_alice/sites/{domain}/app");
        manager
            .provision_site(SiteProvisionRequest {
                site_id: id,
                username: "p_alice".into(),
                uid: 10001,
                home_dir: "/home/p_alice".into(),
                docroot: app_dir.clone(),
                web_user: Some("www-data".into()),
                index_content: Some(content.into()),
                starter,
            })
            .expect("provision sample starter using mock runner");
        app_dir
    };

    for (id, domain, port) in [
        (1, "express.alice.com", 20051),
        (2, "nextjs.alice.com", 20052),
    ] {
        let app_dir = provision(id, domain, SiteStarter::Nodejs, "// sample server");
        let site_id = format!("site_{id}");
        panel_agent::node_site::apply_node_site(
            runner.as_ref(),
            &NodeSiteApplyRequest {
                site_id: site_id.clone(),
                username: "p_alice".into(),
                uid: 10001,
                node_version: "22.14.0".into(),
                app_dir,
                port,
                entrypoint: NodeSiteEntrypoint::NodeScript("server.js".into()),
                env_vars: BTreeMap::new(),
                auto_restart: true,
                env_file_path: None,
                unit_path: None,
                base_runtime_dir: None,
            },
        )
        .expect("apply sample Node site using mock runner");
        let status = panel_agent::node_site::status_node_site(
            runner.as_ref(),
            &NodeSiteStatusRequest {
                site_id,
                port: Some(port),
                probe_path: Some("/".into()),
            },
        )
        .expect("query sample Node readiness using mock runner");
        assert!(status.active && status.healthy, "{domain}: {status:?}");
        assert_eq!(status.probe_status_code, Some(200));
    }

    for (id, domain, starter, framework, server_type, entrypoint) in [
        (
            3,
            "django.alice.com",
            SiteStarter::PythonDjango,
            PythonSiteFramework::Django,
            PythonSiteServer::GunicornWsgi,
            PythonSiteEntrypoint::Wsgi("myproject.wsgi:application".into()),
        ),
        (
            4,
            "fastapi.alice.com",
            SiteStarter::PythonMain,
            PythonSiteFramework::FastApi,
            PythonSiteServer::UvicornAsgi,
            PythonSiteEntrypoint::Asgi("main:app".into()),
        ),
        (
            5,
            "flask.alice.com",
            SiteStarter::PythonApp,
            PythonSiteFramework::Flask,
            PythonSiteServer::GunicornWsgi,
            PythonSiteEntrypoint::Wsgi("app:app".into()),
        ),
    ] {
        let app_dir = provision(id, domain, starter, "# sample application");
        let site_id = format!("site_{id}");
        let socket_path = format!("/run/panel/python/{site_id}/app.sock");
        panel_agent::python_site::apply_python_site(
            runner.as_ref(),
            &PythonSiteApplyRequest {
                site_id: site_id.clone(),
                username: "p_alice".into(),
                uid: 10001,
                python_version: "3.12.9".into(),
                venv_dir: format!("{app_dir}/.venv"),
                app_dir,
                socket_path: socket_path.clone(),
                framework,
                server_type,
                entrypoint,
                workers: 2,
                env_vars: BTreeMap::new(),
                auto_restart: true,
                env_file_path: None,
                unit_path: None,
                base_runtime_dir: None,
            },
        )
        .expect("apply sample Python site using mock runner");
        let status = panel_agent::python_site::status_python_site(
            runner.as_ref(),
            &PythonSiteStatusRequest {
                site_id,
                socket_path: Some(socket_path),
            },
        )
        .expect("query sample Python readiness using mock runner");
        assert!(
            status.active && status.socket_exists && status.healthy,
            "{domain}: {status:?}"
        );
    }
}

async fn created_site_response(response: axum::response::Response) -> SiteResponse {
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "Sample site creation failed: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).expect("created site response")
}

async fn setup_test_context() -> TestContext {
    let guard = TEST_MUTEX.lock().await;
    let tmp = tempdir().expect("tempdir");
    let db_file = tmp.path().join("panel_sample_apps.db");

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
    configure_sample_runtime_runner(&runner);

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
        let _ = server.run(rx).await;
    });

    let start = std::time::Instant::now();
    while !sock_path.exists() && start.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let default_plan = plan_repo
        .create(&CreatePlan {
            name: "Standard Sample Apps Plan".to_string(),
            description: Some("Hosting plan with SSRF external upstream blocked".to_string()),
            cpu_quota_percent: Some(100),
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(100),
            io_weight: Some(100),
            disk_limit_bytes: Some(10 * 1024 * 1024 * 1024),
            inode_limit: Some(200_000),
            max_sites: Some(10),
            max_domains: Some(20),
            max_databases: Some(5),
            max_cron_jobs: Some(5),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(64 * 1024 * 1024),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create plan");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let pass_hash = hash_password("Secret123!").expect("hash pass");

    let _admin = user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: pass_hash.clone(),
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            linux_uid: Some(10000),
            linux_username: Some("admin".to_string()),
            plan_id: None,
        })
        .await
        .expect("create admin");

    let alice = user_repo
        .create(&CreateUser {
            username: "p_alice".to_string(),
            password_hash: pass_hash.clone(),
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            linux_uid: Some(10001),
            linux_username: Some("p_alice".to_string()),
            plan_id: Some(default_plan.id),
        })
        .await
        .expect("create alice");

    let bob = user_repo
        .create(&CreateUser {
            username: "p_bob".to_string(),
            password_hash: pass_hash.clone(),
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            linux_uid: Some(10002),
            linux_username: Some("p_bob".to_string()),
            plan_id: Some(default_plan.id),
        })
        .await
        .expect("create bob");

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };

    let lockout_mgr = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(&sock_path));
    let app_state = AppState::new(
        agent_client,
        pool.clone(),
        "development".to_string(),
        auth_config,
        lockout_mgr,
    );
    let app = create_router(app_state);

    let (admin_session, admin_csrf) = login_helper(&app, "admin", "Secret123!").await;
    let (alice_session, alice_csrf) = login_helper(&app, "p_alice", "Secret123!").await;
    let (bob_session, bob_csrf) = login_helper(&app, "p_bob", "Secret123!").await;

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        _test_guard: guard,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        bob_session,
        bob_csrf,
        alice_id: alice.id,
        bob_id: bob.id,
    }
}

async fn issue_self_signed_ssl_for_site(ctx: &TestContext, site_id: i64) {
    let ssl_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/ssl/self-signed", site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(ssl_req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "Issue self-signed SSL for site {} failed",
        site_id
    );

    // Verify SSL status returns has_ssl: true
    let status_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/ssl", site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();

    let status_res = ctx.app.clone().oneshot(status_req).await.unwrap();
    assert_eq!(status_res.status(), StatusCode::OK);
    let status_bytes = status_res.into_body().collect().await.unwrap().to_bytes();
    let status_data: SslStatusResponse = serde_json::from_slice(&status_bytes).unwrap();
    assert!(
        status_data.has_ssl,
        "Site {} must have has_ssl = true after certificate issuance",
        site_id
    );
}

#[tokio::test]
async fn test_all_five_sample_applications_lifecycle_and_https_ssl() {
    let ctx = setup_test_context().await;

    // 1. Express App (Node.js)
    let express_payload = CreateStaticSiteRequest {
        domain: "express.alice.com".to_string(),
        docroot: Some("/home/p_alice/sites/express.alice.com/app".to_string()),
        spa_fallback: Some(false),
        force_https: Some(true),
        user_id: Some(ctx.alice_id),
        kind: Some("nodejs".to_string()),
        node_version: Some("22.14.0".to_string()),
        framework: Some("express".to_string()),
        port: Some(20051),
        app_dir: Some("/home/p_alice/sites/express.alice.com/app".to_string()),
        entrypoint_type: Some("script".to_string()),
        entrypoint_value: Some("server.js".to_string()),
        auto_restart: Some(true),
        env_vars: Some(BTreeMap::from([(
            "NODE_ENV".to_string(),
            "production".to_string(),
        )])),
        ..Default::default()
    };
    let express_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&express_payload).unwrap()))
        .unwrap();
    let express_res = ctx.app.clone().oneshot(express_req).await.unwrap();
    let express_site = created_site_response(express_res).await;
    assert_eq!(express_site.kind, "nodejs");
    assert_eq!(express_site.runtime.as_deref(), Some("nodejs"));
    issue_self_signed_ssl_for_site(&ctx, express_site.id).await;

    // 2. Next.js Standalone App (Node.js)
    let nextjs_payload = CreateStaticSiteRequest {
        domain: "nextjs.alice.com".to_string(),
        docroot: Some("/home/p_alice/sites/nextjs.alice.com/app".to_string()),
        spa_fallback: Some(false),
        force_https: Some(true),
        user_id: Some(ctx.alice_id),
        kind: Some("nodejs".to_string()),
        node_version: Some("22.14.0".to_string()),
        framework: Some("nextjs".to_string()),
        port: Some(20052),
        app_dir: Some("/home/p_alice/sites/nextjs.alice.com/app".to_string()),
        entrypoint_type: Some("script".to_string()),
        entrypoint_value: Some("server.js".to_string()),
        auto_restart: Some(true),
        ..Default::default()
    };
    let nextjs_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&nextjs_payload).unwrap()))
        .unwrap();
    let nextjs_res = ctx.app.clone().oneshot(nextjs_req).await.unwrap();
    let nextjs_site = created_site_response(nextjs_res).await;
    assert_eq!(nextjs_site.kind, "nodejs");
    issue_self_signed_ssl_for_site(&ctx, nextjs_site.id).await;

    // 3. Django App (Python WSGI)
    let django_payload = CreateStaticSiteRequest {
        domain: "django.alice.com".to_string(),
        docroot: Some("/home/p_alice/sites/django.alice.com/app".to_string()),
        spa_fallback: Some(false),
        force_https: Some(true),
        user_id: Some(ctx.alice_id),
        kind: Some("python".to_string()),
        python_version: Some("3.12.9".to_string()),
        framework: Some("django".to_string()),
        server_type: Some("gunicorn_wsgi".to_string()),
        entrypoint_type: Some("wsgi".to_string()),
        entrypoint_value: Some("myproject.wsgi:application".to_string()),
        workers: Some(3),
        app_dir: Some("/home/p_alice/sites/django.alice.com/app".to_string()),
        venv_dir: Some("/home/p_alice/sites/django.alice.com/app/.venv".to_string()),
        static_url: Some("/static/".to_string()),
        static_dir: Some("/home/p_alice/sites/django.alice.com/app/static".to_string()),
        media_url: Some("/media/".to_string()),
        media_dir: Some("/home/p_alice/sites/django.alice.com/app/media".to_string()),
        auto_restart: Some(true),
        ..Default::default()
    };
    let django_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&django_payload).unwrap()))
        .unwrap();
    let django_res = ctx.app.clone().oneshot(django_req).await.unwrap();
    let django_site = created_site_response(django_res).await;
    assert_eq!(django_site.kind, "python");
    issue_self_signed_ssl_for_site(&ctx, django_site.id).await;

    // 4. FastAPI App (Python ASGI)
    let fastapi_payload = CreateStaticSiteRequest {
        domain: "fastapi.alice.com".to_string(),
        docroot: Some("/home/p_alice/sites/fastapi.alice.com/app".to_string()),
        spa_fallback: Some(false),
        force_https: Some(true),
        user_id: Some(ctx.alice_id),
        kind: Some("python".to_string()),
        python_version: Some("3.12.9".to_string()),
        framework: Some("fastapi".to_string()),
        server_type: Some("uvicorn_asgi".to_string()),
        entrypoint_type: Some("asgi".to_string()),
        entrypoint_value: Some("main:app".to_string()),
        workers: Some(2),
        app_dir: Some("/home/p_alice/sites/fastapi.alice.com/app".to_string()),
        venv_dir: Some("/home/p_alice/sites/fastapi.alice.com/app/.venv".to_string()),
        auto_restart: Some(true),
        ..Default::default()
    };
    let fastapi_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&fastapi_payload).unwrap()))
        .unwrap();
    let fastapi_res = ctx.app.clone().oneshot(fastapi_req).await.unwrap();
    let fastapi_site = created_site_response(fastapi_res).await;
    assert_eq!(fastapi_site.kind, "python");
    issue_self_signed_ssl_for_site(&ctx, fastapi_site.id).await;

    // 5. Flask App (Python WSGI)
    let flask_payload = CreateStaticSiteRequest {
        domain: "flask.alice.com".to_string(),
        docroot: Some("/home/p_alice/sites/flask.alice.com/app".to_string()),
        spa_fallback: Some(false),
        force_https: Some(true),
        user_id: Some(ctx.alice_id),
        kind: Some("python".to_string()),
        python_version: Some("3.12.9".to_string()),
        framework: Some("flask".to_string()),
        server_type: Some("gunicorn_wsgi".to_string()),
        entrypoint_type: Some("wsgi".to_string()),
        entrypoint_value: Some("app:app".to_string()),
        workers: Some(3),
        app_dir: Some("/home/p_alice/sites/flask.alice.com/app".to_string()),
        venv_dir: Some("/home/p_alice/sites/flask.alice.com/app/.venv".to_string()),
        static_url: Some("/static/".to_string()),
        static_dir: Some("/home/p_alice/sites/flask.alice.com/app/static".to_string()),
        auto_restart: Some(true),
        ..Default::default()
    };
    let flask_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&flask_payload).unwrap()))
        .unwrap();
    let flask_res = ctx.app.clone().oneshot(flask_req).await.unwrap();
    let flask_site = created_site_response(flask_res).await;
    assert_eq!(flask_site.kind, "python");
    issue_self_signed_ssl_for_site(&ctx, flask_site.id).await;

    // Verify all 5 sites are retrieved when Alice lists her sites
    let list_req = Request::builder()
        .method("GET")
        .uri("/api/v1/sites")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();
    let list_res = ctx.app.clone().oneshot(list_req).await.unwrap();
    assert_eq!(list_res.status(), StatusCode::OK);
    let list_bytes = list_res.into_body().collect().await.unwrap().to_bytes();
    let sites: Vec<SiteResponse> = serde_json::from_slice(&list_bytes).unwrap();
    assert_eq!(sites.len(), 5);
}

#[tokio::test]
async fn test_crash_auto_restart_and_process_settings() {
    let ctx = setup_test_context().await;

    // 1. Create Node.js site with auto_restart = true
    let node_payload = CreateStaticSiteRequest {
        domain: "restart-node.alice.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("nodejs".to_string()),
        node_version: Some("22.14.0".to_string()),
        framework: Some("express".to_string()),
        port: Some(20055),
        auto_restart: Some(true),
        ..Default::default()
    };
    let node_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&node_payload).unwrap()))
        .unwrap();
    let node_res = ctx.app.clone().oneshot(node_req).await.unwrap();
    let node_site = created_site_response(node_res).await;

    // Query node settings: auto_restart must be true
    let get_node_set = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/node/settings", node_site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();
    let get_node_res = ctx.app.clone().oneshot(get_node_set).await.unwrap();
    assert_eq!(get_node_res.status(), StatusCode::OK);
    let node_settings: Value =
        serde_json::from_slice(&get_node_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(node_settings["auto_restart"], true);

    // 2. Create Python site with auto_restart = true
    let py_payload = CreateStaticSiteRequest {
        domain: "restart-py.alice.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("python".to_string()),
        python_version: Some("3.12.9".to_string()),
        framework: Some("fastapi".to_string()),
        auto_restart: Some(true),
        ..Default::default()
    };
    let py_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&py_payload).unwrap()))
        .unwrap();
    let py_res = ctx.app.clone().oneshot(py_req).await.unwrap();
    let py_site = created_site_response(py_res).await;

    // Query python settings: auto_restart must be true
    let get_py_set = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/python/settings", py_site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .unwrap();
    let get_py_res = ctx.app.clone().oneshot(get_py_set).await.unwrap();
    assert_eq!(get_py_res.status(), StatusCode::OK);
    let py_settings: Value =
        serde_json::from_slice(&get_py_res.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(py_settings["auto_restart"], true);
}

#[tokio::test]
async fn test_ssrf_external_upstream_rejection_for_unauthorized_tenants() {
    let ctx = setup_test_context().await;

    // Alice's plan has allow_external_upstream = false.
    // 1. Attempting to create reverse proxy site targeting external internet IP/host
    let bad_proxy_payload = serde_json::json!({
        "domain": "proxy-evil.alice.com",
        "user_id": ctx.alice_id,
        "kind": "proxy",
        "upstream_url": "http://evil-external-host.com:8080"
    });
    let bad_proxy_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&bad_proxy_payload).unwrap(),
        ))
        .unwrap();
    let bad_proxy_res = ctx.app.clone().oneshot(bad_proxy_req).await.unwrap();
    assert_eq!(
        bad_proxy_res.status(),
        StatusCode::FORBIDDEN,
        "External upstream must be rejected with 403 Forbidden for tenant without allow_external_upstream permission"
    );

    // 2. Attempting to target cloud metadata IP
    let metadata_proxy_payload = serde_json::json!({
        "domain": "proxy-meta.alice.com",
        "user_id": ctx.alice_id,
        "kind": "proxy",
        "upstream_url": "http://169.254.169.254/latest/meta-data/"
    });
    let metadata_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&metadata_proxy_payload).unwrap(),
        ))
        .unwrap();
    let metadata_res = ctx.app.clone().oneshot(metadata_req).await.unwrap();
    assert_eq!(
        metadata_res.status(),
        StatusCode::BAD_REQUEST,
        "Cloud metadata IP upstream must be strictly rejected"
    );

    // 3. Creating reverse proxy site targeting local loopback: Allowed!
    let good_proxy_payload = serde_json::json!({
        "domain": "proxy-local.alice.com",
        "user_id": ctx.alice_id,
        "kind": "proxy",
        "upstream_url": "http://127.0.0.1:3000"
    });
    let good_proxy_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&good_proxy_payload).unwrap(),
        ))
        .unwrap();
    let good_proxy_res = ctx.app.clone().oneshot(good_proxy_req).await.unwrap();
    assert_eq!(
        good_proxy_res.status(),
        StatusCode::CREATED,
        "Local loopback upstream must be permitted"
    );
}

#[tokio::test]
async fn test_runtime_in_use_deletion_conflict_409() {
    let ctx = setup_test_context().await;

    // 1. Create Alice's Express app (uses Node.js 22.14.0)
    let express_payload = CreateStaticSiteRequest {
        domain: "express-guard.alice.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("nodejs".to_string()),
        node_version: Some("22.14.0".to_string()),
        framework: Some("express".to_string()),
        port: Some(20058),
        ..Default::default()
    };
    let express_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&express_payload).unwrap()))
        .unwrap();
    let express_res = ctx.app.clone().oneshot(express_req).await.unwrap();
    created_site_response(express_res).await;

    // 2. Admin attempts to delete Node.js 22.14.0 -> Rejected with 409 Conflict!
    let del_node_req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/runtimes/node/22.14.0")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let del_node_res = ctx.app.clone().oneshot(del_node_req).await.unwrap();
    assert_eq!(
        del_node_res.status(),
        StatusCode::CONFLICT,
        "Deleting in-use Node.js version must return 409 Conflict"
    );

    // 3. Create Alice's Django app (uses Python 3.12.9)
    let django_payload = CreateStaticSiteRequest {
        domain: "django-guard.alice.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("python".to_string()),
        python_version: Some("3.12.9".to_string()),
        framework: Some("django".to_string()),
        ..Default::default()
    };
    let django_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&django_payload).unwrap()))
        .unwrap();
    let django_res = ctx.app.clone().oneshot(django_req).await.unwrap();
    created_site_response(django_res).await;

    // 4. Admin attempts to delete Python 3.12.9 -> Rejected with 409 Conflict!
    let del_py_req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/runtimes/python/3.12.9")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let del_py_res = ctx.app.clone().oneshot(del_py_req).await.unwrap();
    assert_eq!(
        del_py_res.status(),
        StatusCode::CONFLICT,
        "Deleting in-use Python version must return 409 Conflict"
    );

    // 5. Admin deletes unused Python 3.13.2 -> Returns 200 OK
    let del_unused_req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/runtimes/python/3.13.2")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();
    let del_unused_res = ctx.app.clone().oneshot(del_unused_req).await.unwrap();
    assert_eq!(del_unused_res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_cross_tenant_isolation_boundary_api() {
    let ctx = setup_test_context().await;

    // 1. Alice creates an Express site and a FastAPI site
    let express_payload = CreateStaticSiteRequest {
        domain: "alice-exp.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("nodejs".to_string()),
        node_version: Some("22.14.0".to_string()),
        framework: Some("express".to_string()),
        port: Some(20061),
        ..Default::default()
    };
    let exp_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&express_payload).unwrap()))
        .unwrap();
    let exp_res = ctx.app.clone().oneshot(exp_req).await.unwrap();
    let exp_site = created_site_response(exp_res).await;

    let fastapi_payload = CreateStaticSiteRequest {
        domain: "alice-fast.com".to_string(),
        user_id: Some(ctx.alice_id),
        kind: Some("python".to_string()),
        python_version: Some("3.12.9".to_string()),
        framework: Some("fastapi".to_string()),
        ..Default::default()
    };
    let fast_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_string(&fastapi_payload).unwrap()))
        .unwrap();
    let fast_res = ctx.app.clone().oneshot(fast_req).await.unwrap();
    let fast_site = created_site_response(fast_res).await;

    // 2. Bob attempts to query Alice's Express node status -> Rejected (Forbidden / Not Found)
    let bob_query_exp = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/node/status", exp_site.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::empty())
        .unwrap();
    let bob_exp_res = ctx.app.clone().oneshot(bob_query_exp).await.unwrap();
    assert!(
        bob_exp_res.status() == StatusCode::FORBIDDEN
            || bob_exp_res.status() == StatusCode::NOT_FOUND,
        "Bob must NOT be able to view Alice's site node status"
    );

    // 3. Bob attempts to perform restart action on Alice's FastAPI site -> Rejected
    let bob_action = serde_json::json!({ "action": "restart" });
    let bob_act_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/python/action", fast_site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::from(serde_json::to_string(&bob_action).unwrap()))
        .unwrap();
    let bob_act_res = ctx.app.clone().oneshot(bob_act_req).await.unwrap();
    assert!(
        bob_act_res.status() == StatusCode::FORBIDDEN
            || bob_act_res.status() == StatusCode::NOT_FOUND,
        "Bob must NOT be able to trigger actions on Alice's site process"
    );

    // 4. Bob lists sites -> Sees 0 sites (Alice's sites are completely isolated)
    let bob_list_sites = Request::builder()
        .method("GET")
        .uri("/api/v1/sites")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::empty())
        .unwrap();
    let bob_sites_res = ctx.app.clone().oneshot(bob_list_sites).await.unwrap();
    assert_eq!(bob_sites_res.status(), StatusCode::OK);
    let bob_sites: Vec<SiteResponse> = serde_json::from_slice(
        &bob_sites_res
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(
        bob_sites.len(),
        0,
        "Bob must see 0 sites, completely isolated from Alice's sites"
    );

    // 5. Bob lists unified processes -> Sees 0 processes
    let bob_list_procs = Request::builder()
        .method("GET")
        .uri("/api/v1/processes")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.bob_csrf)
        .body(Body::empty())
        .unwrap();
    let bob_procs_res = ctx.app.clone().oneshot(bob_list_procs).await.unwrap();
    assert_eq!(bob_procs_res.status(), StatusCode::OK);
    let bob_procs: Value = serde_json::from_slice(
        &bob_procs_res
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    let procs_array = bob_procs["processes"].as_array().unwrap();
    assert_eq!(procs_array.len(), 0, "Bob must see 0 running processes");
    assert_eq!(bob_procs["total_active"], 0);
}
