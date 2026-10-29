//! DNS-01 for registered site hostnames; panel IP certificates remain separate.
use crate::{
    agent_client::AgentClient,
    auth::{RequireAdmin, RequireAdminCsrf},
    routes::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
use futures_util::future::BoxFuture;
use panel_db::{
    DbError, DbPool,
    models::{Domain, Site, User},
    repositories::{
        SqliteCertificateRepository, SqliteDomainRepository, SqliteSiteRepository,
        SqliteUserRepository,
    },
};
use panel_proto::Dns01Request;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use utoipa::ToSchema;
type Error = (StatusCode, Json<Value>);
pub(crate) const TXT_PROPAGATION_ERROR: &str =
    "TXT belum terlihat pada dua resolver publik. Periksa delegasi/propagasi lalu coba lagi.";
pub(crate) const DNS_AGENT_STATUS_ERROR: &str = "Status DNS-01 agent tidak dapat dibaca.";
pub(crate) const TXT_PRESENT_ERROR: &str = "Agent gagal menyiapkan TXT DNS-01 di Cloudflare.";
fn problem(status: StatusCode, message: impl Into<String>) -> Error {
    (status, Json(json!({"error":message.into()})))
}
fn db_error(_: impl std::fmt::Display) -> Error {
    problem(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Inventaris DNS-01 gagal dimuat/disimpan.",
    )
}
fn json_error(_: JsonRejection) -> Error {
    problem(StatusCode::BAD_REQUEST, "Isian DNS-01 tidak valid.")
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn upstream(e: impl std::fmt::Display) -> Error {
    problem(StatusCode::BAD_GATEWAY, e.to_string())
}
#[derive(Clone, sqlx::FromRow)]
struct Order {
    id: String,
    site_id: i64,
    owner_id: i64,
    actor_id: Option<i64>,
    domains_json: String,
    settings_json: String,
    zone_id: String,
    zone_name: String,
    credential_revision: String,
    email: String,
    staging: i64,
    source_id: Option<i64>,
    certificate_id: Option<i64>,
    state: String,
    fullchain_pem: String,
    private_key_pem: String,
    job_id: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    zone_id: String,
    zone_name: String,
    api_token: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    site_id: i64,
    email: String,
    staging: bool,
    confirmed: bool,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Cleanup {
    id: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Discard {
    hostname: String,
}
async fn locked(pool: &DbPool, seconds: i64) -> Result<String, String> {
    let token = uuid::Uuid::new_v4().to_string();
    let result =
        sqlx::query("UPDATE dns01_lock SET token=?,expires_at=? WHERE id=1 AND expires_at<?")
            .bind(&token)
            .bind(now() + seconds)
            .bind(now())
            .execute(pool)
            .await
            .map_err(err)?;
    if result.rows_affected() != 1 {
        return Err("Operasi DNS-01 masih berjalan. Tunggu sampai selesai.".into());
    }
    Ok(token)
}
async fn release(pool: &DbPool, token: &str) {
    let _ = sqlx::query("UPDATE dns01_lock SET token=NULL,expires_at=0 WHERE token=?")
        .bind(token)
        .execute(pool)
        .await;
}
async fn order(pool: &DbPool, id: &str) -> Result<Order, String> {
    sqlx::query_as("SELECT * FROM dns01_orders WHERE id=?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|_| "Order DNS-01 tidak ditemukan.".into())
}
async fn load_site(pool: &DbPool, id: i64) -> Result<(Site, User, Vec<Domain>), String> {
    let site = SqliteSiteRepository::new(pool.clone())
        .find_by_id(id)
        .await
        .map_err(err)?
        .ok_or("Project tidak ditemukan.")?;
    let owner = SqliteUserRepository::new(pool.clone())
        .find_by_id(site.user_id)
        .await
        .map_err(err)?
        .ok_or("Pemilik project tidak ditemukan.")?;
    if owner.status != "active" || site.status != "active" {
        return Err("Project dan pemilik harus aktif.".into());
    }
    crate::tenant_context::execution_identity(&owner).map_err(|(_, e)| e)?;
    let mut domains = SqliteDomainRepository::new(pool.clone())
        .find_by_site_id(id)
        .await
        .map_err(err)?;
    domains.sort_by_key(|d| d.id);
    Ok((site, owner, domains))
}
fn names(domains: &[Domain], zone: &str) -> Result<Vec<String>, String> {
    if domains.is_empty()
        || domains.len() > 5
        || domains.iter().filter(|d| d.is_primary).count() != 1
    {
        return Err("Gunakan satu domain utama dan maksimal lima hostname per project.".into());
    }
    let mut names = Vec::new();
    for d in domains {
        let host = panel_core::DomainName::parse(&d.name)
            .map_err(|_| "Hostname tidak valid.")?
            .as_ascii()
            .to_owned();
        if host.parse::<std::net::IpAddr>().is_ok()
            || host.contains('*')
            || (host != zone && !host.ends_with(&format!(".{zone}")))
        {
            return Err("Seluruh hostname project harus berada dalam zona DNS yang dipilih; IP/wildcard tidak didukung.".into());
        }
        // Redirect variants may introduce an extra www name; require explicit registered SANs.
        if matches!(d.redirect_mode.as_str(), "to_www" | "to_non_www" | "custom") {
            return Err("Nonaktifkan redirect www/custom otomatis atau daftarkan hostname secara eksplisit sebelum DNS-01.".into());
        }
        names.push(host);
    }
    Ok(names)
}
async fn authorized(pool: &DbPool, r: &Order) -> Result<(Site, User, Vec<Domain>), String> {
    if let Some(actor) = r.actor_id {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin' AND status='active')",
        )
        .bind(actor)
        .fetch_one(pool)
        .await
        .map_err(err)?;
        if !active {
            return Err("Administrator penerbitan tidak aktif.".into());
        }
    }
    let (site, owner, domains) = load_site(pool, r.site_id).await?;
    if site.user_id != r.owner_id
        || site.settings_json != r.settings_json
        || serde_json::to_string(&domains).map_err(err)? != r.domains_json
    {
        return Err("Project/domain berubah sejak order dibuat. Kandidat tidak dipasang.".into());
    }
    let active = SqliteCertificateRepository::new(pool.clone())
        .find_active_by_site_id(site.id)
        .await
        .map_err(err)?
        .map(|c| c.id);
    if active != r.source_id {
        return Err("Sertifikat aktif project berubah.".into());
    }
    names(&domains, &r.zone_name)?;
    Ok((site, owner, domains))
}
#[utoipa::path(get,path="/api/v1/network/dns01",responses((status=200,description="Konfigurasi DNS-01 tersunting dan inventaris")),tag="ssl")]
pub async fn overview(
    State(s): State<AppState>,
    RequireAdmin(_actor): RequireAdmin,
) -> Result<Json<Value>, Error> {
    let provider = s
        .agent_client
        .dns01(Dns01Request::Status)
        .await
        .map_err(upstream)?;
    let projects: Vec<(i64, String)> = sqlx::query_as(
        "SELECT site_id,name FROM domains WHERE is_primary=1 ORDER BY name LIMIT 200",
    )
    .fetch_all(&s.pool)
    .await
    .map_err(db_error)?;
    type InventoryOrder = (String, i64, String, i64, Option<String>, String, i64);
    let rows:Vec<InventoryOrder>=sqlx::query_as("SELECT id,site_id,state,staging,error,job_id,created_at FROM dns01_orders ORDER BY created_at DESC,id DESC LIMIT 20").fetch_all(&s.pool).await.map_err(db_error)?;
    let bindings:Vec<(i64,i64,i64,Option<String>,i64)>=sqlx::query_as("SELECT b.site_id,b.certificate_id,b.renew_at,b.error,c.not_after FROM dns01_bindings b JOIN certificates c ON c.id=b.certificate_id AND c.status='active' AND c.id=(SELECT MAX(id) FROM certificates WHERE site_id=b.site_id AND status='active') JOIN sites s ON s.id=b.site_id AND json_extract(s.settings_json,'$.has_ssl')=1 AND json_extract(s.settings_json,'$.ssl_cert')=c.cert_path ORDER BY b.site_id LIMIT 200").fetch_all(&s.pool).await.map_err(db_error)?;
    Ok(Json(
        json!({"provider":provider,"projects":projects.into_iter().map(|(id,hostname)|json!({"site_id":id,"hostname":hostname})).collect::<Vec<_>>(),"orders":rows.into_iter().map(|(id,site_id,state,staging,error,job_id,created_at)|json!({"id":id,"site_id":site_id,"state":state,"staging":staging!=0,"error":error,"job_id":job_id,"created_at":created_at})).collect::<Vec<_>>(),"bindings":bindings.into_iter().map(|(site_id,certificate_id,renew_at,error,not_after)|json!({"site_id":site_id,"certificate_id":certificate_id,"renew_at":renew_at,"not_after":not_after,"error":error})).collect::<Vec<_>>()}),
    ))
}
async fn no_pending(pool: &DbPool) -> Result<(), Error> {
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM dns01_orders WHERE state IN ('queued','issuing','candidate')) OR EXISTS(SELECT 1 FROM managed_storage_tls WHERE state IN ('queued','issuing','configuring'))",
    )
    .fetch_one(pool)
    .await
    .map_err(db_error)?;
    if pending {
        return Err(problem(
            StatusCode::CONFLICT,
            "Selesaikan atau buang kandidat sebelum mengganti konfigurasi DNS.",
        ));
    }
    Ok(())
}
#[utoipa::path(put,path="/api/v1/network/dns01",request_body=Configuration,responses((status=200,description="Token DNS khusus zona disimpan pada agent")),tag="ssl")]
pub async fn configure(
    State(s): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    body: Result<Json<Configuration>, JsonRejection>,
) -> Result<Json<Value>, Error> {
    let Json(body) = body.map_err(json_error)?;
    let token = locked(&s.pool, 300)
        .await
        .map_err(|e| problem(StatusCode::CONFLICT, &e))?;
    let result = async {
        no_pending(&s.pool).await?;
        s.agent_client
            .dns01(Dns01Request::Configure {
                zone_id: body.zone_id,
                zone_name: body.zone_name,
                api_token: panel_proto::CloudflareSecret(body.api_token),
            })
            .await
            .map(Json)
            .map_err(upstream)
    }
    .await;
    release(&s.pool, &token).await;
    s.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "dns01.configure",
        "dns01",
        if result.is_ok() { "success" } else { "failure" },
        None,
        None,
    );
    result
}
#[utoipa::path(delete,path="/api/v1/network/dns01",responses((status=200,description="Token lokal dilepas; token provider harus dicabut terpisah")),tag="ssl")]
pub async fn revoke(
    State(s): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
) -> Result<Json<Value>, Error> {
    let token = locked(&s.pool, 300)
        .await
        .map_err(|e| problem(StatusCode::CONFLICT, &e))?;
    let result = async {
        no_pending(&s.pool).await?;
        s.agent_client
            .dns01(Dns01Request::Revoke)
            .await
            .map(Json)
            .map_err(upstream)
    }
    .await;
    release(&s.pool, &token).await;
    s.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "dns01.revoke",
        "dns01",
        if result.is_ok() { "success" } else { "failure" },
        None,
        None,
    );
    result
}
async fn dispatch(s: &AppState, r: &Order) -> Result<panel_db::models::Job, DbError> {
    s.job_runner
        .enqueue(
            r.actor_id,
            Some(r.job_id.clone()),
            "ssl.dns01.issue".into(),
            json!({"order_id":r.id}).to_string(),
            Some(0),
        )
        .await
        .map(|(j, _)| j)
}
async fn enqueue(
    s: &AppState,
    site_id: i64,
    email: &str,
    staging: bool,
    actor: Option<i64>,
) -> Result<panel_db::models::Job, Error> {
    let p = s
        .agent_client
        .dns01(Dns01Request::Status)
        .await
        .map_err(upstream)?;
    if p["configured"] != true {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            "Simpan token khusus DNS-01 terlebih dahulu.",
        ));
    }
    if p["pending"].as_array().is_none_or(|v| !v.is_empty()) {
        return Err(problem(
            StatusCode::CONFLICT,
            "Cleanup TXT tertunda diperlukan sebelum order baru.",
        ));
    }
    let zone = p["zone_name"]
        .as_str()
        .ok_or_else(|| upstream("Status zona tidak valid."))?;
    let (site, owner, domains) = load_site(&s.pool, site_id)
        .await
        .map_err(|e| problem(StatusCode::BAD_REQUEST, &e))?;
    names(&domains, zone).map_err(|e| problem(StatusCode::BAD_REQUEST, &e))?;
    let active = SqliteCertificateRepository::new(s.pool.clone())
        .find_active_by_site_id(site.id)
        .await
        .map_err(db_error)?;
    let id = uuid::Uuid::new_v4().to_string();
    let job_id = uuid::Uuid::new_v4().to_string();
    let mut tx = s
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(db_error)?;
    let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM dns01_lock WHERE expires_at>?) OR EXISTS(SELECT 1 FROM dns01_orders WHERE state IN ('queued','issuing','candidate'))").bind(now()).fetch_one(&mut *tx).await.map_err(db_error)?;
    if busy {
        return Err(problem(
            StatusCode::CONFLICT,
            "Operasi/kandidat DNS-01 lain masih berlangsung.",
        ));
    }
    let recent: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM dns01_orders WHERE site_id=? AND last_attempt>?)",
    )
    .bind(site_id)
    .bind(now() - 60)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_error)?;
    if recent {
        return Err(problem(
            StatusCode::TOO_MANY_REQUESTS,
            "Tunggu satu menit sebelum menerbitkan ulang.",
        ));
    }
    sqlx::query("INSERT INTO dns01_orders(id,site_id,owner_id,actor_id,domains_json,settings_json,zone_id,zone_name,credential_revision,email,staging,source_id,state,job_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,'queued',?,?)")
 .bind(&id).bind(site_id).bind(owner.id).bind(actor).bind(serde_json::to_string(&domains).map_err(db_error)?).bind(&site.settings_json).bind(p["zone_id"].as_str().ok_or_else(||upstream("Zona hilang."))?).bind(zone).bind(p["revision"].as_str().ok_or_else(||upstream("Revisi token hilang."))?).bind(email).bind(i64::from(staging)).bind(active.map(|c|c.id)).bind(job_id).bind(now()).execute(&mut *tx).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    dispatch(s, &order(&s.pool, &id).await.map_err(upstream)?)
        .await
        .map_err(db_error)
}
#[utoipa::path(post,path="/api/v1/network/dns01/issue",request_body=Issue,responses((status=202,description="Penerbitan hostname masuk antrean")),tag="ssl")]
pub async fn issue(
    State(s): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    body: Result<Json<Issue>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), Error> {
    let Json(body) = body.map_err(json_error)?;
    if !body.confirmed
        || body.email.len() > 254
        || !body.email.contains('@')
        || body.email.chars().any(char::is_whitespace)
        || body.email.chars().any(char::is_control)
    {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            "Email dan persetujuan kontrol DNS/ketentuan CA diperlukan.",
        ));
    }
    let job = enqueue(&s, body.site_id, &body.email, body.staging, Some(actor.id)).await?;
    s.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "dns01.issue",
        format!("site:{}", body.site_id),
        "success",
        None,
        None,
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::to_value(job).map_err(db_error)?),
    ))
}
async fn terminal(s: &AppState, r: &Order) -> Result<(), Error> {
    if s.job_runner
        .job_repo
        .find_by_id(&r.job_id)
        .await
        .map_err(db_error)?
        .is_some_and(|j| matches!(j.status.as_str(), "queued" | "running"))
    {
        return Err(problem(
            StatusCode::CONFLICT,
            "Tunggu tugas selesai terlebih dahulu.",
        ));
    }
    Ok(())
}
#[utoipa::path(post,path="/api/v1/network/dns01/{id}/retry",params(("id"=String,Path)),responses((status=202,description="Kandidat yang sama masuk antrean")),tag="ssl")]
pub async fn retry(
    State(s): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<Value>), Error> {
    let r = order(&s.pool, &id).await.map_err(upstream)?;
    terminal(&s, &r).await?;
    let token = locked(&s.pool, 30)
        .await
        .map_err(|e| problem(StatusCode::CONFLICT, &e))?;
    let result=async {
 if r.state!="candidate" {return Err(problem(StatusCode::CONFLICT,"Hanya kandidat dapat dicoba kembali."));}
 authorized(&s.pool,&r).await.map_err(|e|problem(StatusCode::CONFLICT,&e))?;
 sqlx::query("UPDATE dns01_orders SET actor_id=?,job_id=?,last_attempt=? WHERE id=? AND state='candidate'").bind(actor.id).bind(uuid::Uuid::new_v4().to_string()).bind(now()).bind(&id).execute(&s.pool).await.map_err(db_error)?;
 let job=dispatch(&s,&order(&s.pool,&id).await.map_err(upstream)?).await.map_err(db_error)?;
 Ok((StatusCode::ACCEPTED,Json(serde_json::to_value(job).map_err(db_error)?)))
 }.await;
    release(&s.pool, &token).await;
    result
}
#[utoipa::path(post,path="/api/v1/network/dns01/cleanup",request_body=Cleanup,responses((status=200,description="Cleanup challenge yang ditinggalkan")),tag="ssl")]
pub async fn cleanup(
    State(s): State<AppState>,
    RequireAdminCsrf(_actor): RequireAdminCsrf,
    body: Result<Json<Cleanup>, JsonRejection>,
) -> Result<Json<Value>, Error> {
    let Json(body) = body.map_err(json_error)?;
    let token = locked(&s.pool, 300)
        .await
        .map_err(|e| problem(StatusCode::CONFLICT, &e))?;
    let result = async {
        let status = s
            .agent_client
            .dns01(Dns01Request::Status)
            .await
            .map_err(upstream)?;
        let pending = status["pending"]
            .as_array()
            .ok_or_else(|| upstream("Jurnal DNS tidak valid."))?;
        if pending.iter().any(|e| {
            e["id"] == body.id && e["created_at"].as_i64().is_none_or(|t| now() - t < 2400)
        }) {
            return Err(problem(
                StatusCode::CONFLICT,
                "Challenge masih baru; tunggu 40 menit sebelum cleanup manual.",
            ));
        }
        s.agent_client
            .dns01(Dns01Request::Cleanup { id: body.id })
            .await
            .map(Json)
            .map_err(upstream)
    }
    .await;
    release(&s.pool, &token).await;
    result
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/network/dns01",
            get(overview).put(configure).delete(revoke),
        )
        .route("/api/v1/network/dns01/issue", post(issue))
        .route("/api/v1/network/dns01/cleanup", post(cleanup))
        .route("/api/v1/network/dns01/{id}/retry", post(retry))
        .route("/api/v1/network/dns01/{id}/discard", post(discard))
}

#[utoipa::path(post,path="/api/v1/network/dns01/{id}/discard",params(("id"=String,Path)),request_body=Discard,responses((status=200,description="Origin dipulihkan sebelum kandidat dibuang")),tag="ssl")]
pub async fn discard(
    State(s): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    body: Result<Json<Discard>, JsonRejection>,
) -> Result<Json<Value>, Error> {
    let Json(body) = body.map_err(json_error)?;
    let r = order(&s.pool, &id).await.map_err(upstream)?;
    terminal(&s, &r).await?;
    let token = locked(&s.pool, 300)
        .await
        .map_err(|e| problem(StatusCode::CONFLICT, e))?;
    let result=async {
  if r.state!="candidate" {return Err(problem(StatusCode::CONFLICT,"Hanya kandidat dapat dibuang."));}
  let (site,user,domains)=load_site(&s.pool,r.site_id).await.map_err(upstream)?;
  let primary=domains.iter().find(|d|d.is_primary).ok_or_else(||upstream("Domain utama hilang."))?;
  if primary.name!=body.hostname {return Err(problem(StatusCode::BAD_REQUEST,"Konfirmasi hostname tidak cocok."));}
  apply(&s.agent_client,&site,&user,&domains).await.map_err(upstream)?;
  let mut tx=s.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_error)?;
  sqlx::query("UPDATE certificates SET status='failed',auto_renew=0,privkey_pem='' WHERE id=? AND status='renewing'").bind(r.certificate_id).execute(&mut *tx).await.map_err(db_error)?;
  sqlx::query("UPDATE dns01_orders SET state='failed',private_key_pem='',error='Kandidat dibuang setelah origin dipulihkan.' WHERE id=? AND state='candidate'").bind(&id).execute(&mut *tx).await.map_err(db_error)?;
  tx.commit().await.map_err(db_error)?;Ok(Json(json!({"discarded":true})))
 }.await;
    release(&s.pool, &token).await;
    s.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "dns01.discard",
        format!("site:{}", r.site_id),
        if result.is_ok() { "success" } else { "failure" },
        None,
        None,
    );
    result
}

pub struct Handler(pub Arc<AgentClient>);
impl crate::jobs::registry::JobHandler for Handler {
    fn run<'a>(
        &'a self,
        j: &'a panel_db::models::Job,
        c: &'a crate::jobs::registry::JobContext,
    ) -> crate::jobs::registry::JobFuture<'a> {
        Box::pin(async move {
            let body: Value = serde_json::from_str(&j.payload_json).map_err(err)?;
            let id = body["order_id"].as_str().ok_or("ID order tidak valid.")?;
            let r = order(&c.pool, id).await?;
            if r.actor_id != j.user_id {
                return Err("Identitas tugas DNS-01 tidak cocok.".into());
            }
            c.report_progress(10, Some("Memeriksa DNS-01, zona dan domain project."))
                .await
                .map_err(err)?;
            process(
                &c.pool,
                &self.0,
                id,
                &RealIssuer {
                    pool: &c.pool,
                    agent: &self.0,
                },
            )
            .await?;
            Ok(Some(json!({"order_id":id}).to_string()))
        })
    }
}
pub(crate) struct Issued {
    pub(crate) chain: String,
    pub(crate) key: String,
}
trait Issuer: Sync {
    fn issue<'a>(
        &'a self,
        r: &'a Order,
        names: &'a [String],
    ) -> BoxFuture<'a, Result<Issued, String>>;
}
struct RealIssuer<'a> {
    pool: &'a DbPool,
    agent: &'a AgentClient,
}
impl Issuer for RealIssuer<'_> {
    fn issue<'a>(
        &'a self,
        r: &'a Order,
        names: &'a [String],
    ) -> BoxFuture<'a, Result<Issued, String>> {
        Box::pin(order_dns(self.pool, self.agent, r, names))
    }
}
async fn process(
    pool: &DbPool,
    agent: &AgentClient,
    id: &str,
    issuer: &dyn Issuer,
) -> Result<(), String> {
    let token = locked(pool, 2400).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(1800),
        process_inner(pool, agent, id, issuer, &token),
    )
    .await
    .unwrap_or_else(|_| Err("Batas waktu DNS-01 habis; periksa kandidat dan cleanup.".into()));
    if let Err(e) = &result {
        let _=sqlx::query("UPDATE dns01_orders SET error=?,state=CASE WHEN fullchain_pem='' THEN 'failed' ELSE 'candidate' END WHERE id=? AND state NOT IN ('active','staging')").bind(e).bind(id).execute(pool).await;
        let _=sqlx::query("UPDATE dns01_bindings SET error=?,last_attempt=? WHERE site_id=(SELECT site_id FROM dns01_orders WHERE id=?)").bind(e).bind(now()).bind(id).execute(pool).await;
    }
    release(pool, &token).await;
    result
}
async fn apply(
    agent: &AgentClient,
    site: &Site,
    user: &User,
    domains: &[Domain],
) -> Result<(), String> {
    let config = crate::sites::render_site_vhost(site, user, domains).map_err(|(_, e)| e)?;
    let (username, _) = crate::tenant_context::execution_identity(user).map_err(|(_, e)| e)?;
    let domain = domains
        .iter()
        .find(|d| d.is_primary)
        .ok_or("Domain utama hilang.")?;
    agent
        .apply_site(panel_proto::SiteApplyRequest {
            site_id: format!("site_{}", site.id),
            config_content: config,
            enable: true,
            nginx_base_dir: None,
            log_context: Some(panel_proto::SiteLogContext {
                username: username.to_string(),
                domain: domain.name.clone(),
            }),
            test_config: true,
            reload: true,
        })
        .await
        .map_err(|_| "Penerapan TLS origin belum dapat dipastikan.".to_owned())?;
    Ok(())
}
async fn process_inner(
    pool: &DbPool,
    agent: &AgentClient,
    id: &str,
    issuer: &dyn Issuer,
    token: &str,
) -> Result<(), String> {
    let mut r = order(pool, id).await?;
    if matches!(r.state.as_str(), "active" | "staging") {
        return Ok(());
    }
    if !matches!(r.state.as_str(), "queued" | "issuing" | "candidate") {
        return Err("Order bukan kandidat/antrean aktif.".into());
    }
    let (_, _, domains) = authorized(pool, &r).await?;
    let dns_names = names(&domains, &r.zone_name)?;
    if r.fullchain_pem.is_empty() {
        sqlx::query("UPDATE dns01_orders SET state='issuing',last_attempt=? WHERE id=?")
            .bind(now())
            .bind(id)
            .execute(pool)
            .await
            .map_err(err)?;
        let issued = issuer.issue(&r, &dns_names).await?;
        panel_core::panel_tls::validate_dns_candidate(
            &issued.chain,
            &issued.key,
            &dns_names,
            now(),
        )
        .map_err(err)?;
        if r.staging != 0 {
            sqlx::query("UPDATE dns01_orders SET state='staging',error=NULL WHERE id=?")
                .bind(id)
                .execute(pool)
                .await
                .map_err(err)?;
            return Ok(());
        }
        sqlx::query("UPDATE dns01_orders SET fullchain_pem=?,private_key_pem=?,state='candidate' WHERE id=?").bind(issued.chain).bind(issued.key).bind(id).execute(pool).await.map_err(err)?;
        r = order(pool, id).await?;
    }
    let (old, user, domains) = authorized(pool, &r).await?;
    let validity = panel_core::panel_tls::validate_dns_candidate(
        &r.fullchain_pem,
        &r.private_key_pem,
        &dns_names,
        now(),
    )
    .map_err(err)?;
    let primary = domains
        .iter()
        .find(|d| d.is_primary)
        .ok_or("Domain utama hilang.")?;
    let paths = agent
        .dns01(Dns01Request::StageCertificate {
            id: r.id.clone(),
            hostname: primary.name.clone(),
            names: dns_names.clone(),
            fullchain_pem: r.fullchain_pem.clone(),
            private_key_pem: r.private_key_pem.clone(),
        })
        .await
        .map_err(err)?;
    let expected_dir = format!("/etc/panel/dns01/certs/{}", r.id);
    let cert_path = format!("{expected_dir}/fullchain.pem");
    let key_path = format!("{expected_dir}/privkey.pem");
    if paths["cert_path"] != cert_path || paths["key_path"] != key_path {
        return Err("Path kandidat agent tidak sesuai.".into());
    }
    let repo = SqliteCertificateRepository::new(pool.clone());
    if r.certificate_id.is_none() {
        let cert = repo
            .create(&panel_db::CreateCertificate {
                site_id: r.site_id,
                primary_domain: primary.name.clone(),
                san_domains_json: serde_json::to_string(&dns_names).map_err(err)?,
                issuer: "letsencrypt".into(),
                cert_path: cert_path.clone(),
                key_path: key_path.clone(),
                fullchain_pem: r.fullchain_pem.clone(),
                privkey_pem: r.private_key_pem.clone(),
                not_before: validity.not_before,
                not_after: validity.not_after,
                auto_renew: true,
                ocsp_stapling: false,
                status: "renewing".into(),
            })
            .await
            .map_err(err)?;
        sqlx::query("UPDATE dns01_orders SET certificate_id=? WHERE id=?")
            .bind(cert.id)
            .bind(id)
            .execute(pool)
            .await
            .map_err(err)?;
        r.certificate_id = Some(cert.id);
    }
    let mut candidate = old.clone();
    let mut settings: Value = serde_json::from_str(&old.settings_json).map_err(err)?;
    settings["has_ssl"] = json!(true);
    settings["ssl_cert"] = json!(cert_path);
    settings["ssl_key"] = json!(key_path);
    settings["ocsp_stapling"] = json!(false);
    candidate.settings_json = settings.to_string();
    authorized(pool, &r).await?;
    let result=async {
  let live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM dns01_lock WHERE token=? AND expires_at>?)").bind(token).bind(now()+60).fetch_one(pool).await.map_err(err)?;
  if !live {return Err("Lease DNS-01 berakhir sebelum deployment.".to_owned());}
  apply(agent,&candidate,&user,&domains).await?;
  for hostname in &dns_names {agent.dns01(Dns01Request::VerifyOrigin{id:r.id.clone(),hostname:hostname.clone()}).await.map_err(|_|"Listener origin belum menyajikan kandidat TLS yang diminta.".to_owned())?;}

  let mut tx=pool.begin_with("BEGIN IMMEDIATE").await.map_err(err)?;
  let actor_active:bool=sqlx::query_scalar("SELECT (? IS NULL OR EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin' AND status='active')) AND EXISTS(SELECT 1 FROM users WHERE id=? AND status='active')").bind(r.actor_id).bind(r.actor_id).bind(r.owner_id).fetch_one(&mut *tx).await.map_err(err)?;
  if !actor_active {return Err("Otorisasi berubah saat deployment.".into());}

  let changed=sqlx::query("UPDATE sites SET settings_json=? WHERE id=? AND user_id=? AND settings_json=? AND status='active'").bind(&candidate.settings_json).bind(old.id).bind(old.user_id).bind(&old.settings_json).execute(&mut *tx).await.map_err(err)?.rows_affected();
  let current_domains=SqliteDomainRepository::new(pool.clone()).find_by_site_id(old.id).await.map_err(err)?;
  let mut current_domains=current_domains;current_domains.sort_by_key(|d|d.id);
  if changed!=1 || serde_json::to_string(&current_domains).map_err(err)?!=r.domains_json {return Err("Project berubah saat aktivasi; inventaris tidak diganti.".into());}
  sqlx::query("UPDATE certificates SET status='revoked',auto_renew=0 WHERE site_id=? AND status='active'").bind(old.id).execute(&mut *tx).await.map_err(err)?;
  sqlx::query("UPDATE certificates SET status='active' WHERE id=?").bind(r.certificate_id).execute(&mut *tx).await.map_err(err)?;
  sqlx::query("INSERT INTO dns01_bindings(site_id,certificate_id,zone_id,zone_name,email,renew_at,last_attempt,error) VALUES(?,?,?,?,?,?,?,NULL) ON CONFLICT(site_id) DO UPDATE SET certificate_id=excluded.certificate_id,zone_id=excluded.zone_id,zone_name=excluded.zone_name,email=excluded.email,renew_at=excluded.renew_at,last_attempt=excluded.last_attempt,error=NULL").bind(old.id).bind(r.certificate_id).bind(&r.zone_id).bind(&r.zone_name).bind(&r.email).bind(panel_core::panel_tls::renewal_at(validity.not_before,validity.not_after,None)).bind(now()).execute(&mut *tx).await.map_err(err)?;
  sqlx::query("UPDATE dns01_orders SET state='active',private_key_pem='',error=NULL WHERE id=?").bind(id).execute(&mut *tx).await.map_err(err)?;
  tx.commit().await.map_err(err)?;Ok(())
 }.await;
    if let Err(e) = result {
        // Restore the CURRENT DB configuration if an operator edited the project concurrently.
        let (current, current_user, current_domains) = load_site(pool, old.id).await?;
        apply(agent,&current,&current_user,&current_domains).await.map_err(|_|format!("{e} Rollback origin belum dapat dipastikan; periksa Nginx sebelum mencoba kembali."))?;
        return Err(format!(
            "{e} Konfigurasi sebelumnya telah diterapkan kembali; kandidat dipertahankan."
        ));
    }
    Ok(())
}

async fn order_dns(
    pool: &DbPool,
    agent: &AgentClient,
    r: &Order,
    names: &[String],
) -> Result<Issued, String> {
    issue_dns_names(
        pool,
        agent,
        &r.email,
        r.staging != 0,
        &r.credential_revision,
        names,
    )
    .await
}

pub(crate) async fn issue_storage_certificate(
    pool: &DbPool,
    agent: &AgentClient,
    hostname: &str,
    email: &str,
    revision: &str,
) -> Result<Issued, String> {
    let token = locked(pool, 900).await?;
    let result = issue_dns_names(pool, agent, email, false, revision, &[hostname.to_owned()]).await;
    release(pool, &token).await;
    result
}

async fn issue_dns_names(
    pool: &DbPool,
    agent: &AgentClient,
    email: &str,
    staging: bool,
    revision: &str,
    names: &[String],
) -> Result<Issued, String> {
    use instant_acme::{
        AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt, NewOrder, OrderStatus,
        RetryPolicy,
    };
    let mut cleanup_ids = Vec::new();
    let flow = async {
        let status = agent
            .dns01(Dns01Request::Status)
            .await
            .map_err(|_| DNS_AGENT_STATUS_ERROR.to_owned())?;
        if status["revision"] != revision {
            return Err("Token DNS berubah; buat order baru.".to_owned());
        }
        let repo = SqliteCertificateRepository::new(pool.clone());
        let account = crate::ssl::get_acme_account(
            pool,
            &repo,
            email,
            if staging {
                LetsEncrypt::Staging.url()
            } else {
                LetsEncrypt::Production.url()
            },
        )
        .await
        .map_err(|_| "Akun ACME DNS-01 tidak tersedia.")?;
        let identifiers = names
            .iter()
            .map(|n| Identifier::Dns(n.clone()))
            .collect::<Vec<_>>();
        let mut order = account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .map_err(|_| "CA menolak order DNS.")?;
        {
            let mut auths = order.authorizations();
            while let Some(result) = auths.next().await {
                let mut auth = result.map_err(|_| "Otorisasi DNS gagal.")?;
                if auth.status == AuthorizationStatus::Valid {
                    continue;
                }
                let host = match auth.identifier().identifier {
                    Identifier::Dns(n) => n.clone(),
                    _ => return Err("CA mengembalikan identifier bukan DNS.".into()),
                };
                if auth.wildcard || !names.contains(&host) {
                    return Err("Hostname otorisasi bukan milik snapshot project.".into());
                }
                let mut challenge = auth
                    .challenge(ChallengeType::Dns01)
                    .ok_or("DNS-01 tidak didukung CA.")?;
                let digest = challenge.key_authorization().dns_value();
                let id = uuid::Uuid::new_v4().to_string();
                cleanup_ids.push(id.clone());
                agent
                    .dns01(Dns01Request::Present {
                        id,
                        revision: revision.to_owned(),
                        hostname: host.clone(),
                        digest: digest.clone(),
                    })
                    .await
                    .map_err(|_| TXT_PRESENT_ERROR.to_owned())?;
                propagation(&host, &digest).await?;
                challenge
                    .set_ready()
                    .await
                    .map_err(|_| "CA belum dapat memeriksa TXT.")?;
            }
        }
        if order
            .poll_ready(&RetryPolicy::default())
            .await
            .map_err(|_| "Validasi DNS-01 gagal di CA.")?
            != OrderStatus::Ready
        {
            return Err("Order DNS belum siap.".into());
        }
        let mut params =
            rcgen::CertificateParams::new(names.to_vec()).map_err(|_| "CSR DNS gagal.")?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        let key = rcgen::KeyPair::generate().map_err(|_| "Key DNS gagal dibuat.")?;
        let csr = params
            .serialize_request(&key)
            .map_err(|_| "CSR DNS gagal.")?;
        order
            .finalize_csr(csr.der())
            .await
            .map_err(|_| "Finalisasi DNS gagal.")?;
        let chain = order
            .poll_certificate(&RetryPolicy::default())
            .await
            .map_err(|_| "Sertifikat DNS belum tersedia.")?;
        let key = key.serialize_pem();
        panel_core::panel_tls::validate_dns_candidate(&chain, &key, names, now()).map_err(err)?;
        if !staging {
            for name in names {
                crate::panel_tls::verify_chain_name(
                    &chain,
                    tokio_rustls::rustls::pki_types::ServerName::try_from(name.clone())
                        .map_err(err)?,
                )
                .map_err(err)?;
            }
        }
        Ok(Issued { chain, key })
    };
    let result = tokio::time::timeout(Duration::from_secs(600), flow)
        .await
        .unwrap_or_else(|_| Err("Batas waktu order DNS-01 habis.".into()));
    let mut cleanup_failed = false;
    for id in cleanup_ids {
        if agent.dns01(Dns01Request::Cleanup { id }).await.is_err() {
            cleanup_failed = true;
        }
    }
    if cleanup_failed {
        return Err("Cleanup TXT belum selesai; jurnal dipertahankan. Bersihkan sebelum order berikutnya. Sertifikat aktif dipertahankan.".into());
    }
    result
}
fn txt_matches(response: &Value, name: &str, digest: &str) -> bool {
    response["Status"] == 0
        && response["Answer"].as_array().is_some_and(|answers| {
            !answers.iter().any(|a| a["type"] == 5)
                && answers.iter().any(|a| {
                    a["type"] == 16
                        && a["name"]
                            .as_str()
                            .is_some_and(|n| n.trim_end_matches('.').eq_ignore_ascii_case(name))
                        && a["data"]
                            .as_str()
                            .is_some_and(|v| v == format!("\"{digest}\"") || v == digest)
                })
        })
}
// Fixed public DoH endpoints, bounded stdout and kill-on-drop; no shell or credential.
async fn resolve_txt(url: &str, name: &str) -> Result<Value, String> {
    use tokio::io::AsyncReadExt;
    let query = format!("name={name}");
    let mut child = tokio::process::Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--proto",
            "=https",
            "--connect-timeout",
            "3",
            "--max-time",
            "8",
            "--max-filesize",
            "65536",
            "--header",
            "Accept: application/dns-json",
            "--get",
            "--data-urlencode",
            &query,
            "--data-urlencode",
            "type=TXT",
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Resolver memerlukan curl terpasang.".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Output resolver tidak tersedia.")?;
    let mut bytes = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        stdout.take(65537).read_to_end(&mut bytes),
    )
    .await
    .map_err(|_| "Resolver DNS timeout.")?
    .map_err(|_| "Respons DNS gagal dibaca.")?;
    if bytes.len() > 65536 {
        return Err("Respons DNS terlalu besar.".into());
    }
    let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .map_err(|_| "Resolver DNS timeout.")?
        .map_err(|_| "Resolver DNS gagal.")?;
    if !status.success() {
        return Err("Resolver DNS gagal.".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Respons resolver DNS tidak valid.".into())
}
trait TxtResolver: Sync {
    fn resolve<'a>(&'a self, url: &'a str, name: &'a str) -> BoxFuture<'a, Result<Value, String>>;
}
struct CurlResolver;
impl TxtResolver for CurlResolver {
    fn resolve<'a>(&'a self, url: &'a str, name: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(resolve_txt(url, name))
    }
}
async fn propagation(host: &str, digest: &str) -> Result<(), String> {
    propagation_with(
        host,
        digest,
        &CurlResolver,
        Duration::from_secs(180),
        Duration::from_secs(5),
    )
    .await
}
async fn propagation_with(
    host: &str,
    digest: &str,
    resolver: &dyn TxtResolver,
    budget: Duration,
    interval: Duration,
) -> Result<(), String> {
    let name = format!("_acme-challenge.{host}");
    tokio::time::timeout(budget, async {
        loop {
            let mut okay = true;
            for url in [
                "https://cloudflare-dns.com/dns-query",
                "https://dns.google/resolve",
            ] {
                if !resolver
                    .resolve(url, &name)
                    .await
                    .is_ok_and(|v| txt_matches(&v, &name, digest))
                {
                    okay = false;
                }
            }
            if okay {
                return;
            }
            tokio::time::sleep(interval).await;
        }
    })
    .await
    .map_err(|_| TXT_PROPAGATION_ERROR.into())
}
pub async fn schedule(s: &AppState) {
    let pending:Vec<(String,String,Option<i64>,String)>=sqlx::query_as("SELECT id,job_id,actor_id,state FROM dns01_orders WHERE state IN ('queued','issuing','candidate')").fetch_all(&s.pool).await.unwrap_or_default();
    for (id, job, _, state) in pending {
        match s.job_runner.job_repo.find_by_id(&job).await {
            Ok(None) => {
                if let Ok(r) = order(&s.pool, &id).await {
                    let _ = dispatch(s, &r).await;
                }
            }
            Ok(Some(j))
                if matches!(j.status.as_str(), "failed" | "cancelled" | "succeeded")
                    && state != "candidate" =>
            {
                let _=sqlx::query("UPDATE dns01_orders SET state=CASE WHEN fullchain_pem='' THEN 'failed' ELSE 'candidate' END,error='Tugas terputus atau dibatalkan; periksa cleanup DNS.' WHERE id=? AND NOT EXISTS(SELECT 1 FROM dns01_lock WHERE expires_at>?)").bind(&id).bind(now()).execute(&s.pool).await;
            }
            _ => {}
        }
    }
    let due:Vec<(i64,String,String,String)>=sqlx::query_as("SELECT b.site_id,b.email,b.zone_id,b.zone_name FROM dns01_bindings b JOIN certificates c ON c.id=b.certificate_id JOIN sites s ON s.id=b.site_id AND json_extract(s.settings_json,'$.has_ssl')=1 AND json_extract(s.settings_json,'$.ssl_cert')=c.cert_path WHERE c.status='active' AND c.auto_renew=1 AND c.id=(SELECT MAX(id) FROM certificates WHERE site_id=b.site_id AND status='active') AND b.renew_at<=? AND b.last_attempt<? ORDER BY b.renew_at LIMIT 5").bind(now()).bind(now()-3600).fetch_all(&s.pool).await.unwrap_or_default();
    for (site, email, zone_id, zone_name) in due {
        let status = s.agent_client.dns01(Dns01Request::Status).await;
        let result = if status
            .as_ref()
            .is_ok_and(|p| p["zone_id"] == zone_id && p["zone_name"] == zone_name)
        {
            enqueue(s, site, &email, false, None).await.map(|_| ())
        } else {
            Err(upstream("Token zona renewal tidak tersedia atau berubah."))
        };
        let error = result.err().map(|e| {
            e.1.0["error"]
                .as_str()
                .unwrap_or("Renewal belum dapat diantrekan.")
                .to_owned()
        });
        let _ = sqlx::query("UPDATE dns01_bindings SET last_attempt=?,error=? WHERE site_id=?")
            .bind(now())
            .bind(error)
            .bind(site)
            .execute(&s.pool)
            .await;
    }
}
/// Existing HTTP-01 lifecycle may not mutate a pending DNS candidate.
pub async fn guard_site(pool: &DbPool, site: i64) -> Result<(), (StatusCode, String)> {
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM dns01_orders WHERE site_id=? AND state IN ('queued','issuing','candidate'))").bind(site).fetch_one(pool).await.map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"Inventaris DNS-01 gagal diperiksa.".into()))?;
    if pending {
        return Err((
            StatusCode::CONFLICT,
            "Selesaikan kandidat DNS-01 sebelum mengganti SSL project.".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "dns01_tests.rs"]
mod tests;
