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

#[test]
fn update_openapi_includes_resolvable_request_and_status_contracts() {
    let doc = serde_json::to_value(panel_api::routes::ApiDoc::openapi()).unwrap();
    for name in [
        "ApplyUpdate",
        "PanelUpdateStatus",
        "PanelUpdateRun",
        "PanelRelease",
        "UpdatePhase",
    ] {
        assert!(
            doc["components"]["schemas"].get(name).is_some(),
            "Missing schema: {name}"
        );
    }
    for path in [
        "/api/v1/system/update",
        "/api/v1/system/update/check",
        "/api/v1/system/update/apply",
        "/api/v1/system/update/logs",
    ] {
        assert!(doc["paths"].get(path).is_some());
    }
    assert_eq!(
        doc["components"]["schemas"]["ApplyUpdate"]["additionalProperties"],
        false
    );
    assert_eq!(
        doc["paths"]["/api/v1/system/update/apply"]["post"]["responses"]["202"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/PanelUpdateStatus"
    );
}

#[tokio::test]
async fn update_routes_require_admin_csrf_and_valid_typed_input_before_rpc() {
    let temp = tempfile::tempdir().unwrap();
    let pool = panel_db::init_pool("sqlite::memory:").await.unwrap();
    panel_db::run_migrations(&pool).await.unwrap();
    let mut cookies = vec![];
    for (index, role) in ["admin", "user"].iter().enumerate() {
        let user = SqliteUserRepository::new(pool.clone())
            .create(&CreateUser {
                username: format!("actor{index}"),
                password_hash: "fixture".into(),
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
    for path in ["/api/v1/system/update", "/api/v1/system/update/logs"] {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let request = Request::builder()
            .uri(path)
            .header("Cookie", &cookies[1].0)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let request = Request::builder()
            .uri(path)
            .header("Cookie", &cookies[0].0)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    for path in ["/api/v1/system/update/check", "/api/v1/system/update/apply"] {
        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header("Cookie", &cookies[0].0)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header("Cookie", &cookies[1].0)
            .header(CSRF_HEADER_NAME, &cookies[1].1)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    for body in [
        json!({"id":"invalid","version":"v1.0.0","digest":"a".repeat(64)}),
        json!({"id":"12345678-1234-1234-1234-123456789abc","version":"v1.0.0;id","digest":"a".repeat(64)}),
        json!({"id":"12345678-1234-1234-1234-123456789abc","version":"v1.0.0","digest":"a".repeat(64),"url":"https://untrusted.invalid"}),
    ] {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/system/update/apply")
            .header("Cookie", &cookies[0].0)
            .header(CSRF_HEADER_NAME, &cookies[0].1)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}
