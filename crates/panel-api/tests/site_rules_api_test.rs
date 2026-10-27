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
use panel_api::site_rules::{
    HashPasswordRequest, HashPasswordResponse, ValidateSnippetRequest, ValidateSnippetResponse,
};
use panel_core::auth::hash_password;
use panel_core::site_rules::{
    BasicAuthConfig, BasicAuthUser, CorsConfig, CorsPreset, CustomErrorPage, CustomHeader,
    HotlinkAction, HotlinkConfig, IpAccessConfig, IpAccessRule, IpAction, MaintenanceConfig,
    NginxSnippetConfig, RedirectMatchType, SecurityHeadersConfig, SiteRulesConfig,
    StructuredRedirect,
};
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
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

    let nginx_dir = tmp.path().join("etc/nginx");
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
            linux_username: Some("p_admin".to_string()),
            linux_uid: Some(2000),
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
            plan_id: Some(1),
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
    let app_state = AppState::new(
        agent_client,
        pool.clone(),
        "development".to_string(),
        auth_config,
        lockout_manager,
    );

    let app = create_router(app_state);

    // Login Admin
    let admin_login_req = Request::builder()
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
        .expect("build login");

    let admin_login_resp = app.clone().oneshot(admin_login_req).await.expect("resp");
    assert_eq!(admin_login_resp.status(), StatusCode::OK);

    let mut admin_session = String::new();
    let mut admin_csrf = String::new();
    for val in admin_login_resp.headers().get_all(header::SET_COOKIE) {
        let cookie_str = val.to_str().expect("cookie str");
        if let Some(s) = extract_cookie_value(cookie_str, SESSION_COOKIE_NAME) {
            admin_session = s;
        }
        if let Some(c) = extract_cookie_value(cookie_str, "csrf_token") {
            admin_csrf = c;
        }
    }

    // Login Alice
    let alice_login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "alice",
                "password": "AlicePass123!"
            }))
            .expect("json"),
        ))
        .expect("build login");

    let alice_login_resp = app.clone().oneshot(alice_login_req).await.expect("resp");
    assert_eq!(alice_login_resp.status(), StatusCode::OK);

    let mut alice_session = String::new();
    let mut alice_csrf = String::new();
    for val in alice_login_resp.headers().get_all(header::SET_COOKIE) {
        let cookie_str = val.to_str().expect("cookie str");
        if let Some(s) = extract_cookie_value(cookie_str, SESSION_COOKIE_NAME) {
            alice_session = s;
        }
        if let Some(c) = extract_cookie_value(cookie_str, "csrf_token") {
            alice_csrf = c;
        }
    }

    // Create a static site owned by Alice
    let create_site_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_session),
        )
        .header(CSRF_HEADER_NAME, &alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "domain": "alice-rules.com",
                "user_id": alice.id,
            }))
            .expect("json"),
        ))
        .expect("req");

    let create_site_resp = app.clone().oneshot(create_site_req).await.expect("resp");
    assert_eq!(create_site_resp.status(), StatusCode::CREATED);
    let site_body_bytes = create_site_resp
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let site_json: serde_json::Value =
        serde_json::from_slice(&site_body_bytes).expect("parse site");
    let site_id = site_json["id"].as_i64().expect("site id");

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_session,
        alice_csrf,
        site_id,
    }
}

#[tokio::test]
async fn test_frontend_site_rules_json_round_trip_and_invalid_redirect_preserves_rules() {
    let ctx = setup_test_context().await;
    // Shared with the browser test: do not serialize Rust structs to create this input.
    let vhost_path = ctx
        ._temp
        .path()
        .join(format!("etc/nginx/sites-enabled/site_{}.conf", ctx.site_id));
    let initial_vhost = fs::read_to_string(&vhost_path).unwrap();
    assert!(initial_vhost.contains("location @qpanel_not_found"));
    assert!(initial_vhost.contains("Qinpanel"));
    let mut payload: serde_json::Value = serde_json::from_str(include_str!(
        "../../../web/scripts/fixtures/site-rules.json"
    ))
    .expect("frontend JSON fixture");
    payload["redirects"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "redirect_new", "source": "/old-blog", "target": "/new-blog",
            "status_code": 301, "match_type": "exact", "enabled": true
        }));

    for invalid in [false, true] {
        let mut request_payload = payload.clone();
        if invalid {
            request_payload["redirects"][1]["status_code"] = serde_json::json!(307);
        }
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/sites/{}/rules", ctx.site_id))
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::COOKIE,
                format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
            )
            .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
            .body(Body::from(serde_json::to_vec(&request_payload).unwrap()))
            .unwrap();
        let response = ctx.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            status,
            if invalid {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::OK
            },
            "{}",
            String::from_utf8_lossy(&body)
        );
        if !invalid {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                payload
            );
            let vhost = fs::read_to_string(&vhost_path).unwrap();
            assert!(vhost.contains("error_page 404 /errors/404.html;"));
            assert!(!vhost.contains("location @qpanel_not_found"));
        }
        let get = Request::builder()
            .uri(format!("/api/v1/sites/{}/rules", ctx.site_id))
            .header(
                header::COOKIE,
                format!("{SESSION_COOKIE_NAME}={}", ctx.alice_session),
            )
            .body(Body::empty())
            .unwrap();
        let response = ctx.app.clone().oneshot(get).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            payload
        );
    }
}

#[tokio::test]
async fn test_site_rules_full_lifecycle_and_security() {
    let ctx = setup_test_context().await;

    // 1. GET /api/v1/sites/{id}/rules - default initial rules
    let get_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/rules", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("get req");

    let get_resp = ctx.app.clone().oneshot(get_req).await.expect("resp");
    assert_eq!(get_resp.status(), StatusCode::OK);
    let body_bytes = get_resp
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let initial_rules: SiteRulesConfig = serde_json::from_slice(&body_bytes).expect("parse rules");
    assert!(initial_rules.redirects.is_empty());
    assert!(!initial_rules.basic_auth.enabled);

    // 2. Hash Password endpoint
    let hash_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/rules/hash-password", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&HashPasswordRequest {
                password: "MySuperSecretPassword!".to_string(),
            })
            .expect("json"),
        ))
        .expect("hash req");

    let hash_resp = ctx.app.clone().oneshot(hash_req).await.expect("resp");
    assert_eq!(hash_resp.status(), StatusCode::OK);
    let hash_bytes = hash_resp
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let hash_data: HashPasswordResponse = serde_json::from_slice(&hash_bytes).expect("parse hash");
    assert!(
        hash_data.hash.starts_with("$2b$")
            || hash_data.hash.starts_with("$2a$")
            || hash_data.hash.starts_with("$2y$")
    );

    // 3. Alice attempts to enable custom Nginx snippet -> FORBIDDEN (Admin only)
    let bad_snippet_rules = SiteRulesConfig {
        nginx_snippet: NginxSnippetConfig {
            enabled: true,
            content: "client_body_buffer_size 16k;".to_string(),
        },
        ..SiteRulesConfig::default()
    };

    let put_forbidden_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/rules", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&bad_snippet_rules).expect("json"),
        ))
        .expect("forbidden put");

    let put_forbidden_resp = ctx
        .app
        .clone()
        .oneshot(put_forbidden_req)
        .await
        .expect("resp");
    assert_eq!(put_forbidden_resp.status(), StatusCode::FORBIDDEN);

    // 4. Update with comprehensive site rules as Alice (without snippet)
    let full_rules = SiteRulesConfig {
        redirects: vec![StructuredRedirect {
            id: "redir_1".to_string(),
            source: "/old-blog".to_string(),
            target: "/new-blog".to_string(),
            status_code: 301,
            match_type: RedirectMatchType::Prefix,
            enabled: true,
        }],
        custom_headers: vec![CustomHeader {
            id: "hdr_1".to_string(),
            name: "X-Custom-Env".to_string(),
            value: "Testing".to_string(),
            always: true,
            enabled: true,
        }],
        security_headers: SecurityHeadersConfig {
            hsts_enabled: true,
            hsts_subdomains: true,
            hsts_preload: false,
            x_frame_options: Some("DENY".to_string()),
            x_content_type_options: true,
            referrer_policy: Some("strict-origin-when-cross-origin".to_string()),
            permissions_policy: Some("camera=(), microphone=()".to_string()),
            csp: Some("default-src 'self'".to_string()),
        },
        cors: CorsConfig {
            enabled: true,
            preset: CorsPreset::AllowAll,
            allowed_origins: vec![],
            allowed_methods: vec!["GET".to_string(), "POST".to_string()],
            allowed_headers: vec!["Content-Type".to_string(), "Authorization".to_string()],
            exposed_headers: vec![],
            allow_credentials: true,
            max_age_secs: 86400,
        },
        hotlink_protection: HotlinkConfig {
            enabled: true,
            allowed_domains: vec!["cdn.example.com".to_string()],
            file_extensions: vec!["jpg".to_string(), "png".to_string(), "webp".to_string()],
            allow_empty_referer: true,
            action: HotlinkAction::Deny,
            redirect_url: None,
        },
        basic_auth: BasicAuthConfig {
            enabled: true,
            path: "/admin".to_string(),
            realm: "Area Terproteksi".to_string(),
            users: vec![BasicAuthUser {
                username: "alice_admin".to_string(),
                password_hash: hash_data.hash.clone(),
            }],
        },
        ip_access: IpAccessConfig {
            enabled: true,
            scope: "path".to_string(),
            default_action: IpAction::Deny,
            path: Some("/api/internal".to_string()),
            rules: vec![IpAccessRule {
                id: "rule_1".to_string(),
                cidr_or_ip: "192.168.1.0/24".to_string(),
                action: IpAction::Allow,
                description: Some("Kantor internal".to_string()),
            }],
        },
        maintenance: MaintenanceConfig {
            enabled: true,
            bypass_ips: vec!["127.0.0.1".to_string(), "10.0.0.1".to_string()],
            retry_after_secs: 300,
            custom_message: "Situs sedang dalam pemeliharaan rutin".to_string(),
        },
        custom_error_pages: vec![
            CustomErrorPage {
                status_code: 404,
                target: "/custom_404.html".to_string(),
            },
            CustomErrorPage {
                status_code: 503,
                target: "/custom_503.html".to_string(),
            },
        ],
        nginx_snippet: NginxSnippetConfig::default(),
    };

    let put_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/rules", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_string(&full_rules).expect("json"),
        ))
        .expect("put req");

    let put_resp = ctx.app.clone().oneshot(put_req).await.expect("resp");
    assert_eq!(put_resp.status(), StatusCode::OK);
    let put_bytes = put_resp
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let saved_rules: SiteRulesConfig = serde_json::from_slice(&put_bytes).expect("parse saved");
    assert_eq!(saved_rules.redirects.len(), 1);
    assert_eq!(saved_rules.custom_headers.len(), 1);
    assert!(saved_rules.basic_auth.enabled);
    assert_eq!(saved_rules.basic_auth.users.len(), 1);
    assert!(saved_rules.maintenance.enabled);

    // 5. Admin can validate snippet and enable valid snippet
    let snippet_validate_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/sites/{}/rules/validate-snippet",
            ctx.site_id
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&ValidateSnippetRequest {
                content: "client_body_buffer_size 32k;\nlarge_client_header_buffers 4 16k;"
                    .to_string(),
            })
            .expect("json"),
        ))
        .expect("validate req");

    let val_resp = ctx
        .app
        .clone()
        .oneshot(snippet_validate_req)
        .await
        .expect("resp");
    assert_eq!(val_resp.status(), StatusCode::OK);
    let val_bytes = val_resp
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let val_data: ValidateSnippetResponse = serde_json::from_slice(&val_bytes).expect("parse val");
    assert!(val_data.valid);

    // Dangerous snippet validation fails
    let bad_snippet_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/sites/{}/rules/validate-snippet",
            ctx.site_id
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&ValidateSnippetRequest {
                content: "root /; # malicious root override".to_string(),
            })
            .expect("json"),
        ))
        .expect("validate bad req");

    let bad_val_resp = ctx
        .app
        .clone()
        .oneshot(bad_snippet_req)
        .await
        .expect("resp");
    assert_eq!(bad_val_resp.status(), StatusCode::BAD_REQUEST);

    // Admin updates rules with valid snippet
    let mut admin_rules = full_rules.clone();
    admin_rules.nginx_snippet = NginxSnippetConfig {
        enabled: true,
        content: "client_body_buffer_size 32k;".to_string(),
    };

    let admin_put_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/rules", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&admin_rules).expect("json"),
        ))
        .expect("admin put req");

    let admin_put_resp = ctx.app.clone().oneshot(admin_put_req).await.expect("resp");
    assert_eq!(admin_put_resp.status(), StatusCode::OK);
}
