#![allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]
//! Local SQLite, mock agent RPC and fake SMTP only. No panel or mail daemons.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_agent::system::MockSystemRunner;
use panel_api::{
    agent_client::AgentClient,
    auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME},
    config::AuthConfig,
    jobs::registry::JobContext,
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
use panel_proto::{RequestPayload, ResponsePayload, RpcResponse, RpcStream};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
struct Fixture {
    state: AppState,
    _temp: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
    domain: i64,
    other_domain: i64,
    customer: i64,
    runner: Arc<MockSystemRunner>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("mock-agent");
        #[cfg(unix)]
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        #[cfg(not(unix))]
        let listener = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            std::fs::write(&path, l.local_addr().unwrap().port().to_string()).unwrap();
            l
        };
        let runner = Arc::new(MockSystemRunner::new());
        for file in ["fullchain.pem", "privkey.pem"] {
            runner.set_file_content(
                format!("/etc/panel/ssl/mail.example.com/{file}"),
                b"fixture",
            );
        }
        for (name, uid) in [("customer", 10001), ("other", 10002)] {
            runner.set_response(
                &format!("getent passwd p_{name}"),
                0,
                format!("p_{name}:x:{uid}:{uid}::/home/p_{name}:/usr/sbin/nologin").as_bytes(),
                b"",
            );
        }
        runner.set_response("getent group dovecot", 0, b"dovecot:x:111:", b"");
        runner.set_response("getent group opendkim", 0, b"opendkim:x:112:", b"");
        runner.set_response("getent group postfix", 0, b"postfix:x:113:", b"");
        runner.set_response(
            "getent passwd opendkim",
            0,
            b"opendkim:x:110:112::/run/opendkim:/usr/sbin/nologin",
            b"",
        );
        runner.set_response("doveadm pw -l", 0, b"ARGON2ID", b"");
        for domain in ["example.com", "other.example.com"] {
            runner.set_file_content(
                format!("/etc/opendkim/keys/{domain}/qpanel.private"),
                b"private-fixture",
            );
            runner.set_file_content(
                format!("/etc/opendkim/keys/{domain}/qpanel.txt"),
                b"(\"v=DKIM1; p=PUBLIC\")",
            );
        }
        panel_agent::email::install(
            runner.as_ref(),
            panel_proto::EmailServerConfig {
                hostname: "mail.example.com".into(),
                certificate_id: 1,
                certificate_domain: "mail.example.com".into(),
            },
        )
        .unwrap();
        let mock = runner.clone();
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let runner = mock.clone();
                tokio::spawn(async move {
                    let mut rpc = RpcStream::new(socket);
                    while let Ok(req) = rpc.recv_request().await {
                        let result = match req.payload {
                            RequestPayload::Email(cmd) => {
                                panel_agent::email::execute(runner.as_ref(), cmd)
                                    .map(ResponsePayload::Email)
                            }
                            RequestPayload::InstallWithProgress(
                                panel_proto::InstallOperation::EmailServer(config),
                            ) => panel_agent::email::install(runner.as_ref(), config)
                                .map(ResponsePayload::Email),
                            _ => Ok(ResponsePayload::Pong),
                        };
                        let response = match result {
                            Ok(v) => RpcResponse::ok(&req.request_id, v),
                            Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                        };
                        if rpc.send_response(&response).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        let pool = init_pool(&format!(
            "sqlite://{}",
            temp.path()
                .join("email.db")
                .to_string_lossy()
                .replace('\\', "/")
        ))
        .await
        .unwrap();
        run_migrations(&pool).await.unwrap();
        let hash = hash_password("FixturePassword123!").unwrap();
        let mut customer = 0;
        let mut domain = 0;
        let mut other_domain = 0;
        for (name, role, uid) in [
            ("admin", "admin", None),
            ("customer", "user", Some(10001)),
            ("other", "user", Some(10002)),
        ] {
            let user = SqliteUserRepository::new(pool.clone())
                .create(&CreateUser {
                    username: name.into(),
                    password_hash: hash.clone(),
                    role: role.into(),
                    email: Some(format!("{name}@example.net")),
                    linux_username: uid.map(|_| format!("p_{name}")),
                    linux_uid: uid,
                    plan_id: None,
                })
                .await
                .unwrap();
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
            if role == "user" {
                let site = SqliteSiteRepository::new(pool.clone())
                    .create(&CreateSite {
                        user_id: user.id,
                        kind: "static".into(),
                        docroot: format!("/home/{name}/app"),
                        runtime: None,
                        version: None,
                        status: Some("active".into()),
                        settings_json: None,
                    })
                    .await
                    .unwrap();
                let host = if name == "customer" {
                    "example.com"
                } else {
                    "other.example.com"
                };
                let d = SqliteDomainRepository::new(pool.clone())
                    .create(&CreateDomain {
                        site_id: site.id,
                        name: host.into(),
                        unicode_name: host.into(),
                        is_primary: true,
                        redirect_mode: None,
                        redirect_target: None,
                        status: None,
                    })
                    .await
                    .unwrap();
                if name == "customer" {
                    domain = d.id;
                    customer = user.id;
                } else {
                    other_domain = d.id;
                }
            }
        }
        let state = AppState::new(
            Arc::new(AgentClient::new(path)),
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
            task,
            domain,
            other_domain,
            customer,
            runner,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        user: Option<&str>,
        csrf: bool,
        payload: Value,
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
            .oneshot(req.body(Body::from(payload.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    async fn create(&self) -> Value {
        let (s, v) = self
            .call(
                "POST",
                "/api/v1/email/mailboxes",
                Some("customer"),
                true,
                json!({"domain_id":self.domain,"local_part":"admin","quota_mb":1024}),
            )
            .await;
        assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        v
    }
    async fn run(&self, id: &str) -> Result<Option<String>, String> {
        let r = &self.state.job_runner;
        let job = r.job_repo.find_by_id(id).await.unwrap().unwrap();
        r.job_repo
            .update_status(id, "running", None, None)
            .await
            .unwrap();
        let ctx = JobContext {
            job_id: id.into(),
            pool: self.state.pool.clone(),
            job_repo: r.job_repo.clone(),
            event_tx: r.event_tx.clone(),
            cancel_token: tokio_util::sync::CancellationToken::new(),
        };
        let result = r.registry.get(&job.name).unwrap().run(&job, &ctx).await;
        r.job_repo
            .update_status(
                id,
                if result.is_ok() {
                    "succeeded"
                } else {
                    "failed"
                },
                result.as_ref().ok().and_then(|s| s.as_deref()),
                result.as_ref().err().map(String::as_str),
            )
            .await
            .unwrap();
        result
    }
}
#[tokio::test]
async fn ownership_csrf_and_admin_configuration_are_enforced() {
    let f = Fixture::new().await;
    assert_eq!(
        f.call("GET", "/api/v1/email", None, false, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for path in ["/api/v1/email/templates", "/api/v1/email/install"] {
        assert_eq!(
            f.call(
                if path.ends_with("install") {
                    "POST"
                } else {
                    "GET"
                },
                path,
                Some("customer"),
                true,
                json!({"hostname":"mail.example.com","certificate_id":1})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/email/mailboxes",
            Some("customer"),
            false,
            json!({"domain_id":f.domain,"local_part":"admin","quota_mb":1024})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/email/mailboxes",
            Some("customer"),
            true,
            json!({"domain_id":f.other_domain,"local_part":"admin","quota_mb":1024})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let created = f.create().await;
    let id = created["id"].as_str().unwrap();
    for suffix in ["/messages?folder=inbox", "/dns"] {
        assert_eq!(
            f.call(
                "GET",
                &format!("/api/v1/email/mailboxes/{id}{suffix}"),
                Some("other"),
                false,
                Value::Null
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        f.call(
            "POST",
            &format!("/api/v1/email/mailboxes/{id}"),
            Some("other"),
            true,
            json!({"action":"reset_password"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let overview = f
        .call("GET", "/api/v1/email", Some("other"), false, Value::Null)
        .await
        .1;
    assert!(overview["mailboxes"].as_array().unwrap().is_empty());
}
#[tokio::test]
async fn mailbox_jobs_never_store_plaintext_or_hash_and_recover_cancelled_changes() {
    let f = Fixture::new().await;
    let created = f.create().await;
    let id = created["id"].as_str().unwrap();
    let job = created["job"]["id"].as_str().unwrap();
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM email_mailboxes WHERE id=?")
        .bind(id)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert!(panel_core::verify_password(created["password"].as_str().unwrap(), &hash).unwrap());
    assert!(!created["job"].to_string().contains("argon2"));
    let duplicate = f
        .call(
            "POST",
            "/api/v1/email/mailboxes",
            Some("customer"),
            true,
            json!({"domain_id":f.domain,"local_part":"admin","quota_mb":1024}),
        )
        .await;
    assert_eq!(duplicate.0, StatusCode::CONFLICT);
    f.run(job).await.unwrap();
    let list = f
        .call("GET", "/api/v1/email", Some("customer"), false, Value::Null)
        .await
        .1;
    assert_eq!(list["mailboxes"][0]["state"], "active");
    assert!(!list.to_string().contains("argon2"));
    assert!(
        !list
            .to_string()
            .contains(created["password"].as_str().unwrap())
    );
    let (_, reset) = f
        .call(
            "POST",
            &format!("/api/v1/email/mailboxes/{id}"),
            Some("customer"),
            true,
            json!({"action":"reset_password","password":"CustomPassword123!"}),
        )
        .await;
    let reset_id = reset["job"]["id"].as_str().unwrap();
    f.state.job_runner.cancel(reset_id).await.unwrap();
    let list = f
        .call("GET", "/api/v1/email", Some("customer"), false, Value::Null)
        .await
        .1;
    assert_eq!(list["mailboxes"][0]["state"], "failed");
    let (_, retry) = f
        .call(
            "POST",
            &format!("/api/v1/email/mailboxes/{id}"),
            Some("customer"),
            true,
            json!({"action":"retry"}),
        )
        .await;
    f.run(retry["job"]["id"].as_str().unwrap()).await.unwrap();
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM email_mailboxes WHERE id=?")
        .bind(id)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert!(panel_core::verify_password("CustomPassword123!", &hash).unwrap());
}
#[tokio::test]
async fn smtp_password_is_sealed_masked_and_legacy_plaintext_is_migrated() {
    let f = Fixture::new().await;
    let (s, v) = f
        .call(
            "PUT",
            "/api/v1/notifications/settings",
            Some("admin"),
            true,
            json!({"smtp_password":"smtp_fixture_secret","smtp_port":587}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!v.to_string().contains("smtp_fixture_secret"));
    let cipher: Option<String> =
        sqlx::query_scalar("SELECT smtp_password_enc FROM notification_settings WHERE id=1")
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    let cipher = cipher.unwrap();
    assert!(cipher.starts_with("enc:v1:"));
    assert_eq!(
        panel_api::email::smtp_password(&f.state.pool, &f.state.agent_client, Some(cipher))
            .await
            .unwrap()
            .as_deref(),
        Some("smtp_fixture_secret")
    );
    sqlx::query("UPDATE notification_settings SET smtp_password_enc='legacy_fixture' WHERE id=1")
        .execute(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(
        panel_api::email::smtp_password(
            &f.state.pool,
            &f.state.agent_client,
            Some("legacy_fixture".into())
        )
        .await
        .unwrap()
        .as_deref(),
        Some("legacy_fixture")
    );
    let cipher: String =
        sqlx::query_scalar("SELECT smtp_password_enc FROM notification_settings WHERE id=1")
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    assert!(cipher.starts_with("enc:v1:"));
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/notifications/settings",
            Some("admin"),
            true,
            json!({"smtp_port":65537})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn template_validation_is_text_only_single_pass_and_admin_only() {
    let f = Fixture::new().await;
    let t = json!({"panel_name":"Brand","subject":"[{panel_name}] {title}","body":"{message}\n{action_url}"});
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/email/templates",
            Some("admin"),
            true,
            t.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("PUT", "/api/v1/email/templates", Some("customer"), true, t)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/email/templates",
            Some("admin"),
            true,
            json!({"panel_name":"Brand","subject":"{password}","body":"Hello"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let t = panel_api::email::load_template(&f.state.pool)
        .await
        .unwrap();
    let (s, b) = t
        .render("Alert", "Literal {title}", "job_failed", "/jobs")
        .unwrap();
    assert_eq!(s, "[Brand] Alert");
    assert_eq!(b, "Literal {title}\n/jobs");
}
#[tokio::test]
async fn sends_are_idempotent_and_do_not_retry_uncertain_acceptance() {
    let f = Fixture::new().await;
    let created = f.create().await;
    let id = created["id"].as_str().unwrap();
    f.run(created["job"]["id"].as_str().unwrap()).await.unwrap();
    let message = json!({"request_id":uuid::Uuid::new_v4(),"to":"recipient@example.net","subject":"Hello","body":"Private fixture text"});
    let path = format!("/api/v1/email/mailboxes/{id}/send");
    let collision = uuid::Uuid::new_v4().to_string();
    f.state
        .job_runner
        .enqueue(
            Some(f.customer + 1),
            Some(collision.clone()),
            "system.echo".into(),
            json!({"private":"another-user-fixture"}).to_string(),
            Some(0),
        )
        .await
        .unwrap();
    let(status,rejected)=f.call("POST",&path,Some("customer"),true,json!({"request_id":collision,"to":"recipient@example.net","subject":"Hello","body":"Text"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!rejected.to_string().contains("another-user-fixture"));
    let (s, v) = f
        .call("POST", &path, Some("customer"), true, message.clone())
        .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let job = v["job"]["id"].as_str().unwrap();
    assert!(!v.to_string().contains("Private fixture text"));
    assert_eq!(v["job"]["max_retries"], 0);
    let retry = f
        .call("POST", &path, Some("customer"), true, message.clone())
        .await
        .1;
    assert_eq!(retry["job"]["id"], job);
    f.run(job).await.unwrap();
    assert!(f.run(job).await.is_err());
    let (s, v) = f.call("POST", &path, Some("customer"), true, message).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!(v["job"]["id"], job);
    let state: (String, String) =
        sqlx::query_as("SELECT state,body FROM email_deliveries WHERE id=?")
            .bind(job)
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    assert_eq!(state, ("accepted".into(), "".into()));
    let commands = format!("{:?}", f.runner.recorded_actions());
    assert!(!commands.contains("Private fixture text"));
}
#[tokio::test]
async fn notification_dispatch_uses_persisted_delivery_jobs_without_smtp_network() {
    let f = Fixture::new().await;
    sqlx::query("UPDATE notification_settings SET smtp_enabled=1,smtp_host='smtp.example.com',admin_email='ops@example.net' WHERE id=1").execute(&f.state.pool).await.unwrap();
    let service = panel_api::notifications::NotificationService::new(
        f.state.pool.clone(),
        Some(f.state.agent_client.clone()),
    )
    .with_job_runner(f.state.job_runner.clone());
    service
        .dispatch(panel_api::notifications::DispatchNotificationEvent {
            user_id: Some(f.customer),
            site_id: None,
            category: panel_core::notification::NotificationCategory::JobFailed,
            severity: panel_core::notification::NotificationSeverity::Error,
            title: "Fixture failed".into(),
            message: "Private notification text".into(),
            metadata_json: None,
            action_url: Some("/jobs".into()),
        })
        .await
        .unwrap();
    let jobs: Vec<String> =
        sqlx::query_scalar("SELECT payload_json FROM jobs WHERE name='email.deliver'")
            .fetch_all(&f.state.pool)
            .await
            .unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(
        jobs.iter()
            .all(|s| !s.contains("Private notification text"))
    );
}

#[tokio::test]
async fn email_tls_catalog_excludes_unusable_certificates_and_links_owned_projects() {
    let f = Fixture::new().await;
    let now = chrono::Utc::now().timestamp();
    let site: i64 = sqlx::query_scalar("SELECT id FROM sites WHERE user_id=?")
        .bind(f.customer)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    let mut expected = Vec::new();
    let mut rejected = Vec::new();
    for (issuer, status, not_before, not_after, usable) in [
        ("letsencrypt", "active", now - 60, now + 86400, true),
        ("custom", "active", now - 60, now + 86400, true),
        (
            "letsencrypt_staging",
            "active",
            now - 60,
            now + 86400,
            false,
        ),
        ("custom", "active", now + 3600, now + 86400, false),
        ("custom", "active", now - 86400, now - 1, false),
        ("custom", "revoked", now - 60, now + 86400, false),
    ] {
        let id = sqlx::query("INSERT INTO certificates(site_id,primary_domain,issuer,status,cert_path,key_path,fullchain_pem,privkey_pem,not_before,not_after,created_at,updated_at) VALUES(?,'mail.example.com',?,?,'fixture','fixture','PRIVATE-CHAIN','PRIVATE-KEY',?,?,?,?)")
            .bind(site).bind(issuer).bind(status).bind(not_before).bind(not_after).bind(now).bind(now)
            .execute(&f.state.pool).await.unwrap().last_insert_rowid();
        if usable {
            expected.push(id);
        } else {
            rejected.push(id);
        }
    }
    let (status, overview) = f
        .call("GET", "/api/v1/email", Some("admin"), false, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    let actual: Vec<i64> = overview["certificates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    assert_eq!(actual, expected);
    assert!(
        overview["domains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["site_id"] == site)
    );
    assert!(!overview.to_string().contains("PRIVATE-"));
    for certificate_id in rejected {
        let (status, _) = f
            .call(
                "POST",
                "/api/v1/email/install",
                Some("admin"),
                true,
                json!({"hostname":"mail.example.com","certificate_id":certificate_id}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (_, tenant) = f
        .call("GET", "/api/v1/email", Some("customer"), false, Value::Null)
        .await;
    assert_eq!(tenant["certificates"], json!([]));
    assert!(
        tenant["domains"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["site_id"] == site)
    );
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE name='email.install'")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
}

#[tokio::test]
async fn certificate_selection_and_alert_evaluation_use_the_email_integration() {
    let f = Fixture::new().await;
    let now = chrono::Utc::now().timestamp();
    let site: i64 = sqlx::query_scalar("SELECT id FROM sites WHERE user_id=?")
        .bind(f.customer)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO certificates(site_id,primary_domain,issuer,cert_path,key_path,fullchain_pem,privkey_pem,not_before,not_after,created_at,updated_at) VALUES(?,'mail.example.com','custom','fixture','fixture','fixture','fixture',?,?,?,?)").bind(site).bind(now-60).bind(now+86400).bind(now).bind(now).execute(&f.state.pool).await.unwrap();
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/email/install",
            Some("admin"),
            true,
            json!({"hostname":"wrong.example.com","certificate_id":1}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let(status,installed)=f.call("POST","/api/v1/email/install",Some("admin"),true,json!({"hostname":"mail.example.com","certificate_id":1,"certificate_domain":"attacker.example.com"})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let payload: Value =
        serde_json::from_str(installed["job"]["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(payload["certificate_domain"], "mail.example.com");
    f.run(installed["job"]["id"].as_str().unwrap())
        .await
        .unwrap();
    // The existing expiry evaluator selects auto-renewable ACME certificates.
    sqlx::query("UPDATE certificates SET issuer='letsencrypt' WHERE id=1")
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE notification_settings SET smtp_enabled=1,smtp_host='fixture.smtp.example',admin_email='ops@example.net' WHERE id=1").execute(&f.state.pool).await.unwrap();
    let (status, value) = f
        .call(
            "POST",
            "/api/v1/notifications/evaluate",
            Some("admin"),
            true,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["triggered_count"], 1, "{value}");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE name='email.deliver'")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
