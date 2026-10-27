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
use panel_api::sites::{
    AddSiteDomainRequest, CreateStaticSiteRequest, SetForceHttpsRequest, SiteDomainResponse,
    SiteResponse, UpdateSiteDomainRequest,
};
use panel_core::auth::hash_password;
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
            serde_json::json!({
                "username": "admin",
                "password": "AdminPass123!"
            })
            .to_string(),
        ))
        .expect("build login req");

    let login_res = app.clone().oneshot(login_req).await.expect("login oneshot");
    assert_eq!(login_res.status(), StatusCode::OK);

    let cookie_hdr = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .expect("cookie str");
    let admin_session =
        extract_cookie_value(cookie_hdr, SESSION_COOKIE_NAME).expect("session cookie");

    let csrf_req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={admin_session}"),
        )
        .body(Body::empty())
        .expect("build csrf req");

    let csrf_res = app.clone().oneshot(csrf_req).await.expect("csrf oneshot");
    let csrf_body = csrf_res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let csrf_json: serde_json::Value = serde_json::from_slice(&csrf_body).expect("parse json");
    let admin_csrf = csrf_json["csrf_token"]
        .as_str()
        .expect("csrf_token")
        .to_string();

    TestContext {
        app,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        admin_session,
        admin_csrf,
        alice_id: alice_user.id,
    }
}

#[tokio::test]
async fn test_domain_alias_redirect_and_force_https_lifecycle() {
    let ctx = setup_test_context().await;

    // 1. Create site with primary domain
    let create_payload = CreateStaticSiteRequest {
        domain: "main-site.test".to_string(),
        user_id: Some(ctx.alice_id),
        spa_fallback: Some(false),
        force_https: Some(false),
        docroot: None,
        ..Default::default()
    };

    let create_req = Request::builder()
        .method("POST")
        .uri("/api/v1/sites")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&create_payload).unwrap()))
        .unwrap();

    let create_res = ctx.app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(create_res.status(), StatusCode::CREATED);
    let body = create_res.into_body().collect().await.unwrap().to_bytes();
    let site: SiteResponse = serde_json::from_slice(&body).unwrap();
    let site_id = site.id;
    assert_eq!(site.primary_domain, "main-site.test");
    assert!(!site.force_https);

    // 2. Add an alias domain (redirect_mode: "none")
    let alias_payload = AddSiteDomainRequest {
        domain: "alias-site.test".to_string(),
        redirect_mode: Some("none".to_string()),
        redirect_target: None,
        is_primary: Some(false),
    };
    let add_alias_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/domains"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&alias_payload).unwrap()))
        .unwrap();

    let add_alias_res = ctx.app.clone().oneshot(add_alias_req).await.unwrap();
    assert_eq!(add_alias_res.status(), StatusCode::CREATED);
    let alias_body = add_alias_res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let alias_domain: SiteDomainResponse = serde_json::from_slice(&alias_body).unwrap();
    assert_eq!(alias_domain.name, "alias-site.test");
    assert_eq!(alias_domain.redirect_mode, "none");
    assert!(!alias_domain.is_primary);
    let alias_domain_id = alias_domain.id;

    // 3. Add a subdomain (redirect_mode: "none")
    let sub_payload = AddSiteDomainRequest {
        domain: "blog.main-site.test".to_string(),
        redirect_mode: Some("none".to_string()),
        redirect_target: None,
        is_primary: Some(false),
    };
    let add_sub_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/domains"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&sub_payload).unwrap()))
        .unwrap();

    let add_sub_res = ctx.app.clone().oneshot(add_sub_req).await.unwrap();
    assert_eq!(add_sub_res.status(), StatusCode::CREATED);

    // 4. Add domain with redirect_mode "to_primary"
    let redir_primary_payload = AddSiteDomainRequest {
        domain: "old-portal.test".to_string(),
        redirect_mode: Some("to_primary".to_string()),
        redirect_target: None,
        is_primary: Some(false),
    };
    let add_redir_prim_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/domains"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&redir_primary_payload).unwrap(),
        ))
        .unwrap();

    let add_redir_prim_res = ctx.app.clone().oneshot(add_redir_prim_req).await.unwrap();
    assert_eq!(add_redir_prim_res.status(), StatusCode::CREATED);

    // 5. Add domain with redirect_mode "to_www"
    let redir_www_payload = AddSiteDomainRequest {
        domain: "shop-brand.test".to_string(),
        redirect_mode: Some("to_www".to_string()),
        redirect_target: None,
        is_primary: Some(false),
    };
    let add_redir_www_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/domains"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&redir_www_payload).unwrap(),
        ))
        .unwrap();

    let add_redir_www_res = ctx.app.clone().oneshot(add_redir_www_req).await.unwrap();
    assert_eq!(add_redir_www_res.status(), StatusCode::CREATED);
    let redir_www_body = add_redir_www_res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let redir_www_domain: SiteDomainResponse = serde_json::from_slice(&redir_www_body).unwrap();
    let redir_www_domain_id = redir_www_domain.id;

    // 6. Add domain with redirect_mode "to_non_www"
    let redir_nonwww_payload = AddSiteDomainRequest {
        domain: "www.extra-brand.test".to_string(),
        redirect_mode: Some("to_non_www".to_string()),
        redirect_target: None,
        is_primary: Some(false),
    };
    let add_redir_nonwww_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/domains"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&redir_nonwww_payload).unwrap(),
        ))
        .unwrap();

    let add_redir_nonwww_res = ctx.app.clone().oneshot(add_redir_nonwww_req).await.unwrap();
    assert_eq!(add_redir_nonwww_res.status(), StatusCode::CREATED);

    // 7. Add domain with custom redirect
    let redir_custom_payload = AddSiteDomainRequest {
        domain: "promo-link.test".to_string(),
        redirect_mode: Some("custom".to_string()),
        redirect_target: Some("https://landing-page.com/special".to_string()),
        is_primary: Some(false),
    };
    let add_redir_custom_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/domains"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(
            serde_json::to_string(&redir_custom_payload).unwrap(),
        ))
        .unwrap();

    let add_redir_custom_res = ctx.app.clone().oneshot(add_redir_custom_req).await.unwrap();
    assert_eq!(add_redir_custom_res.status(), StatusCode::CREATED);
    let custom_body = add_redir_custom_res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let custom_domain: SiteDomainResponse = serde_json::from_slice(&custom_body).unwrap();
    let custom_domain_id = custom_domain.id;

    // 8. List domains for site
    let list_domains_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{site_id}/domains"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let list_domains_res = ctx.app.clone().oneshot(list_domains_req).await.unwrap();
    assert_eq!(list_domains_res.status(), StatusCode::OK);
    let list_domains_body = list_domains_res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let domain_list: Vec<SiteDomainResponse> = serde_json::from_slice(&list_domains_body).unwrap();
    assert_eq!(domain_list.len(), 7); // 1 primary + 6 added

    // 9. Update domain configuration (change shop-brand.test to "to_primary")
    let update_payload = UpdateSiteDomainRequest {
        redirect_mode: Some("to_primary".to_string()),
        redirect_target: None,
        is_primary: Some(false),
    };
    let update_req = Request::builder()
        .method("PUT")
        .uri(format!(
            "/api/v1/sites/{site_id}/domains/{redir_www_domain_id}"
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&update_payload).unwrap()))
        .unwrap();

    let update_res = ctx.app.clone().oneshot(update_req).await.unwrap();
    assert_eq!(update_res.status(), StatusCode::OK);
    let update_body = update_res.into_body().collect().await.unwrap().to_bytes();
    let updated_domain: SiteDomainResponse = serde_json::from_slice(&update_body).unwrap();
    assert_eq!(updated_domain.redirect_mode, "to_primary");

    // 10. Promote alias-site.test to primary domain
    let promote_payload = UpdateSiteDomainRequest {
        redirect_mode: Some("none".to_string()),
        redirect_target: None,
        is_primary: Some(true),
    };
    let promote_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{site_id}/domains/{alias_domain_id}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&promote_payload).unwrap()))
        .unwrap();

    let promote_res = ctx.app.clone().oneshot(promote_req).await.unwrap();
    assert_eq!(promote_res.status(), StatusCode::OK);

    // Verify site now has alias-site.test as primary_domain
    let get_site_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{site_id}"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let get_site_res = ctx.app.clone().oneshot(get_site_req).await.unwrap();
    let get_site_body = get_site_res.into_body().collect().await.unwrap().to_bytes();
    let updated_site: SiteResponse = serde_json::from_slice(&get_site_body).unwrap();
    assert_eq!(updated_site.primary_domain, "alias-site.test");

    // 11. Reject deleting current primary domain
    let del_primary_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/sites/{site_id}/domains/{alias_domain_id}"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let del_primary_res = ctx.app.clone().oneshot(del_primary_req).await.unwrap();
    assert_eq!(del_primary_res.status(), StatusCode::BAD_REQUEST);

    // 12. Delete auxiliary custom domain
    let del_custom_req = Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/v1/sites/{site_id}/domains/{custom_domain_id}"
        ))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::empty())
        .unwrap();

    let del_custom_res = ctx.app.clone().oneshot(del_custom_req).await.unwrap();
    assert_eq!(del_custom_res.status(), StatusCode::OK);

    // Verify domain count decreased to 6
    let list_after_del_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{site_id}/domains"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .body(Body::empty())
        .unwrap();

    let list_after_del_res = ctx.app.clone().oneshot(list_after_del_req).await.unwrap();
    let list_after_body = list_after_del_res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let remaining_domains: Vec<SiteDomainResponse> =
        serde_json::from_slice(&list_after_body).unwrap();
    assert_eq!(remaining_domains.len(), 6);

    // 13. Toggle force-HTTPS
    let https_payload = SetForceHttpsRequest { force_https: true };
    let https_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{site_id}/force-https"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", ctx.admin_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.admin_csrf)
        .body(Body::from(serde_json::to_string(&https_payload).unwrap()))
        .unwrap();

    let https_res = ctx.app.clone().oneshot(https_req).await.unwrap();
    assert_eq!(https_res.status(), StatusCode::OK);
    let https_body = https_res.into_body().collect().await.unwrap().to_bytes();
    let site_with_https: SiteResponse = serde_json::from_slice(&https_body).unwrap();
    assert!(site_with_https.force_https);
}
