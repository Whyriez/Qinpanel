#![allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]
//! Local router, SQLite, generated certificates and typed RPC with virtual files only.
use super::*;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use panel_db::{CreateSession, CreateUser, SqliteSessionRepository, SqliteUserRepository};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tower::ServiceExt;
struct Fixture {
    state: AppState,
    app: Router,
    actor: i64,
    cookies: Vec<String>,
    csrf: Vec<String>,
    _runner: Arc<panel_agent::system::MockSystemRunner>,
    lost_ack: Arc<AtomicBool>,
    fail_apply: Arc<AtomicBool>,
    configs: Arc<std::sync::Mutex<Vec<String>>>,
    provider: Arc<std::sync::Mutex<Value>>,
    site: i64,
    task: tokio::task::JoinHandle<()>,
    _tmp: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("mock-rpc");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    #[cfg(not(unix))]
    let listener = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::fs::write(&path, listener.local_addr().unwrap().port().to_string()).unwrap();
        listener
    };

    let runner = Arc::new(panel_agent::system::MockSystemRunner::new());
    let server = runner.clone();
    let lost_ack = Arc::new(AtomicBool::new(false));
    let lost = lost_ack.clone();
    let fail_apply = Arc::new(AtomicBool::new(false));
    let fail = fail_apply.clone();
    let configs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let applied = configs.clone();
    let provider = Arc::new(std::sync::Mutex::new(
        json!({"configured":true,"zone_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","zone_name":"example.com","revision":"aa2cc435-972c-4327-aa11-eb1db6422026","pending":[]}),
    ));
    let status = provider.clone();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let mut rpc = panel_proto::RpcStream::new(stream);
            let req = rpc.recv_request().await.unwrap();
            let result = match req.payload {
                panel_proto::RequestPayload::Dns01(Dns01Request::StageCertificate {
                    id,
                    hostname,
                    names,
                    fullchain_pem,
                    private_key_pem,
                }) => panel_agent::dns01::execute(
                    server.as_ref(),
                    Dns01Request::StageCertificate {
                        id,
                        hostname,
                        names,
                        fullchain_pem,
                        private_key_pem,
                    },
                )
                .map(panel_proto::ResponsePayload::Dns01)
                .map_err(|e| e.to_string()),
                panel_proto::RequestPayload::Dns01(Dns01Request::VerifyOrigin { .. }) => {
                    if status.lock().unwrap()["fail_probe"] == true {
                        Err("fixture origin certificate mismatch".into())
                    } else {
                        Ok(panel_proto::ResponsePayload::Dns01(
                            json!({"verified":true}),
                        ))
                    }
                }
                panel_proto::RequestPayload::Dns01(Dns01Request::Status) => Ok(
                    panel_proto::ResponsePayload::Dns01(status.lock().unwrap().clone()),
                ),
                panel_proto::RequestPayload::Dns01(Dns01Request::Cleanup { .. }) => {
                    Ok(panel_proto::ResponsePayload::Dns01(json!({"cleaned":true})))
                }
                panel_proto::RequestPayload::SiteApply(data) => {
                    applied.lock().unwrap().push(data.config_content.clone());
                    if lost.swap(false, Ordering::SeqCst) {
                        continue;
                    }
                    if fail.swap(false, Ordering::SeqCst) {
                        Err("fixture nginx reload failed".into())
                    } else {
                        Ok(panel_proto::ResponsePayload::SiteApplied(
                            panel_proto::SiteAppliedResponse {
                                site_id: data.site_id,
                                available_path: "/fixture".into(),
                                enabled_path: None,
                                enabled: true,
                                reloaded: true,
                            },
                        ))
                    }
                }
                _ => panic!("Unexpected DNS test RPC"),
            };
            let res = match result {
                Ok(v) => panel_proto::RpcResponse::ok(req.request_id, v),
                Err(e) => panel_proto::RpcResponse::err(req.request_id, e),
            };
            rpc.send_response(&res).await.unwrap();
        }
    });
    let db = tmp
        .path()
        .join("core.db")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = panel_db::init_pool(&format!("sqlite://{db}"))
        .await
        .unwrap();
    panel_db::run_migrations(&pool).await.unwrap();
    let operations = panel_db::operations::open(&pool).await.unwrap();
    let mut cookies = Vec::new();
    let mut csrf = Vec::new();
    let mut actor = 0;
    for (i, role) in ["admin", "user"].into_iter().enumerate() {
        let user = SqliteUserRepository::new(pool.clone())
            .create(&CreateUser {
                username: format!("actor{i}"),
                password_hash: "fixture-hash".into(),
                role: role.into(),
                email: None,
                linux_username: None,
                linux_uid: None,
                plan_id: None,
            })
            .await
            .unwrap();
        if i == 0 {
            actor = user.id;
        }
        let token = format!("fixture-token-{i}");
        let sid = format!("session-{i}");
        SqliteSessionRepository::new(pool.clone())
            .create(&CreateSession {
                id: sid.clone(),
                user_id: user.id,
                token_hash: panel_core::hash_token(&token),
                ip_address: None,
                user_agent: None,
                expires_at: now() + 3600,
            })
            .await
            .unwrap();
        cookies.push(format!("{}={token}", crate::auth::SESSION_COOKIE_NAME));
        csrf.push(crate::auth::generate_csrf_token(&sid));
    }

    sqlx::query("UPDATE users SET linux_username='p_fixture',linux_uid=10001 WHERE id=?")
        .bind(actor + 1)
        .execute(&pool)
        .await
        .unwrap();
    let site = SqliteSiteRepository::new(pool.clone())
        .create(&panel_db::CreateSite {
            user_id: actor + 1,
            kind: "static".into(),
            docroot: "/home/p_fixture/sites/app.example.com/public".into(),
            runtime: None,
            version: None,
            status: Some("active".into()),
            settings_json: Some("{}".into()),
        })
        .await
        .unwrap();
    SqliteDomainRepository::new(pool.clone())
        .create(&panel_db::CreateDomain {
            site_id: site.id,
            name: "app.example.com".into(),
            unicode_name: "app.example.com".into(),
            is_primary: true,
            redirect_mode: Some("none".into()),
            redirect_target: None,
            status: Some("active".into()),
        })
        .await
        .unwrap();
    let state = AppState::with_operations(
        Arc::new(AgentClient::new(path)),
        pool,
        operations,
        "production".into(),
        crate::config::AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(crate::auth::LockoutManager::new(5, 60)),
    );
    Fixture {
        app: crate::create_router(state.clone()),
        state,
        actor,
        cookies,
        csrf,
        _runner: runner,
        lost_ack,
        fail_apply,
        configs,
        provider,
        site: site.id,
        task,
        _tmp: tmp,
    }
}
async fn call(
    f: &Fixture,
    method: &str,
    path: &str,
    actor: Option<usize>,
    csrf: bool,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "old.panel.test:17890")
        .header("content-type", "application/json");
    if let Some(i) = actor {
        builder = builder.header("cookie", &f.cookies[i]);
        if csrf {
            builder = builder.header(crate::auth::CSRF_HEADER_NAME, &f.csrf[i]);
        }
    }
    let response = f
        .app
        .clone()
        .oneshot(
            builder
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
            .unwrap_or_else(|_| json!({"error":String::from_utf8_lossy(&bytes)})),
    )
}

async fn queued(f: &Fixture, staging: bool) -> String {
    let (status, body) = call(
        f,
        "POST",
        "/api/v1/network/dns01/issue",
        Some(0),
        true,
        json!({"site_id":f.site,"email":"ops@example.test","staging":staging,"confirmed":true}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let payload: Value = serde_json::from_str(body["payload_json"].as_str().unwrap()).unwrap();
    payload["order_id"].as_str().unwrap().into()
}
struct FakeIssuer {
    calls: AtomicUsize,
    fail: bool,
}
impl FakeIssuer {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail: false,
        }
    }
}
impl Issuer for FakeIssuer {
    fn issue<'a>(
        &'a self,
        _: &'a Order,
        names: &'a [String],
    ) -> BoxFuture<'a, Result<Issued, String>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err("fixture CA unavailable".into());
            }
            let mut params = rcgen::CertificateParams::new(names.to_vec()).unwrap();
            params.not_before = time::OffsetDateTime::from_unix_timestamp(now() - 60).unwrap();
            params.not_after =
                time::OffsetDateTime::from_unix_timestamp(now() + 90 * 86400).unwrap();
            let key = rcgen::KeyPair::generate().unwrap();
            Ok(Issued {
                chain: params.self_signed(&key).unwrap().pem(),
                key: key.serialize_pem(),
            })
        })
    }
}
async fn run(f: &Fixture, id: &str, issuer: &dyn Issuer) -> Result<(), String> {
    process(&f.state.pool, &f.state.agent_client, id, issuer).await
}
async fn reset_cooldown(f: &Fixture) {
    sqlx::query("UPDATE dns01_orders SET last_attempt=0")
        .execute(&f.state.pool)
        .await
        .unwrap();
}
#[tokio::test]
async fn dns01_admin_csrf_strict_body_and_registered_project_are_required() {
    let f = fixture().await;
    assert_eq!(
        call(&f, "GET", "/api/v1/network/dns01", None, false, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &f,
            "GET",
            "/api/v1/network/dns01",
            Some(1),
            false,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let body = json!({"site_id":f.site,"email":"ops@example.test","staging":true,"confirmed":true});
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/network/dns01/issue",
            Some(0),
            false,
            body.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut bad = body.clone();
    bad["hostname"] = json!("arbitrary.other.test");
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/network/dns01/issue",
            Some(0),
            true,
            bad
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut bad = body.clone();
    bad["confirmed"] = json!(false);
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/network/dns01/issue",
            Some(0),
            true,
            bad
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut bad = body;
    bad["site_id"] = json!(99999);
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/network/dns01/issue",
            Some(0),
            true,
            bad
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn dns01_staging_persists_partitioned_job_and_never_installs_test_certificate() {
    let f = fixture().await;
    let id = queued(&f, true).await;
    let r = order(&f.state.pool, &id).await.unwrap();
    assert!(
        f.state
            .job_runner
            .job_repo
            .find_by_id(&r.job_id)
            .await
            .unwrap()
            .is_some()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let (status, _) = call(
        &f,
        "POST",
        "/api/v1/network/dns01/issue",
        Some(0),
        true,
        json!({"site_id":f.site,"email":"ops@example.test","staging":true,"confirmed":true}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    run(&f, &id, &FakeIssuer::new()).await.unwrap();
    assert_eq!(order(&f.state.pool, &id).await.unwrap().state, "staging");
    assert!(f.configs.lock().unwrap().is_empty());
    assert!(
        order(&f.state.pool, &id)
            .await
            .unwrap()
            .private_key_pem
            .is_empty()
    );
    let (status, body) = call(
        &f,
        "GET",
        "/api/v1/network/dns01",
        Some(0),
        false,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.to_string().contains("PRIVATE KEY"));
    assert!(!body.to_string().contains("ops@example.test"));
}
#[tokio::test]
async fn dns01_production_installs_immutable_origin_certificate_and_preserves_tls_tunnel_policy() {
    let f = fixture().await;
    let id = queued(&f, false).await;
    run(&f, &id, &FakeIssuer::new()).await.unwrap();
    let r = order(&f.state.pool, &id).await.unwrap();
    assert_eq!(r.state, "active");
    assert!(r.private_key_pem.is_empty());
    let cert = SqliteCertificateRepository::new(f.state.pool.clone())
        .find_active_by_site_id(f.site)
        .await
        .unwrap()
        .unwrap();
    assert!(cert.cert_path.contains(&id));
    {
        let configs = f.configs.lock().unwrap();
        assert!(configs.last().unwrap().contains("ssl_certificate"));
    }
    let b: (i64, i64) =
        sqlx::query_as("SELECT certificate_id,renew_at FROM dns01_bindings WHERE site_id=?")
            .bind(f.site)
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    assert_eq!(b.0, cert.id);
    assert!(b.1 > now() + 50 * 86400);
    let command = crate::network::validate_publication(
        &f.state.pool,
        panel_proto::CloudflareRequest::Publish {
            zone_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            tunnel_id: uuid::Uuid::new_v4().to_string(),
            hostname: "app.example.com".into(),
            tls: false,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        command,
        panel_proto::CloudflareRequest::Publish { tls: true, .. }
    ));
    f.state
        .job_runner
        .job_repo
        .update_status(&r.job_id, "running", None, None)
        .await
        .unwrap();
    assert!(!f.state.job_runner.job_repo.cancel(&r.job_id).await.unwrap());
}
#[tokio::test]
async fn dns01_automatic_renewal_uses_dns_and_failure_keeps_active_certificate() {
    let f = fixture().await;
    let first = queued(&f, false).await;
    run(&f, &first, &FakeIssuer::new()).await.unwrap();
    let old = order(&f.state.pool, &first).await.unwrap().certificate_id;
    reset_cooldown(&f).await;
    sqlx::query("UPDATE dns01_bindings SET renew_at=?,last_attempt=0")
        .bind(now() - 1)
        .execute(&f.state.pool)
        .await
        .unwrap();
    schedule(&f.state).await;
    let id: String = sqlx::query_scalar("SELECT id FROM dns01_orders WHERE state='queued'")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    let r = order(&f.state.pool, &id).await.unwrap();
    assert_eq!(r.actor_id, None);
    assert_eq!(r.source_id, old);
    assert!(
        run(
            &f,
            &id,
            &FakeIssuer {
                calls: AtomicUsize::new(0),
                fail: true
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        SqliteCertificateRepository::new(f.state.pool.clone())
            .find_active_by_site_id(f.site)
            .await
            .unwrap()
            .unwrap()
            .id,
        old.unwrap()
    );
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dns01_orders")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    schedule(&f.state).await;
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dns01_orders")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        call(
            &f,
            "POST",
            &format!("/api/v1/sites/{}/ssl/renew", f.site),
            Some(0),
            true,
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // Disabling SSL must not resurrect an old DNS binding even if legacy inventory still marks it active.
    sqlx::query("UPDATE sites SET settings_json='{}' WHERE id=?")
        .bind(f.site)
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE dns01_bindings SET last_attempt=0")
        .execute(&f.state.pool)
        .await
        .unwrap();
    schedule(&f.state).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dns01_orders")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(count, after);
}
#[tokio::test]
async fn dns01_stale_snapshot_and_revoked_actor_do_not_issue() {
    for revoke in [true, false] {
        let f = fixture().await;
        let id = queued(&f, false).await;
        if revoke {
            sqlx::query("UPDATE users SET status='suspended' WHERE id=?")
                .bind(f.actor)
                .execute(&f.state.pool)
                .await
                .unwrap();
        } else {
            sqlx::query("UPDATE domains SET name='other.example.com' WHERE site_id=?")
                .bind(f.site)
                .execute(&f.state.pool)
                .await
                .unwrap();
        }
        let fake = FakeIssuer::new();
        assert!(run(&f, &id, &fake).await.is_err());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
        assert!(f.configs.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn dns01_commit_failure_rolls_back_origin_and_retry_uses_same_candidate() {
    let f = fixture().await;
    let id = queued(&f, false).await;
    sqlx::query("CREATE TRIGGER dns_fail BEFORE UPDATE OF status ON certificates WHEN NEW.status='active' BEGIN SELECT RAISE(ABORT,'fixture commit failure'); END").execute(&f.state.pool).await.unwrap();
    let fake = FakeIssuer::new();
    assert!(run(&f, &id, &fake).await.is_err());
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    let r = order(&f.state.pool, &id).await.unwrap();
    assert_eq!(r.state, "candidate");
    assert!(!r.private_key_pem.is_empty());
    assert!(
        !f.configs
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("ssl_certificate")
    );
    sqlx::query("DROP TRIGGER dns_fail")
        .execute(&f.state.pool)
        .await
        .unwrap();
    f.state
        .job_runner
        .job_repo
        .update_status(&r.job_id, "failed", None, Some("fixture"))
        .await
        .unwrap();
    assert_eq!(
        call(
            &f,
            "POST",
            &format!("/api/v1/network/dns01/{id}/retry"),
            Some(0),
            true,
            Value::Null
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let fake = FakeIssuer {
        calls: AtomicUsize::new(0),
        fail: true,
    };
    run(&f, &id, &fake).await.unwrap();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert_eq!(order(&f.state.pool, &id).await.unwrap().state, "active");
}
#[tokio::test]
async fn dns01_failed_apply_probe_or_ack_restores_previous_origin() {
    for mode in ["lost_ack", "reload", "probe"] {
        let f = fixture().await;
        let id = queued(&f, false).await;
        match mode {
            "lost_ack" => f.lost_ack.store(true, Ordering::SeqCst),
            "reload" => f.fail_apply.store(true, Ordering::SeqCst),
            _ => f.provider.lock().unwrap()["fail_probe"] = json!(true),
        }
        assert!(run(&f, &id, &FakeIssuer::new()).await.is_err());
        assert_eq!(order(&f.state.pool, &id).await.unwrap().state, "candidate");
        assert!(
            !f.configs
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains("ssl_certificate")
        );
    }
}
#[tokio::test]
async fn dns01_outbox_lease_cleanup_and_candidate_discard_are_persistent() {
    let f = fixture().await;
    let id = queued(&f, false).await;
    let r = order(&f.state.pool, &id).await.unwrap();
    sqlx::query("DELETE FROM jobs WHERE id=?")
        .bind(&r.job_id)
        .execute(&f.state.job_runner.pool)
        .await
        .unwrap();
    schedule(&f.state).await;
    assert!(
        f.state
            .job_runner
            .job_repo
            .find_by_id(&r.job_id)
            .await
            .unwrap()
            .is_some()
    );
    let lease = locked(&f.state.pool, 60).await.unwrap();
    let fake = FakeIssuer::new();
    assert!(run(&f, &id, &fake).await.is_err());
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    release(&f.state.pool, &lease).await;
    f.fail_apply.store(true, Ordering::SeqCst);
    assert!(run(&f, &id, &FakeIssuer::new()).await.is_err());
    f.state
        .job_runner
        .job_repo
        .update_status(&r.job_id, "failed", None, None)
        .await
        .unwrap();
    assert_eq!(
        call(
            &f,
            "POST",
            &format!("/api/v1/network/dns01/{id}/discard"),
            Some(0),
            true,
            json!({"hostname":"wrong"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &f,
            "POST",
            &format!("/api/v1/network/dns01/{id}/discard"),
            Some(0),
            true,
            json!({"hostname":"app.example.com"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(order(&f.state.pool, &id).await.unwrap().state, "failed");
    assert!(
        order(&f.state.pool, &id)
            .await
            .unwrap()
            .private_key_pem
            .is_empty()
    );
}
#[test]
fn dns01_txt_propagation_requires_exact_name_digest_and_no_cname() {
    let name = "_acme-challenge.app.example.com";
    let digest = "A".repeat(43);
    let good = json!({"Status":0,"Answer":[{"name":format!("{name}."),"type":16,"data":format!("\"{digest}\"")} ]});
    assert!(txt_matches(&good, name, &digest));
    assert!(!txt_matches(&good, name, "other"));
    assert!(!txt_matches(&good, "other.example.com", &digest));
    let mut cname = good;
    cname["Answer"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":5}));
    assert!(!txt_matches(&cname, name, &digest));
    assert!(!txt_matches(&json!({"Status":2}), name, &digest));
}

#[tokio::test]
async fn dns01_fresh_cleanup_is_blocked_and_configuration_cannot_replace_pending_order() {
    let f = fixture().await;
    let id = queued(&f, true).await;
    f.provider.lock().unwrap()["pending"] = json!([{"id":"a416cf3a-e13b-45b0-a905-b3312bd12a99","hostname":"_acme-challenge.app.example.com","created_at":now()}]);
    let (status, _) = call(
        &f,
        "POST",
        "/api/v1/network/dns01/cleanup",
        Some(0),
        true,
        json!({"id":"a416cf3a-e13b-45b0-a905-b3312bd12a99"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        call(
            &f,
            "DELETE",
            "/api/v1/network/dns01",
            Some(0),
            true,
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(order(&f.state.pool, &id).await.unwrap().state, "queued");
}

#[tokio::test]
async fn dns01_propagation_waits_for_both_resolvers_and_bounds_missing_or_hanging_answers() {
    struct Resolver(u8);
    impl TxtResolver for Resolver {
        fn resolve<'a>(
            &'a self,
            url: &'a str,
            name: &'a str,
        ) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async move {
                if self.0 == 3 {
                    return std::future::pending().await;
                }
                if self.0 == 1 || (self.0 == 2 && url.contains("google")) {
                    return Err("fixture resolver unavailable".into());
                }
                Ok(
                    json!({"Status":0,"Answer":[{"name":name,"type":16,"data":format!("\"{}\"","A".repeat(43))}]}),
                )
            })
        }
    }
    for mode in 0..4 {
        let result = propagation_with(
            "app.example.com",
            &"A".repeat(43),
            &Resolver(mode),
            Duration::from_millis(15),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(result.is_ok(), mode == 0);
    }
}
