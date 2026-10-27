#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Router/SQLite contract tests; no agent server or host services are started.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    agent_client::AgentClient,
    auth::{LockoutManager, SESSION_COOKIE_NAME, generate_csrf_token},
    config::AuthConfig,
    create_router,
    routes::AppState,
};
use panel_core::auth::hash_token;
use panel_db::{
    models::{CreateSession, CreateUser},
    pool::{DbPool, init_pool, run_migrations},
    repositories::{SqlitePlanRepository, SqliteSessionRepository, SqliteUserRepository},
};
use serde_json::{Value, json};
use std::sync::Arc;

async fn fixture() -> (Router, DbPool, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .join("panel.db")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = init_pool(&format!("sqlite://{path}")).await.unwrap();
    run_migrations(&pool).await.unwrap();
    for role in ["admin", "user"] {
        let user = SqliteUserRepository::new(pool.clone())
            .create(&CreateUser {
                username: role.into(),
                password_hash: "unused-no-login".into(),
                role: role.into(),
                email: None,
                plan_id: None,
                linux_username: None,
                linux_uid: None,
            })
            .await
            .unwrap();
        SqliteSessionRepository::new(pool.clone())
            .create(&CreateSession {
                id: format!("{role}-session"),
                user_id: user.id,
                token_hash: hash_token(&format!("{role}-token")),
                ip_address: None,
                user_agent: None,
                expires_at: i64::MAX,
            })
            .await
            .unwrap();
    }
    let state = AppState::new(
        Arc::new(AgentClient::new(temp.path().join("no-agent"))),
        pool.clone(),
        "testing".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 300)),
    );
    (create_router(state), pool, temp)
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    role: Option<&str>,
    csrf: bool,
    payload: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(role) = role {
        req = req.header("cookie", format!("{SESSION_COOKIE_NAME}={role}-token"));
        if csrf {
            req = req.header(
                "x-csrf-token",
                generate_csrf_token(&format!("{role}-session")),
            );
        }
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(payload.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

use tower::ServiceExt;

#[tokio::test]
async fn canonical_plan_shell_policies_round_trip_without_agent() {
    let (app, pool, temp) = fixture().await;
    for (policy, next) in [("none", "sftp"), ("sftp", "full"), ("full", "none")] {
        let (status, body) = request(
            &app,
            "POST",
            "/api/v1/plans",
            Some("admin"),
            true,
            json!({"name":format!("Plan {policy}"),"shell_policy":policy}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["plan"]["shell_policy"], policy);
        let id = body["plan"]["id"].as_i64().unwrap();
        let (status, body) = request(
            &app,
            "PUT",
            &format!("/api/v1/plans/{id}"),
            Some("admin"),
            true,
            json!({"shell_policy":next}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["plan"]["shell_policy"], next);
        let (status, body) = request(
            &app,
            "PUT",
            &format!("/api/v1/plans/{id}"),
            Some("admin"),
            true,
            json!({"description":"Leave shell access unchanged"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["plan"]["shell_policy"], next);
        assert_eq!(
            SqlitePlanRepository::new(pool.clone())
                .find_by_id(id)
                .await
                .unwrap()
                .unwrap()
                .shell_policy,
            next
        );
    }
    let (status, body) = request(
        &app,
        "POST",
        "/api/v1/plans",
        Some("admin"),
        true,
        json!({"name":"Default shell access"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["plan"]["shell_policy"], "none");
    assert!(!temp.path().join("no-agent").exists());
}

#[tokio::test]
async fn invalid_plan_shell_policies_are_rejected_without_database_changes() {
    let (app, pool, _temp) = fixture().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/v1/plans",
        Some("admin"),
        true,
        json!({"name":"Keep this plan","shell_policy":"none"}),
    )
    .await;
    let id = body["plan"]["id"].as_i64().unwrap();
    let repo = SqlitePlanRepository::new(pool);
    let count = repo.list_all().await.unwrap().len();
    for policy in [
        "sftp_only",
        "bash_chroot",
        "sftponly",
        "/bin/bash",
        "FULL",
        "",
        "other",
    ] {
        for (method, path) in [
            ("POST", "/api/v1/plans".to_owned()),
            ("PUT", format!("/api/v1/plans/{id}")),
        ] {
            let (status, body) = request(
                &app,
                method,
                &path,
                Some("admin"),
                true,
                json!({"name":"Must not persist","shell_policy":policy}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{policy}: {body}");
            assert_eq!(body["error"], "Akses shell harus none, sftp, atau full.");
        }
    }
    assert_eq!(repo.list_all().await.unwrap().len(), count);
    let saved = repo.find_by_id(id).await.unwrap().unwrap();
    assert_eq!(saved.name, "Keep this plan");
    assert_eq!(saved.shell_policy, "none");
}

#[tokio::test]
async fn plan_shell_policy_changes_require_admin_and_csrf() {
    let (app, _pool, _temp) = fixture().await;
    for (role, csrf, expected) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some("user"), true, StatusCode::FORBIDDEN),
        (Some("admin"), false, StatusCode::FORBIDDEN),
    ] {
        for (method, path) in [("POST", "/api/v1/plans"), ("PUT", "/api/v1/plans/1")] {
            let (status, body) = request(
                &app,
                method,
                path,
                role,
                csrf,
                json!({"name":"Unauthorized change","shell_policy":"full"}),
            )
            .await;
            assert_eq!(status, expected, "{body}");
        }
    }
}
