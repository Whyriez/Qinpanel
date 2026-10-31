#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::CreateUser;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, panel_db::pool::DbPool, tempfile::TempDir) {
    let tmp = tempdir().expect("tempdir");
    let db_file = tmp.path().join("panel.db");
    let sock_path = tmp.path().join("agent.sock");

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

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

    (create_router(state), pool, tmp)
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

async fn login(app: &axum::Router, username: &str, password: &str) -> (String, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": username,
                "password": password
            }))
            .expect("json"),
        ))
        .expect("build");

    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let session_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|h| extract_cookie_value(h, SESSION_COOKIE_NAME))
        .expect("session cookie");

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes).expect("parse json");
    let csrf_token = body["csrf_token"].as_str().expect("csrf").to_string();

    (session_cookie, csrf_token)
}

#[tokio::test]
async fn test_unauthenticated_requests_denied() {
    let (app, _pool, _tmp) = setup_test_app().await;

    // Public routes accessible without credentials
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(Body::empty())
                .expect("build"),
        )
        .await
        .expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/system/info")
                .body(Body::empty())
                .expect("build"),
        )
        .await
        .expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Protected user & admin routes reject unauthenticated requests with 401 Unauthorized
    let endpoints = vec![
        ("GET", "/api/v1/auth/me"),
        ("GET", "/api/v1/auth/csrf"),
        ("GET", "/api/v1/admin/users"),
        ("GET", "/api/v1/admin/users/1"),
        ("GET", "/api/v1/admin/settings?key=app.name"),
        ("GET", "/api/v1/user/1/profile"),
    ];

    for (method, uri) in endpoints {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("build");
        let res = app.clone().oneshot(req).await.expect("res");
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "Endpoint {} should require authentication",
            uri
        );
    }
}

#[tokio::test]
async fn test_user_forbidden_on_admin_routes() {
    let (app, pool, _tmp) = setup_test_app().await;

    // Create regular tenant user
    let user_repo = SqliteUserRepository::new(pool.clone());
    let password_hash = hash_password("TenantPass123!").expect("hash");
    let user = user_repo
        .create(&CreateUser {
            username: "regular_user".to_string(),
            password_hash,
            role: "user".to_string(),
            email: Some("tenant@example.com".to_string()),
            linux_username: Some("p_tenant".to_string()),
            linux_uid: Some(10001),
            plan_id: None,
        })
        .await
        .expect("create user");

    let (cookie, csrf) = login(&app, "regular_user", "TenantPass123!").await;

    // 1. Regular user can access user-level endpoints
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // 2. Regular user is FORBIDDEN from all admin-only GET endpoints
    let admin_get_endpoints = vec![
        "/api/v1/admin/users",
        "/api/v1/admin/users/1",
        "/api/v1/admin/settings?key=panel.name",
    ];

    for uri in admin_get_endpoints {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .header(
                header::COOKIE,
                format!("{}={}", SESSION_COOKIE_NAME, cookie),
            )
            .body(Body::empty())
            .expect("build");
        let res = app.clone().oneshot(req).await.expect("res");
        assert_eq!(
            res.status(),
            StatusCode::FORBIDDEN,
            "User should be forbidden from {}",
            uri
        );

        let body_bytes = res.into_body().collect().await.expect("b").to_bytes();
        let body: Value = serde_json::from_slice(&body_bytes).expect("json");
        assert!(
            body["error"]
                .as_str()
                .expect("err")
                .contains("Administrator privileges required")
        );
    }

    // 3. Regular user is FORBIDDEN from admin-only POST endpoints
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/admin/users/{}/status", user.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .header(CSRF_HEADER_NAME, &csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"status":"suspended"}"#))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/settings")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .header(CSRF_HEADER_NAME, &csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"key":"panel.title","value":"Hacked"}"#))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_admin_allowed_on_admin_routes_with_csrf() {
    let (app, pool, _tmp) = setup_test_app().await;

    // Create admin user and a target tenant user
    let user_repo = SqliteUserRepository::new(pool.clone());
    let admin_hash = hash_password("AdminPass123!").expect("hash");
    let admin = user_repo
        .create(&CreateUser {
            username: "root_admin".to_string(),
            password_hash: admin_hash,
            role: "admin".to_string(),
            email: Some("admin@nusa.local".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create admin");

    let tenant_hash = hash_password("TenantPass123!").expect("hash");
    let tenant = user_repo
        .create(&CreateUser {
            username: "managed_tenant".to_string(),
            password_hash: tenant_hash,
            role: "user".to_string(),
            email: Some("tenant@nusa.local".to_string()),
            linux_username: Some("p_managed".to_string()),
            linux_uid: Some(10002),
            plan_id: None,
        })
        .await
        .expect("create tenant");

    let (cookie, csrf) = login(&app, "root_admin", "AdminPass123!").await;

    // 1. Admin lists all users -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/admin/users")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let users: Vec<Value> =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("json");
    assert_eq!(users.len(), 2);

    // 2. Admin inspects tenant details -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/admin/users/{}", tenant.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let tenant_res: Value =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("json");
    assert_eq!(tenant_res["username"], "managed_tenant");
    assert_eq!(tenant_res["role"], "user");

    // 3. Admin updates settings WITHOUT CSRF -> 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/settings")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"key":"site.title","value":"Nusa Hosting"}"#))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 4. Admin updates settings WITH CSRF -> 200 OK
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/settings")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .header(CSRF_HEADER_NAME, &csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"key":"site.title","value":"Nusa Hosting"}"#))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // 5. Admin reads back setting -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/admin/settings?key=site.title")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let setting_res: Value =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("json");
    assert_eq!(setting_res["value"], "Nusa Hosting");

    // 6. Admin updates user status -> 200 OK
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/admin/users/{}/status", tenant.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .header(CSRF_HEADER_NAME, &csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"status":"suspended"}"#))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // Verify tenant status is now suspended in DB
    let user_in_db = user_repo
        .find_by_id(tenant.id)
        .await
        .expect("db")
        .expect("exists");
    assert_eq!(user_in_db.status, "suspended");
    assert_eq!(admin.id, admin.id); // sanity check
}

#[tokio::test]
async fn test_tenant_resource_isolation_and_cross_user_denial() {
    let (app, pool, _tmp) = setup_test_app().await;

    let user_repo = SqliteUserRepository::new(pool.clone());
    let pass_a = hash_password("PasswordA123!").expect("hash");
    let user_a = user_repo
        .create(&CreateUser {
            username: "tenant_a".to_string(),
            password_hash: pass_a,
            role: "user".to_string(),
            email: Some("a@example.com".to_string()),
            linux_username: Some("p_a".to_string()),
            linux_uid: Some(10010),
            plan_id: None,
        })
        .await
        .expect("create a");

    let pass_b = hash_password("PasswordB123!").expect("hash");
    let user_b = user_repo
        .create(&CreateUser {
            username: "tenant_b".to_string(),
            password_hash: pass_b,
            role: "user".to_string(),
            email: Some("b@example.com".to_string()),
            linux_username: Some("p_b".to_string()),
            linux_uid: Some(10011),
            plan_id: None,
        })
        .await
        .expect("create b");

    let (cookie_a, _) = login(&app, "tenant_a", "PasswordA123!").await;

    // Tenant A can access Tenant A's own profile -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/user/{}/profile", user_a.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie_a),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // Tenant A attempts to access Tenant B's profile -> 403 Forbidden!
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/user/{}/profile", user_b.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie_a),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let body_bytes = res.into_body().collect().await.expect("b").to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes).expect("json");
    assert!(
        body["error"]
            .as_str()
            .expect("err")
            .contains("another user")
    );
}

#[tokio::test]
async fn test_suspended_user_immediately_rejected_by_middleware() {
    let (app, pool, _tmp) = setup_test_app().await;

    let user_repo = SqliteUserRepository::new(pool.clone());
    let pass = hash_password("PasswordActive123!").expect("hash");
    let user = user_repo
        .create(&CreateUser {
            username: "active_then_suspended".to_string(),
            password_hash: pass,
            role: "user".to_string(),
            email: Some("test@example.com".to_string()),
            linux_username: Some("p_test".to_string()),
            linux_uid: Some(10020),
            plan_id: None,
        })
        .await
        .expect("create user");

    // Login while still active -> succeeds
    let (cookie, _) = login(&app, "active_then_suspended", "PasswordActive123!").await;

    // Confirm session is valid
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // Suspend user in DB
    user_repo
        .update_status(user.id, "suspended")
        .await
        .expect("suspend user");

    // Subsequent request using existing session is rejected with 403 Forbidden
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let body_bytes = res.into_body().collect().await.expect("b").to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes).expect("json");
    assert!(body["error"].as_str().expect("err").contains("suspended"));
}
