//! Explicit URL activation and safe transition aliases; no listener/DNS changes.
use crate::{
    auth::{RequireAdmin, RequireAdminCsrf},
    connectivity::{Error, db_error, json_error, problem},
    public_url::{PanelOrigin, PublicRequest},
    routes::AppState,
};
use axum::http::StatusCode;
use axum::{
    Json, Router,
    extract::{Extension, State, rejection::JsonRejection},
    routing::{get, post},
};
use panel_core::connectivity::{EndpointScope, EndpointService, EndpointTlsMode};
use panel_db::{DbError, SqliteConnectivityRepository};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct PanelUrlSettings {
    pub revision: i64,
    pub canonical_url: Option<String>,
    pub current_url: String,
    pub endpoint_id: Option<String>,
    pub endpoint_revision: Option<i64>,
    pub aliases: Vec<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivatePanelUrl {
    pub endpoint_id: String,
    pub expected_endpoint_revision: i64,
    pub expected_revision: i64,
    pub reachable_confirmed: bool,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeAlias {
    pub origin: String,
    pub expected_revision: i64,
}
async fn settings(state: &AppState, current: &PublicRequest) -> Result<PanelUrlSettings, Error> {
    let row:Option<(i64,String,String,i64)>=sqlx::query_as("SELECT revision,canonical_url,endpoint_id,endpoint_revision FROM panel_url_policy WHERE id=1").fetch_optional(&state.pool).await.map_err(DbError::from).map_err(db_error)?;
    let aliases = sqlx::query_scalar("SELECT origin FROM panel_url_aliases ORDER BY origin")
        .fetch_all(&state.pool)
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    Ok(match row {
        Some((revision, url, id, endpoint_revision)) => PanelUrlSettings {
            revision,
            canonical_url: Some(url),
            current_url: current.origin.url(),
            endpoint_id: Some(id),
            endpoint_revision: Some(endpoint_revision),
            aliases,
        },
        None => PanelUrlSettings {
            revision: 0,
            canonical_url: None,
            current_url: current.origin.url(),
            endpoint_id: None,
            endpoint_revision: None,
            aliases,
        },
    })
}
#[utoipa::path(get,path="/api/v1/connectivity/panel-url",responses((status=200,body=PanelUrlSettings)),tag="connectivity")]
pub async fn overview(
    State(state): State<AppState>,
    RequireAdmin(_actor): RequireAdmin,
    Extension(current): Extension<PublicRequest>,
) -> Result<Json<PanelUrlSettings>, Error> {
    settings(&state, &current).await.map(Json)
}
#[utoipa::path(post,path="/api/v1/connectivity/panel-url/activate",request_body=ActivatePanelUrl,responses((status=200,body=PanelUrlSettings),(status=409,description="Revision conflict")),tag="connectivity")]
pub async fn activate(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Extension(current): Extension<PublicRequest>,
    payload: Result<Json<ActivatePanelUrl>, JsonRejection>,
) -> Result<Json<PanelUrlSettings>, Error> {
    let Json(body) = payload.map_err(json_error)?;
    if !body.reachable_confirmed {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Buka alamat baru dan konfirmasikan bahwa Anda dapat mengaksesnya.",
        ));
    }
    let repo = SqliteConnectivityRepository::new(state.pool.clone());
    let endpoint = repo
        .find(actor.id, &body.endpoint_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| problem(StatusCode::NOT_FOUND, "Alamat panel tidak ditemukan."))?;
    if endpoint.config.service != EndpointService::Panel
        || endpoint.config.scope != EndpointScope::Host
        || !endpoint.config.enabled
        || !endpoint.config.primary
        || endpoint.revision != body.expected_endpoint_revision
    {
        return Err(problem(
            StatusCode::CONFLICT,
            "Pilih alamat utama Panel yang aktif dengan revisi terbaru.",
        ));
    }
    if endpoint.config.port.get() == panel_core::production::INTERNAL_API_PORT {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Port 7700 adalah API internal, bukan URL publik panel.",
        ));
    }
    let secure = endpoint.config.tls_mode == EndpointTlsMode::Tls;
    if !secure && (current.origin.secure || state.auth_config.cookie_secure == Some(true)) {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "URL HTTPS aktif tidak dapat diturunkan ke HTTP. Siapkan alamat HTTPS untuk transisi ini.",
        ));
    }
    let target = PanelOrigin::from_authority(
        &endpoint.config.host.authority(endpoint.config.port),
        secure,
    )
    .map_err(|s| problem(StatusCode::UNPROCESSABLE_ENTITY, s))?;
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND status='active' AND role='admin')",
    )
    .bind(actor.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(DbError::from)
    .map_err(db_error)?;
    let revision: Option<i64> =
        sqlx::query_scalar("SELECT revision FROM panel_url_policy WHERE id=1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)
            .map_err(db_error)?;
    let endpoint_matches:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM service_endpoints WHERE id=? AND revision=? AND service='panel' AND scope_kind='host' AND enabled=1 AND is_primary=1)").bind(&endpoint.id).bind(endpoint.revision).fetch_one(&mut *tx).await.map_err(DbError::from).map_err(db_error)?;
    if !active || !endpoint_matches || revision.unwrap_or(0) != body.expected_revision {
        return Err(problem(
            StatusCode::CONFLICT,
            "Pengaturan berubah. Muat ulang sebelum mengaktifkan URL.",
        ));
    }
    let previous: Option<String> =
        sqlx::query_scalar("SELECT canonical_url FROM panel_url_policy WHERE id=1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)
            .map_err(db_error)?;
    if !secure
        && previous
            .as_deref()
            .is_some_and(|url| PanelOrigin::parse(url).is_ok_and(|origin| origin.secure))
    {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "URL HTTPS aktif tidak dapat diturunkan ke HTTP.",
        ));
    }
    let aliases: Vec<String> = sqlx::query_scalar("SELECT origin FROM panel_url_aliases")
        .fetch_all(&mut *tx)
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    let mut origins: std::collections::BTreeSet<String> = aliases.into_iter().collect();
    origins.insert(current.origin.url());
    if let Some(previous) = previous {
        origins.insert(previous);
    }
    // Snapshot all explicitly registered panel alternatives at activation only.
    let registered:Vec<(String,i64,String)>=sqlx::query_as("SELECT host,port,tls_mode FROM service_endpoints WHERE service='panel' AND scope_kind='host' AND enabled=1").fetch_all(&mut *tx).await.map_err(DbError::from).map_err(db_error)?;
    for (host, port, tls) in registered {
        let host = panel_core::connectivity::EndpointHost::parse(&host)
            .map_err(|_| problem(StatusCode::CONFLICT, "Alamat alternatif tidak valid."))?;
        let port = u16::try_from(port)
            .ok()
            .and_then(std::num::NonZeroU16::new)
            .ok_or_else(|| problem(StatusCode::CONFLICT, "Port alternatif tidak valid."))?;
        if port.get() == panel_core::production::INTERNAL_API_PORT {
            continue;
        }
        let origin = PanelOrigin::from_authority(&host.authority(port), tls == "tls")
            .map_err(|s| problem(StatusCode::CONFLICT, s))?;
        origins.insert(origin.url());
    }
    origins.retain(|url| !PanelOrigin::parse(url).is_ok_and(|o| o.same(&target)));
    if origins.len() > 16 {
        return Err(problem(
            StatusCode::CONFLICT,
            "Maksimal 16 alamat transisi. Cabut alamat lama sebelum menambahkan URL lain.",
        ));
    }
    sqlx::query("INSERT INTO panel_url_policy(id,revision,canonical_url,endpoint_id,endpoint_revision,updated_at) VALUES(1,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,canonical_url=excluded.canonical_url,endpoint_id=excluded.endpoint_id,endpoint_revision=excluded.endpoint_revision,updated_at=excluded.updated_at")
        .bind(body.expected_revision+1).bind(target.url()).bind(&endpoint.id).bind(endpoint.revision).bind(chrono::Utc::now().timestamp()).execute(&mut *tx).await.map_err(DbError::from).map_err(db_error)?;
    sqlx::query("DELETE FROM panel_url_aliases")
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    for origin in origins {
        sqlx::query("INSERT INTO panel_url_aliases(origin) VALUES(?)")
            .bind(origin)
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(DbError::from).map_err(db_error)?;
    state.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "connectivity.panel_url.activate",
        format!("connectivity:{}", endpoint.id),
        "success",
        None,
        None,
    );
    settings(&state, &current).await.map(Json)
}
#[utoipa::path(post,path="/api/v1/connectivity/panel-url/revoke",request_body=RevokeAlias,responses((status=200,body=PanelUrlSettings)),tag="connectivity")]
pub async fn revoke(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Extension(current): Extension<PublicRequest>,
    payload: Result<Json<RevokeAlias>, JsonRejection>,
) -> Result<Json<PanelUrlSettings>, Error> {
    let Json(body) = payload.map_err(json_error)?;
    let origin = PanelOrigin::parse(&body.origin)
        .map_err(|s| problem(StatusCode::UNPROCESSABLE_ENTITY, s))?;
    if current.origin.same(&origin) {
        return Err(problem(
            StatusCode::CONFLICT,
            "Alamat yang sedang dipakai tidak dapat dicabut. Masuk melalui URL aktif terlebih dahulu.",
        ));
    }
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    let policy: Option<(i64, String)> =
        sqlx::query_as("SELECT revision,canonical_url FROM panel_url_policy WHERE id=1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)
            .map_err(db_error)?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin' AND status='active')",
    )
    .bind(actor.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(DbError::from)
    .map_err(db_error)?;
    if !active
        || !policy.as_ref().is_some_and(|(rev, url)| {
            *rev == body.expected_revision
                && PanelOrigin::parse(url).is_ok_and(|main| !main.same(&origin))
        })
    {
        return Err(problem(
            StatusCode::CONFLICT,
            "URL utama atau pengaturan yang berubah tidak dapat dicabut.",
        ));
    }
    sqlx::query("DELETE FROM panel_url_aliases WHERE origin=?")
        .bind(origin.url())
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    sqlx::query("UPDATE panel_url_policy SET revision=revision+1,updated_at=? WHERE id=1")
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)
        .map_err(db_error)?;
    tx.commit().await.map_err(DbError::from).map_err(db_error)?;
    state.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "connectivity.panel_url.revoke",
        "panel_url",
        "success",
        None,
        None,
    );
    settings(&state, &current).await.map(Json)
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/connectivity/panel-url", get(overview))
        .route("/api/v1/connectivity/panel-url/activate", post(activate))
        .route("/api/v1/connectivity/panel-url/revoke", post(revoke))
}
