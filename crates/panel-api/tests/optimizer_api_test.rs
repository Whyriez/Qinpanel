#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::clone_on_ref_ptr,
    clippy::bool_assert_comparison
)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::hardening::HardeningManager;
use panel_agent::optimizer::OptimizerManager;
use panel_agent::quota::QuotaManager;
use panel_agent::resource::CgroupReader;
use panel_agent::slice::SliceManager;
use panel_agent::system::{MockSystemRunner, SystemRunner};
use panel_agent::user::UserLifecycleManager;
use panel_agent::webserver::NginxManager;
use panel_agent::{AgentConfig, AgentServer};
use panel_analytics::AnalyticsStorage;
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::optimizer::{
    ApplyOptimizerRequestDto, OptimizerActionResponseDto, OptimizerStatusResponseDto,
};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    runner: Arc<MockSystemRunner>,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    admin_session: String,
    admin_csrf: String,
    tenant_session: String,
    _tenant_csrf: String,
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

    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response("sysctl", 0, b"", b"");

    let opt_base = tmp.path().join("optimizer_base");
    let opt_sysctl = tmp.path().join("sysctl.conf");
    let opt_nginx = tmp.path().join("conf.d");
    let optimizer_mgr = Arc::new(OptimizerManager::with_paths(
        runner.clone(),
        opt_base,
        opt_sysctl,
        opt_nginx,
    ));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let user_mgr = Arc::new(UserLifecycleManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let slice_mgr = Arc::new(SliceManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let quota_mgr = Arc::new(QuotaManager::new(runner.clone() as Arc<dyn SystemRunner>));
    let cgroup = Arc::new(CgroupReader::default());
    let hard_mgr = Arc::new(HardeningManager::new(
        runner.clone() as Arc<dyn SystemRunner>
    ));
    let web_mgr = Arc::new(NginxManager::new(runner.clone() as Arc<dyn SystemRunner>));

    let agent_server = Arc::new(
        AgentServer::with_all_managers(
            agent_cfg, user_mgr, slice_mgr, quota_mgr, cgroup, hard_mgr, web_mgr,
        )
        .with_optimizer_manager(optimizer_mgr),
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let srv_clone = agent_server.clone();
    tokio::spawn(async move {
        let _ = srv_clone.run(shutdown_rx).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init db pool");
    run_migrations(&pool).await.expect("run db migrations");
    let analytics = Arc::new(
        AnalyticsStorage::connect(&analytics_db_file)
            .await
            .expect("open analytics db"),
    );

    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_pass_hash = hash_password("AdminSecret123!").expect("hash admin");
    let _admin_user = user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            password_hash: admin_pass_hash,
            role: "admin".to_string(),
            plan_id: None,
            linux_username: Some("p_admin".to_string()),
            linux_uid: Some(10001),
        })
        .await
        .expect("create admin");

    let tenant_pass_hash = hash_password("TenantSecret123!").expect("hash tenant");
    let _tenant_user = user_repo
        .create(&CreateUser {
            username: "tenant1".to_string(),
            email: Some("tenant1@example.com".to_string()),
            password_hash: tenant_pass_hash,
            role: "user".to_string(),
            plan_id: None,
            linux_username: Some("p_tenant1".to_string()),
            linux_uid: Some(10002),
        })
        .await
        .expect("create tenant");

    let auth_cfg = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };
    let agent_client = Arc::new(AgentClient::new(sock_path));
    let lockout = Arc::new(LockoutManager::new(5, 300));
    let state = AppState::new(
        agent_client,
        pool.clone(),
        "test".to_string(),
        auth_cfg,
        lockout,
    )
    .with_analytics(analytics);
    let app = create_router(state);

    // Login Admin
    let login_admin_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "username": "admin",
                "password": "AdminSecret123!"
            })
            .to_string(),
        ))
        .unwrap();

    let admin_resp = app.clone().oneshot(login_admin_req).await.unwrap();
    assert_eq!(admin_resp.status(), StatusCode::OK);
    let admin_cookie_hdr = admin_resp
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let admin_session = extract_cookie_value(&admin_cookie_hdr, SESSION_COOKIE_NAME).unwrap();

    let csrf_admin_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let csrf_resp = app.clone().oneshot(csrf_admin_req).await.unwrap();
    let body_bytes = csrf_resp.into_body().collect().await.unwrap().to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let admin_csrf = csrf_json["csrf_token"].as_str().unwrap().to_string();

    // Login Tenant
    let login_tenant_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "username": "tenant1",
                "password": "TenantSecret123!"
            })
            .to_string(),
        ))
        .unwrap();

    let tenant_resp = app.clone().oneshot(login_tenant_req).await.unwrap();
    assert_eq!(tenant_resp.status(), StatusCode::OK);
    let tenant_cookie_hdr = tenant_resp
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let tenant_session = extract_cookie_value(&tenant_cookie_hdr, SESSION_COOKIE_NAME).unwrap();

    let csrf_tenant_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, tenant_session),
        )
        .body(Body::empty())
        .unwrap();
    let csrf_t_resp = app.clone().oneshot(csrf_tenant_req).await.unwrap();
    let t_body = csrf_t_resp.into_body().collect().await.unwrap().to_bytes();
    let csrf_t_json: serde_json::Value = serde_json::from_slice(&t_body).unwrap();
    let tenant_csrf = csrf_t_json["csrf_token"].as_str().unwrap().to_string();

    TestContext {
        app,
        runner,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        tenant_session,
        _tenant_csrf: tenant_csrf,
    }
}

#[tokio::test]
async fn test_get_optimizer_status_admin_and_permissions() {
    let ctx = setup_test_context().await;

    // 1. Unauthenticated -> 401
    let unauth_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/optimizer?capacity_usage=personal")
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(unauth_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. Tenant (non-admin) -> 403 Forbidden
    let tenant_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/optimizer?capacity_usage=personal")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.tenant_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(tenant_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 3. Admin -> 200 OK
    let admin_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/optimizer")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(admin_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let status_dto: OptimizerStatusResponseDto = serde_json::from_slice(&bytes).unwrap();
    assert!(status_dto.hardware.vcpu_count >= 1);
    assert!(!status_dto.recommended_profile.is_empty());
    assert_eq!(status_dto.current_settings.nginx.multi_accept, true);
    assert_eq!(status_dto.recommended_settings.nginx.multi_accept, true);
    let capacity = status_dto
        .capacity
        .as_ref()
        .expect("additive capacity snapshot");
    assert_eq!(capacity.scope, "panel.slice");
    assert!(capacity.observed_at > 0);
    // Missing fixture readings must remain unknown even though the legacy
    // hardware DTO retains its historical display defaults.
    assert_eq!(capacity.cpu_quota_percent.value, None);
    assert_eq!(capacity.memory_available_bytes.value, None);
    let mut legacy = serde_json::to_value(&status_dto).expect("status JSON");
    legacy
        .as_object_mut()
        .expect("status object")
        .remove("capacity");
    let old_payload: OptimizerStatusResponseDto =
        serde_json::from_value(legacy).expect("old payload compatibility");
    assert!(old_payload.capacity.is_none());
}

#[tokio::test]
async fn test_apply_and_rollback_optimizer_lifecycle() {
    let ctx = setup_test_context().await;

    // 1. Admin applies "standard" profile
    let apply_payload = ApplyOptimizerRequestDto {
        profile: "standard".to_string(),
    };
    let apply_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/optimizer/apply")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&apply_payload).unwrap()))
        .unwrap();

    let res = ctx.app.clone().oneshot(apply_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let action_res: OptimizerActionResponseDto = serde_json::from_slice(&bytes).unwrap();
    assert!(action_res.success);
    assert_eq!(action_res.profile, Some("standard".to_string()));

    // Verify GET status shows standard as active
    let get_req = Request::builder()
        .method("GET")
        .uri("/api/v1/system/optimizer")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let status_dto: OptimizerStatusResponseDto = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status_dto.active_profile, Some("standard".to_string()));
    assert!(status_dto.has_backup);

    // 2. Admin applies "high_performance" profile
    let apply_hp_payload = ApplyOptimizerRequestDto {
        profile: "high_performance".to_string(),
    };
    let apply_hp_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/optimizer/apply")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&apply_hp_payload).unwrap(),
        ))
        .unwrap();

    let res = ctx.app.clone().oneshot(apply_hp_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. Admin triggers rollback
    let rollback_req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/optimizer/rollback")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let res = ctx.app.clone().oneshot(rollback_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let rb_dto: OptimizerActionResponseDto = serde_json::from_slice(&bytes).unwrap();
    assert!(rb_dto.success);
    assert_eq!(rb_dto.profile, Some("standard".to_string()));

    // Verify GET status is back to standard
    let get_req2 = Request::builder()
        .method("GET")
        .uri("/api/v1/system/optimizer")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(get_req2).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let status_dto2: OptimizerStatusResponseDto = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status_dto2.active_profile, Some("standard".to_string()));
}

fn measured_capacity(runner: &MockSystemRunner) {
    for (path, content) in [
        (
            "/proc/self/mountinfo",
            "29 23 0:26 / /sys/fs/cgroup rw - cgroup2 cgroup rw\n",
        ),
        (
            "/proc/meminfo",
            "MemTotal: 8388608 kB\nMemAvailable: 6291456 kB\n",
        ),
        ("/sys/devices/system/cpu/online", "0-3"),
        ("/sys/fs/cgroup/cpuset.cpus.effective", "0-3"),
        ("/sys/fs/cgroup/panel.slice/cgroup.type", "domain"),
        ("/sys/fs/cgroup/panel.slice/cpu.max", "250000 100000"),
        ("/sys/fs/cgroup/panel.slice/memory.max", "4294967296"),
        ("/sys/fs/cgroup/panel.slice/memory.current", "1073741824"),
    ] {
        runner.set_file_content(path, content.as_bytes());
    }
    runner.set_response(
        "df",
        0,
        b"1B-blocks Avail Inodes IFree\n104857600000 52428800000 1000000 800000\n",
        b"",
    );
}

async fn capacity_request(ctx: &TestContext, uri: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .body(Body::empty())
        .expect("request");
    ctx.app.clone().oneshot(request).await.expect("response")
}

#[tokio::test]
async fn capacity_preview_refuses_fabricated_legacy_defaults() {
    let ctx = setup_test_context().await;
    let response = capacity_request(&ctx, "/api/v1/system/optimizer?capacity_usage=personal").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(error["code"], "capacity_unavailable");
}

#[tokio::test]
async fn capacity_preview_is_bounded_read_only_and_has_no_security_fields() {
    let ctx = setup_test_context().await;
    let before = ctx.runner.actions.lock().expect("initial action log").len();
    measured_capacity(&ctx.runner);
    for query in [
        "capacity_usage=personal",
        "capacity_usage=multi_project&projects_per_tenant=3",
        "capacity_usage=multi_tenant&tenant_slots=3",
    ] {
        let response = capacity_request(&ctx, &format!("/api/v1/system/optimizer?{query}")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .expect("cache control"),
            "no-store"
        );
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let status: OptimizerStatusResponseDto = serde_json::from_slice(&bytes).expect("status");
        let preset = status.package_preset.expect("preview");
        let slots = u64::from(preset.demand.tenant_slots);
        assert!(preset.cpu_quota_percent * slots <= 250 - preset.cpu_headroom_percent);
        assert!(
            preset.memory_limit_bytes * slots <= 3 * 1_073_741_824 - preset.memory_headroom_bytes
        );
        assert!(preset.memory_high_bytes <= preset.memory_limit_bytes);
        assert!(preset.expires_at > preset.observed_at);
        let json = serde_json::to_value(&preset).expect("preset JSON");
        assert!(json.get("shell_policy").is_none());
        assert!(json.get("allow_external_upstream").is_none());
    }
    let actions = ctx.runner.actions.lock().expect("action log");
    assert_eq!(actions.len() - before, 3);
    assert!(actions[before..].iter().all(|action| matches!(action, panel_agent::system::MockAction::Command { binary, .. } if binary == "df")));
}

#[tokio::test]
async fn invalid_capacity_demands_are_rejected_before_agent_reads() {
    let ctx = setup_test_context().await;
    let before = ctx.runner.actions.lock().expect("initial action log").len();
    for query in [
        "projects_per_tenant=2",
        "capacity_usage=personal&tenant_slots=2",
        "capacity_usage=multi_project&projects_per_tenant=0",
        "capacity_usage=multi_tenant&tenant_slots=10001",
        "capacity_usage=unknown",
        "capacity_usage=personal&tenant_slots=-1",
    ] {
        assert_eq!(
            capacity_request(&ctx, &format!("/api/v1/system/optimizer?{query}"))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(ctx.runner.actions.lock().expect("action log").len(), before);
}

#[tokio::test]
async fn full_filesystem_returns_capacity_conflict_instead_of_a_zero_limit() {
    let ctx = setup_test_context().await;
    measured_capacity(&ctx.runner);
    ctx.runner
        .set_response("df", 0, b"header\n104857600000 0 1000000 0\n", b"");
    let response = capacity_request(&ctx, "/api/v1/system/optimizer?capacity_usage=personal").await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(error["code"], "capacity_insufficient");
}
