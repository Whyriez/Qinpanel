//! IP certificates for the panel's dedicated TLS proxy; no tenant inventory reuse.
use crate::{
    agent_client::AgentClient,
    auth::{RequireAdmin, RequireAdminCsrf},
    connectivity::{Error, db_error, json_error, problem},
    routes::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
use futures_util::future::BoxFuture;
use panel_core::{
    connectivity::{EndpointScope, EndpointService, EndpointTlsMode, EndpointTransport},
    panel_tls::{renewal_at, validate_ip_candidate},
};
use panel_db::{DbError, DbPool, SqliteConnectivityRepository};
use panel_proto::PanelTlsRequest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{net::IpAddr, sync::Arc, time::Duration};
use utoipa::ToSchema;
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
#[derive(Clone, sqlx::FromRow)]
struct Record {
    id: String,
    endpoint_id: String,
    endpoint_revision: i64,
    ip: String,
    port: i64,
    staging: i64,
    email: String,
    fullchain_pem: String,
    private_key_pem: String,
    fingerprint: Option<String>,
    not_before: Option<i64>,
    not_after: Option<i64>,
    renew_at: Option<i64>,
    state: String,
    error: Option<String>,
    job_id: Option<String>,
    source_id: Option<String>,
    ari_checked_at: Option<i64>,
}
#[derive(Serialize, ToSchema)]
pub struct PanelCertificate {
    pub id: String,
    pub scope: String,
    pub ip: String,
    pub port: i64,
    pub staging: bool,
    pub state: String,
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
    pub renew_at: Option<i64>,
    pub expired: bool,
    pub error: Option<String>,
    pub job_id: Option<String>,
}
impl From<Record> for PanelCertificate {
    fn from(r: Record) -> Self {
        Self {
            id: r.id,
            scope: "panel".into(),
            ip: r.ip,
            port: r.port,
            staging: r.staging != 0,
            state: r.state,
            not_before: r.not_before,
            not_after: r.not_after,
            renew_at: r.renew_at,
            expired: r.not_after.is_some_and(|n| n <= now()),
            error: r.error,
            job_id: r.job_id,
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IssuePanelCertificate {
    pub endpoint_id: String,
    pub expected_revision: i64,
    pub email: String,
    pub staging: bool,
    pub http01_confirmed: bool,
}
#[derive(Serialize, ToSchema)]
pub struct PanelCertificates {
    pub certificates: Vec<PanelCertificate>,
    pub challenge_port: u16,
    pub profile: String,
}
#[utoipa::path(get,path="/api/v1/connectivity/panel-tls",responses((status=200,body=PanelCertificates)),tag="ssl")]
pub async fn overview(
    State(state): State<AppState>,
    RequireAdmin(_actor): RequireAdmin,
) -> Result<Json<PanelCertificates>, Error> {
    let records: Vec<Record> = sqlx::query_as(
        "SELECT * FROM panel_tls_certificates ORDER BY CASE state WHEN 'active' THEN 0 WHEN 'candidate' THEN 1 WHEN 'issuing' THEN 1 WHEN 'queued' THEN 1 ELSE 2 END,created_at DESC,id DESC LIMIT 20",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(DbError::from)
    .map_err(db_error)?;
    Ok(Json(PanelCertificates {
        certificates: records.into_iter().map(Into::into).collect(),
        challenge_port: 80,
        profile: "shortlived".into(),
    }))
}
async fn active(pool: &DbPool) -> Result<Option<Record>, String> {
    sqlx::query_as("SELECT * FROM panel_tls_certificates WHERE state='active'")
        .fetch_optional(pool)
        .await
        .map_err(|_| "Inventaris TLS panel tidak dapat dimuat.".into())
}
async fn record(pool: &DbPool, id: &str) -> Result<Record, String> {
    sqlx::query_as("SELECT * FROM panel_tls_certificates WHERE id=?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|_| "Sertifikat panel tidak ditemukan.".into())
}
async fn queue(
    state: &AppState,
    actor: Option<i64>,
    r: &Record,
) -> Result<panel_db::models::Job, Error> {
    repair_outbox(state).await;
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;

    let pending: Option<String> = sqlx::query_scalar(
        "SELECT id FROM panel_tls_certificates WHERE state IN ('queued','issuing','candidate')",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(DbError::from)
    .map_err(db_error)?;
    if pending.is_some() {
        return Err(problem(
            StatusCode::CONFLICT,
            "Masih ada penerbitan atau kandidat TLS. Tunggu atau sinkronkan kandidat tersebut.",
        ));
    }
    let existing: Option<(String, i64)> =
        sqlx::query_as("SELECT ip,port FROM panel_tls_certificates WHERE state='active'")
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)
            .map_err(db_error)?;
    if existing.is_some_and(|(ip, port)| ip != r.ip || port != r.port) {
        return Err(problem(
            StatusCode::CONFLICT,
            "Listener TLS aktif memakai IP atau port lain. Siapkan transisi listener terpisah sebelum menggantinya.",
        ));
    }
    if let Some(actor) = actor {
        let authorized: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin' AND status='active')",
        )
        .bind(actor)
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
        if !authorized {
            return Err(problem(StatusCode::FORBIDDEN, "Administrator tidak aktif."));
        }
    }
    let job_id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO panel_tls_certificates(id,endpoint_id,endpoint_revision,ip,port,staging,email,state,created_at,job_id,source_id,actor_id) VALUES(?,?,?,?,?,?,?,'queued',?,?,?,?)")
        .bind(&r.id).bind(&r.endpoint_id).bind(r.endpoint_revision).bind(&r.ip).bind(r.port).bind(r.staging).bind(&r.email).bind(now()).bind(&job_id).bind(&r.source_id).bind(actor).execute(&mut *tx).await.map_err(DbError::from).map_err(db_error)?;
    tx.commit().await.map_err(DbError::from).map_err(db_error)?;
    dispatch(state, &job_id, actor, &r.id)
        .await
        .map_err(db_error)
}
async fn dispatch(
    state: &AppState,
    id: &str,
    actor: Option<i64>,
    cert: &str,
) -> Result<panel_db::models::Job, DbError> {
    state
        .job_runner
        .enqueue(
            actor,
            Some(id.into()),
            "panel.tls.issue".into(),
            serde_json::json!({"certificate_id":cert}).to_string(),
            Some(0),
        )
        .await
        .map(|(job, _)| job)
}
async fn repair_outbox(state: &AppState) {
    let rows:Vec<(String,String,Option<i64>,String)>=sqlx::query_as("SELECT id,job_id,actor_id,state FROM panel_tls_certificates WHERE state IN ('queued','issuing','candidate') AND job_id IS NOT NULL LIMIT 10").fetch_all(&state.pool).await.unwrap_or_default();
    for (id, job_id, actor, status) in rows {
        match state.job_runner.job_repo.find_by_id(&job_id).await {
            Ok(None) => {
                let _ = dispatch(state, &job_id, actor, &id).await;
            }
            Ok(Some(job))
                if matches!(job.status.as_str(), "failed" | "cancelled" | "succeeded")
                    && status != "candidate" =>
            {
                let _=sqlx::query("UPDATE panel_tls_certificates SET state='failed',error='Tugas terputus atau dibatalkan sebelum aktivasi.',private_key_pem='' WHERE id=? AND NOT EXISTS(SELECT 1 FROM panel_tls_lock WHERE expires_at>?)").bind(id).bind(now()).execute(&state.pool).await;
            }
            _ => {}
        }
    }
}

fn new_record(
    endpoint_id: String,
    revision: i64,
    ip: String,
    port: i64,
    email: String,
    staging: bool,
    source_id: Option<String>,
) -> Record {
    Record {
        id: uuid::Uuid::new_v4().to_string(),
        endpoint_id,
        endpoint_revision: revision,
        ip,
        port,
        email,
        staging: i64::from(staging),
        fullchain_pem: String::new(),
        private_key_pem: String::new(),
        fingerprint: None,
        not_before: None,
        not_after: None,
        renew_at: None,
        state: "queued".into(),
        error: None,
        job_id: None,
        source_id,
        ari_checked_at: None,
    }
}
#[utoipa::path(post,path="/api/v1/connectivity/panel-tls/issue",request_body=IssuePanelCertificate,responses((status=202,description="Antrean sertifikat")),tag="ssl")]
pub async fn issue(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    payload: Result<Json<IssuePanelCertificate>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), Error> {
    let Json(body) = payload.map_err(json_error)?;
    if !body.http01_confirmed
        || body.email.len() > 254
        || body
            .email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || !body
            .email
            .split_once('@')
            .is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.contains('@'))
    {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Email valid dan konfirmasi akses HTTP-01 port 80 diperlukan.",
        ));
    }
    let e = SqliteConnectivityRepository::new(state.pool.clone())
        .find(actor.id, &body.endpoint_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| problem(StatusCode::NOT_FOUND, "Endpoint tidak ditemukan."))?;
    let ip: IpAddr = e.config.host.as_str().parse().map_err(|_| {
        problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Pilih alamat IP publik, bukan hostname. DNS-01 terpisah.",
        )
    })?;
    if e.revision != body.expected_revision
        || e.config.service != EndpointService::Panel
        || e.config.scope != EndpointScope::Host
        || !e.config.enabled
        || e.config.transport != EndpointTransport::Direct
        || e.config.tls_mode != EndpointTlsMode::Tls
        || e.config.port.get() < 1024
        || matches!(e.config.port.get(), 7700 | 17890)
        || !crate::connectivity_diagnosis::public_probe_ip(ip)
    {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Pilih endpoint Panel langsung, enabled, TLS, IP publik dan port TLS terpisah (misalnya 19443), dengan revisi terbaru.",
        ));
    }
    let r = new_record(
        e.id,
        e.revision,
        ip.to_string(),
        i64::from(e.config.port.get()),
        body.email,
        body.staging,
        None,
    );
    let job = queue(&state, Some(actor.id), &r).await?;
    state.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "panel.tls.issue",
        format!("certificate:{}", r.id),
        "queued",
        None,
        None,
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"job_id":job.id,"certificate_id":r.id})),
    ))
}
#[utoipa::path(post,path="/api/v1/connectivity/panel-tls/renew",responses((status=202,description="Renewal diantrekan")),tag="ssl")]
pub async fn renew(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
) -> Result<(StatusCode, Json<serde_json::Value>), Error> {
    let old = active(&state.pool)
        .await
        .map_err(|_| {
            problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Inventaris belum tersedia.",
            )
        })?
        .ok_or_else(|| problem(StatusCode::NOT_FOUND, "Sertifikat panel aktif belum ada."))?;
    let r = new_record(
        old.endpoint_id,
        old.endpoint_revision,
        old.ip,
        old.port,
        old.email,
        false,
        Some(old.id),
    );
    let job = queue(&state, Some(actor.id), &r).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"job_id":job.id,"certificate_id":r.id})),
    ))
}
#[utoipa::path(post,path="/api/v1/connectivity/panel-tls/{id}/retry",params(("id"=String,Path)),responses((status=202,description="Sinkronisasi kandidat")),tag="ssl")]
pub async fn retry(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), Error> {
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    let row: Option<(String, Option<String>, Option<i64>)> =
        sqlx::query_as("SELECT state,job_id,last_attempt FROM panel_tls_certificates WHERE id=?")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)
            .map_err(db_error)?;
    let Some((status, job, last)) = row else {
        return Err(problem(StatusCode::NOT_FOUND, "Kandidat tidak ditemukan."));
    };
    let busy = if let Some(job) = job {
        state
            .job_runner
            .job_repo
            .find_by_id(&job)
            .await
            .map_err(db_error)?
            .is_some_and(|j| matches!(j.status.as_str(), "queued" | "running"))
    } else {
        false
    };
    if status != "candidate" || busy || last.is_some_and(|t| now() - t < 60) {
        return Err(problem(
            StatusCode::CONFLICT,
            "Tunggu satu menit atau sampai tugas selesai sebelum sinkronisasi kandidat.",
        ));
    }
    let job_id = uuid::Uuid::new_v4().to_string();
    sqlx::query("UPDATE panel_tls_certificates SET job_id=?,actor_id=?,last_attempt=? WHERE id=?")
        .bind(&job_id)
        .bind(actor.id)
        .bind(now())
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    tx.commit().await.map_err(DbError::from).map_err(db_error)?;
    let job = dispatch(&state, &job_id, Some(actor.id), &id)
        .await
        .map_err(db_error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"job_id":job.id,"certificate_id":id})),
    ))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/connectivity/panel-tls", get(overview))
        .route("/api/v1/connectivity/panel-tls/issue", post(issue))
        .route("/api/v1/connectivity/panel-tls/renew", post(renew))
        .route("/api/v1/connectivity/panel-tls/{id}/retry", post(retry))
        .route("/api/v1/connectivity/panel-tls/{id}/discard", post(discard))
}

#[utoipa::path(post,path="/api/v1/connectivity/panel-tls/{id}/discard",params(("id"=String,Path)),responses((status=200,description="Kandidat dibuang jika belum aktif")),tag="ssl")]
pub async fn discard(
    State(state): State<AppState>,
    RequireAdminCsrf(_actor): RequireAdminCsrf,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Error> {
    let r = record(&state.pool, &id)
        .await
        .map_err(|_| problem(StatusCode::NOT_FOUND, "Kandidat tidak ditemukan."))?;
    if r.state != "candidate" {
        return Err(problem(
            StatusCode::CONFLICT,
            "Hanya kandidat yang belum selesai yang dapat dibuang.",
        ));
    }
    if let Some(job_id) = &r.job_id
        && state
            .job_runner
            .job_repo
            .find_by_id(job_id)
            .await
            .map_err(db_error)?
            .is_some_and(|j| matches!(j.status.as_str(), "queued" | "running"))
    {
        return Err(problem(
            StatusCode::CONFLICT,
            "Tunggu tugas selesai sebelum membuang kandidat.",
        ));
    }
    let token = uuid::Uuid::new_v4().to_string();
    let locked =
        sqlx::query("UPDATE panel_tls_lock SET token=?,expires_at=? WHERE id=1 AND expires_at<?")
            .bind(&token)
            .bind(now() + 120)
            .bind(now())
            .execute(&state.pool)
            .await
            .map_err(DbError::from)
            .map_err(db_error)?
            .rows_affected()
            == 1;
    if !locked {
        return Err(problem(StatusCode::CONFLICT, "Operasi TLS masih berjalan."));
    }
    let result=async {
        let server=state.agent_client.panel_tls(PanelTlsRequest::Inspect).await.map_err(|_|problem(StatusCode::SERVICE_UNAVAILABLE,"Status agent belum diketahui. Kandidat dipertahankan."))?;
        if server.active_id.as_deref()==Some(&id) { return Err(problem(StatusCode::CONFLICT,"Kandidat sudah terpasang pada agent. Sinkronkan agar inventaris sesuai.")); }
        sqlx::query("UPDATE panel_tls_certificates SET state='failed',private_key_pem='',error='Kandidat dibuang administrator.' WHERE id=? AND state='candidate'").bind(&id).execute(&state.pool).await.map_err(DbError::from).map_err(db_error)?;
        Ok(Json(serde_json::json!({"discarded":true})))
    }.await;
    let _ = sqlx::query("UPDATE panel_tls_lock SET token=NULL,expires_at=0 WHERE token=?")
        .bind(token)
        .execute(&state.pool)
        .await;
    result
}

pub struct PanelTlsHandler(pub Arc<AgentClient>);
impl crate::jobs::registry::JobHandler for PanelTlsHandler {
    fn run<'a>(
        &'a self,
        job: &'a panel_db::models::Job,
        ctx: &'a crate::jobs::registry::JobContext,
    ) -> crate::jobs::registry::JobFuture<'a> {
        Box::pin(async move {
            let payload: serde_json::Value = serde_json::from_str(&job.payload_json)
                .map_err(|_| "Payload sertifikat tidak valid.")?;
            let id = payload["certificate_id"]
                .as_str()
                .ok_or("ID sertifikat tidak valid.")?;
            ctx.report_progress(10, Some("Memeriksa scope panel dan jalur HTTP-01 port 80."))
                .await
                .map_err(|_| "Progres gagal disimpan.")?;
            process(
                &ctx.pool,
                &self.0,
                id,
                job.user_id,
                &RealIssuer {
                    pool: &ctx.pool,
                    agent: &self.0,
                },
            )
            .await?;
            Ok(Some(serde_json::json!({"certificate_id":id}).to_string()))
        })
    }
}
trait Issuer: Sync {
    fn issue<'a>(&'a self, r: &'a Record) -> BoxFuture<'a, Result<Issued, String>>;
}
struct Issued {
    chain: String,
    key: String,
    renew_at: i64,
}
struct RealIssuer<'a> {
    pool: &'a DbPool,
    agent: &'a AgentClient,
}
impl Issuer for RealIssuer<'_> {
    fn issue<'a>(&'a self, r: &'a Record) -> BoxFuture<'a, Result<Issued, String>> {
        Box::pin(order_ip(self.pool, self.agent, r))
    }
}

async fn authorized(pool: &DbPool, r: &Record, actor: Option<i64>) -> Result<(), String> {
    if let Some(actor) = actor {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin' AND status='active')",
        )
        .bind(actor)
        .fetch_one(pool)
        .await
        .map_err(|_| "Otorisasi gagal.")?;
        if !ok {
            return Err("Administrator tidak aktif.".into());
        }
    }
    if let Some(source) = &r.source_id {
        let old = active(pool).await?.ok_or("Sertifikat aktif berubah.")?;
        if &old.id != source || old.ip != r.ip || old.port != r.port || old.staging != 0 {
            return Err("Snapshot renewal berubah.".into());
        }
    } else {
        if actor.is_none() {
            return Err("Penerbitan awal memerlukan administrator.".into());
        }
        let ok:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM service_endpoints WHERE id=? AND revision=? AND service='panel' AND scope_kind='host' AND enabled=1 AND transport='direct' AND tls_mode='tls' AND host=? AND port=?)").bind(&r.endpoint_id).bind(r.endpoint_revision).bind(&r.ip).bind(r.port).fetch_one(pool).await.map_err(|_|"Endpoint gagal diperiksa.")?;
        if !ok {
            return Err("Endpoint berubah; penerbitan dibatalkan.".into());
        }
    }
    Ok(())
}
async fn process(
    pool: &DbPool,
    agent: &AgentClient,
    id: &str,
    actor: Option<i64>,
    issuer: &dyn Issuer,
) -> Result<(), String> {
    let token = uuid::Uuid::new_v4().to_string();
    let locked =
        sqlx::query("UPDATE panel_tls_lock SET token=?,expires_at=? WHERE id=1 AND expires_at<?")
            .bind(&token)
            .bind(now() + 1200)
            .bind(now())
            .execute(pool)
            .await
            .map_err(|_| "Lease TLS gagal.")?
            .rows_affected()
            == 1;
    if !locked {
        return Err("Operasi TLS lain masih berjalan.".into());
    }
    let result = tokio::time::timeout(
        Duration::from_secs(600),
        process_inner(pool, agent, id, actor, issuer, &token),
    )
    .await
    .unwrap_or_else(|_| {
        Err("Batas waktu penerbitan habis; periksa atau sinkronkan kandidat.".into())
    });
    if let Err(error) = &result {
        sqlx::query("UPDATE panel_tls_certificates SET error=?,state=CASE WHEN state='candidate' THEN 'candidate' ELSE 'failed' END,private_key_pem=CASE WHEN state='candidate' THEN private_key_pem ELSE '' END WHERE id=? AND state NOT IN ('active','retired','staging')").bind(error).bind(id).execute(pool).await.map_err(|_|"Status kegagalan gagal disimpan.")?;
    }
    sqlx::query("UPDATE panel_tls_lock SET token=NULL,expires_at=0 WHERE id=1 AND token=?")
        .bind(token)
        .execute(pool)
        .await
        .map_err(|_| "Lease TLS gagal dilepas.")?;
    result
}
async fn process_inner(
    pool: &DbPool,
    agent: &AgentClient,
    id: &str,
    actor: Option<i64>,
    issuer: &dyn Issuer,
    token: &str,
) -> Result<(), String> {
    let mut r = record(pool, id).await?;
    if r.state == "active" || r.state == "staging" {
        return Ok(());
    }
    if !matches!(r.state.as_str(), "queued" | "issuing" | "candidate") {
        return Err("Status kandidat tidak dapat diproses.".into());
    }
    authorized(pool, &r, actor).await?;
    sqlx::query("UPDATE panel_tls_certificates SET last_attempt=? WHERE id=?")
        .bind(now())
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| "Waktu percobaan gagal disimpan.")?;
    if r.state != "candidate" {
        sqlx::query("UPDATE panel_tls_certificates SET state='issuing' WHERE id=?")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|_| "Status penerbitan gagal disimpan.")?;
        let issued = issuer.issue(&r).await?;
        let ip = r.ip.parse().map_err(|_| "IP tersimpan tidak valid.")?;
        let validity = validate_ip_candidate(&issued.chain, &issued.key, ip, now())?;
        let fingerprint = format!("{:x}", Sha256::digest(issued.chain.as_bytes()));
        sqlx::query("UPDATE panel_tls_certificates SET state='candidate',fullchain_pem=?,private_key_pem=?,fingerprint=?,not_before=?,not_after=?,renew_at=?,ari_checked_at=strftime('%s','now') WHERE id=?")
            .bind(issued.chain).bind(issued.key).bind(fingerprint).bind(validity.not_before).bind(validity.not_after).bind(issued.renew_at.clamp(now(),validity.not_after-3600)).bind(id).execute(pool).await.map_err(|_|"Kandidat gagal disimpan.")?;
        r = record(pool, id).await?;
    }
    if r.staging != 0 {
        sqlx::query("UPDATE panel_tls_certificates SET state='staging',private_key_pem='',error=NULL WHERE id=?").bind(id).execute(pool).await.map_err(|_|"Status staging gagal disimpan.")?;
        return Ok(());
    }
    authorized(pool, &r, actor).await?;
    let lease: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panel_tls_lock WHERE id=1 AND token=? AND expires_at>?)",
    )
    .bind(token)
    .bind(now())
    .fetch_one(pool)
    .await
    .map_err(|_| "Lease tidak tersedia.")?;
    if !lease {
        return Err("Lease TLS berakhir; kandidat belum diaktifkan.".into());
    }
    let previous = active(pool).await?;
    let expected = previous.and_then(|c| c.fingerprint);
    let request = PanelTlsRequest::Activate {
        id: r.id.clone(),
        ip: r.ip.parse().map_err(|_| "IP tidak valid.")?,
        port: u16::try_from(r.port).map_err(|_| "Port tidak valid.")?,
        expected_fingerprint: expected,
        fullchain_pem: r.fullchain_pem,
        private_key_pem: r.private_key_pem,
    };
    let deployed=match agent.panel_tls(request).await { Ok(v)=>v,Err(_)=>agent.panel_tls(PanelTlsRequest::Inspect).await.map_err(|_|"Hasil aktivasi belum diketahui. Sinkronkan kandidat; sertifikat lama tidak dihapus.")? };
    if deployed.active_id.as_deref() != Some(id) || deployed.fingerprint != r.fingerprint {
        return Err("Kandidat belum aktif. Sertifikat sebelumnya tetap disimpan; periksa Nginx dan sinkronkan.".into());
    }
    commit_active(pool, id, token).await
}
async fn commit_active(pool: &DbPool, id: &str, token: &str) -> Result<(), String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|_| "Transaksi aktivasi gagal.")?;
    let lease: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panel_tls_lock WHERE id=1 AND token=? AND expires_at>?)",
    )
    .bind(token)
    .bind(now())
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "Lease gagal diperiksa.")?;
    if !lease {
        return Err("Lease berakhir setelah aktivasi; sinkronisasi diperlukan.".into());
    }
    sqlx::query(
        "UPDATE panel_tls_certificates SET state='retired',private_key_pem='' WHERE state='active'",
    )
    .execute(&mut *tx)
    .await
    .map_err(|_| "Snapshot lama gagal disimpan.")?;
    sqlx::query("UPDATE panel_tls_certificates SET state='active',error=NULL,private_key_pem='' WHERE id=? AND state='candidate'").bind(id).execute(&mut *tx).await.map_err(|_|"Status aktif gagal disimpan.")?;
    tx.commit()
        .await
        .map_err(|_| "Commit aktivasi gagal; sinkronkan kandidat.".into())
}

async fn order_ip(pool: &DbPool, agent: &AgentClient, r: &Record) -> Result<Issued, String> {
    use instant_acme::{
        AuthorizationStatus, CertificateIdentifier, ChallengeType, Identifier, LetsEncrypt,
        OrderStatus, RetryPolicy,
    };
    use tokio_rustls::rustls::pki_types::{CertificateDer, pem::PemObject};
    let url = if r.staging != 0 {
        LetsEncrypt::Staging.url()
    } else {
        LetsEncrypt::Production.url()
    };
    let repo = panel_db::SqliteCertificateRepository::new(pool.clone());
    let account = crate::ssl::get_acme_account(pool, &repo, &r.email, url)
        .await
        .map_err(|_| "Akun ACME belum dapat digunakan.".to_owned())?;
    if !account.profiles().any(|p| p.name == "shortlived") {
        return Err(
            "CA tidak mengiklankan profil shortlived. Tidak diturunkan ke profil lain.".into(),
        );
    }
    let ip: IpAddr = r.ip.parse().map_err(|_| "Identifier IP tidak valid.")?;
    let ids = [Identifier::Ip(ip)];
    let mut new_order = ip_order(&ids);
    if let Some(source) = &r.source_id
        && let Ok(previous) = record(pool, source).await
        && let Ok(cert) = CertificateDer::from_pem_slice(previous.fullchain_pem.as_bytes())
        && let Ok(cert_id) = CertificateIdentifier::try_from(&cert)
        && matches!(
            tokio::time::timeout(Duration::from_secs(10), account.renewal_info(&cert_id)).await,
            Ok(Ok(_))
        )
    {
        new_order = new_order.replaces(cert_id.into_owned());
    }
    let mut order = account
        .new_order(&new_order)
        .await
        .map_err(|_| "CA menolak order IP shortlived.".to_owned())?;
    let mut tokens = Vec::new();
    let flow = async {
        {
            let mut auths = order.authorizations();
            while let Some(result) = auths.next().await {
                let mut auth = result.map_err(|_| "Otorisasi ACME gagal.")?;
                if auth.status == AuthorizationStatus::Valid {
                    continue;
                }
                let mut challenge = auth
                    .challenge(ChallengeType::Http01)
                    .ok_or("HTTP-01 tidak tersedia untuk identifier IP ini.")?;
                let token = challenge.token.clone();
                tokens.push(token.clone());
                agent
                    .panel_tls(PanelTlsRequest::Challenge {
                        ip,
                        token,
                        key_authorization: challenge.key_authorization().as_str().into(),
                    })
                    .await
                    .map_err(|_| "Challenge port 80 gagal disiapkan; periksa guard Nginx.")?;
                challenge
                    .set_ready()
                    .await
                    .map_err(|_| "Challenge belum dapat diverifikasi CA.")?;
            }
        }
        if order.poll_ready(&RetryPolicy::default()).await.map_err(|_|"Validasi HTTP-01 gagal. Pastikan IP milik server dan port publik 80 dapat dijangkau.")?!=OrderStatus::Ready { return Err("Order belum siap.".to_string()); }
        // Explicit IP SAN CSR rather than treating an IP string as a DNS name.
        let (csr, key) = ip_csr(ip)?;
        order
            .finalize_csr(&csr)
            .await
            .map_err(|_| "Finalisasi CSR IP gagal.")?;
        let chain = order
            .poll_certificate(&RetryPolicy::default())
            .await
            .map_err(|_| "Sertifikat belum dapat diunduh.")?;
        let validity = validate_ip_candidate(&chain, &key, ip, now())?;
        if validity.not_after - validity.not_before > 7 * 86400 {
            return Err("CA tidak menerbitkan sertifikat pendek yang diminta.".into());
        }
        let cert = CertificateDer::from_pem_slice(chain.as_bytes())
            .map_err(|_| "Rantai PEM tidak valid.")?;
        if r.staging == 0 {
            verify_public_chain(&chain, ip)?;
        }
        let ari = if let Ok(cert_id) = CertificateIdentifier::try_from(&cert) {
            tokio::time::timeout(Duration::from_secs(10), account.renewal_info(&cert_id))
                .await
                .ok()
                .and_then(Result::ok)
                .map(|(info, _)| {
                    (
                        info.suggested_window.start.unix_timestamp(),
                        info.suggested_window.end.unix_timestamp(),
                    )
                })
        } else {
            None
        };
        Ok(Issued {
            chain,
            key,
            renew_at: renewal_at(validity.not_before, validity.not_after, ari),
        })
    };
    // Cleanup runs on both success and failure, including bounded ACME timeouts.
    let result = tokio::time::timeout(Duration::from_secs(480), flow)
        .await
        .unwrap_or_else(|_| Err("Batas waktu order ACME habis.".into()));
    for token in tokens {
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            agent.panel_tls(PanelTlsRequest::Cleanup { token }),
        )
        .await;
    }
    result
}
fn ip_order(ids: &[instant_acme::Identifier]) -> instant_acme::NewOrder<'_> {
    instant_acme::NewOrder::new(ids).profile("shortlived")
}
fn ip_csr(ip: IpAddr) -> Result<(Vec<u8>, String), String> {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())
        .map_err(|_| "Parameter CSR gagal dibuat.")?;
    params.subject_alt_names = vec![rcgen::SanType::IpAddress(ip)];
    params.distinguished_name = rcgen::DistinguishedName::new();
    let key = rcgen::KeyPair::generate().map_err(|_| "Private key gagal dibuat.")?;
    let csr = params
        .serialize_request(&key)
        .map_err(|_| "CSR IP gagal dibuat.")?;
    Ok((csr.der().to_vec(), key.serialize_pem()))
}
fn verify_public_chain(chain: &str, ip: IpAddr) -> Result<(), String> {
    verify_chain_name(
        chain,
        tokio_rustls::rustls::pki_types::ServerName::IpAddress(ip.into()),
    )
}
pub(crate) fn verify_chain_name(
    chain: &str,
    name: tokio_rustls::rustls::pki_types::ServerName<'static>,
) -> Result<(), String> {
    use tokio_rustls::rustls::{
        RootCertStore,
        client::{WebPkiServerVerifier, danger::ServerCertVerifier},
        pki_types::{CertificateDer, UnixTime, pem::PemObject},
    };
    let certs = CertificateDer::pem_slice_iter(chain.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Rantai sertifikat tidak valid.")?;
    let (leaf, rest) = certs.split_first().ok_or("Rantai sertifikat kosong.")?;
    let mut roots = RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    let verifier = WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(tokio_rustls::rustls::crypto::ring::default_provider()),
    )
    .build()
    .map_err(|_| "Trust store sistem tidak tersedia.")?;
    verifier
        .verify_server_cert(leaf, rest, &name, &[], UnixTime::now())
        .map_err(
            |_| "Rantai CA atau identitas sertifikat tidak dipercaya oleh trust store sistem.",
        )?;
    Ok(())
}

pub async fn schedule(state: &AppState) {
    repair_outbox(state).await;
    let Ok(Some(mut old)) = active(&state.pool).await else {
        return;
    };
    if old.ari_checked_at.is_none_or(|t| now() - t >= 3600)
        && let Some(deadline) = refresh_ari(&state.pool, &old).await
    {
        old.renew_at = Some(deadline);
    }
    if old.renew_at.is_none_or(|t| t > now()) {
        return;
    }
    let last: Option<i64> =
        sqlx::query_scalar("SELECT MAX(created_at) FROM panel_tls_certificates WHERE source_id=?")
            .bind(&old.id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(None);
    if last.is_some_and(|t| now() - t < 3600) {
        return;
    }
    let r = new_record(
        old.endpoint_id,
        old.endpoint_revision,
        old.ip,
        old.port,
        old.email,
        false,
        Some(old.id),
    );
    let _ = queue(state, None, &r).await;
}

async fn refresh_ari(pool: &DbPool, old: &Record) -> Option<i64> {
    use instant_acme::{CertificateIdentifier, LetsEncrypt};
    use tokio_rustls::rustls::pki_types::{CertificateDer, pem::PemObject};
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let repo = panel_db::SqliteCertificateRepository::new(pool.clone());
        let account =
            crate::ssl::get_acme_account(pool, &repo, &old.email, LetsEncrypt::Production.url())
                .await
                .ok()?;
        let cert = CertificateDer::from_pem_slice(old.fullchain_pem.as_bytes()).ok()?;
        let id = CertificateIdentifier::try_from(&cert).ok()?;
        let (info, _) = account.renewal_info(&id).await.ok()?;
        Some(renewal_at(
            old.not_before?,
            old.not_after?,
            Some((
                info.suggested_window.start.unix_timestamp(),
                info.suggested_window.end.unix_timestamp(),
            )),
        ))
    })
    .await
    .ok()
    .flatten();
    let _=sqlx::query("UPDATE panel_tls_certificates SET ari_checked_at=?,renew_at=COALESCE(?,renew_at) WHERE id=? AND state='active'").bind(now()).bind(result).bind(&old.id).execute(pool).await;
    result
}

#[cfg(test)]
#[path = "panel_tls_tests.rs"]
mod tests;
