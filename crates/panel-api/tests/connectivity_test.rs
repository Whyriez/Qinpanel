#![allow(clippy::unwrap_used, clippy::expect_used)]
//! In-process router + temporary SQLite only. No agent daemon, RPC server or probe.
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
    CreateSession, CreateSite, CreateUser, DbPool, SqliteSessionRepository, SqliteSiteRepository,
    SqliteUserRepository,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    app: axum::Router,
    pool: DbPool,
    cookies: Vec<String>,
    csrf: Vec<String>,
    users: Vec<i64>,
    sites: Vec<i64>,
    _tmp: tempfile::TempDir,
}
async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let pool = panel_db::init_pool("sqlite::memory:").await.unwrap();
    panel_db::run_migrations(&pool).await.unwrap();
    let mut cookies = vec![];
    let mut csrf = vec![];
    let mut users = vec![];
    let mut sites = vec![];
    for (i, role) in ["admin", "user", "user"].into_iter().enumerate() {
        let user = SqliteUserRepository::new(pool.clone())
            .create(&CreateUser {
                username: format!("actor{i}"),
                password_hash: "fixture".into(),
                role: role.into(),
                email: None,
                linux_username: None,
                linux_uid: None,
                plan_id: None,
            })
            .await
            .unwrap();
        users.push(user.id);
        let sid = format!("session-{i}");
        let token = format!("cookie-{i}");
        SqliteSessionRepository::new(pool.clone())
            .create(&CreateSession {
                id: sid.clone(),
                user_id: user.id,
                token_hash: panel_core::hash_token(&token),
                ip_address: None,
                user_agent: None,
                expires_at: chrono::Utc::now().timestamp() + 3600,
            })
            .await
            .unwrap();
        cookies.push(format!("{SESSION_COOKIE_NAME}={token}"));
        csrf.push(generate_csrf_token(&sid));
        if i > 0 {
            sites.push(
                SqliteSiteRepository::new(pool.clone())
                    .create(&CreateSite {
                        user_id: user.id,
                        kind: "static".into(),
                        docroot: "/fixture/unused".into(),
                        runtime: None,
                        version: None,
                        status: None,
                        settings_json: None,
                    })
                    .await
                    .unwrap()
                    .id,
            );
        }
    }
    let state = AppState::new(
        Arc::new(panel_api::agent_client::AgentClient::new(
            tmp.path().join("missing-rpc"),
        )),
        pool.clone(),
        "development".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 60)),
    );
    Fixture {
        app: panel_api::create_router(state),
        pool,
        cookies,
        csrf,
        users,
        sites,
        _tmp: tmp,
    }
}
async fn call(
    f: &Fixture,
    actor: Option<usize>,
    csrf: bool,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(actor) = actor {
        request = request.header("cookie", &f.cookies[actor]);
        if csrf {
            request = request.header(CSRF_HEADER_NAME, &f.csrf[actor]);
        }
    }
    let response = f
        .app
        .clone()
        .oneshot(
            request
                .body(if body.is_null() {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)})),
    )
}
fn config(site: i64) -> Value {
    json!({"service":"website","scope":{"kind":"project","site_id":site},"transport":"direct","host":"APP.Example.test","port":8443,"tls_mode":"tls","certificate_id":null,"primary":true,"enabled":true})
}
async fn create(f: &Fixture, cfg: Value) -> Value {
    let (status, value) = call(f, Some(0), true, "POST", "/api/v1/connectivity", cfg).await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value
}

async fn keyed_create(f: &Fixture, key: &str, cfg: Value) -> (StatusCode, Value) {
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/connectivity")
                .header("content-type", "application/json")
                .header("cookie", &f.cookies[0])
                .header(CSRF_HEADER_NAME, &f.csrf[0])
                .header("Idempotency-Key", key)
                .body(Body::from(cfg.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn create_key_is_persisted_and_retries_cannot_duplicate_or_resurrect_deleted_addresses() {
    let f = fixture().await;
    let key = uuid::Uuid::new_v4().to_string();
    let mut cfg = config(f.sites[0]);
    cfg["primary"] = json!(false);
    let (one, two) = tokio::join!(
        keyed_create(&f, &key, cfg.clone()),
        keyed_create(&f, &key, cfg.clone())
    );
    assert!([one.0, two.0].contains(&StatusCode::CREATED));
    assert!([one.0, two.0].contains(&StatusCode::OK));
    assert_eq!(one.1["id"], two.1["id"]);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM service_endpoints")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let mut changed = cfg.clone();
    changed["port"] = json!(9443);
    assert_eq!(
        keyed_create(&f, &key, changed).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        keyed_create(&f, "invalid-key", cfg.clone()).await.0,
        StatusCode::BAD_REQUEST
    );
    let path = format!("/api/v1/connectivity/{}", one.1["id"].as_str().unwrap());
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "DELETE",
            &path,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(keyed_create(&f, &key, cfg).await.0, StatusCode::CONFLICT);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM service_endpoints")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn routes_enforce_session_admin_csrf_and_publish_openapi_contract() {
    let f = fixture().await;
    assert_eq!(
        call(&f, None, false, "GET", "/api/v1/connectivity", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &f,
            Some(1),
            true,
            "POST",
            "/api/v1/connectivity",
            config(f.sites[0])
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &f,
            Some(0),
            false,
            "POST",
            "/api/v1/connectivity",
            config(f.sites[0])
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let created = create(&f, config(f.sites[0])).await;
    let path = format!("/api/v1/connectivity/{}", created["id"].as_str().unwrap());
    for method in ["PUT", "DELETE"] {
        let body = if method == "PUT" {
            json!({"expected_revision":1,"config":config(f.sites[0])})
        } else {
            json!({"expected_revision":1})
        };
        assert_eq!(
            call(&f, Some(1), true, method, &path, body.clone()).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&f, Some(0), false, method, &path, body).await.0,
            StatusCode::FORBIDDEN
        );
    }
    use utoipa::OpenApi;
    let doc = panel_api::routes::ApiDoc::openapi().to_json().unwrap();
    let doc: Value = serde_json::from_str(&doc).unwrap();
    assert!(doc["paths"]["/api/v1/connectivity"]["get"]["responses"]["200"]["content"]["application/json"]["schema"].is_object());
    assert!(doc["paths"]["/api/v1/connectivity/{id}"]["put"].is_object());
    assert!(doc["components"]["schemas"]["EndpointConfig"].is_object());
    assert!(
        !doc["components"]["schemas"]["EndpointConfig"]
            .to_string()
            .contains("secret_ref")
    );
}

#[tokio::test]
async fn overview_is_scoped_and_never_exposes_bindings_or_cross_tenant_project_choices() {
    let f = fixture().await;
    let mine = create(&f, config(f.sites[0])).await;
    create(&f, config(f.sites[1])).await;
    create(&f,json!({"service":"panel","scope":{"kind":"host"},"transport":"direct","host":"2001:db8::1","port":17890,"tls_mode":"none","certificate_id":null,"primary":true,"enabled":true})).await;
    sqlx::query("UPDATE service_endpoints SET secret_ref='vault-private',route_id='route-private',connection_id='connection-private'").execute(&f.pool).await.unwrap();
    let (_, all) = call(
        &f,
        Some(0),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert_eq!(all["can_manage"], true);
    assert_eq!(all["endpoints"].as_array().unwrap().len(), 3);
    let (_, tenant) = call(
        &f,
        Some(1),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert_eq!(tenant["can_manage"], false);
    assert_eq!(tenant["endpoints"].as_array().unwrap().len(), 1);
    assert_eq!(tenant["endpoints"][0]["endpoint"]["id"], mine["id"]);
    assert_eq!(tenant["projects"].as_array().unwrap().len(), 1);
    assert_eq!(tenant["projects"][0]["id"], f.sites[0]);
    assert_eq!(tenant["certificates"], json!([]));
    assert_eq!(tenant["endpoints"][0]["evidence"], json!([]));
    for body in [all, tenant] {
        for secret in [
            "secret_ref",
            "vault-private",
            "route-private",
            "connection-private",
            "privkey_pem",
        ] {
            assert!(!body.to_string().contains(secret));
        }
    }
    sqlx::query("UPDATE users SET status='suspended' WHERE id=?")
        .bind(f.users[1])
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &f,
            Some(1),
            false,
            "GET",
            "/api/v1/connectivity",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn create_replace_delete_persist_and_conflicts_do_not_overwrite_newer_edits() {
    let f = fixture().await;
    let created = create(&f, config(f.sites[0])).await;
    assert_eq!(created["host"], "app.example.test");
    assert_eq!(created["client_requirement"], "none");
    let path = format!("/api/v1/connectivity/{}", created["id"].as_str().unwrap());
    let mut cfg = config(f.sites[0]);
    cfg["host"] = json!("2001:0db8::8");
    cfg["port"] = json!(9443);
    let (status, replaced) = call(
        &f,
        Some(0),
        true,
        "PUT",
        &path,
        json!({"expected_revision":1,"config":cfg}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replaced["revision"], 2);
    assert_eq!(replaced["host"], "2001:db8::8");
    let (status, problem) = call(
        &f,
        Some(0),
        true,
        "PUT",
        &path,
        json!({"expected_revision":1,"config":config(f.sites[0])}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(problem["status"], 409);
    assert!(problem["detail"].is_string());
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "DELETE",
            &path,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, loaded) = call(
        &f,
        Some(0),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert_eq!(loaded["endpoints"][0]["endpoint"]["port"], 9443);
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "DELETE",
            &path,
            json!({"expected_revision":2})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (_, empty) = call(
        &f,
        Some(0),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert_eq!(empty["endpoints"], json!([]));
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM audit_logs WHERE action LIKE 'connectivity.%'",
            )
            .fetch_one(&f.pool)
            .await
            .unwrap();
            if count == 3 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn invalid_fields_host_port_scope_and_duplicate_primary_have_explicit_failures() {
    let f = fixture().await;
    let mut invalid = config(f.sites[0]);
    invalid["secret_ref"] = json!("never-return-this-token");
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/connectivity")
                .header("content-type", "application/json")
                .header("cookie", &f.cookies[0])
                .header(CSRF_HEADER_NAME, &f.csrf[0])
                .body(Body::from(invalid.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.headers()["content-type"],
        "application/problem+json"
    );
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["type"], "about:blank");
    assert_eq!(body["status"], 422);
    assert!(!body.to_string().contains("never-return-this-token"));
    for (key, value) in [
        ("host", json!("https://example.test/path")),
        ("port", json!(0)),
        ("secret_ref", json!("forbidden")),
        ("client_requirement", json!("none")),
        ("scope", json!({"kind":"host"})),
    ] {
        let mut cfg = config(f.sites[0]);
        cfg[key] = value;
        assert_eq!(
            call(&f, Some(0), true, "POST", "/api/v1/connectivity", cfg)
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{key}"
        );
    }
    create(&f, config(f.sites[0])).await;
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "POST",
            "/api/v1/connectivity",
            config(f.sites[0])
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut alternate = config(f.sites[0]);
    alternate["primary"] = json!(false);
    create(&f, alternate).await;
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "DELETE",
            "/api/v1/connectivity/not-an-id",
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "POST",
            "/api/v1/connectivity",
            config(99999)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn evidence_read_preserves_unknown_expired_revision_and_family_dimensions() {
    use panel_core::connectivity::*;
    let f = fixture().await;
    let created = create(&f, config(f.sites[0])).await;
    let id = created["id"].as_str().unwrap();
    let repo = panel_db::SqliteConnectivityRepository::new(f.pool.clone());
    let now = chrono::Utc::now().timestamp();
    let mut report = EndpointEvidence {
        endpoint_revision: 1,
        kind: EvidenceKind::ExternalReachability,
        observation_point: ObservationPoint::UserClient,
        observer_id: "client-a".into(),
        address_family: Some(AddressFamily::Ipv4),
        observed_at: now - 10,
        expires_at: now + 120,
        outcome: EvidenceOutcome::Pass,
        reason: EvidenceReason::Verified,
    };
    repo.record_evidence(f.users[0], id, &report).await.unwrap();
    report.address_family = Some(AddressFamily::Ipv6);
    report.outcome = EvidenceOutcome::Unknown;
    report.reason = EvidenceReason::Timeout;
    repo.record_evidence(f.users[0], id, &report).await.unwrap();
    report.kind = EvidenceKind::Service;
    report.observation_point = ObservationPoint::LocalServer;
    report.address_family = None;
    report.observed_at = now - 100;
    report.expires_at = now - 1;
    repo.record_evidence(f.users[0], id, &report).await.unwrap();
    let (_, body) = call(
        &f,
        Some(1),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    let reports = body["endpoints"][0]["evidence"].as_array().unwrap();
    assert_eq!(reports.len(), 3);
    assert!(
        reports
            .iter()
            .any(|r| r["address_family"] == "ipv6" && r["outcome"] == "unknown")
    );
    assert!(
        reports
            .iter()
            .any(|r| r["expired"] == true && r["current"] == false)
    );
    let (_, other) = call(
        &f,
        Some(2),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert_eq!(other["endpoints"], json!([]));
    call(
        &f,
        Some(0),
        true,
        "PUT",
        &format!("/api/v1/connectivity/{id}"),
        json!({"expected_revision":1,"config":config(f.sites[0])}),
    )
    .await;
    let (_, changed) = call(
        &f,
        Some(1),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert!(
        changed["endpoints"][0]["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["current"] == false && r["revision_matches"] == false)
    );
}

#[tokio::test]
async fn metadata_does_not_infer_endpoints_from_ips_or_create_system_jobs() {
    let f = fixture().await;
    panel_db::SqliteSettingRepository::new(f.pool.clone())
        .set("network.public_ipv4", "203.0.113.9")
        .await
        .unwrap();
    let (_, before) = call(
        &f,
        Some(0),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert_eq!(before["endpoints"], json!([]));
    let mut cfg = config(f.sites[0]);
    cfg["transport"] = json!("cloudflare_http");
    create(&f, cfg).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        panel_db::SqliteSettingRepository::new(f.pool.clone())
            .get("network.public_ipv4")
            .await
            .unwrap()
            .as_deref(),
        Some("203.0.113.9")
    );
}

#[tokio::test]
async fn list_pages_and_counts_are_authorized_and_single_resource_reload_is_scoped() {
    let f = fixture().await;
    let first = create(&f, config(f.sites[0])).await;
    for port in [9443, 10443] {
        let mut cfg = config(f.sites[0]);
        cfg["primary"] = json!(false);
        cfg["port"] = json!(port);
        create(&f, cfg).await;
    }
    let other = create(&f, config(f.sites[1])).await;
    let (status, body) = call(
        &f,
        Some(1),
        false,
        "GET",
        "/api/v1/connectivity?page=2&per_page=1&service=website",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["page"], 2);
    assert_eq!(body["total"], 3);
    assert_eq!(body["total_pages"], 3);
    assert_eq!(body["service_counts"]["website"], 3);
    assert_eq!(body["endpoints"].as_array().unwrap().len(), 1);
    assert_ne!(body["endpoints"][0]["endpoint"]["id"], other["id"]);
    for query in ["?page=0", "?per_page=0", "?per_page=101"] {
        assert_eq!(
            call(
                &f,
                Some(0),
                false,
                "GET",
                &format!("/api/v1/connectivity{query}"),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let path = format!("/api/v1/connectivity/{}", first["id"].as_str().unwrap());
    assert_eq!(
        call(&f, Some(1), false, "GET", &path, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&f, Some(2), false, "GET", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn project_filter_scopes_counts_pages_and_options_without_relaxing_ownership() {
    let f = fixture().await;
    let second_project = SqliteSiteRepository::new(f.pool.clone())
        .create(&CreateSite {
            user_id: f.users[1],
            kind: "static".into(),
            docroot: "/fixture/unused".into(),
            runtime: None,
            version: None,
            status: None,
            settings_json: None,
        })
        .await
        .unwrap();
    create(&f, config(second_project.id)).await;
    create(&f, config(f.sites[1])).await;
    create(&f, config(f.sites[0])).await;
    let mut alternate = config(f.sites[0]);
    alternate["primary"] = json!(false);
    alternate["port"] = json!(9443);
    create(&f, alternate).await;
    for actor in [0, 1] {
        let mut ids = vec![];
        for page in [1, 2] {
            let (status, body) = call(
                &f,
                Some(actor),
                false,
                "GET",
                &format!(
                    "/api/v1/connectivity?site_id={}&page={page}&per_page=1&service=website",
                    f.sites[0]
                ),
                Value::Null,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["can_manage"], actor == 0);
            assert_eq!(body["total"], 2);
            assert_eq!(body["total_pages"], 2);
            assert_eq!(body["service_counts"]["website"], 2);
            assert_eq!(body["projects"].as_array().unwrap().len(), 1);
            assert_eq!(body["projects"][0]["id"], f.sites[0]);
            assert_eq!(body["endpoints"].as_array().unwrap().len(), 1);
            assert_eq!(
                body["endpoints"][0]["endpoint"]["scope"]["site_id"],
                f.sites[0]
            );
            ids.push(body["endpoints"][0]["endpoint"]["id"].clone());
        }
        assert_ne!(ids[0], ids[1]);
    }
    for site_id in [f.sites[1], 999999, 0, -1] {
        let (status, _) = call(
            &f,
            Some(1),
            false,
            "GET",
            &format!("/api/v1/connectivity?site_id={site_id}"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    sqlx::query("UPDATE sites SET status='suspended' WHERE id=?")
        .bind(f.sites[0])
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &f,
            Some(1),
            false,
            "GET",
            &format!("/api/v1/connectivity?site_id={}", f.sites[0]),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn diagnosis_requires_admin_csrf_registered_current_enabled_endpoint_and_no_target_input() {
    let f = fixture().await;
    let mut cfg = config(f.sites[0]);
    cfg["host"] = json!("127.0.0.1");
    let endpoint = create(&f, cfg).await;
    let path = format!(
        "/api/v1/connectivity/{}/diagnose",
        endpoint["id"].as_str().unwrap()
    );
    for (actor, csrf, expected) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some(1), true, StatusCode::FORBIDDEN),
        (Some(2), true, StatusCode::FORBIDDEN),
        (Some(0), false, StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            call(
                &f,
                actor,
                csrf,
                "POST",
                &path,
                json!({"expected_revision":1})
            )
            .await
            .0,
            expected
        );
    }
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "POST",
            &path,
            json!({"expected_revision":99})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, body) = call(
        &f,
        Some(0),
        true,
        "POST",
        &path,
        json!({"expected_revision":1,"host":"metadata.invalid","port":80,"token":"do-not-reflect"}),
    )
    .await;
    assert!(status.is_client_error());
    assert!(!body.to_string().contains("do-not-reflect"));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM connectivity_diagnosis_leases")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let nonexistent = format!("/api/v1/connectivity/{}/diagnose", uuid::Uuid::new_v4());
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "POST",
            &nonexistent,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE service_endpoints SET enabled=0,is_primary=0 WHERE id=?")
        .bind(endpoint["id"].as_str().unwrap())
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "POST",
            &path,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn local_diagnosis_is_persisted_without_internet_claims_or_system_mutations_and_is_cooled_down()
 {
    let f = fixture().await;
    let mut cfg = config(f.sites[0]);
    cfg["host"] = json!("127.0.0.1");
    let endpoint = create(&f, cfg).await;
    let id = endpoint["id"].as_str().unwrap();
    let path = format!("/api/v1/connectivity/{id}/diagnose");
    let (status, body) = call(
        &f,
        Some(0),
        true,
        "POST",
        &path,
        json!({"expected_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let evidence = body["evidence"].as_array().unwrap();
    assert!(!evidence.is_empty());
    assert!(
        evidence
            .iter()
            .filter(|r| r["kind"] == "dns")
            .all(|r| r["outcome"] == "unknown" && r["reason"] == "not_tested")
    );
    for row in evidence {
        assert_eq!(row["observation_point"], "local_server");
        assert_ne!(row["kind"], "external_reachability");
        assert_eq!(row["endpoint_revision"], 1);
        assert_eq!(
            row["expires_at"].as_i64().unwrap() - row["observed_at"].as_i64().unwrap(),
            300
        );
    }
    assert!(evidence.iter().any(|r| r["kind"] == "route"
        && r["reason"] == "policy_blocked"
        && r["outcome"] == "unknown"));
    assert!(
        evidence
            .iter()
            .any(|r| r["kind"] == "service" && r["reason"] == "agent_unavailable")
    );
    assert_eq!(
        call(
            &f,
            Some(0),
            true,
            "POST",
            &path,
            json!({"expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
    let (_, own) = call(
        &f,
        Some(1),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert!(
        !own["endpoints"][0]["evidence"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (_, other) = call(
        &f,
        Some(2),
        false,
        "GET",
        "/api/v1/connectivity",
        Value::Null,
    )
    .await;
    assert!(other["endpoints"].as_array().unwrap().is_empty());
}
