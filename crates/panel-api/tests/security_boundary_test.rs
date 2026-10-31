//! In-process HTTP/SQLite fixtures. No agent server or system commands are started.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use panel_api::{
    agent_client::AgentClient,
    auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME, generate_csrf_token},
    config::AuthConfig,
    create_router,
    routes::AppState,
};
use panel_core::auth::hash_token;
use panel_db::{
    models::{CreateSession, CreateSite, CreateUser, UpsertSiteGitConfig},
    pool::{init_pool, run_migrations},
    repositories::{
        SqliteGitDeployRepository, SqliteSessionRepository, SqliteSiteRepository,
        SqliteUserRepository,
    },
};
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    app: Router,
    pool: panel_db::pool::DbPool,
    _dir: tempfile::TempDir,
    admin: String,
    tenant: String,
    admin_csrf: String,
    tenant_csrf: String,
    tenant_id: i64,
}
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_pool(&format!(
        "sqlite://{}",
        dir.path()
            .join("audit.db")
            .to_string_lossy()
            .replace('\\', "/")
    ))
    .await
    .unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = SqliteUserRepository::new(pool.clone());
    let mut actors = Vec::new();
    for (name, role, uid) in [
        ("auditadmin", "admin", None),
        ("audituser", "user", Some(10042)),
    ] {
        let user = repo
            .create(&CreateUser {
                username: name.into(),
                email: None,
                password_hash: "unused-fixture-hash".into(),
                role: role.into(),
                linux_username: uid.map(|_| "p_audituser".into()),
                linux_uid: uid,
                plan_id: None,
            })
            .await
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        SqliteSessionRepository::new(pool.clone())
            .create(&CreateSession {
                id: id.clone(),
                user_id: user.id,
                token_hash: hash_token(&token),
                ip_address: None,
                user_agent: None,
                expires_at: chrono::Utc::now().timestamp() + 3600,
            })
            .await
            .unwrap();
        actors.push((token, generate_csrf_token(&id), user.id));
    }
    let state = AppState::new(
        Arc::new(AgentClient::new(dir.path().join("no-agent.sock"))),
        pool.clone(),
        "development".into(),
        AuthConfig::default(),
        Arc::new(LockoutManager::new(5, 900)),
    );
    Fixture {
        app: create_router(state),
        pool,
        _dir: dir,
        admin: actors[0].0.clone(),
        tenant: actors[1].0.clone(),
        admin_csrf: actors[0].1.clone(),
        tenant_csrf: actors[1].1.clone(),
        tenant_id: actors[1].2,
    }
}
async fn status(
    f: &Fixture,
    method: &str,
    path: &str,
    token: Option<&str>,
    csrf: Option<&str>,
    body: &str,
) -> StatusCode {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("cookie", format!("{SESSION_COOKIE_NAME}={token}"));
    }
    if let Some(csrf) = csrf {
        req = req.header(CSRF_HEADER_NAME, csrf);
    }
    f.app
        .clone()
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
        .status()
}
const SYSTEM_ROUTES: [(&str, &str, &str); 7] = [
    ("GET", "/api/v1/system/info", ""),
    ("GET", "/api/v1/system/doctor", ""),
    ("GET", "/api/v1/system/reconcile", ""),
    ("GET", "/api/v1/system/backups", ""),
    ("POST", "/api/v1/system/reconcile", "{}"),
    ("POST", "/api/v1/system/backups", "null"),
    (
        "POST",
        "/api/v1/system/backups/restore",
        r#"{"filepath":"/not-a-backup"}"#,
    ),
];

#[tokio::test]
async fn reconcile_does_not_report_a_clean_host_when_agent_is_unavailable() {
    let f = fixture().await;
    for method in ["GET", "POST"] {
        assert_eq!(
            status(
                &f,
                method,
                "/api/v1/system/reconcile",
                Some(&f.admin),
                Some(&f.admin_csrf),
                ""
            )
            .await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}

#[tokio::test]
async fn job_idempotency_cannot_read_another_actors_payload_or_enqueue_host_work() {
    let f = fixture().await;
    let admin: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username='auditadmin'")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    panel_db::repositories::SqliteJobRepository::new(f.pool.clone())
        .create(&panel_db::models::CreateJob {
            id: "another-actors-job".into(),
            user_id: Some(admin),
            name: "system.echo".into(),
            payload_json: r#"{"secret":"private-admin-fixture"}"#.into(),
            max_retries: 0,
        })
        .await
        .unwrap();
    assert_eq!(
        status(
            &f,
            "POST",
            "/api/v1/jobs",
            Some(&f.tenant),
            Some(&f.tenant_csrf),
            r#"{"id":"another-actors-job","name":"system.echo","payload":{}}"#
        )
        .await,
        StatusCode::CONFLICT
    );
    for name in [
        "database.server.install",
        "server.backup.restore",
        "cloudflare.apply",
        "database.import",
    ] {
        assert_eq!(
            status(
                &f,
                "POST",
                "/api/v1/jobs",
                Some(&f.tenant),
                Some(&f.tenant_csrf),
                &serde_json::json!({"name":name}).to_string()
            )
            .await,
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn panel_hostname_cannot_be_claimed_as_a_tenant_website() {
    let f = fixture().await;
    sqlx::query("INSERT INTO panel_url_aliases(origin) VALUES('https://admin.example.test:17890')")
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        status(
            &f,
            "POST",
            "/api/v1/sites",
            Some(&f.tenant),
            Some(&f.tenant_csrf),
            r#"{"domain":"admin.example.test","kind":"static"}"#
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM domains WHERE name='admin.example.test'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn enrollment_cannot_replace_an_active_factor_or_recovery_codes() {
    let f = fixture().await;
    let repo = SqliteUserRepository::new(f.pool.clone());
    repo.update_totp(f.tenant_id, Some("original-factor"), true)
        .await
        .unwrap();
    assert_eq!(
        status(
            &f,
            "POST",
            "/api/v1/auth/2fa/enable",
            Some(&f.tenant),
            Some(&f.tenant_csrf),
            r#"{"secret":"attacker-factor","code":"000000","recovery_codes":[]}"#
        )
        .await,
        StatusCode::CONFLICT
    );
    // A stale request that passed the HTTP-level check still fails atomically.
    let session: String = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id=?")
        .bind(f.tenant_id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(
        !repo
            .enable_totp_if_disabled(
                f.tenant_id,
                &session,
                "attacker-factor",
                &["new-code".into()]
            )
            .await
            .unwrap()
    );
    assert_eq!(
        repo.find_by_id(f.tenant_id)
            .await
            .unwrap()
            .unwrap()
            .totp_secret
            .as_deref(),
        Some("original-factor")
    );
}

#[tokio::test]
async fn concurrent_enrollment_commits_one_matching_factor_and_code_set() {
    let f = fixture().await;
    let repo = SqliteUserRepository::new(f.pool.clone());
    let session: String = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id=?")
        .bind(f.tenant_id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let first = vec!["first-code".to_owned()];
    let second = vec!["second-code".to_owned()];
    let (a, b) = tokio::join!(
        repo.enable_totp_if_disabled(f.tenant_id, &session, "first-factor", &first),
        repo.enable_totp_if_disabled(f.tenant_id, &session, "second-factor", &second)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_ne!(a, b);
    let secret = repo
        .find_by_id(f.tenant_id)
        .await
        .unwrap()
        .unwrap()
        .totp_secret
        .unwrap();
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT code_hash FROM user_recovery_codes WHERE user_id=?")
            .bind(f.tenant_id)
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(
        (secret, hashes),
        if a {
            ("first-factor".to_owned(), first)
        } else {
            ("second-factor".to_owned(), second)
        }
    );
}
#[tokio::test]
async fn system_operations_reject_anonymous_and_tenant_before_agent_access() {
    let f = fixture().await;
    for (method, path, body) in SYSTEM_ROUTES {
        assert_eq!(
            status(&f, method, path, None, None, body).await,
            StatusCode::UNAUTHORIZED,
            "anonymous {method} {path}"
        );
        assert_eq!(
            status(
                &f,
                method,
                path,
                Some(&f.tenant),
                Some(&f.tenant_csrf),
                body
            )
            .await,
            StatusCode::FORBIDDEN,
            "tenant {method} {path}"
        );
    }
}
#[tokio::test]
async fn admin_system_writes_require_own_csrf_even_with_invalid_json() {
    let f = fixture().await;
    for (_, path, _) in SYSTEM_ROUTES
        .into_iter()
        .filter(|(method, _, _)| *method == "POST")
    {
        for csrf in [None, Some("invalid"), Some(f.tenant_csrf.as_str())] {
            assert_eq!(
                status(&f, "POST", path, Some(&f.admin), csrf, "invalid-json").await,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
    }
    assert_eq!(
        status(&f, "GET", "/api/v1/system/info", Some(&f.admin), None, "").await,
        StatusCode::OK
    );
    assert_eq!(
        status(
            &f,
            "POST",
            "/api/v1/system/backups/restore",
            Some(&f.admin),
            Some(&f.admin_csrf),
            "invalid-json"
        )
        .await,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn api_writes_cannot_bypass_csrf_in_legacy_handlers() {
    let f = fixture().await;
    for (method, path) in [
        ("POST", "/api/v1/files/mkdir"),
        ("POST", "/api/v1/files/upload/init"),
        ("PUT", "/api/v1/sites/1/git"),
        ("POST", "/api/v1/sites/1/cron"),
        ("DELETE", "/api/v1/sites/1"),
        ("PUT", "/api/v1/sites/1/proxy/settings"),
    ] {
        assert_eq!(
            status(&f, method, path, Some(&f.tenant), None, "invalid-json").await,
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
        assert_eq!(
            status(
                &f,
                method,
                path,
                Some(&f.tenant),
                Some(&f.tenant_csrf),
                "invalid-json"
            )
            .await,
            if method == "DELETE" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            },
            "valid CSRF must reach the JSON extractor for {path}"
        );
    }
    assert_eq!(
        status(&f, "POST", "/api/v1/auth/setup", None, None, "{}").await,
        StatusCode::GONE
    );
    assert_eq!(
        status(&f, "GET", "/healthz", None, None, "").await,
        StatusCode::OK
    );
}
#[tokio::test]
async fn webhook_never_deploys_without_a_nonempty_secret_and_valid_signature() {
    let f = fixture().await;
    let site = SqliteSiteRepository::new(f.pool.clone())
        .create(&CreateSite {
            user_id: f.tenant_id,
            kind: "static".into(),
            docroot: "/home/p_audituser/sites/audit/public".into(),
            runtime: None,
            version: None,
            status: Some("active".into()),
            settings_json: None,
        })
        .await
        .unwrap();
    let repo = SqliteGitDeployRepository::new(f.pool.clone());
    for secret in [None, Some(""), Some("   "), Some("fixture-webhook-secret")] {
        repo.upsert_config(&UpsertSiteGitConfig {
            site_id: site.id,
            repo_url: "https://github.com/example/fixture.git".into(),
            branch: "main".into(),
            deploy_key_pub: None,
            deploy_key_priv_path: None,
            build_command: None,
            post_deploy_hook: None,
            docroot_subpath: None,
            releases_to_keep: None,
            webhook_secret: secret.map(String::from),
            auto_deploy_enabled: Some(true),
            current_release_id: None,
        })
        .await
        .unwrap();
        // A different branch ensures the vulnerable implementation has no side effects.
        let result = status(
            &f,
            "POST",
            &format!("/api/v1/webhooks/deploy/{}", site.id),
            None,
            None,
            r#"{"ref":"refs/heads/other"}"#,
        )
        .await;
        assert!(
            matches!(result, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
            "unsigned webhook with secret {secret:?} returned {result}"
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM site_deployments")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
