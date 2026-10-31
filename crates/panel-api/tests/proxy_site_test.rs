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
use panel_api::proxy_site::{
    ProxyHeaderConfig, ProxySiteSettingsDto, UpdateProxySiteSettingsRequest,
};
use panel_api::routes::AppState;
use panel_api::sites::{CreateStaticSiteRequest, SiteResponse};
use panel_core::auth::hash_password;
use panel_db::models::{CreatePlan, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqlitePlanRepository, SqliteUserRepository};
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

    let runner = Arc::new(MockSystemRunner::new());
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
    let nginx_dir = tmp.path().join("etc/nginx");
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

    // A local proxy must target an active runtime owned by the same tenant.
    for port in [20050, 20051] {
        sqlx::query("INSERT INTO sites(user_id,kind,docroot,status,settings_json,created_at,updated_at) VALUES(?,'nodejs','/fixture','active',?,1,1)")
            .bind(alice_user.id).bind(serde_json::json!({"port":port}).to_string())
            .execute(&pool).await.expect("owned runtime");
    }
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

    let (alice_session, alice_csrf) = login_helper(&app, "alice", "AlicePass123!").await;
    let (bob_session, _bob_csrf) = login_helper(&app, "bob", "BobPass123!").await;

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        alice_session,
        alice_csrf,
        bob_session,
        alice_id: alice_user.id,
    }
}

async fn login_helper(app: &axum::Router, username: &str, pass: &str) -> (String, String) {
    let login_payload = serde_json::json!({
        "username": username,
        "password": pass
    });
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&login_payload).expect("vec")))
        .expect("req");
    let res = app.clone().oneshot(login_req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let cookies = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("cookie")
        .to_str()
        .expect("str");
    let session_id = extract_cookie_value(cookies, SESSION_COOKIE_NAME).expect("session_id");
    let body_bytes = res.into_body().collect().await.expect("bytes").to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).expect("json");
    let csrf_token = body_json["csrf_token"].as_str().expect("csrf").to_string();

    (session_id, csrf_token)
}

#[tokio::test]
async fn test_create_proxy_site_loopback_and_get_settings() {
    let ctx = setup_test_context().await;

    // Alice creates a loopback reverse proxy site
    let create_payload = CreateStaticSiteRequest {
        domain: "proxy-app.test".to_string(),
        kind: Some("proxy".to_string()),
        upstream_url: Some("http://127.0.0.1:20050".to_string()),
        preserve_host: Some(true),
        websocket_support: Some(true),
        proxy_connect_timeout: Some("15s".to_string()),
        proxy_read_timeout: Some("45s".to_string()),
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&create_payload).expect("vec"),
        ))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::CREATED);

    let body = res.into_body().collect().await.expect("body").to_bytes();
    let site: SiteResponse = serde_json::from_slice(&body).expect("site json");
    assert_eq!(site.kind, "proxy");

    // Retrieve proxy settings
    let get_req = Request::builder()
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let get_res = ctx.app.clone().oneshot(get_req).await.expect("res");
    assert_eq!(get_res.status(), StatusCode::OK);

    let get_body = get_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let settings: ProxySiteSettingsDto = serde_json::from_slice(&get_body).expect("settings json");
    assert_eq!(settings.upstream_url, "http://127.0.0.1:20050");
    assert!(settings.preserve_host);
    assert!(settings.websocket_support);
    assert_eq!(settings.proxy_connect_timeout.as_deref(), Some("15s"));
    assert_eq!(settings.proxy_read_timeout.as_deref(), Some("45s"));
    assert!(!settings.allow_external_upstream);
}

#[tokio::test]
async fn test_external_upstream_ssrf_guard_enforcement() {
    let ctx = setup_test_context().await;

    // 1. Alice (tenant without allow_external_upstream) tries to create site pointing to external domain -> 403 Forbidden
    let ext_payload = CreateStaticSiteRequest {
        domain: "ext-app.test".to_string(),
        kind: Some("proxy".to_string()),
        upstream_url: Some("https://93.184.216.34".to_string()),
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&ext_payload).expect("vec")))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 2. Give Alice a plan that allows external upstreams
    let plan_repo = SqlitePlanRepository::new(ctx.pool.clone());
    let pro_plan = plan_repo
        .create(&CreatePlan {
            name: "Pro Plan".to_string(),
            description: Some("Plan with external reverse proxy enabled".to_string()),
            cpu_quota_percent: Some(200),
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            memory_high_bytes: None,
            tasks_max: Some(200),
            io_weight: Some(100),
            disk_limit_bytes: Some(10 * 1024 * 1024 * 1024),
            inode_limit: Some(100000),
            max_sites: Some(10),
            max_domains: Some(20),
            max_databases: Some(5),
            max_cron_jobs: Some(10),
            max_terminal_sessions: Some(5),
            max_upload_bytes: Some(100 * 1024 * 1024),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(true),
        })
        .await
        .expect("create plan");

    sqlx::query("UPDATE users SET plan_id = ? WHERE id = ?")
        .bind(pro_plan.id)
        .bind(ctx.alice_id)
        .execute(&ctx.pool)
        .await
        .expect("update alice plan");

    // 3. Now Alice creates external reverse proxy site -> 200 OK
    let req2 = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&ext_payload).expect("vec")))
        .expect("req");

    let res2 = ctx.app.clone().oneshot(req2).await.expect("res");
    assert_eq!(res2.status(), StatusCode::CREATED);
    let body = res2.into_body().collect().await.expect("body").to_bytes();
    let site: SiteResponse = serde_json::from_slice(&body).expect("site");

    // Settings reflect allow_external_upstream: true
    let get_req = Request::builder()
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let get_res = ctx.app.clone().oneshot(get_req).await.expect("res");
    let get_body = get_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let settings: ProxySiteSettingsDto = serde_json::from_slice(&get_body).expect("settings json");
    assert!(settings.allow_external_upstream);
    assert_eq!(settings.upstream_url, "https://93.184.216.34");

    // 4. Hardcoded Blacklist is STILL blocked even when allow_external_upstream == true:
    // a) Cloud metadata IP (169.254.169.254)
    let update_meta = UpdateProxySiteSettingsRequest {
        upstream_url: Some("http://169.254.169.254/latest/meta-data".to_string()),
        ..Default::default()
    };
    let update_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&update_meta).expect("vec")))
        .expect("req");
    let update_res = ctx.app.clone().oneshot(update_req).await.expect("res");
    assert_eq!(update_res.status(), StatusCode::BAD_REQUEST);

    // b) Internal Panel port 7700
    let update_panel_port = UpdateProxySiteSettingsRequest {
        upstream_url: Some("http://127.0.0.1:7700".to_string()),
        ..Default::default()
    };
    let update_req2 = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&update_panel_port).expect("vec"),
        ))
        .expect("req");
    let update_res2 = ctx.app.clone().oneshot(update_req2).await.expect("res");
    assert_eq!(update_res2.status(), StatusCode::BAD_REQUEST);

    // c) Private LAN (192.168.1.50)
    let update_lan = UpdateProxySiteSettingsRequest {
        upstream_url: Some("http://192.168.1.50:8080".to_string()),
        ..Default::default()
    };
    let update_req3 = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&update_lan).expect("vec")))
        .expect("req");
    let update_res3 = ctx.app.clone().oneshot(update_req3).await.expect("res");
    assert_eq!(update_res3.status(), StatusCode::BAD_REQUEST);

    // d) Injection chars
    let update_inj = UpdateProxySiteSettingsRequest {
        upstream_url: Some("http://127.0.0.1:8080; malicious".to_string()),
        ..Default::default()
    };
    let update_req4 = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(serde_json::to_vec(&update_inj).expect("vec")))
        .expect("req");
    let update_res4 = ctx.app.clone().oneshot(update_req4).await.expect("res");
    assert_eq!(update_res4.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_proxy_settings_update_and_header_validation() {
    let ctx = setup_test_context().await;

    let create_payload = CreateStaticSiteRequest {
        domain: "headers-test.com".to_string(),
        kind: Some("proxy".to_string()),
        upstream_url: Some("http://127.0.0.1:20050".to_string()),
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&create_payload).expect("vec"),
        ))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    let status = res.status();
    let body = res.into_body().collect().await.expect("body").to_bytes();
    if status != StatusCode::CREATED {
        panic!(
            "create site failed with status {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    let site: SiteResponse = serde_json::from_slice(&body).expect("site");

    // 1. Update with valid custom headers & options
    let update_payload = UpdateProxySiteSettingsRequest {
        upstream_url: Some("http://127.0.0.1:20051".to_string()),
        preserve_host: Some(false),
        websocket_support: Some(true),
        proxy_connect_timeout: Some("30s".to_string()),
        proxy_buffering: Some(false),
        custom_request_headers: Some(vec![ProxyHeaderConfig {
            name: "X-App-Env".to_string(),
            value: "production".to_string(),
        }]),
        custom_response_headers: Some(vec![ProxyHeaderConfig {
            name: "X-Served-By".to_string(),
            value: "Nusa-Proxy".to_string(),
        }]),
        ..Default::default()
    };

    let put_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&update_payload).expect("vec"),
        ))
        .expect("req");

    let put_res = ctx.app.clone().oneshot(put_req).await.expect("res");
    assert_eq!(put_res.status(), StatusCode::OK);

    let put_body = put_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let updated: ProxySiteSettingsDto = serde_json::from_slice(&put_body).expect("updated json");
    assert_eq!(updated.upstream_url, "http://127.0.0.1:20051");
    assert!(!updated.preserve_host);
    assert_eq!(updated.proxy_buffering, Some(false));
    assert_eq!(updated.custom_request_headers.len(), 1);
    assert_eq!(updated.custom_request_headers[0].name, "X-App-Env");
    assert_eq!(updated.custom_response_headers.len(), 1);
    assert_eq!(updated.custom_response_headers[0].name, "X-Served-By");

    // 2. Reject header with newline injection
    let bad_header_payload = UpdateProxySiteSettingsRequest {
        custom_request_headers: Some(vec![ProxyHeaderConfig {
            name: "X-Injected\nHeader".to_string(),
            value: "val".to_string(),
        }]),
        ..Default::default()
    };
    let bad_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&bad_header_payload).expect("vec"),
        ))
        .expect("req");

    let bad_res = ctx.app.clone().oneshot(bad_req).await.expect("res");
    assert_eq!(bad_res.status(), StatusCode::BAD_REQUEST);

    // 3. Bob cannot access Alice's site settings (RBAC)
    let bob_req = Request::builder()
        .uri(format!("/api/v1/sites/{}/proxy/settings", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.bob_session),
        )
        .body(Body::empty())
        .expect("req");

    let bob_res = ctx.app.clone().oneshot(bob_req).await.expect("res");
    assert_eq!(bob_res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_proxy_upstream_probe_endpoint() {
    let ctx = setup_test_context().await;

    // Start a mock upstream server on loopback
    let mut available = None;
    for port in 20052..=20152 {
        if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            available = Some(listener);
            break;
        }
    }
    let listener = available.expect("available isolated upstream port");
    let port = listener.local_addr().expect("local addr").port();
    sqlx::query("INSERT INTO sites(user_id,kind,docroot,status,settings_json,created_at,updated_at) VALUES(?,'nodejs','/fixture','active',?,1,1)")
        .bind(ctx.alice_id).bind(serde_json::json!({"port":port}).to_string())
        .execute(&ctx.pool).await.expect("registered probe runtime");

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 256];
            let _ = socket.read(&mut buf).await;
            let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
            let _ = socket.write_all(resp).await;
            let _ = socket.shutdown().await;
        }
    });

    let upstream = format!("http://127.0.0.1:{port}/probe");

    let create_payload = CreateStaticSiteRequest {
        domain: "probe-site.test".to_string(),
        kind: Some("proxy".to_string()),
        upstream_url: Some(upstream.clone()),
        ..Default::default()
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&create_payload).expect("vec"),
        ))
        .expect("req");

    let res = ctx.app.clone().oneshot(req).await.expect("res");
    let status = res.status();
    let body = res.into_body().collect().await.expect("body").to_bytes();
    if status != StatusCode::CREATED {
        panic!(
            "create site failed with status {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    let site: SiteResponse = serde_json::from_slice(&body).expect("site");

    // 1. Probe the site's default upstream
    let probe_req = Request::builder()
        .uri(format!("/api/v1/sites/{}/proxy/probe", site.id))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let probe_res = ctx.app.clone().oneshot(probe_req).await.expect("res");
    assert_eq!(probe_res.status(), StatusCode::OK);

    let probe_body = probe_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let probe_json: Value = serde_json::from_slice(&probe_body).expect("json");
    assert_eq!(probe_json["healthy"], true);
    assert_eq!(probe_json["status_code"], 200);

    // 2. Probe query with malicious target (metadata SSRF) is blocked immediately
    let bad_probe_req = Request::builder()
        .uri(format!(
            "/api/v1/sites/{}/proxy/probe?upstream_url=http://169.254.169.254/latest",
            site.id
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let bad_probe_res = ctx.app.clone().oneshot(bad_probe_req).await.expect("res");
    assert_eq!(bad_probe_res.status(), StatusCode::BAD_REQUEST);
}
