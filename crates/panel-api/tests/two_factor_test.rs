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
use panel_core::totp::create_totp_instance;
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

async fn login_user(
    app: &axum::Router,
    username: &str,
    password: &str,
    code: Option<&str>,
) -> (StatusCode, Option<String>, Option<String>, Value) {
    let mut payload = serde_json::json!({
        "username": username,
        "password": password
    });
    if let Some(c) = code {
        payload["code"] = serde_json::json!(c);
    }

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&payload).expect("json")))
        .expect("build");

    let res = app.clone().oneshot(req).await.expect("res");
    let status = res.status();

    let session_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|h| extract_cookie_value(h, SESSION_COOKIE_NAME));

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes).expect("parse json");
    let csrf = body["csrf_token"].as_str().map(|s| s.to_string());

    (status, session_cookie, csrf, body)
}

#[tokio::test]
async fn test_2fa_full_lifecycle() {
    let (app, pool, _tmp) = setup_test_app().await;

    // 1. Create a user
    let user_repo = SqliteUserRepository::new(pool.clone());
    let password_hash = hash_password("Password123!").expect("hash");
    user_repo
        .create(&CreateUser {
            username: "totpuser".to_string(),
            password_hash,
            role: "user".to_string(),
            email: Some("totp@example.com".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create user");

    // 2. Initial login (no 2FA)
    let (status, cookie, csrf, body) = login_user(&app, "totpuser", "Password123!", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["requires_2fa"], false);
    let cookie = cookie.expect("cookie");
    let csrf = csrf.expect("csrf");

    // 3. Check 2FA status (initially disabled)
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/2fa/status")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let status_body: Value =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("json");
    assert_eq!(status_body["enabled"], false);
    assert_eq!(status_body["remaining_recovery_codes"], 0);

    // 4. Setup 2FA (POST /api/v1/auth/2fa/setup)
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/2fa/setup")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .header(CSRF_HEADER_NAME, &csrf)
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let setup_body: Value =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("json");

    let secret = setup_body["secret"].as_str().expect("secret").to_string();
    let otpauth_url = setup_body["otpauth_url"].as_str().expect("otpauth");
    let qr_base64 = setup_body["qr_code_base64"].as_str().expect("qr");
    let recovery_codes: Vec<String> = setup_body["recovery_codes"]
        .as_array()
        .expect("recovery")
        .iter()
        .map(|v| v.as_str().expect("str").to_string())
        .collect();

    assert!(otpauth_url.starts_with("otpauth://totp/"));
    assert!(!qr_base64.is_empty());
    assert_eq!(recovery_codes.len(), 8);

    // 5. Attempt enable with wrong TOTP code -> 400
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/2fa/enable")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .header(CSRF_HEADER_NAME, &csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "secret": secret,
                "code": "000000",
                "recovery_codes": recovery_codes,
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 6. Enable with valid TOTP code
    let totp_instance = create_totp_instance(&secret, "totpuser", None).expect("totp instance");
    let valid_code = totp_instance.generate_current().expect("totp token");

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/2fa/enable")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .header(CSRF_HEADER_NAME, &csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "secret": secret,
                "code": valid_code,
                "recovery_codes": recovery_codes,
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // 7. Status should now show enabled = true and 8 remaining recovery codes
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/2fa/status")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let status_body: Value =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("json");
    assert_eq!(status_body["enabled"], true);
    assert_eq!(status_body["remaining_recovery_codes"], 8);

    // 8. New Login flow: Login without code challenges for 2FA
    let (status, login_cookie, _, body) = login_user(&app, "totpuser", "Password123!", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["requires_2fa"], true);
    assert!(body["user"].is_null());
    assert!(login_cookie.is_none());

    // 9. Login with invalid 2FA code fails
    let (status, _, _, _) = login_user(&app, "totpuser", "Password123!", Some("999999")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 10. Login with valid TOTP code succeeds
    let current_code = totp_instance.generate_current().expect("totp token");
    let (status, login_cookie, _, body) =
        login_user(&app, "totpuser", "Password123!", Some(&current_code)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["requires_2fa"], false);
    assert_eq!(body["user"]["username"], "totpuser");
    assert!(login_cookie.is_some());

    // 11. Login with a recovery code succeeds and decrements remaining codes
    let recovery_code = &recovery_codes[0];
    let (status, rec_cookie, rec_csrf, body) =
        login_user(&app, "totpuser", "Password123!", Some(recovery_code)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["requires_2fa"], false);
    assert!(rec_cookie.is_some());
    let rec_cookie = rec_cookie.expect("cookie");
    let rec_csrf = rec_csrf.expect("csrf");

    // Check remaining recovery codes is now 7
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/2fa/status")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, rec_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let status_body: Value =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("json");
    assert_eq!(status_body["remaining_recovery_codes"], 7);

    // 12. Attempt to reuse the SAME recovery code fails
    let (status, _, _, _) = login_user(&app, "totpuser", "Password123!", Some(recovery_code)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 13. Disable 2FA with another recovery code (or TOTP code)
    let another_recovery_code = &recovery_codes[1];
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/2fa/disable")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, rec_cookie),
        )
        .header(CSRF_HEADER_NAME, &rec_csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "code": another_recovery_code,
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // 14. Check status after disable
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/2fa/status")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, rec_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let status_body: Value =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("json");
    assert_eq!(status_body["enabled"], false);
    assert_eq!(status_body["remaining_recovery_codes"], 0);

    // 15. Subsequent login does not challenge for 2FA anymore
    let (status, final_cookie, _, body) = login_user(&app, "totpuser", "Password123!", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["requires_2fa"], false);
    assert!(final_cookie.is_some());
}
