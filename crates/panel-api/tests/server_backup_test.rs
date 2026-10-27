#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Router, RBAC/CSRF and input checks with SQLite only, no running agent.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use panel_api::{
    agent_client::AgentClient,
    auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME, generate_csrf_token},
    config::AuthConfig,
    routes::AppState,
};
use panel_db::{CreateSession, CreateUser, SqliteSessionRepository, SqliteUserRepository};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;
use utoipa::OpenApi;

#[tokio::test]
async fn migration_routes_require_admin_csrf_reauthentication_and_bounded_uploads() {
    let temp = tempfile::tempdir().unwrap();
    let pool = panel_db::init_pool("sqlite::memory:").await.unwrap();
    panel_db::run_migrations(&pool).await.unwrap();
    let mut cookies = vec![];
    for (index, role) in ["admin", "user"].iter().enumerate() {
        let user = SqliteUserRepository::new(pool.clone())
            .create(&CreateUser {
                username: format!("actor{index}"),
                password_hash: panel_core::hash_password("fixture-valid").unwrap(),
                role: (*role).into(),
                email: None,
                linux_username: None,
                linux_uid: None,
                plan_id: None,
            })
            .await
            .unwrap();
        let token = format!("token{index}");
        let session = format!("session{index}");
        SqliteSessionRepository::new(pool.clone())
            .create(&CreateSession {
                id: session.clone(),
                user_id: user.id,
                token_hash: panel_core::hash_token(&token),
                ip_address: None,
                user_agent: None,
                expires_at: chrono::Utc::now().timestamp() + 3600,
            })
            .await
            .unwrap();
        cookies.push((
            format!("{SESSION_COOKIE_NAME}={token}"),
            generate_csrf_token(&session),
        ));
    }
    let state = AppState::new(
        Arc::new(AgentClient::new(temp.path().join("no-agent"))),
        pool,
        "development".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 60)),
    );
    let app = panel_api::create_router(state);
    for path in [
        "/api/v1/system/server-backup",
        "/api/v1/system/server-backup/archives/not-a-uuid",
    ] {
        let req = Request::builder().uri(path).body(Body::empty()).unwrap();
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let req = Request::builder()
            .uri(path)
            .header("Cookie", &cookies[1].0)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    let endpoint = "/api/v1/system/server-backup/jobs";
    for (cookie, csrf) in [(&cookies[0].0, ""), (&cookies[1].0, cookies[1].1.as_str())] {
        let req = Request::builder()
            .method("POST")
            .uri(endpoint)
            .header("Cookie", cookie)
            .header(CSRF_HEADER_NAME, csrf)
            .header("Content-Type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    for body in [
        json!({"id":"bad","operation":"create","passphrase":"valid fixture passphrase"}),
        json!({"id":"12345678-1234-1234-1234-123456789abc","operation":"create","passphrase":"short"}),
        json!({"id":"12345678-1234-1234-1234-123456789abc","operation":"create","passphrase":"valid fixture passphrase","path":"/etc/shadow"}),
    ] {
        let req = Request::builder()
            .method("POST")
            .uri(endpoint)
            .header("Cookie", &cookies[0].0)
            .header(CSRF_HEADER_NAME, &cookies[0].1)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        assert!(
            app.clone()
                .oneshot(req)
                .await
                .unwrap()
                .status()
                .is_client_error()
        );
    }
    let body = json!({"id":"12345678-1234-1234-1234-123456789abc","operation":"restore","archive_id":"12345678-1234-1234-1234-123456789abc","digest":"a".repeat(64),"passphrase":"valid fixture passphrase","confirm_restore":true,"admin_password":"wrong"});
    let req = Request::builder()
        .method("POST")
        .uri(endpoint)
        .header("Cookie", &cookies[0].0)
        .header(CSRF_HEADER_NAME, &cookies[0].1)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/system/server-backup/uploads/not-valid?offset=0&total=10000000")
        .header("Cookie", &cookies[0].0)
        .header(CSRF_HEADER_NAME, &cookies[0].1)
        .body(Body::from(vec![0u8; 9 * 1024 * 1024]))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}
#[test]
fn migration_endpoints_are_exported_without_secret_examples() {
    let value = serde_json::to_value(panel_api::routes::ApiDoc::openapi()).unwrap();
    for path in [
        "/api/v1/system/server-backup",
        "/api/v1/system/server-backup/jobs",
        "/api/v1/system/server-backup/uploads/{id}",
        "/api/v1/system/server-backup/archives/{id}",
    ] {
        assert!(value["paths"].get(path).is_some());
    }
}
