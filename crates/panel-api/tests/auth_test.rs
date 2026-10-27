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

async fn setup_test_app(
    max_failed_attempts: u32,
    lockout_secs: u64,
) -> (axum::Router, panel_db::pool::DbPool, tempfile::TempDir) {
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
        max_failed_attempts,
        lockout_duration_secs: lockout_secs,
        cookie_secure: Some(false),
    };

    let lockout_manager = Arc::new(LockoutManager::new(max_failed_attempts, lockout_secs));

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

#[tokio::test]
async fn test_auth_login_me_and_session_cookie() {
    let (app, pool, _tmp) = setup_test_app(5, 900).await;

    // Create user in DB
    let user_repo = SqliteUserRepository::new(pool.clone());
    let password_hash = hash_password("AdminSecret123!").expect("hash");
    user_repo
        .create(&CreateUser {
            username: "testadmin".to_string(),
            password_hash,
            role: "admin".to_string(),
            email: Some("admin@nusa.local".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create user");

    // 1. Login with invalid password
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "testadmin",
                "password": "WrongPassword!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. Login with valid credentials
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "testadmin",
                "password": "AdminSecret123!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let cookie_header = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .expect("to_str")
        .to_string();
    assert!(cookie_header.contains(SESSION_COOKIE_NAME));
    assert!(cookie_header.contains("HttpOnly"));
    assert!(cookie_header.contains("SameSite=Strict"));

    let session_token =
        extract_cookie_value(&cookie_header, SESSION_COOKIE_NAME).expect("session token");
    assert!(!session_token.is_empty());

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let login_res: Value = serde_json::from_slice(&body_bytes).expect("parse json");
    let csrf_token = login_res["csrf_token"]
        .as_str()
        .expect("csrf_token")
        .to_string();
    assert_eq!(login_res["user"]["username"], "testadmin");
    assert_eq!(login_res["user"]["role"], "admin");

    // 3. GET /api/v1/auth/me without cookie -> 401
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 4. GET /api/v1/auth/me with session cookie -> 200
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session_token),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let me_res: Value = serde_json::from_slice(&body_bytes).expect("parse json");
    assert_eq!(me_res["user"]["username"], "testadmin");
    assert_eq!(me_res["csrf_token"], csrf_token);
}

#[tokio::test]
async fn test_csrf_protection_and_logout() {
    let (app, pool, _tmp) = setup_test_app(5, 900).await;

    let user_repo = SqliteUserRepository::new(pool.clone());
    let password_hash = hash_password("AdminSecret123!").expect("hash");
    user_repo
        .create(&CreateUser {
            username: "csrftest".to_string(),
            password_hash,
            role: "admin".to_string(),
            email: None,
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create user");

    // Login
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "csrftest",
                "password": "AdminSecret123!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let cookie_header = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .expect("to_str")
        .to_string();
    let session_token =
        extract_cookie_value(&cookie_header, SESSION_COOKIE_NAME).expect("session token");

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let login_res: Value = serde_json::from_slice(&body_bytes).expect("parse json");
    let csrf_token = login_res["csrf_token"]
        .as_str()
        .expect("csrf_token")
        .to_string();

    // 1. Attempt logout WITHOUT CSRF token -> 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/logout")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session_token),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 2. Attempt logout with INVALID CSRF token -> 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/logout")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session_token),
        )
        .header(CSRF_HEADER_NAME, "invalid-csrf-token")
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 3. Logout with VALID CSRF token -> 200 OK
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/logout")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session_token),
        )
        .header(CSRF_HEADER_NAME, &csrf_token)
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let clear_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("clear cookie")
        .to_str()
        .expect("to_str");
    assert!(clear_cookie.contains("Max-Age=0"));

    // 4. Session is now invalid -> /api/v1/auth/me returns 401
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, session_token),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_change_password_and_session_rotation() {
    let (app, pool, _tmp) = setup_test_app(5, 900).await;

    let user_repo = SqliteUserRepository::new(pool.clone());
    let password_hash = hash_password("OldPassword123!").expect("hash");
    user_repo
        .create(&CreateUser {
            username: "rotateuser".to_string(),
            password_hash,
            role: "admin".to_string(),
            email: None,
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create user");

    // Login
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "rotateuser",
                "password": "OldPassword123!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    let cookie_header = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("cookie")
        .to_str()
        .expect("str")
        .to_string();
    let old_session_token =
        extract_cookie_value(&cookie_header, SESSION_COOKIE_NAME).expect("token");
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let login_res: Value = serde_json::from_slice(&body_bytes).expect("json");
    let csrf_token = login_res["csrf_token"].as_str().expect("csrf").to_string();

    // 1. Change password with wrong current password -> 400
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, old_session_token),
        )
        .header(CSRF_HEADER_NAME, &csrf_token)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "current_password": "WrongPassword!",
                "new_password": "NewSecretPassword456!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 2. Change password with short new password (< 8 chars) -> 400
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, old_session_token),
        )
        .header(CSRF_HEADER_NAME, &csrf_token)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "current_password": "OldPassword123!",
                "new_password": "short"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 3. Successful change password -> 200, returns rotated session cookie
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, old_session_token),
        )
        .header(CSRF_HEADER_NAME, &csrf_token)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "current_password": "OldPassword123!",
                "new_password": "NewSecretPassword456!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let new_cookie_header = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("new cookie")
        .to_str()
        .expect("str")
        .to_string();
    let new_session_token =
        extract_cookie_value(&new_cookie_header, SESSION_COOKIE_NAME).expect("new token");
    assert_ne!(new_session_token, old_session_token);

    // 4. Old session token should now be invalidated
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, old_session_token),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 5. New session token works
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, new_session_token),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Old password no longer works for login
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "rotateuser",
                "password": "OldPassword123!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 7. New password succeeds
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "rotateuser",
                "password": "NewSecretPassword456!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_rate_limit_and_lockout() {
    // Set max_failed_attempts = 3, lockout_duration = 300s
    let (app, pool, _tmp) = setup_test_app(3, 300).await;

    let user_repo = SqliteUserRepository::new(pool.clone());
    let password_hash = hash_password("ValidPassword123!").expect("hash");
    user_repo
        .create(&CreateUser {
            username: "lockoutuser".to_string(),
            password_hash,
            role: "admin".to_string(),
            email: None,
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create user");

    // Attempt 1: Failed
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"lockoutuser","password":"wrong1"}"#,
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Attempt 2: Failed
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"lockoutuser","password":"wrong2"}"#,
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Attempt 3: Failed -> triggers lockout!
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"lockoutuser","password":"wrong3"}"#,
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::LOCKED);

    // Attempt 4: Even with CORRECT password, account is locked!
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"lockoutuser","password":"ValidPassword123!"}"#,
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::LOCKED);

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let lock_res: Value = serde_json::from_slice(&body_bytes).expect("parse json");
    assert!(lock_res["error"].as_str().expect("err").contains("locked"));
}
