//! Durable, admin-authorized DNS-01 issuance for the separate storage endpoint.
use super::{Error, LiveRuntime, Services, bad, db_error, handlers, ok};
use crate::{
    agent_client::AgentClient,
    auth::{RequireAdmin, RequireAdminCsrf},
    routes::AppState,
};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use panel_db::{DbPool, models::Job};
use panel_proto::{ManagedStorageRequest, StorageSecret};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use utoipa::ToSchema;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
#[derive(Clone, sqlx::FromRow)]
struct Binding {
    revision: i64,
    request_id: Option<String>,
    job_id: Option<String>,
    actor_id: Option<i64>,
    hostname: Option<String>,
    email: Option<String>,
    zone_id: Option<String>,
    zone_name: Option<String>,
    credential_revision: Option<String>,
    state: String,
    auto_renew: bool,
    not_after: Option<i64>,
    renew_at: Option<i64>,
    last_attempt: i64,
    updated_at: i64,
    error: Option<String>,
}
async fn binding(pool: &DbPool) -> Result<Binding, Error> {
    sqlx::query_as("SELECT * FROM managed_storage_tls WHERE id=1")
        .fetch_one(pool)
        .await
        .map_err(db_error)
}
fn pending(value: &Binding) -> bool {
    matches!(value.state.as_str(), "queued" | "issuing" | "configuring")
}
fn public(value: &Binding) -> Value {
    json!({"revision":value.revision,"request_id":value.request_id,"hostname":value.hostname,"email":value.email,
        "state":value.state,"auto_renew":value.auto_renew,"not_after":value.not_after,"renew_at":value.renew_at,
        "job_id":value.job_id,"error":value.error})
}
fn provider(value: &Value, hostname: &str) -> Result<(String, String, String), Error> {
    let zone = value["zone_name"].as_str().unwrap_or("");
    let revision = value["revision"].as_str().unwrap_or("");
    let zone_id = value["zone_id"].as_str().unwrap_or("");
    if value["configured"] != true || zone.is_empty() || revision.is_empty() || zone_id.is_empty() {
        return Err(bad(
            StatusCode::CONFLICT,
            "Atur DNS-01 Cloudflare di Pengaturan server terlebih dahulu.",
        ));
    }
    if hostname != zone && !hostname.ends_with(&format!(".{zone}")) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Domain storage harus berada dalam zona DNS-01 yang dikonfigurasi.",
        ));
    }
    if value["pending"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        return Err(bad(
            StatusCode::CONFLICT,
            "Selesaikan cleanup challenge DNS-01 sebelum menerbitkan sertifikat baru.",
        ));
    }
    Ok((zone_id.into(), zone.into(), revision.into()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Generate {
    pub request_id: uuid::Uuid,
    pub expected_revision: i64,
    pub hostname: String,
    pub email: String,
    pub confirmed: bool,
}
#[utoipa::path(get, path="/api/v1/storage/managed/tls", responses((status=200,body=Value)), tag="storage")]
pub async fn overview(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    _admin: RequireAdmin,
) -> Result<Response, Error> {
    let row = binding(&state.pool).await?;
    let dns = services.runtime.dns_status().await.unwrap_or(Value::Null);
    Ok(ok(
        json!({"certificate":public(&row),"dns":{"configured":dns["configured"]==true,"zone_name":dns["zone_name"]}}),
    ))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    request_id: String,
    revision: i64,
}
async fn dispatch(state: &AppState, row: &Binding) -> Result<Job, Error> {
    let payload = Payload {
        request_id: row.request_id.clone().ok_or_else(|| db_error(()))?,
        revision: row.revision,
    };
    state
        .job_runner
        .enqueue(
            row.actor_id,
            row.job_id.clone(),
            "storage.tls.issue".into(),
            serde_json::to_string(&payload).map_err(db_error)?,
            Some(0),
        )
        .await
        .map(|(job, _)| job)
        .map_err(db_error)
}
fn validate(value: &Generate) -> Result<(), Error> {
    let host = panel_core::DomainName::parse(&value.hostname).map_err(|_| {
        bad(
            StatusCode::BAD_REQUEST,
            "Gunakan domain storage yang valid.",
        )
    })?;
    if value.request_id.is_nil()
        || value.expected_revision < 1
        || !value.confirmed
        || host.as_ascii() != value.hostname
        || value.hostname.parse::<std::net::IpAddr>().is_ok()
        || value.email.len() > 254
        || value.email.matches('@').count() != 1
        || value.email.starts_with('@')
        || value.email.ends_with('@')
        || value.email.bytes().any(|c| !c.is_ascii_graphic())
    {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Gunakan domain dan email yang valid, lalu setujui penerbitan dan perpanjangan Let's Encrypt.",
        ));
    }
    Ok(())
}
#[utoipa::path(post, path="/api/v1/storage/managed/tls", request_body=Generate, responses((status=202,body=Value)), tag="storage")]
pub async fn generate(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    headers: HeaderMap,
    input: Result<Json<Generate>, JsonRejection>,
) -> Result<Response, Error> {
    let value = handlers::input(input)?;
    validate(&value)?;
    if value.hostname == handlers::panel(&state, &headers).await?.host.as_str() {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Gunakan domain storage terpisah dari domain panel.",
        ));
    }
    let old = binding(&state.pool).await?;
    if old.request_id.as_deref() == Some(value.request_id.to_string().as_str()) {
        if old.hostname.as_deref() != Some(value.hostname.as_str())
            || old.email.as_deref() != Some(value.email.as_str())
        {
            return Err(bad(
                StatusCode::CONFLICT,
                "Identitas permintaan sudah digunakan untuk sertifikat berbeda.",
            ));
        }
        return Ok((StatusCode::ACCEPTED, ok(public(&old))).into_response());
    }
    if pending(&old) || old.revision != value.expected_revision {
        return Err(bad(
            StatusCode::CONFLICT,
            "Pengaturan sertifikat berubah atau operasi masih berjalan. Muat ulang status.",
        ));
    }
    if old.last_attempt > now() - 60 {
        return Err(bad(
            StatusCode::TOO_MANY_REQUESTS,
            "Tunggu satu menit sebelum mencoba penerbitan lagi.",
        ));
    }
    let dns = services
        .runtime
        .dns_status()
        .await
        .map_err(super::failure)?;
    let (zone_id, zone, credential_revision) = provider(&dns, &value.hostname)?;
    let service = services
        .runtime
        .managed(ManagedStorageRequest::Status {})
        .await
        .map_err(super::failure)?;
    if service["configured"] == true && service["endpoint"] != format!("https://{}", value.hostname)
    {
        return Err(bad(
            StatusCode::CONFLICT,
            "Gunakan domain layanan storage yang sudah dikonfigurasi.",
        ));
    }
    let changed = sqlx::query("UPDATE managed_storage_tls SET revision=revision+1,request_id=?,job_id=?,actor_id=?,hostname=?,email=?,zone_id=?,zone_name=?,credential_revision=?,state='queued',auto_renew=1,last_attempt=?,updated_at=?,error=NULL WHERE id=1 AND revision=? AND state NOT IN ('queued','issuing','configuring')")
        .bind(value.request_id.to_string()).bind(uuid::Uuid::new_v4().to_string()).bind(actor.id).bind(&value.hostname).bind(&value.email)
        .bind(zone_id).bind(zone).bind(credential_revision).bind(now()).bind(now()).bind(value.expected_revision)
        .execute(&state.pool).await.map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Pengaturan sertifikat berubah. Muat ulang status.",
        ));
    }
    let row = binding(&state.pool).await?;
    dispatch(&state, &row).await?;
    state.audit_writer.log_async(
        Some(actor.id),
        actor.username,
        "storage.tls.generate",
        "storage",
        "success",
        None,
        Some(json!({"hostname":value.hostname}).to_string()),
    );
    Ok((StatusCode::ACCEPTED, ok(public(&row))).into_response())
}

/// Reserve before any manual engine mutation, preventing a queued job from overwriting custom TLS.
pub(super) async fn reserve_manual(pool: &DbPool) -> Result<i64, Error> {
    let row = sqlx::query_scalar("UPDATE managed_storage_tls SET revision=revision+1,state='configuring',updated_at=?,error=NULL WHERE id=1 AND state NOT IN ('queued','issuing','configuring') RETURNING revision")
        .bind(now()).fetch_optional(pool).await.map_err(db_error)?;
    row.ok_or_else(|| {
        bad(
            StatusCode::CONFLICT,
            "Penerbitan atau pemasangan sertifikat storage masih berjalan.",
        )
    })
}
pub(super) async fn finish_manual(
    pool: &DbPool,
    revision: i64,
    hostname: &str,
    success: bool,
) -> Result<(), Error> {
    sqlx::query("UPDATE managed_storage_tls SET state=?,auto_renew=0,hostname=?,not_before=NULL,not_after=NULL,renew_at=NULL,request_id=NULL,job_id=NULL,updated_at=?,error=? WHERE id=1 AND revision=? AND state='configuring'")
        .bind(if success {"manual"} else {"failed"}).bind(hostname).bind(now())
        .bind(if success {None} else {Some("Pemasangan manual gagal; periksa layanan dan sertifikat sebelum mencoba lagi.")}).bind(revision)
        .execute(pool).await.map_err(db_error)?;
    Ok(())
}

pub struct Handler(pub Arc<AgentClient>);
impl crate::jobs::registry::JobHandler for Handler {
    fn run<'a>(
        &'a self,
        job: &'a Job,
        ctx: &'a crate::jobs::registry::JobContext,
    ) -> crate::jobs::registry::JobFuture<'a> {
        Box::pin(async move {
            let payload: Payload = serde_json::from_str(&job.payload_json)
                .map_err(|_| "Payload TLS storage tidak valid.")?;
            let services = Services {
                runtime: Arc::new(LiveRuntime {
                    agent: Arc::clone(&self.0),
                    network: tokio::sync::Semaphore::new(1),
                }),
                transfers: Arc::new(tokio::sync::Semaphore::new(1)),
            };
            process(&ctx.pool, &services, job, &payload).await?;
            Ok(Some(json!({"request_id":payload.request_id}).to_string()))
        })
    }
}
async fn process(
    pool: &DbPool,
    services: &Services,
    job: &Job,
    payload: &Payload,
) -> Result<(), String> {
    let row = binding(pool)
        .await
        .map_err(|_| "Status sertifikat gagal dimuat.")?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin' AND status='active')",
    )
    .bind(row.actor_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "Identitas penerbitan gagal diperiksa.")?;
    if !active
        || job.user_id != row.actor_id
        || row.job_id.as_deref() != Some(job.id.as_str())
        || row.request_id.as_deref() != Some(payload.request_id.as_str())
        || row.revision != payload.revision
    {
        return Err("Identitas atau revisi tugas sertifikat storage sudah tidak sesuai.".into());
    }
    let claimed=sqlx::query("UPDATE managed_storage_tls SET state='issuing',updated_at=? WHERE id=1 AND revision=? AND state='queued'").bind(now()).bind(row.revision).execute(pool).await.map_err(|_|"Tugas sertifikat gagal diklaim.")?;
    if claimed.rows_affected() != 1 {
        return Err("Tugas sertifikat tidak boleh diputar ulang otomatis.".into());
    }
    let result = issue_and_apply(pool, services, &row).await;
    if result.is_err() {
        // Provider/PEM/transport details never enter durable job or audit logs.
        let _=sqlx::query("UPDATE managed_storage_tls SET state='failed',auto_renew=0,updated_at=?,error='Penerbitan atau pemasangan gagal; perpanjangan otomatis dijeda. Periksa DNS-01 dan layanan storage, lalu pilih Generate lagi. Periksa rollback bila konfigurasi belum sehat.' WHERE id=1 AND revision=? AND state='issuing'").bind(now()).bind(row.revision).execute(pool).await;
    }
    result.map_err(|_|"Penerbitan sertifikat storage gagal; periksa status TLS dan DNS-01. Detail credential tidak dicatat.".into())
}
async fn issue_and_apply(pool: &DbPool, services: &Services, row: &Binding) -> Result<(), String> {
    let host = row.hostname.as_deref().ok_or("Domain tidak tersedia.")?;
    let email = row.email.as_deref().ok_or("Email tidak tersedia.")?;
    let revision = row
        .credential_revision
        .as_deref()
        .ok_or("Revisi DNS tidak tersedia.")?;
    let dns = services
        .runtime
        .dns_status()
        .await
        .map_err(|_| "DNS-01 tidak tersedia.")?;
    let (zone_id, zone, current) =
        provider(&dns, host).map_err(|_| "Konfigurasi DNS-01 berubah.")?;
    if Some(&zone_id) != row.zone_id.as_ref()
        || Some(&zone) != row.zone_name.as_ref()
        || current != revision
    {
        return Err("Zona atau token DNS-01 berubah.".into());
    }
    let certificate = services
        .runtime
        .certificate(pool.clone(), host.into(), email.into(), revision.into())
        .await?;
    let validity = panel_core::panel_tls::validate_dns_candidate(
        &certificate.chain,
        &certificate.key.0,
        &[host.into()],
        now(),
    )
    .map_err(str::to_owned)?;
    let current_row = binding(pool)
        .await
        .map_err(|_| "Revisi storage gagal dimuat.")?;
    if current_row.revision != row.revision || current_row.state != "issuing" {
        return Err("Permintaan storage sudah berubah.".into());
    }
    let result = services
        .runtime
        .managed(ManagedStorageRequest::Configure {
            hostname: host.into(),
            fullchain_pem: StorageSecret(certificate.chain),
            private_key_pem: certificate.key,
        })
        .await
        .map_err(|_| "Pemasangan sertifikat gagal.")?;
    if result["configured"] != true
        || result["running"] != true
        || result["endpoint"] != format!("https://{host}")
    {
        return Err("Listener HTTPS storage belum sehat.".into());
    }
    let changed=sqlx::query("UPDATE managed_storage_tls SET state='active',not_before=?,not_after=?,renew_at=?,updated_at=?,error=NULL WHERE id=1 AND revision=? AND state='issuing'")
        .bind(validity.not_before).bind(validity.not_after).bind(panel_core::panel_tls::renewal_at(validity.not_before,validity.not_after,None)).bind(now()).bind(row.revision).execute(pool).await.map_err(|_|"Metadata sertifikat gagal disimpan.")?;
    if changed.rows_affected() != 1 {
        return Err("Revisi sertifikat berubah setelah pemasangan.".into());
    }
    Ok(())
}

pub async fn schedule(state: &AppState) {
    let Ok(row) = binding(&state.pool).await else {
        return;
    };
    if pending(&row) {
        reconcile_job(state, &row).await;
        return;
    }
    if !row.auto_renew
        || row.last_attempt > now() - 3600
        || row.renew_at.is_some_and(|due| due > now())
    {
        return;
    }
    if row.actor_id.is_none() {
        return;
    }
    let changed=sqlx::query("UPDATE managed_storage_tls SET revision=revision+1,request_id=?,job_id=?,state='queued',last_attempt=?,updated_at=?,error=NULL WHERE id=1 AND revision=? AND auto_renew=1 AND state IN ('active','failed')")
        .bind(uuid::Uuid::new_v4().to_string()).bind(uuid::Uuid::new_v4().to_string()).bind(now()).bind(now()).bind(row.revision).execute(&state.pool).await;
    if changed.is_ok_and(|value| value.rows_affected() == 1)
        && let Ok(row) = binding(&state.pool).await
    {
        let _ = dispatch(state, &row).await;
    }
}
async fn reconcile_job(state: &AppState, row: &Binding) {
    if row.state == "configuring" {
        if row.updated_at < now() - 1800 {
            let _=sqlx::query("UPDATE managed_storage_tls SET state='failed',auto_renew=0,error='Operasi pemasangan terputus; periksa layanan dan muat ulang sebelum mencoba lagi.' WHERE id=1 AND revision=? AND state='configuring'").bind(row.revision).execute(&state.pool).await;
        }
        return;
    }
    let Some(id) = &row.job_id else { return };
    match state.job_runner.job_repo.find_by_id(id).await {
        Ok(None) if row.state == "queued" => {
            let _ = dispatch(state, row).await;
        }
        Ok(Some(job)) if matches!(job.status.as_str(), "succeeded" | "failed" | "cancelled") => {
            let _=sqlx::query("UPDATE managed_storage_tls SET state='failed',auto_renew=0,updated_at=?,error='Tugas penerbitan terputus atau dibatalkan; perpanjangan otomatis dijeda. Periksa DNS-01 dan layanan storage, lalu pilih Generate lagi.' WHERE id=1 AND revision=? AND state IN ('queued','issuing')").bind(now()).bind(row.revision).execute(&state.pool).await;
        }
        _ => {}
    }
}

use axum::response::IntoResponse;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{StorageRuntime, domain::*};
    use futures_util::future::BoxFuture;
    use panel_core::object_storage::identity::StorageEndpoint;
    use panel_proto::{S3Credentials, StorageSecretRequest};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Runtime {
        issued: AtomicUsize,
        applied: AtomicUsize,
        fail: AtomicBool,
        wrong_name: AtomicBool,
    }
    impl StorageRuntime for Runtime {
        fn dns_status(&self) -> BoxFuture<'_, Result<Value, Failure>> {
            Box::pin(async {
                Ok(
                    json!({"configured":true,"zone_id":"zone","zone_name":"example.test","revision":"revision","pending":[]}),
                )
            })
        }
        fn certificate(
            &self,
            _pool: DbPool,
            hostname: String,
            _email: String,
            _revision: String,
        ) -> BoxFuture<'_, Result<StorageCertificate, String>> {
            Box::pin(async move {
                self.issued.fetch_add(1, Ordering::SeqCst);
                if self.fail.load(Ordering::SeqCst) {
                    return Err("fixture_secret_must_not_persist".into());
                }
                let hostname = if self.wrong_name.load(Ordering::SeqCst) {
                    "other.example.test".into()
                } else {
                    hostname
                };
                let key = rcgen::KeyPair::generate().expect("key");
                let cert = rcgen::CertificateParams::new(vec![hostname])
                    .expect("params")
                    .self_signed(&key)
                    .expect("cert");
                Ok(StorageCertificate {
                    chain: cert.pem(),
                    key: StorageSecret(key.serialize_pem()),
                })
            })
        }
        fn managed(&self, request: ManagedStorageRequest) -> BoxFuture<'_, Result<Value, Failure>> {
            Box::pin(async move {
                assert!(matches!(request, ManagedStorageRequest::Configure { .. }));
                self.applied.fetch_add(1, Ordering::SeqCst);
                Ok(
                    json!({"configured":true,"running":true,"endpoint":"https://storage.example.test"}),
                )
            })
        }
        fn secret(
            &self,
            _request: StorageSecretRequest,
        ) -> BoxFuture<'_, Result<StorageSecret, Failure>> {
            Box::pin(async { Err(Failure::Unavailable) })
        }
        fn execute(
            &self,
            _connection: Connection,
            _credentials: S3Credentials,
            _operation: Operation,
            _panel: StorageEndpoint,
        ) -> BoxFuture<'_, Result<Outcome, Failure>> {
            Box::pin(async { Err(Failure::Unavailable) })
        }
    }
    async fn fixture() -> (DbPool, Arc<Runtime>, Services, Job, Payload) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        panel_db::pool::run_migrations(&pool)
            .await
            .expect("migrations");
        sqlx::query("INSERT INTO users(id,username,password_hash,role,status,created_at,updated_at) VALUES(1,'tlsadmin','unused','admin','active',0,0)").execute(&pool).await.expect("admin");
        let payload = Payload {
            request_id: uuid::Uuid::new_v4().to_string(),
            revision: 2,
        };
        let job = panel_db::SqliteJobRepository::new(pool.clone())
            .create(&panel_db::models::CreateJob {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: Some(1),
                name: "storage.tls.issue".into(),
                payload_json: serde_json::to_string(&payload).expect("json"),
                max_retries: 0,
            })
            .await
            .expect("job");
        sqlx::query("UPDATE managed_storage_tls SET revision=2,request_id=?,job_id=?,actor_id=1,hostname='storage.example.test',email='admin@example.test',zone_id='zone',zone_name='example.test',credential_revision='revision',state='queued',auto_renew=1,last_attempt=? WHERE id=1")
            .bind(&payload.request_id).bind(&job.id).bind(now()).execute(&pool).await.expect("binding");
        let runtime = Arc::new(Runtime::default());
        let services = Services {
            runtime: runtime.clone(),
            transfers: Arc::new(tokio::sync::Semaphore::new(1)),
        };
        (pool, runtime, services, job, payload)
    }
    #[tokio::test]
    async fn issuance_applies_once_and_keeps_private_material_out_of_metadata() {
        let (pool, runtime, services, job, payload) = fixture().await;
        process(&pool, &services, &job, &payload)
            .await
            .expect("issue");
        let row = binding(&pool).await.expect("binding");
        assert_eq!(row.state, "active");
        assert!(row.not_after.expect("expiry") > now() + 3600);
        assert!(row.renew_at.expect("renew") < row.not_after.expect("expiry"));
        assert!(!public(&row).to_string().contains("PRIVATE KEY"));
        assert!(process(&pool, &services, &job, &payload).await.is_err());
        assert_eq!(runtime.issued.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.applied.load(Ordering::SeqCst), 1);
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('managed_storage_tls')")
                .fetch_all(&pool)
                .await
                .expect("columns");
        assert!(!columns.iter().any(|name| name.contains("pem")
            || name.contains("private_key")
            || name.contains("token")));
    }
    #[tokio::test]
    async fn failures_and_wrong_names_never_apply_or_persist_provider_details() {
        for wrong_name in [false, true] {
            let (pool, runtime, services, job, payload) = fixture().await;
            runtime.fail.store(!wrong_name, Ordering::SeqCst);
            runtime.wrong_name.store(wrong_name, Ordering::SeqCst);
            let error = process(&pool, &services, &job, &payload)
                .await
                .expect_err("fail");
            assert!(!error.contains("fixture_secret"));
            assert_eq!(runtime.applied.load(Ordering::SeqCst), 0);
            let row = binding(&pool).await.expect("binding");
            assert_eq!(row.state, "failed");
            assert!(!row.auto_renew);
            assert!(!public(&row).to_string().contains("fixture_secret"));
        }
    }
    #[tokio::test]
    async fn manual_tls_reservation_fences_old_jobs_and_stops_automatic_renewal() {
        let (pool, runtime, services, job, payload) = fixture().await;
        assert!(reserve_manual(&pool).await.is_err());
        sqlx::query("UPDATE managed_storage_tls SET state='active'")
            .execute(&pool)
            .await
            .expect("active");
        let revision = reserve_manual(&pool).await.expect("manual");
        assert!(process(&pool, &services, &job, &payload).await.is_err());
        assert_eq!(runtime.issued.load(Ordering::SeqCst), 0);
        finish_manual(&pool, revision, "storage.example.test", true)
            .await
            .expect("finish");
        let row = binding(&pool).await.expect("binding");
        assert_eq!(row.state, "manual");
        assert!(!row.auto_renew);
    }
    #[tokio::test]
    async fn revoked_administrator_cannot_start_issuance() {
        let (pool, runtime, services, job, payload) = fixture().await;
        sqlx::query("UPDATE users SET status='suspended' WHERE id=1")
            .execute(&pool)
            .await
            .expect("suspend");
        assert!(process(&pool, &services, &job, &payload).await.is_err());
        assert_eq!(runtime.issued.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn running_storage_certificate_jobs_cannot_be_cancelled_mid_apply() {
        let (pool, _, _, job, _) = fixture().await;
        let repository = panel_db::SqliteJobRepository::new(pool.clone());
        sqlx::query("UPDATE jobs SET status='running' WHERE id=?")
            .bind(&job.id)
            .execute(&pool)
            .await
            .expect("running");
        assert!(
            !repository
                .cancel(&job.id)
                .await
                .expect("cannot cancel running")
        );
        sqlx::query("UPDATE jobs SET status='queued' WHERE id=?")
            .bind(&job.id)
            .execute(&pool)
            .await
            .expect("queued");
        assert!(repository.cancel(&job.id).await.expect("cancel queued"));
    }
    fn state(pool: DbPool) -> AppState {
        AppState::new(
            Arc::new(AgentClient::new("unused-tls-scheduler-fixture")),
            pool,
            "testing".into(),
            crate::config::AuthConfig::default(),
            Arc::new(crate::auth::LockoutManager::new(5, 900)),
        )
    }
    #[tokio::test]
    async fn renewal_is_enqueued_once_and_interrupted_jobs_are_not_replayed() {
        let (pool, runtime, _, old_job, _) = fixture().await;
        let state = state(pool.clone());
        sqlx::query("UPDATE managed_storage_tls SET state='active',renew_at=?,last_attempt=?")
            .bind(now() - 1)
            .bind(now() - 3601)
            .execute(&pool)
            .await
            .expect("due");
        schedule(&state).await;
        let renewed = binding(&pool).await.expect("renewed");
        assert_eq!(renewed.state, "queued");
        assert_eq!(renewed.revision, 3);
        assert_ne!(renewed.job_id.as_deref(), Some(old_job.id.as_str()));
        schedule(&state).await;
        assert_eq!(binding(&pool).await.expect("binding").revision, 3);
        let id = renewed.job_id.expect("job id");
        let job = state
            .job_runner
            .job_repo
            .find_by_id(&id)
            .await
            .expect("job")
            .expect("exists");
        assert_eq!(job.max_retries, 0);
        assert!(!job.payload_json.contains("PRIVATE KEY"));
        sqlx::query("UPDATE jobs SET status='failed' WHERE id=?")
            .bind(&id)
            .execute(&pool)
            .await
            .expect("interrupted");
        sqlx::query("UPDATE managed_storage_tls SET state='issuing'")
            .execute(&pool)
            .await
            .expect("issuing");
        schedule(&state).await;
        assert_eq!(binding(&pool).await.expect("binding").state, "failed");
        assert_eq!(runtime.issued.load(Ordering::SeqCst), 0);
    }
}
