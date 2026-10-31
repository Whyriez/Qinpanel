#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::create_router;
use panel_api::routes::{AppState, SystemInfoResponse};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
async fn test_ipc_ping_and_version() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");

    let config = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::new(config));

    let server_task = tokio::spawn(async move {
        server.run(shutdown_rx).await.expect("agent server run");
    });

    // Give server a moment to bind
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = AgentClient::new(sock_path);

    // Test Ping
    client.ping().await.expect("ping agent");

    // Test Version
    let ver_info = client.version().await.expect("version agent");
    assert_eq!(ver_info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(ver_info.proto_version, panel_proto::PROTOCOL_VERSION);

    // Shutdown agent
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

#[tokio::test]
async fn test_api_healthz_and_system_info_endpoints() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");

    let config = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::new(config));

    let server_task = tokio::spawn(async move {
        server.run(shutdown_rx).await.expect("agent server run");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let db_file = tmp.path().join("panel.db");
    let pool = panel_db::pool::init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    panel_db::pool::run_migrations(&pool)
        .await
        .expect("run migrations");

    let admin = panel_db::repositories::SqliteUserRepository::new(pool.clone())
        .create(&panel_db::models::CreateUser {
            username: "ipcadmin".into(),
            email: None,
            password_hash: "unused-fixture".into(),
            role: "admin".into(),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .unwrap();
    let token = "ipc-admin-session-fixture";
    panel_db::repositories::SqliteSessionRepository::new(pool.clone())
        .create(&panel_db::models::CreateSession {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: admin.id,
            token_hash: panel_core::auth::hash_token(token),
            ip_address: None,
            user_agent: None,
            expires_at: chrono::Utc::now().timestamp() + 3600,
        })
        .await
        .unwrap();
    let lockout_manager = Arc::new(panel_api::auth::LockoutManager::new(5, 900));

    let agent_client = Arc::new(AgentClient::new(sock_path));
    let state = AppState::new(
        agent_client,
        pool,
        "testing".to_string(),
        panel_api::config::AuthConfig::default(),
        lockout_manager,
    );

    let app = create_router(state);

    // 1. Test /healthz
    let req = Request::builder()
        .uri("/healthz")
        .body(Body::empty())
        .expect("build request");
    let res = app.clone().oneshot(req).await.expect("execute request");
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains("\"status\":\"ok\""));

    // System inventory is private; healthz remains public.
    let denied = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/system/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    // 2. Test /api/v1/system/info
    let req = Request::builder()
        .uri("/api/v1/system/info")
        .header(
            "cookie",
            format!("{}={token}", panel_api::auth::SESSION_COOKIE_NAME),
        )
        .body(Body::empty())
        .expect("build request");
    let res = app.clone().oneshot(req).await.expect("execute request");
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let info: SystemInfoResponse =
        serde_json::from_slice(&body_bytes).expect("decode system info response");
    assert!(info.agent_connected);
    assert_eq!(info.api_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        info.agent_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(info.proto_version, panel_proto::PROTOCOL_VERSION);

    // 3. Test embedded web assets and SPA fallback
    let req = Request::builder()
        .uri("/")
        .body(Body::empty())
        .expect("build request");
    let res = app.clone().oneshot(req).await.expect("execute request");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains("Qinpanel"));

    // 4. Test client-side route fallback to index.html
    let req = Request::builder()
        .uri("/sites")
        .body(Body::empty())
        .expect("build request");
    let res = app.oneshot(req).await.expect("execute request");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains("Qinpanel"));

    // Clean up
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
