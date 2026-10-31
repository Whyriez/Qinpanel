#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_agent::hardening::HardeningManager;
use panel_agent::quota::QuotaManager;
use panel_agent::resource::CgroupReader;
use panel_agent::slice::SliceManager;
use panel_agent::system::{MockSystemRunner, SystemRunner};
use panel_agent::user::UserLifecycleManager;
use panel_agent::webserver::NginxManager;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::git_deploy::{
    RollbackDeployRequest, SiteDeploymentDto, SiteGitConfigResponse, TriggerDeployRequest,
    UpsertGitConfigRequest,
};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_core::git_deploy::compute_hmac_sha256;
use panel_db::models::{CreateDomain, CreateSite, CreateUser};
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::{SqliteDomainRepository, SqliteSiteRepository, SqliteUserRepository};
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

struct TestContext {
    app: axum::Router,
    runner: Arc<MockSystemRunner>,
    #[allow(dead_code)]
    pool: panel_db::pool::DbPool,
    _temp: tempfile::TempDir,
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
    alice_session: String,
    alice_csrf: String,
    site_id: i64,
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

async fn setup_test_context() -> TestContext {
    let tmp = tempdir().expect("tempdir");
    let db_file = tmp.path().join("panel.db");

    #[cfg(unix)]
    let sock_path = tmp.path().join("agent.sock");
    #[cfg(not(unix))]
    let sock_path = tmp.path().join("agent.port");

    let cgroup_dir = tmp.path().join("cgroup");
    fs::create_dir_all(&cgroup_dir).expect("create cgroup dir");
    fs::write(
        cgroup_dir.join("cgroup.controllers"),
        "cpu memory io pids\n",
    )
    .expect("write controllers");

    let mounts_path = tmp.path().join("mounts");
    fs::write(
        &mounts_path,
        "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n",
    )
    .expect("write mounts");

    let group_path = tmp.path().join("group");
    fs::write(&group_path, "root:x:0:\n").expect("write group");

    let nginx_dir = tmp.path().join("etc/nginx");
    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response(
        "getent passwd p_alice",
        0,
        b"p_alice:x:10001:10001::/home/p_alice:/bin/bash\n",
        b"",
    );
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"", b"");
    runner.set_response("setfacl", 0, b"", b"");
    runner.set_response("chown", 0, b"", b"");
    runner.set_response("chmod", 0, b"", b"");
    runner.set_response(
        "git",
        0,
        b"commit 123456\nAuthor: Bob <bob@example.com>\nInitial\n",
        b"",
    );
    runner.set_response("ssh-keygen", 0, b"", b"");

    let user_manager = Arc::new(UserLifecycleManager::new(
        Arc::clone(&runner) as Arc<dyn SystemRunner>
    ));
    let slice_manager = Arc::new(SliceManager::new(
        Arc::clone(&runner) as Arc<dyn SystemRunner>
    ));
    let quota_manager = Arc::new(QuotaManager::new(
        Arc::clone(&runner) as Arc<dyn SystemRunner>
    ));
    let resource_reader = Arc::new(CgroupReader::new(&cgroup_dir));
    let hardening_manager = Arc::new(HardeningManager::with_paths(
        Arc::clone(&runner) as Arc<dyn SystemRunner>,
        &cgroup_dir,
        &mounts_path,
        &group_path,
    ));
    let webserver_manager = Arc::new(NginxManager::with_base_dir(
        Arc::clone(&runner) as Arc<dyn SystemRunner>,
        &nginx_dir,
    ));

    let agent_cfg = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::with_all_managers(
        agent_cfg,
        user_manager,
        slice_manager,
        quota_manager,
        resource_reader,
        hardening_manager,
        webserver_manager,
    ));

    tokio::spawn(async move {
        server.run(rx).await.expect("agent server run");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let pool = init_pool(&format!(
        "sqlite://{}",
        db_file.to_str().expect("db_file").replace('\\', "/")
    ))
    .await
    .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let alice_hash = hash_password("AlicePass123!").expect("hash pass");
    let alice = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            plan_id: Some(1),
            linux_username: Some("p_alice".to_string()),
            linux_uid: Some(10001),
        })
        .await
        .expect("create alice");

    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };
    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(sock_path));
    let audit_writer = Arc::new(panel_api::audit::AuditWriter::new(pool.clone()));
    let job_runner = Arc::new(panel_api::jobs::JobRunner::with_registry(
        pool.clone(),
        panel_api::jobs::JobRegistry::new_with_agent(Arc::clone(&agent_client)),
    ));

    let state = AppState {
        agent_client: Arc::clone(&agent_client),
        operations_pool: pool.clone(),
        operations_read_pool: pool.clone(),
        pool: pool.clone(),
        environment: "testing".to_string(),
        auth_config,
        lockout_manager,
        audit_writer,
        job_runner,
        terminal_hub: Arc::new(panel_api::terminal::TerminalHub::new(Arc::clone(
            &agent_client,
        ))),
        analytics_storage: Arc::new(tokio::sync::RwLock::new(None)),
        analytics_db_path: tmp.path().join("analytics.db"),
        collector_status: Arc::new(tokio::sync::RwLock::new(Default::default())),
    };

    let app = create_router(state);

    // Login alice to obtain session cookie & CSRF token
    let login_body = serde_json::json!({
        "username": "alice",
        "password": "AlicePass123!"
    });
    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&login_body).expect("json")))
        .expect("req");

    let login_res = app.clone().oneshot(login_req).await.expect("login");
    assert_eq!(login_res.status(), StatusCode::OK);

    let cookie_val = login_res
        .headers()
        .get(header::SET_COOKIE)
        .expect("set cookie")
        .to_str()
        .expect("cookie str");
    let alice_session =
        extract_cookie_value(cookie_val, SESSION_COOKIE_NAME).expect("alice session");
    let login_json: serde_json::Value =
        serde_json::from_slice(&login_res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let alice_csrf = login_json["csrf_token"].as_str().unwrap().to_string();

    // Git tests start with an existing site; PHP provisioning has its own suite.
    let site_id = SqliteSiteRepository::new(pool.clone())
        .create(&CreateSite {
            user_id: alice.id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/example.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("seed existing Alice site for Git tests")
        .id;
    SqliteDomainRepository::new(pool.clone())
        .create(&CreateDomain {
            site_id,
            name: "example.com".to_string(),
            unicode_name: "example.com".to_string(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .expect("seed primary domain for Git tests");

    TestContext {
        app,
        runner,
        pool,
        _temp: tmp,
        _shutdown_tx: shutdown_tx,
        alice_session,
        alice_csrf,
        site_id,
    }
}

#[tokio::test]
async fn test_git_deploy_full_workflow() {
    let ctx = setup_test_context().await;

    // 1. GET /api/v1/sites/{id}/git (initial empty)
    let get_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/git", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let res = ctx
        .app
        .clone()
        .oneshot(get_req)
        .await
        .expect("get git config");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let config: SiteGitConfigResponse = serde_json::from_slice(&body_bytes).expect("parse json");
    assert_eq!(config.site_id, ctx.site_id);
    assert_eq!(config.branch, "main");
    assert!(config.repo_url.is_empty());

    // 2. PUT /api/v1/sites/{id}/git (save config)
    let upsert_payload = UpsertGitConfigRequest {
        repo_url: "git@github.com:myorg/myapp.git".to_string(),
        branch: "main".to_string(),
        build_cmd: Some("npm run build".to_string()),
        post_deploy_cmd: Some("npm run migrate".to_string()),
        docroot_subpath: Some("dist".to_string()),
        auto_deploy: Some(true),
        keep_releases: Some(5),
        webhook_secret: Some("secret1234567890abcdef".to_string()),
        rotate_webhook_secret: None,
    };

    let put_req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sites/{}/git", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&upsert_payload).expect("json"),
        ))
        .expect("req");

    let res = ctx
        .app
        .clone()
        .oneshot(put_req)
        .await
        .expect("put git config");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let updated: SiteGitConfigResponse = serde_json::from_slice(&body_bytes).expect("parse json");
    assert_eq!(updated.repo_url, "git@github.com:myorg/myapp.git");
    assert_eq!(updated.branch, "main");
    assert_eq!(updated.build_cmd.as_deref(), Some("npm run build"));
    assert_eq!(updated.docroot_subpath.as_deref(), Some("dist"));
    assert!(updated.has_webhook_secret);

    // 3. POST /api/v1/sites/{id}/git/key/generate (generate deploy key)
    let key_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/git/key/generate", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::empty())
        .expect("req");

    let res = ctx
        .app
        .clone()
        .oneshot(key_req)
        .await
        .expect("generate key");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let key_res: serde_json::Value = serde_json::from_slice(&body_bytes).expect("parse json");
    assert!(key_res["public_key"].as_str().is_some());

    // 4. POST /api/v1/sites/{id}/git/deploy (trigger manual deploy)
    let deploy_payload = TriggerDeployRequest {
        branch: Some("main".to_string()),
        commit_hash: None,
    };
    let deploy_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/git/deploy", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&deploy_payload).expect("json"),
        ))
        .expect("req");

    let res = ctx.app.clone().oneshot(deploy_req).await.expect("deploy");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let deploy_out: SiteDeploymentDto = serde_json::from_slice(&body_bytes).expect("parse json");
    assert_eq!(deploy_out.site_id, ctx.site_id);
    assert_eq!(deploy_out.status, "success");
    assert!(deploy_out.is_current);

    // 5. GET /api/v1/sites/{id}/git/deployments (list deployments)
    let list_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/sites/{}/git/deployments", ctx.site_id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .body(Body::empty())
        .expect("req");

    let res = ctx
        .app
        .clone()
        .oneshot(list_req)
        .await
        .expect("list deployments");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let list: Vec<SiteDeploymentDto> = serde_json::from_slice(&body_bytes).expect("parse json");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, deploy_out.id);

    // 6. POST /api/v1/sites/{id}/git/rollback
    // The runner records tenant mkdir/clone/ln commands without executing them.
    // Seed their filesystem effects before exercising rollback's existence guard.
    let base = std::path::Path::new("/home/p_alice/sites/example.com");
    let release = base.join("releases").join(&deploy_out.release_id);
    ctx.runner.add_existing_path(&release);
    ctx.runner.add_existing_path(release.join("dist"));
    ctx.runner
        .create_symlink(&release.join("dist"), &base.join("current"))
        .expect("mock active release");
    let rollback_payload = RollbackDeployRequest {
        target_release_id: deploy_out.release_id.clone(),
    };
    let rollback_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/sites/{}/git/rollback", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, ctx.alice_session),
        )
        .header(CSRF_HEADER_NAME, &ctx.alice_csrf)
        .body(Body::from(
            serde_json::to_vec(&rollback_payload).expect("json"),
        ))
        .expect("req");

    let res = ctx
        .app
        .clone()
        .oneshot(rollback_req)
        .await
        .expect("rollback");
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let rollback_out: SiteDeploymentDto = serde_json::from_slice(&body_bytes).expect("parse json");
    assert_eq!(rollback_out.status, "success");
    assert_eq!(rollback_out.trigger_type, "rollback");

    // 7. POST /api/v1/webhooks/deploy/{site_id}
    // Test with invalid signature -> 401 Unauthorized
    let webhook_body = serde_json::json!({
        "ref": "refs/heads/main",
        "head_commit": {
            "id": "112233445566",
            "message": "fix: update readme",
            "author": { "name": "Alice", "email": "alice@example.com" }
        }
    });
    let webhook_bytes = serde_json::to_vec(&webhook_body).expect("json");

    let bad_webhook_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/webhooks/deploy/{}", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            "X-Hub-Signature-256",
            "sha256=invalidhashvalue00000000000000",
        )
        .body(Body::from(webhook_bytes.clone()))
        .expect("req");

    let res = ctx
        .app
        .clone()
        .oneshot(bad_webhook_req)
        .await
        .expect("bad webhook");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Test with valid signature -> 202 Accepted
    let secret = "secret1234567890abcdef";
    let computed_sig = compute_hmac_sha256(secret.as_bytes(), &webhook_bytes);
    let sig_header = format!(
        "sha256={}",
        panel_core::git_deploy::bytes_to_hex(&computed_sig)
    );

    let good_webhook_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/webhooks/deploy/{}", ctx.site_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-Hub-Signature-256", sig_header)
        .body(Body::from(webhook_bytes))
        .expect("req");

    let res = ctx
        .app
        .clone()
        .oneshot(good_webhook_req)
        .await
        .expect("good webhook");
    let status = res.status();
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "Expected 202 Accepted but got {}: {}",
        status,
        body_str
    );
}
