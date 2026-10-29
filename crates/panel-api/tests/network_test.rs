#![allow(clippy::unwrap_used, clippy::expect_used)]
//! SQLite/router fixtures only; no network calls or panel daemons.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    agent_client::AgentClient,
    auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME},
    config::AuthConfig,
    routes::AppState,
};
use panel_core::{hash_password, hash_token};
use panel_db::{
    models::{CreateDomain, CreateSession, CreateSite, CreateUser},
    pool::{init_pool, run_migrations},
    repositories::{
        SqliteDomainRepository, SqliteSessionRepository, SqliteSiteRepository, SqliteUserRepository,
    },
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    _temp: tempfile::TempDir,
    site: i64,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let pool = init_pool(&format!(
            "sqlite://{}",
            temp.path()
                .join("network.db")
                .to_string_lossy()
                .replace('\\', "/")
        ))
        .await
        .unwrap();
        run_migrations(&pool).await.unwrap();
        let repo = SqliteUserRepository::new(pool.clone());
        let hash = hash_password("FixturePassword123!").unwrap();
        let mut admin_id = 0;
        for (name, role) in [("administrator", "admin"), ("customer", "user")] {
            let user = repo
                .create(&CreateUser {
                    username: name.into(),
                    password_hash: hash.clone(),
                    role: role.into(),
                    email: None,
                    linux_username: None,
                    linux_uid: None,
                    plan_id: None,
                })
                .await
                .unwrap();
            if role == "admin" {
                admin_id = user.id;
            }
            SqliteSessionRepository::new(pool.clone())
                .create(&CreateSession {
                    id: format!("session-{name}"),
                    user_id: user.id,
                    token_hash: hash_token(&format!("token-{name}")),
                    ip_address: None,
                    user_agent: None,
                    expires_at: chrono::Utc::now().timestamp() + 3600,
                })
                .await
                .unwrap();
        }
        let site = SqliteSiteRepository::new(pool.clone())
            .create(&CreateSite {
                user_id: admin_id,
                kind: "static".into(),
                docroot: "/fixture/sites/app".into(),
                runtime: None,
                version: None,
                status: Some("active".into()),
                settings_json: Some(json!({"has_ssl":true,"force_https":true}).to_string()),
            })
            .await
            .unwrap();
        SqliteDomainRepository::new(pool.clone())
            .create(&CreateDomain {
                site_id: site.id,
                name: "app.example.com".into(),
                unicode_name: "app.example.com".into(),
                is_primary: true,
                redirect_mode: None,
                redirect_target: None,
                status: None,
            })
            .await
            .unwrap();
        let state = AppState::new(
            Arc::new(AgentClient::new(temp.path().join("no-agent"))),
            pool,
            "development".into(),
            AuthConfig {
                cookie_secure: Some(false),
                ..Default::default()
            },
            Arc::new(LockoutManager::new(5, 900)),
        );
        Self {
            state,
            _temp: temp,
            site: site.id,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        user: Option<&str>,
        csrf: bool,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(user) = user {
            req = req.header("cookie", format!("{SESSION_COOKIE_NAME}=token-{user}"));
            if csrf {
                req = req.header(
                    CSRF_HEADER_NAME,
                    panel_api::auth::csrf::generate_csrf_token(&format!("session-{user}")),
                );
            }
        }
        let response = panel_api::create_router(self.state.clone())
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}
#[tokio::test]
async fn network_routes_require_admin_and_mutations_require_csrf() {
    let f = Fixture::new().await;
    for path in ["/api/v1/network", "/api/v1/network/cloudflare"] {
        assert_eq!(
            f.call("GET", path, None, false, Value::Null).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            f.call("GET", path, Some("customer"), false, Value::Null)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    for (method, path, payload) in [
        ("PUT", "/api/v1/network", json!({"ipv4":"8.8.8.8"})),
        ("POST", "/api/v1/network/dns-check", json!({"domain_id":1})),
        (
            "POST",
            "/api/v1/network/cloudflare",
            json!({"action":"zones","page":1}),
        ),
        ("POST", "/api/v1/network/cloudflare/install", json!({})),
    ] {
        assert_eq!(
            f.call(method, path, Some("administrator"), false, payload.clone())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            f.call(method, path, Some("customer"), true, payload)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
}
#[tokio::test]
async fn public_addresses_persist_normalize_and_clear_without_touching_cloudflare() {
    let f = Fixture::new().await;
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/network",
            Some("administrator"),
            true,
            json!({"ipv4":" 8.8.8.8 ","ipv6":"2606:4700:4700:0:0:0:0:1111"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, body) = f
        .call(
            "GET",
            "/api/v1/network",
            Some("administrator"),
            false,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ipv4"], "8.8.8.8");
    assert_eq!(body["ipv6"], "2606:4700:4700::1111");
    assert_eq!(body["projects"][0]["hostname"], "app.example.com");
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/network",
            Some("administrator"),
            true,
            json!({"ipv4":"","ipv6":null})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            "GET",
            "/api/v1/network",
            Some("administrator"),
            false,
            Value::Null
        )
        .await
        .1["ipv4"],
        Value::Null
    );
}
#[tokio::test]
async fn invalid_private_and_wrong_family_addresses_are_rejected_atomically() {
    let f = Fixture::new().await;
    for value in [
        "127.0.0.1",
        "10.0.0.1",
        "100.64.0.1",
        "169.254.1.1",
        "0.0.0.0",
        "224.0.0.1",
        "::1",
        "not-an-ip",
    ] {
        assert_eq!(
            f.call(
                "PUT",
                "/api/v1/network",
                Some("administrator"),
                true,
                json!({"ipv4":value,"ipv6":null})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "{value}"
        );
    }
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/network",
            Some("administrator"),
            true,
            json!({"ipv4":"8.8.8.8","ipv6":"fc00::1"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.call(
            "GET",
            "/api/v1/network",
            Some("administrator"),
            false,
            Value::Null
        )
        .await
        .1["ipv4"],
        Value::Null
    );
}
#[tokio::test]
async fn tunnel_publication_uses_project_tls_and_rejects_unknown_or_inactive_projects() {
    let f = Fixture::new().await;
    let publish = json!({"action":"publish","zone_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","tunnel_id":"01234567-89ab-4cde-8fab-0123456789ab","hostname":"app.example.com","tls":false});
    let (code, job) = f
        .call(
            "POST",
            "/api/v1/network/cloudflare",
            Some("administrator"),
            true,
            publish.clone(),
        )
        .await;
    assert_eq!(code, StatusCode::ACCEPTED);
    let payload: Value = serde_json::from_str(job["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(payload["tls"], true);
    assert_eq!(job["max_retries"], 0);
    let repeated = f
        .call(
            "POST",
            "/api/v1/network/cloudflare",
            Some("administrator"),
            true,
            publish.clone(),
        )
        .await
        .1;
    assert_eq!(job["id"], repeated["id"]);
    sqlx::query("UPDATE sites SET status='disabled' WHERE id=?")
        .bind(f.site)
        .execute(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/network/cloudflare",
            Some("administrator"),
            true,
            publish.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut external = publish;
    external["hostname"] = json!("metadata.internal");
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/network/cloudflare",
            Some("administrator"),
            true,
            external
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn connector_install_is_deduplicated_and_running_cloudflare_jobs_cannot_be_cancelled() {
    let f = Fixture::new().await;
    let request = || {
        f.call(
            "POST",
            "/api/v1/network/cloudflare/install",
            Some("administrator"),
            true,
            json!({}),
        )
    };
    let (first, second) = tokio::join!(request(), request());
    assert_eq!(first.0, StatusCode::ACCEPTED);
    assert_eq!(first.1["id"], second.1["id"]);
    let id = first.1["id"].as_str().unwrap();
    sqlx::query("UPDATE jobs SET status='running' WHERE id=?")
        .bind(id)
        .execute(&f.state.operations_pool)
        .await
        .unwrap();
    assert!(!f.state.job_runner.job_repo.cancel(id).await.unwrap());
}
#[tokio::test]
async fn failed_agent_status_never_reports_configured_and_arbitrary_commands_are_rejected() {
    let f = Fixture::new().await;
    let (status, body) = f
        .call(
            "GET",
            "/api/v1/network/cloudflare",
            Some("administrator"),
            false,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"].as_str().unwrap().contains("panel-agent"));
    assert!(body.get("configured").is_none());
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&f.state.operations_pool)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/network/cloudflare",
            Some("administrator"),
            true,
            json!({"action":"exec","command":"whoami"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/network/dns-check",
            Some("administrator"),
            true,
            json!({"domain_id":99999})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
