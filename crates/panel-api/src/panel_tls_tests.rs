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
    runner: Arc<panel_agent::system::MockSystemRunner>,
    lost_ack: Arc<AtomicBool>,
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
    runner.set_response(
        "nginx -T",
        0,
        b"# configuration file /etc/nginx/nginx.conf:\nevents {} http {server {listen 17890;}}\n",
        b"",
    );
    let server = runner.clone();
    let lost_ack = Arc::new(AtomicBool::new(false));
    let lost = lost_ack.clone();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let mut rpc = panel_proto::RpcStream::new(stream);
            let req = rpc.recv_request().await.unwrap();
            if let panel_proto::RequestPayload::PanelTls(command) = req.payload {
                let activation = matches!(&command, PanelTlsRequest::Activate { .. });
                let result = panel_agent::panel_tls::execute(server.as_ref(), command);
                if activation && lost.swap(false, Ordering::SeqCst) {
                    continue;
                }
                let response = match result {
                    Ok(value) => panel_proto::RpcResponse::ok(
                        req.request_id,
                        panel_proto::ResponsePayload::PanelTls(value),
                    ),
                    Err(e) => panel_proto::RpcResponse::err(req.request_id, e.to_string()),
                };
                rpc.send_response(&response).await.unwrap();
            } else {
                panic!("Unexpected RPC");
            }
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
        runner,
        lost_ack,
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
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn endpoint(f: &Fixture) -> String {
    let (status,body)=call(f,"POST","/api/v1/connectivity",Some(0),true,json!({"service":"panel","scope":{"kind":"host"},"transport":"direct","host":"1.1.1.1","port":19443,"tls_mode":"tls","certificate_id":null,"enabled":true,"primary":true})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().into()
}
async fn enqueue(f: &Fixture, staging: bool) -> String {
    let id = endpoint(f).await;
    let (status,body)=call(f,"POST","/api/v1/connectivity/panel-tls/issue",Some(0),true,json!({"endpoint_id":id,"expected_revision":1,"email":"ops@example.test","staging":staging,"http01_confirmed":true})).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body["certificate_id"].as_str().unwrap().into()
}
struct FakeIssuer {
    fail: bool,
    calls: AtomicUsize,
}
impl FakeIssuer {
    fn new() -> Self {
        Self {
            fail: false,
            calls: AtomicUsize::new(0),
        }
    }
    fn failing() -> Self {
        Self {
            fail: true,
            calls: AtomicUsize::new(0),
        }
    }
}
impl Issuer for FakeIssuer {
    fn issue<'a>(&'a self, r: &'a Record) -> BoxFuture<'a, Result<Issued, String>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err("Fixture: CA unavailable".into());
            }
            let mut params = rcgen::CertificateParams::default();
            params.subject_alt_names = vec![rcgen::SanType::IpAddress(r.ip.parse().unwrap())];
            params.not_before = time::OffsetDateTime::from_unix_timestamp(now() - 60).unwrap();
            params.not_after =
                time::OffsetDateTime::from_unix_timestamp(now() + 160 * 3600).unwrap();
            let key = rcgen::KeyPair::generate().unwrap();
            Ok(Issued {
                chain: params.self_signed(&key).unwrap().pem(),
                key: key.serialize_pem(),
                renew_at: now() + 100 * 3600,
            })
        })
    }
}
#[tokio::test]
async fn api_requires_admin_csrf_registered_ip_and_partitioned_persistent_job() {
    let f = fixture().await;
    let id = endpoint(&f).await;
    let payload = json!({"endpoint_id":id,"expected_revision":1,"email":"ops@example.test","staging":true,"http01_confirmed":true});
    for (actor, csrf, status) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some(1), true, StatusCode::FORBIDDEN),
        (Some(0), false, StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            call(
                &f,
                "POST",
                "/api/v1/connectivity/panel-tls/issue",
                actor,
                csrf,
                payload.clone()
            )
            .await
            .0,
            status
        );
    }
    for bad in [
        json!({"http01_confirmed":false}),
        json!({"expected_revision":0}),
        json!({"email":"bad\n@example.test"}),
        json!({"url":"https://untrusted.test"}),
    ] {
        let mut body = payload.clone();
        for (key, value) in bad.as_object().unwrap() {
            body[key] = value.clone();
        }
        assert_eq!(
            call(
                &f,
                "POST",
                "/api/v1/connectivity/panel-tls/issue",
                Some(0),
                true,
                body
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let (status, accepted) = call(
        &f,
        "POST",
        "/api/v1/connectivity/panel-tls/issue",
        Some(0),
        true,
        payload.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/connectivity/panel-tls/issue",
            Some(0),
            true,
            payload
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let job = f
        .state
        .job_runner
        .job_repo
        .find_by_id(accepted["job_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.name, "panel.tls.issue");
    assert_eq!(job.max_retries, 0);
    assert!(!job.payload_json.contains("ops@"));
    let core_jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(core_jobs, 0);
    let (_, body) = call(
        &f,
        "GET",
        "/api/v1/connectivity/panel-tls",
        Some(0),
        false,
        Value::Null,
    )
    .await;
    assert_eq!(body["certificates"][0]["scope"], "panel");
    assert!(!body.to_string().contains("private_key"));
    assert!(!body.to_string().contains("ops@"));
    assert_eq!(
        call(
            &f,
            "GET",
            "/api/v1/connectivity/panel-tls",
            Some(1),
            false,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(f.runner.get_commands().is_empty());
}
#[tokio::test]
async fn staging_is_not_installed_and_order_requests_ip_profile() {
    let f = fixture().await;
    let id = enqueue(&f, true).await;
    process(
        &f.state.pool,
        &f.state.agent_client,
        &id,
        Some(f.actor),
        &FakeIssuer::new(),
    )
    .await
    .unwrap();
    let row = record(&f.state.pool, &id).await.unwrap();
    assert_eq!(row.state, "staging");
    assert!(row.private_key_pem.is_empty());
    assert!(active(&f.state.pool).await.unwrap().is_none());
    assert!(f.runner.get_commands().is_empty());
    for ip in ["1.1.1.1", "2606:4700:4700::1111"] {
        let ids = [instant_acme::Identifier::Ip(ip.parse().unwrap())];
        let value = serde_json::to_value(ip_order(&ids)).unwrap();
        assert_eq!(value["profile"], "shortlived");
        assert_eq!(value["identifiers"][0], json!({"type":"ip","value":ip}));
        use x509_parser::prelude::FromDer;
        let (der, _) = ip_csr(ip.parse().unwrap()).unwrap();
        let (_, csr) =
            x509_parser::certification_request::X509CertificationRequest::from_der(&der).unwrap();
        assert!(
            csr.certification_request_info
                .subject
                .iter_common_name()
                .next()
                .is_none()
        );
        assert!(csr.requested_extensions().unwrap().any(|extension|matches!(extension,x509_parser::extensions::ParsedExtension::SubjectAlternativeName(san) if san.general_names.iter().any(|name|matches!(name,x509_parser::extensions::GeneralName::IPAddress(_))))));
    }
}
#[tokio::test]
async fn failed_renewal_preserves_valid_active_then_success_retains_identity_snapshot() {
    let f = fixture().await;
    let id = enqueue(&f, false).await;
    process(
        &f.state.pool,
        &f.state.agent_client,
        &id,
        Some(f.actor),
        &FakeIssuer::new(),
    )
    .await
    .unwrap();
    let old = active(&f.state.pool).await.unwrap().unwrap();
    let old_config = panel_agent::system::SystemRunner::read_file(
        f.runner.as_ref(),
        std::path::Path::new("/etc/nginx/conf.d/10-qpanel-panel-tls.conf"),
    )
    .unwrap();
    let (_, result) = call(
        &f,
        "POST",
        "/api/v1/connectivity/panel-tls/renew",
        Some(0),
        true,
        Value::Null,
    )
    .await;
    let failed = result["certificate_id"].as_str().unwrap();
    assert!(
        process(
            &f.state.pool,
            &f.state.agent_client,
            failed,
            Some(f.actor),
            &FakeIssuer::failing()
        )
        .await
        .is_err()
    );
    assert_eq!(
        active(&f.state.pool).await.unwrap().unwrap().fingerprint,
        old.fingerprint
    );
    assert_eq!(record(&f.state.pool, failed).await.unwrap().state, "failed");
    assert_eq!(
        panel_agent::system::SystemRunner::read_file(
            f.runner.as_ref(),
            std::path::Path::new("/etc/nginx/conf.d/10-qpanel-panel-tls.conf")
        )
        .unwrap(),
        old_config
    );
    // Metadata edits cannot stop renewal of the already deployed panel identity.
    sqlx::query("UPDATE service_endpoints SET revision=revision+1,host='8.8.8.8' WHERE id=?")
        .bind(&old.endpoint_id)
        .execute(&f.state.pool)
        .await
        .unwrap();
    let r = new_record(
        old.endpoint_id,
        old.endpoint_revision,
        old.ip,
        old.port,
        old.email,
        false,
        Some(id.clone()),
    );
    assert!(queue(&f.state, None, &r).await.is_ok());
    process(
        &f.state.pool,
        &f.state.agent_client,
        &r.id,
        None,
        &FakeIssuer::new(),
    )
    .await
    .unwrap();
    assert_eq!(active(&f.state.pool).await.unwrap().unwrap().id, r.id);
    assert_eq!(record(&f.state.pool, &id).await.unwrap().state, "retired");
    assert!(
        record(&f.state.pool, &id)
            .await
            .unwrap()
            .private_key_pem
            .is_empty()
    );
    let sites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM certificates")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(sites, 0);
}
#[tokio::test]
async fn stale_endpoint_revoked_actor_and_existing_lease_do_not_issue() {
    let f = fixture().await;
    let id = enqueue(&f, false).await;
    let issuer = FakeIssuer::new();
    sqlx::query("UPDATE panel_tls_lock SET expires_at=? WHERE id=1")
        .bind(now() + 1200)
        .execute(&f.state.pool)
        .await
        .unwrap();
    assert!(
        process(
            &f.state.pool,
            &f.state.agent_client,
            &id,
            Some(f.actor),
            &issuer
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE panel_tls_lock SET expires_at=0 WHERE id=1")
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE service_endpoints SET revision=revision+1")
        .execute(&f.state.pool)
        .await
        .unwrap();
    assert!(
        process(
            &f.state.pool,
            &f.state.agent_client,
            &id,
            Some(f.actor),
            &issuer
        )
        .await
        .is_err()
    );
    assert_eq!(issuer.calls.load(Ordering::SeqCst), 0);
    assert!(f.runner.get_commands().is_empty());
    sqlx::query("UPDATE service_endpoints SET revision=revision-1")
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE panel_tls_certificates SET state='queued' WHERE id=?")
        .bind(&id)
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='suspended' WHERE id=?")
        .bind(f.actor)
        .execute(&f.state.pool)
        .await
        .unwrap();
    assert!(
        process(
            &f.state.pool,
            &f.state.agent_client,
            &id,
            Some(f.actor),
            &issuer
        )
        .await
        .is_err()
    );
    assert_eq!(issuer.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn lost_agent_acknowledgement_is_reconciled_without_second_issuance() {
    let f = fixture().await;
    let id = enqueue(&f, false).await;
    let issuer = FakeIssuer::new();
    f.lost_ack.store(true, Ordering::SeqCst);
    process(
        &f.state.pool,
        &f.state.agent_client,
        &id,
        Some(f.actor),
        &issuer,
    )
    .await
    .unwrap();
    assert_eq!(active(&f.state.pool).await.unwrap().unwrap().id, id);
    assert!(
        active(&f.state.pool)
            .await
            .unwrap()
            .unwrap()
            .private_key_pem
            .is_empty()
    );
    assert_eq!(issuer.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn crash_after_agent_activation_retains_candidate_and_retries_same_certificate() {
    let f = fixture().await;
    let id = enqueue(&f, false).await;
    sqlx::query("CREATE TRIGGER fail_panel_tls_commit BEFORE UPDATE OF state ON panel_tls_certificates WHEN NEW.state='active' BEGIN SELECT RAISE(ABORT,'fixture commit failure'); END").execute(&f.state.pool).await.unwrap();
    assert!(
        process(
            &f.state.pool,
            &f.state.agent_client,
            &id,
            Some(f.actor),
            &FakeIssuer::new()
        )
        .await
        .is_err()
    );
    assert_eq!(record(&f.state.pool, &id).await.unwrap().state, "candidate");
    let pending_job = record(&f.state.pool, &id).await.unwrap().job_id.unwrap();
    sqlx::query("UPDATE jobs SET status='failed' WHERE id=?")
        .bind(pending_job)
        .execute(&f.state.operations_pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &f,
            "POST",
            &format!("/api/v1/connectivity/panel-tls/{id}/discard"),
            Some(0),
            true,
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(
        !record(&f.state.pool, &id)
            .await
            .unwrap()
            .private_key_pem
            .is_empty()
    );
    sqlx::query("DROP TRIGGER fail_panel_tls_commit")
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE panel_tls_certificates SET last_attempt=? WHERE id=?")
        .bind(now() - 61)
        .bind(&id)
        .execute(&f.state.pool)
        .await
        .unwrap();
    let (status, accepted) = call(
        &f,
        "POST",
        &format!("/api/v1/connectivity/panel-tls/{id}/retry"),
        Some(0),
        true,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(accepted["job_id"].as_str().is_some());
    let failing = FakeIssuer::failing();
    process(
        &f.state.pool,
        &f.state.agent_client,
        &id,
        Some(f.actor),
        &failing,
    )
    .await
    .unwrap();
    assert_eq!(failing.calls.load(Ordering::SeqCst), 0);
    assert_eq!(active(&f.state.pool).await.unwrap().unwrap().id, id);
}
#[tokio::test]
async fn outbox_recovers_missing_operational_job_and_running_tasks_cannot_cancel() {
    let f = fixture().await;
    let id = enqueue(&f, true).await;
    let row = record(&f.state.pool, &id).await.unwrap();
    let job = row.job_id.unwrap();
    sqlx::query("DELETE FROM jobs WHERE id=?")
        .bind(&job)
        .execute(&f.state.operations_pool)
        .await
        .unwrap();
    repair_outbox(&f.state).await;
    assert!(
        f.state
            .job_runner
            .job_repo
            .find_by_id(&job)
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("UPDATE jobs SET status='running' WHERE id=?")
        .bind(&job)
        .execute(&f.state.operations_pool)
        .await
        .unwrap();
    assert!(f.state.job_runner.cancel(&job).await.is_err());
    sqlx::query("UPDATE jobs SET status='queued' WHERE id=?")
        .bind(&job)
        .execute(&f.state.operations_pool)
        .await
        .unwrap();
    f.state.job_runner.cancel(&job).await.unwrap();
    repair_outbox(&f.state).await;
    assert_eq!(record(&f.state.pool, &id).await.unwrap().state, "failed");
}
#[tokio::test]
async fn automatic_renewal_is_due_at_lifetime_deadline_and_cooldown_is_persistent() {
    let f = fixture().await;
    let id = enqueue(&f, false).await;
    process(
        &f.state.pool,
        &f.state.agent_client,
        &id,
        Some(f.actor),
        &FakeIssuer::new(),
    )
    .await
    .unwrap();
    schedule(&f.state).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM panel_tls_certificates")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    sqlx::query("UPDATE panel_tls_certificates SET renew_at=?,ari_checked_at=? WHERE id=?")
        .bind(now() - 1)
        .bind(now())
        .bind(&id)
        .execute(&f.state.pool)
        .await
        .unwrap();
    schedule(&f.state).await;
    schedule(&f.state).await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM panel_tls_certificates WHERE source_id=?")
            .bind(&id)
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn uninstalled_candidate_can_be_discarded_without_deleting_active_certificate() {
    let f = fixture().await;
    let old_id = enqueue(&f, false).await;
    process(
        &f.state.pool,
        &f.state.agent_client,
        &old_id,
        Some(f.actor),
        &FakeIssuer::new(),
    )
    .await
    .unwrap();
    let (status, result) = call(
        &f,
        "POST",
        "/api/v1/connectivity/panel-tls/renew",
        Some(0),
        true,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = result["certificate_id"].as_str().unwrap();
    f.runner
        .set_response("nginx -t", 1, b"", b"fixture failure");
    assert!(
        process(
            &f.state.pool,
            &f.state.agent_client,
            id,
            Some(f.actor),
            &FakeIssuer::new()
        )
        .await
        .is_err()
    );
    let pending = record(&f.state.pool, id).await.unwrap();
    assert_eq!(pending.state, "candidate");
    sqlx::query("UPDATE jobs SET status='failed' WHERE id=?")
        .bind(pending.job_id)
        .execute(&f.state.operations_pool)
        .await
        .unwrap();
    let route = format!("/api/v1/connectivity/panel-tls/{id}/discard");
    assert_eq!(
        call(&f, "POST", &route, Some(1), true, Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&f, "POST", &route, Some(0), false, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&f, "POST", &route, Some(0), true, Value::Null).await.0,
        StatusCode::OK
    );
    assert!(
        record(&f.state.pool, id)
            .await
            .unwrap()
            .private_key_pem
            .is_empty()
    );
    assert_eq!(active(&f.state.pool).await.unwrap().unwrap().id, old_id);
}

#[tokio::test]
async fn public_ip_validation_is_not_inferred_from_dns_or_private_endpoints_and_ipv6_is_supported()
{
    let f = fixture().await;
    let endpoint = endpoint(&f).await;
    for host in [
        "10.0.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "panel.example.test",
        "fd00::1",
        "2001:db8::1",
    ] {
        sqlx::query("UPDATE service_endpoints SET host=? WHERE id=?")
            .bind(host)
            .bind(&endpoint)
            .execute(&f.state.pool)
            .await
            .unwrap();
        let (status,_)=call(&f,"POST","/api/v1/connectivity/panel-tls/issue",Some(0),true,json!({"endpoint_id":endpoint,"expected_revision":1,"email":"ops@example.test","staging":false,"http01_confirmed":true})).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{host}");
    }
    sqlx::query("UPDATE service_endpoints SET host='2606:4700:4700::1111' WHERE id=?")
        .bind(&endpoint)
        .execute(&f.state.pool)
        .await
        .unwrap();
    let (status,result)=call(&f,"POST","/api/v1/connectivity/panel-tls/issue",Some(0),true,json!({"endpoint_id":endpoint,"expected_revision":1,"email":"ops@example.test","staging":false,"http01_confirmed":true})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    process(
        &f.state.pool,
        &f.state.agent_client,
        result["certificate_id"].as_str().unwrap(),
        Some(f.actor),
        &FakeIssuer::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        active(&f.state.pool).await.unwrap().unwrap().ip,
        "2606:4700:4700::1111"
    );
}
