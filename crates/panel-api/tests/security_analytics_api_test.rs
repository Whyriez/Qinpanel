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
use panel_analytics::{AnalyticsStorage, SecurityEvent, SecurityEventType};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_api::security_analytics::{
    BanIpRequest, BannedIpDto, SecurityOverviewResponse, UnbanIpResponse,
};
use panel_core::auth::hash_password;
use panel_core::site_rules::SiteRulesConfig;
use panel_db::models::{CreateDomain, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqliteDomainRepository, SqliteSiteRepository, SqliteUserRepository};
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    analytics: Arc<AnalyticsStorage>,
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    alice_session: String,
    alice_csrf: String,
    bob_session: String,
    alice_site_id: i64,
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
    let analytics_db_file = tmp.path().join("analytics.db");

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
    let nginx_dir = tmp.path().join("nginx");
    fs::create_dir_all(&nginx_dir).expect("create nginx dir");

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

    tokio::time::sleep(Duration::from_millis(100)).await;

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let analytics = Arc::new(
        AnalyticsStorage::connect(&analytics_db_file)
            .await
            .expect("init analytics storage"),
    );

    let user_repo = SqliteUserRepository::new(pool.clone());
    let alice_hash = hash_password("AlicePass123!").expect("hash pass");
    let alice = user_repo
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
    let _bob = user_repo
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

    let site_repo = SqliteSiteRepository::new(pool.clone());
    let domain_repo = SqliteDomainRepository::new(pool.clone());

    let alice_site = site_repo
        .create(&CreateSite {
            user_id: alice.id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/public_html".to_string(),
            runtime: Some("php:8.3".to_string()),
            version: None,
            status: Some("active".to_string()),
            settings_json: Some("{}".to_string()),
        })
        .await
        .expect("create alice site");

    domain_repo
        .create(&CreateDomain {
            site_id: alice_site.id,
            name: "alice.com".to_string(),
            unicode_name: "alice.com".to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .expect("create alice domain");

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
        "test".to_string(),
        auth_config,
        lockout_manager,
    )
    .with_analytics(Arc::clone(&analytics));

    let app = create_router(state);

    async fn login_helper(app_router: &axum::Router, user: &str, pass: &str) -> (String, String) {
        let login_body = serde_json::json!({
            "username": user,
            "password": pass,
        })
        .to_string();

        let login_req = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(login_body))
            .expect("login req");

        let login_resp = app_router
            .clone()
            .oneshot(login_req)
            .await
            .expect("login resp");
        assert_eq!(login_resp.status(), StatusCode::OK);

        let cookies = login_resp
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().expect("to str").to_string())
            .collect::<Vec<_>>();

        let mut session_token = String::new();
        let mut csrf_token = String::new();
        for c in &cookies {
            if let Some(s) = extract_cookie_value(c, SESSION_COOKIE_NAME) {
                session_token = s;
            }
            if let Some(cs) = extract_cookie_value(c, "csrf_token") {
                csrf_token = cs;
            }
        }

        (session_token, csrf_token)
    }

    let (alice_session, alice_csrf) = login_helper(&app, "alice", "AlicePass123!").await;
    let (bob_session, _bob_csrf) = login_helper(&app, "bob", "BobPass123!").await;

    TestContext {
        app,
        analytics,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        alice_session,
        alice_csrf,
        bob_session,
        alice_site_id: alice_site.id,
    }
}

#[tokio::test]
async fn test_security_analytics_full_lifecycle_and_ip_banning() {
    let ctx = setup_test_context().await;
    let now = chrono::Utc::now().timestamp();

    // 1. Seed security events into analytics storage
    let events = vec![
        SecurityEvent {
            id: None,
            site_id: ctx.alice_site_id,
            timestamp: now - 300,
            client_ip: "198.51.100.44".to_string(),
            method: "GET".to_string(),
            uri: "/.env".to_string(),
            status: 404,
            user_agent: Some("curl/8.0".to_string()),
            country: Some("ID".to_string()),
            event_type: SecurityEventType::Probing,
            threat_score: 85,
        },
        SecurityEvent {
            id: None,
            site_id: ctx.alice_site_id,
            timestamp: now - 250,
            client_ip: "198.51.100.44".to_string(),
            method: "GET".to_string(),
            uri: "/.git/config".to_string(),
            status: 404,
            user_agent: Some("curl/8.0".to_string()),
            country: Some("ID".to_string()),
            event_type: SecurityEventType::Probing,
            threat_score: 80,
        },
        SecurityEvent {
            id: None,
            site_id: ctx.alice_site_id,
            timestamp: now - 100,
            client_ip: "203.0.113.88".to_string(),
            method: "POST".to_string(),
            uri: "/wp-login.php".to_string(),
            status: 401,
            user_agent: Some("python-requests/2.31".to_string()),
            country: Some("RU".to_string()),
            event_type: SecurityEventType::BruteForce,
            threat_score: 70,
        },
    ];

    ctx.analytics
        .insert_security_events(&events)
        .await
        .expect("insert security events");

    // 2. Query Security Overview as Alice
    let overview_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/analytics/security/overview?limit=10",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(overview_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let overview: SecurityOverviewResponse = serde_json::from_slice(&bytes).expect("overview");

    assert_eq!(overview.site_id, ctx.alice_site_id);
    assert_eq!(overview.total_probing_requests, 2);
    assert_eq!(overview.total_brute_force_attempts, 1);
    assert_eq!(overview.unique_suspicious_ips, 2);
    assert_eq!(overview.top_ips.len(), 2);
    assert_eq!(overview.probing_paths.len(), 2);
    assert_eq!(overview.brute_force_incidents.len(), 1);

    // 3. Authorization check: Bob cannot access Alice's security analytics
    let forbidden_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/analytics/security/overview",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.bob_session),
        )
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(forbidden_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 4. Ban IP with TTL = 3600 seconds as Alice
    let ban_payload = BanIpRequest {
        ip: "198.51.100.44".to_string(),
        ttl_secs: Some(3600),
        reason: Some("Probing berkas rahasia .env".to_string()),
    };

    let ban_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/sites/{}/security/ban-ip",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&ban_payload).expect("json"),
        ))
        .expect("req");

    let resp = ctx.app.clone().oneshot(ban_req).await.expect("resp");
    if resp.status() != StatusCode::OK {
        let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
        panic!("Ban IP failed: {}", String::from_utf8_lossy(&bytes));
    }
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let banned_res: BannedIpDto = serde_json::from_slice(&bytes).expect("banned");

    assert_eq!(banned_res.ip_address, "198.51.100.44");
    assert!(banned_res.is_active);
    assert!(banned_res.expires_at.is_some());
    assert!(banned_res.remaining_ttl_secs.unwrap_or(0) > 3500);

    // Check site_rules Nginx synchronization in database
    let site_repo = SqliteSiteRepository::new(ctx.pool.clone());
    let site = site_repo
        .find_by_id(ctx.alice_site_id)
        .await
        .expect("site")
        .expect("found");
    let settings: serde_json::Value = serde_json::from_str(&site.settings_json).expect("parse");
    let rules: SiteRulesConfig =
        serde_json::from_value(settings["site_rules"].clone()).expect("rules");
    assert!(rules.ip_access.enabled);
    assert!(
        rules
            .ip_access
            .rules
            .iter()
            .any(|r| r.cidr_or_ip == "198.51.100.44"
                && r.action == panel_core::site_rules::IpAction::Deny)
    );

    // 5. Test Invalid IP rejection (400 Bad Request)
    let invalid_ban_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/sites/{}/security/ban-ip",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "ip": "not_an_ip_address_999",
                "ttl_secs": 3600
            }))
            .expect("json"),
        ))
        .expect("req");

    let resp = ctx
        .app
        .clone()
        .oneshot(invalid_ban_req)
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 6. List active banned IPs
    let list_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/security/banned-ips",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(list_req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let banned_list: Vec<BannedIpDto> = serde_json::from_slice(&bytes).expect("list");
    assert_eq!(banned_list.len(), 1);
    assert_eq!(banned_list[0].ip_address, "198.51.100.44");

    // 7. Unban IP
    let unban_req = Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/v1/sites/{}/security/ban-ip/198.51.100.44",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(unban_req).await.expect("resp");
    if resp.status() != StatusCode::OK {
        let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
        panic!("Unban IP failed: {}", String::from_utf8_lossy(&bytes));
    }
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let unban_res: UnbanIpResponse = serde_json::from_slice(&bytes).expect("unban");
    assert!(unban_res.success);
    assert_eq!(unban_res.unbanned_count, 1);

    // Verify Nginx rules no longer contain 198.51.100.44
    let site_after = site_repo
        .find_by_id(ctx.alice_site_id)
        .await
        .expect("site")
        .expect("found");
    let settings_after: serde_json::Value =
        serde_json::from_str(&site_after.settings_json).expect("parse");
    let rules_after: SiteRulesConfig =
        serde_json::from_value(settings_after["site_rules"].clone()).expect("rules");
    assert!(
        !rules_after
            .ip_access
            .rules
            .iter()
            .any(|r| r.cidr_or_ip == "198.51.100.44")
    );

    // Verify active banned IPs is now empty
    let list_req_after = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/sites/{}/security/banned-ips",
            ctx.alice_site_id
        ))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let resp = ctx.app.clone().oneshot(list_req_after).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.expect("bytes").to_bytes();
    let empty_list: Vec<BannedIpDto> = serde_json::from_slice(&bytes).expect("list");
    assert_eq!(empty_list.len(), 0);
}
