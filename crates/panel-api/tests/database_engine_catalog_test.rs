#![allow(clippy::unwrap_used, clippy::expect_used)]
//! SQLite, router and typed RPC fixtures only; never starts AgentServer or a database.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME, generate_csrf_token},
    config::AuthConfig,
    routes::AppState,
};
use panel_db::{
    models::{CreateSession, CreateUser},
    repositories::{SqliteSessionRepository, SqliteUserRepository},
};
use panel_proto::{RequestPayload, ResponsePayload};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    app: axum::Router,
    state: AppState,
    cookie: String,
    csrf: String,
    tmp: tempfile::TempDir,
}
async fn fixture(role: &str) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let pool = panel_db::pool::init_pool("sqlite::memory:").await.unwrap();
    panel_db::pool::run_migrations(&pool).await.unwrap();
    let user = SqliteUserRepository::new(pool.clone())
        .create(&CreateUser {
            username: "alice".into(),
            role: role.into(),
            password_hash: "unused".into(),
            email: None,
            plan_id: None,
            linux_username: Some("p_alice".into()),
            linux_uid: Some(10001),
        })
        .await
        .unwrap();
    SqliteSessionRepository::new(pool.clone())
        .create(&CreateSession {
            id: "session".into(),
            user_id: user.id,
            token_hash: panel_core::hash_token("token"),
            ip_address: None,
            user_agent: None,
            expires_at: chrono::Utc::now().timestamp() + 3600,
        })
        .await
        .unwrap();
    let state = AppState::new(
        Arc::new(panel_api::agent_client::AgentClient::new(
            tmp.path().join("rpc"),
        )),
        pool,
        "development".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 60)),
    );
    Fixture {
        app: panel_api::create_router(state.clone()),
        state,
        cookie: format!("{SESSION_COOKIE_NAME}=token"),
        csrf: generate_csrf_token("session"),
        tmp,
    }
}
async fn call(f: &Fixture, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("cookie", &f.cookie)
                .header(CSRF_HEADER_NAME, &f.csrf)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    if path == "/api/v1/databases/engines" {
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let body =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body)
}
fn statuses(maria: (bool, bool), postgres: (bool, bool)) -> Value {
    json!({"type":"DatabaseServerStatus","data":{"servers":[
        {"engine":"mariadb","is_installed":maria.0,"is_running":maria.1,"version":"fixture","port":3306,"socket_path":"/private/mysql.sock"},
        {"engine":"postgresql","is_installed":postgres.0,"is_running":postgres.1,"version":"fixture","port":5432,"socket_path":"/private/pg.sock"}
    ]}})
}
async fn rpc(f: &Fixture, status: Value, create: bool) -> tokio::task::JoinHandle<()> {
    let socket = f.tmp.path().join("rpc");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
    tokio::spawn(async move {
        let (connection, _) = listener.accept().await.unwrap();
        let mut stream = panel_proto::RpcStream::new(connection);
        let request = stream.recv_request().await.unwrap();
        let RequestPayload::DatabaseServerStatus(payload) = request.payload else {
            panic!("Expected live status before any mutation")
        };
        assert_eq!(payload.engines, ["mariadb", "postgresql"]);
        stream
            .send_response(&panel_proto::RpcResponse::ok(
                request.request_id,
                serde_json::from_value(status).unwrap(),
            ))
            .await
            .unwrap();
        if create {
            let (connection, _) = listener.accept().await.unwrap();
            let mut stream = panel_proto::RpcStream::new(connection);
            let request = stream.recv_request().await.unwrap();
            let RequestPayload::DatabaseCreate(payload) = request.payload else {
                panic!("Expected typed create")
            };
            assert_eq!(payload.engine, "postgresql");
            assert_eq!(payload.name, "p_alice_shop");
            stream
                .send_response(&panel_proto::RpcResponse::ok(
                    request.request_id,
                    ResponsePayload::DatabaseCreated(panel_proto::DatabaseCreateResponse {
                        engine: payload.engine,
                        name: payload.name,
                        db_user: payload.db_user,
                        success: true,
                        message: "fixture".into(),
                    }),
                ))
                .await
                .unwrap();
        }
    })
}

#[tokio::test]
async fn live_catalog_filters_uninstalled_and_stopped_engines_for_admin_and_tenant() {
    for role in ["admin", "user"] {
        for (maria, postgres, expected) in [
            ((false, false), (false, false), vec![]),
            ((true, false), (true, true), vec!["postgresql"]),
            ((true, true), (false, true), vec!["mariadb"]),
            ((true, true), (true, true), vec!["mariadb", "postgresql"]),
        ] {
            let f = fixture(role).await;
            let worker = rpc(&f, statuses(maria, postgres), false).await;
            let (status, body) = call(&f, "GET", "/api/v1/databases/engines", Value::Null).await;
            worker.await.unwrap();
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(
                body.as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["engine"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(!body.to_string().contains("socket"));
            assert!(!body.to_string().contains("version"));
        }
    }
}

#[tokio::test]
async fn cached_inventory_never_replaces_live_verification_and_auth_is_required() {
    let f = fixture("user").await;
    panel_db::SqliteDatabaseRepository::new(f.state.pool.clone())
        .upsert_server(&panel_db::models::DatabaseServer {
            engine: "mariadb".into(),
            is_installed: true,
            is_running: true,
            version: None,
            port: 3306,
            socket_path: None,
            updated_at: chrono::Utc::now().timestamp(),
        })
        .await
        .unwrap();
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/databases/engines")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        call(&f, "GET", "/api/v1/databases/engines", Value::Null)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (status, _) = call(
        &f,
        "POST",
        "/api/v1/databases",
        json!({"engine":"mariadb","name":"shop"}),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn incomplete_status_is_an_error_instead_of_an_empty_success() {
    let f = fixture("user").await;
    let worker = rpc(
        &f,
        json!({"type":"DatabaseServerStatus","data":{"servers":[]}}),
        false,
    )
    .await;
    assert_eq!(
        call(&f, "GET", "/api/v1/databases/engines", Value::Null)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    worker.await.unwrap();
}

#[tokio::test]
async fn create_rechecks_engine_before_typed_mutation_or_database_record() {
    for ready in [false, true] {
        let f = fixture("user").await;
        let worker = rpc(&f, statuses((false, false), (ready, ready)), ready).await;
        let (status, body) = call(
            &f,
            "POST",
            "/api/v1/databases",
            json!({"engine":"postgresql","name":"shop"}),
        )
        .await;
        worker.await.unwrap();
        assert_eq!(
            status,
            if ready {
                StatusCode::CREATED
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            "{body}"
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM databases")
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
        assert_eq!(count, i64::from(ready));
    }
}
