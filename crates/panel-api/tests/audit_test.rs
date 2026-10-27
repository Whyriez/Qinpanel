#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_api::agent_client::AgentClient;
use panel_api::audit::{AuditLogItem, AuditLogListResponse, AuditWriter};
use panel_api::auth::{LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::routes::AppState;
use panel_core::audit::AuditCategory;
use panel_core::auth::hash_password;
use panel_db::models::{CreateAuditLog, CreateUser};
use panel_db::pool::{DbPool, init_pool, run_migrations};
use panel_db::repositories::{SqliteAuditLogRepository, SqliteUserRepository};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, DbPool, Arc<AuditWriter>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("audit_test.db");
    let db_url = format!(
        "sqlite://{}",
        db_path.to_str().expect("to_str").replace('\\', "/")
    );

    let pool = init_pool(&db_url).await.expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let user_repo = SqliteUserRepository::new(pool.clone());

    // 1. Create admin user
    let admin_hash = hash_password("AdminPass123!").expect("hash admin");
    user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: admin_hash,
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create admin");

    // 2. Create regular user: alice
    let alice_hash = hash_password("AlicePass123!").expect("hash alice");
    user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create alice");

    // 3. Create regular user: bob
    let bob_hash = hash_password("BobPass123!").expect("hash bob");
    user_repo
        .create(&CreateUser {
            username: "bob".to_string(),
            password_hash: bob_hash,
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create bob");

    let sock_path = tmp.path().join("agent.sock");
    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };

    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(sock_path));
    let operations = panel_db::operations::open(&pool).await.expect("operations");
    let state = AppState::with_operations(
        agent_client,
        pool.clone(),
        operations,
        "development".to_string(),
        auth_config,
        lockout_manager,
    );

    let audit_writer = Arc::clone(&state.audit_writer);
    (create_router(state), pool, audit_writer, tmp)
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
async fn test_audit_writer_direct_sync_and_async() {
    let (_app, pool, audit_writer, _tmp) = setup_test_app().await;

    // 1. Synchronous record
    let entry1 = CreateAuditLog {
        user_id: None,
        actor_username: "system".to_string(),
        action: "system.boot".to_string(),
        target_resource: "host".to_string(),
        details_json: Some(r#"{"version":"0.1.0"}"#.to_string()),
        ip_address: Some("127.0.0.1".to_string()),
        status: "success".to_string(),
    };
    let created = audit_writer.record(entry1).await.expect("record sync");
    assert_eq!(created.action, "system.boot");
    assert_eq!(created.status, "success");

    // 2. Asynchronous record
    audit_writer.log_async(
        None,
        "backup-worker",
        "backup.create",
        "backup:1",
        "success",
        Some("127.0.0.1".to_string()),
        None,
    );

    // Wait for background task to write
    let repo =
        SqliteAuditLogRepository::new(panel_db::operations::for_maintenance(&pool).await.unwrap());
    let mut list = Vec::new();
    for _ in 0..40 {
        list = repo.list(10, 0).await.expect("list");
        if list.iter().any(|l| l.action == "backup.create") {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
    assert!(list.iter().any(|l| l.action == "backup.create"));
}

#[tokio::test]
async fn test_auth_actions_recorded_in_audit_log() {
    let (app, pool, _, _tmp) = setup_test_app().await;

    // 1. Failed login: non-existent user
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "unknown_user",
                "password": "WrongPassword123!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. Failed login: wrong password for valid user
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-Forwarded-For", "203.0.113.195")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": "alice",
                "password": "WrongPassword123!"
            }))
            .expect("json"),
        ))
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 3. Successful login: alice
    let (alice_cookie, alice_csrf) = login(&app, "alice", "AlicePass123!").await;

    // 4. Successful logout: alice
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/logout")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .header("X-CSRF-Token", &alice_csrf)
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // Wait for background tasks to settle
    let repo =
        SqliteAuditLogRepository::new(panel_db::operations::for_maintenance(&pool).await.unwrap());
    let mut all_logs = Vec::new();
    for _ in 0..60 {
        all_logs = repo.list(50, 0).await.expect("list");
        let has_unknown = all_logs.iter().any(|l| l.actor_username == "unknown_user");
        let has_alice_fail = all_logs
            .iter()
            .any(|l| l.actor_username == "alice" && l.status == "failure");
        let has_alice_success = all_logs.iter().any(|l| {
            l.actor_username == "alice" && l.status == "success" && l.action == "auth.login"
        });
        let has_alice_logout = all_logs
            .iter()
            .any(|l| l.actor_username == "alice" && l.action == "auth.logout");

        if has_unknown && has_alice_fail && has_alice_success && has_alice_logout {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    // Check unknown user failed attempt
    let unknown_log = all_logs
        .iter()
        .find(|l| l.actor_username == "unknown_user")
        .expect("unknown_user log");
    assert_eq!(unknown_log.action, "auth.login");
    assert_eq!(unknown_log.status, "failure");

    // Check alice wrong password failed attempt with extracted IP
    let alice_fail_log = all_logs
        .iter()
        .find(|l| l.actor_username == "alice" && l.status == "failure")
        .expect("alice fail log");
    assert_eq!(alice_fail_log.ip_address.as_deref(), Some("203.0.113.195"));

    // Check alice successful login
    let alice_success_log = all_logs
        .iter()
        .find(|l| l.actor_username == "alice" && l.status == "success" && l.action == "auth.login")
        .expect("alice success log");
    assert!(alice_success_log.user_id.is_some());

    // Check alice logout
    let alice_logout_log = all_logs
        .iter()
        .find(|l| l.actor_username == "alice" && l.action == "auth.logout")
        .expect("alice logout log");
    assert_eq!(alice_logout_log.status, "success");
}

#[tokio::test]
async fn test_audit_api_list_and_filters() {
    let (app, pool, audit_writer, _tmp) = setup_test_app().await;

    let user_repo = SqliteUserRepository::new(pool.clone());
    let alice = user_repo
        .find_by_username("alice")
        .await
        .expect("find")
        .expect("exists");
    let bob = user_repo
        .find_by_username("bob")
        .await
        .expect("find")
        .expect("exists");

    // Pre-populate some audit logs
    audit_writer
        .record(CreateAuditLog {
            user_id: Some(alice.id),
            actor_username: "alice".to_string(),
            action: "site.create".to_string(),
            target_resource: "alice.test".to_string(),
            details_json: Some(r#"{"kind":"static"}"#.to_string()),
            ip_address: Some("192.168.1.10".to_string()),
            status: "success".to_string(),
        })
        .await
        .expect("create");

    audit_writer
        .record(CreateAuditLog {
            user_id: Some(bob.id),
            actor_username: "bob".to_string(),
            action: "site.create".to_string(),
            target_resource: "bob.test".to_string(),
            details_json: Some(r#"{"kind":"php"}"#.to_string()),
            ip_address: Some("192.168.1.20".to_string()),
            status: "failure".to_string(),
        })
        .await
        .expect("create");

    audit_writer
        .record(CreateAuditLog {
            user_id: Some(alice.id),
            actor_username: "alice".to_string(),
            action: "domain.add".to_string(),
            target_resource: "sub.alice.test".to_string(),
            details_json: None,
            ip_address: Some("192.168.1.10".to_string()),
            status: "success".to_string(),
        })
        .await
        .expect("create");

    let (admin_cookie, _) = login(&app, "admin", "AdminPass123!").await;

    // 1. Admin lists all logs without filters
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/audit/logs")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let resp: AuditLogListResponse = serde_json::from_slice(&body_bytes).expect("parse response");
    assert!(resp.total >= 3);
    assert_eq!(resp.page, 1);
    assert_eq!(resp.per_page, 20);

    // 2. Admin filters by action
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/audit/logs?action=domain.add")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let resp: AuditLogListResponse = serde_json::from_slice(&body_bytes).expect("parse response");
    assert_eq!(resp.total, 1);
    assert_eq!(resp.items[0].action, "domain.add");

    // 3. Admin filters by status
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/audit/logs?status=failure")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let resp: AuditLogListResponse = serde_json::from_slice(&body_bytes).expect("parse response");
    assert!(resp.items.iter().all(|i| i.status == "failure"));

    // 4. Admin filters by actor
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/audit/logs?actor=bob")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let resp: AuditLogListResponse = serde_json::from_slice(&body_bytes).expect("parse response");
    assert_eq!(resp.items.len(), 1);
    assert_eq!(resp.items[0].actor_username, "bob");

    // 5. Pagination test
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/audit/logs?page=1&per_page=2")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let resp: AuditLogListResponse = serde_json::from_slice(&body_bytes).expect("parse response");
    assert_eq!(resp.items.len(), 2);
    assert_eq!(resp.page, 1);
    assert_eq!(resp.per_page, 2);
    assert!(resp.total_pages >= 2);
}

#[tokio::test]
async fn test_audit_api_get_by_id() {
    let (app, _pool, audit_writer, _tmp) = setup_test_app().await;

    let created = audit_writer
        .record(CreateAuditLog {
            user_id: None,
            actor_username: "admin".to_string(),
            action: "setting.update".to_string(),
            target_resource: "setting:panel.title".to_string(),
            details_json: Some(r#"{"title":"Nusa Panel"}"#.to_string()),
            ip_address: Some("127.0.0.1".to_string()),
            status: "success".to_string(),
        })
        .await
        .expect("create");

    let (admin_cookie, _) = login(&app, "admin", "AdminPass123!").await;

    // 1. Existing ID -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/audit/logs/{}", created.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let item: AuditLogItem = serde_json::from_slice(&body_bytes).expect("parse");
    assert_eq!(item.id, created.id);
    assert_eq!(item.action, "setting.update");

    // 2. Non-existent ID -> 404 NOT FOUND
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/audit/logs/999999")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_audit_api_rbac_tenant_isolation() {
    let (app, pool, audit_writer, _tmp) = setup_test_app().await;

    let user_repo = SqliteUserRepository::new(pool.clone());
    let alice = user_repo
        .find_by_username("alice")
        .await
        .expect("find")
        .expect("exists");
    let bob = user_repo
        .find_by_username("bob")
        .await
        .expect("find")
        .expect("exists");

    // Alice log
    let alice_log = audit_writer
        .record(CreateAuditLog {
            user_id: Some(alice.id),
            actor_username: "alice".to_string(),
            action: "site.create".to_string(),
            target_resource: "alice.test".to_string(),
            details_json: None,
            ip_address: Some("192.168.1.10".to_string()),
            status: "success".to_string(),
        })
        .await
        .expect("create");

    // Bob log
    let bob_log = audit_writer
        .record(CreateAuditLog {
            user_id: Some(bob.id),
            actor_username: "bob".to_string(),
            action: "site.create".to_string(),
            target_resource: "bob.test".to_string(),
            details_json: None,
            ip_address: Some("192.168.1.20".to_string()),
            status: "success".to_string(),
        })
        .await
        .expect("create");

    // Unauthenticated request denied
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/audit/logs")
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Alice logs in
    let (alice_cookie, _) = login(&app, "alice", "AlicePass123!").await;

    // 1. Alice queries logs: automatically scoped to Alice only
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/audit/logs")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let resp: AuditLogListResponse = serde_json::from_slice(&body_bytes).expect("parse");
    // All items must belong to Alice
    assert!(resp.items.iter().all(|item| item.user_id == Some(alice.id)));
    assert!(!resp.items.iter().any(|item| item.user_id == Some(bob.id)));

    // 2. Alice attempts to explicitly query Bob's logs -> 403 FORBIDDEN
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/audit/logs?user_id={}", bob.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 3. Alice accesses her own log by ID -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/audit/logs/{}", alice_log.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    // 4. Alice attempts to access Bob's log by ID -> 403 FORBIDDEN
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/audit/logs/{}", bob_log.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 5. Admin can access Bob's log by ID -> 200 OK
    let (admin_cookie, _) = login(&app, "admin", "AdminPass123!").await;
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/audit/logs/{}", bob_log.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("build");
    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
}

async fn audit_get(app: &axum::Router, cookie: &str, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .uri(uri)
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, cookie),
        )
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn test_audit_categories_search_pagination_and_legacy_exact_action() {
    let (app, _, writer, _tmp) = setup_test_app().await;
    for (action, target, status) in [
        ("site.create", "demo%_site.test", "failure"),
        ("domain.create", "demo.test", "success"),
        ("site.ssl.issue", "demo.test", "success"),
        ("sshXkey.create", "legacy", "success"),
    ] {
        writer
            .log(None, "legacy-admin", action, target, status, None, None)
            .await
            .expect("record");
    }
    let (cookie, _) = login(&app, "admin", "AdminPass123!").await;
    let (status, body) = audit_get(
        &app,
        &cookie,
        "/api/v1/audit/logs?category=websites&per_page=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let response: AuditLogListResponse = serde_json::from_value(body).expect("list");
    assert_eq!(response.total, 3);
    assert_eq!(response.total_pages, 2);
    assert_eq!(response.items.len(), 2);
    assert!(
        response
            .items
            .iter()
            .all(|item| item.category == AuditCategory::Websites)
    );
    let (_, second) = audit_get(
        &app,
        &cookie,
        "/api/v1/audit/logs?category=websites&per_page=2&page=2",
    )
    .await;
    assert_eq!(second["items"].as_array().expect("items").len(), 1);
    assert!(
        response
            .items
            .iter()
            .all(|item| Some(item.id) != second["items"][0]["id"].as_i64())
    );
    for query in [
        "category=websites&q=%25_&status=failure",
        "category=websites&q=LEGACY-ADMIN&status=failure",
        "category=websites&action=site.create",
    ] {
        let (status, body) = audit_get(&app, &cookie, &format!("/api/v1/audit/logs?{query}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["total"], 1);
        assert_eq!(body["items"][0]["action"], "site.create");
    }
    let (_, body) = audit_get(
        &app,
        &cookie,
        "/api/v1/audit/logs?category=authentication&action=site.create",
    )
    .await;
    assert_eq!(body["total"], 0);
    let (_, body) = audit_get(&app, &cookie, "/api/v1/audit/logs?category=other").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["category"], "other");
    assert_eq!(body["items"][0]["action"], "sshXkey.create");
    let (status, _) = audit_get(&app, &cookie, "/api/v1/audit/logs?category=unknown").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = audit_get(
        &app,
        &cookie,
        &format!("/api/v1/audit/logs?q={}", "x".repeat(201)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = audit_get(&app, &cookie, "/api/v1/audit/logs?page=9223372036854775807").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_audit_category_search_keeps_tenant_scope() {
    let (app, pool, writer, _tmp) = setup_test_app().await;
    let users = SqliteUserRepository::new(pool);
    let alice = users
        .find_by_username("alice")
        .await
        .expect("alice")
        .expect("exists");
    let bob = users
        .find_by_username("bob")
        .await
        .expect("bob")
        .expect("exists");
    for (id, username) in [(alice.id, "alice"), (bob.id, "bob")] {
        writer
            .log(
                Some(id),
                username,
                "site.create",
                "example.test",
                "success",
                None,
                None,
            )
            .await
            .expect("record");
    }
    let (cookie, _) = login(&app, "alice", "AlicePass123!").await;
    let (status, body) = audit_get(
        &app,
        &cookie,
        "/api/v1/audit/logs?category=websites&q=example.test",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["user_id"], alice.id);
    let (_, body) = audit_get(&app, &cookie, "/api/v1/audit/logs?category=websites&q=bob").await;
    assert_eq!(body["total"], 0);
    let (status, _) = audit_get(
        &app,
        &cookie,
        &format!("/api/v1/audit/logs?category=websites&user_id={}", bob.id),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
