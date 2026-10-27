//! Authenticated endpoint metadata and bounded, administrator-only diagnosis.
use crate::{
    auth::{RequireAdminCsrf, RequireAuth},
    routes::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use panel_core::connectivity::*;
use panel_db::{DbError, EndpointEvidenceRecord, ServiceEndpoint, SqliteConnectivityRepository};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Serialize, ToSchema)]
pub struct EndpointDto {
    pub id: String,
    pub revision: i64,
    #[serde(flatten)]
    pub config: EndpointConfig,
    pub client_requirement: ClientRequirement,
    pub created_at: i64,
    pub updated_at: i64,
}
impl From<ServiceEndpoint> for EndpointDto {
    fn from(e: ServiceEndpoint) -> Self {
        Self {
            id: e.id,
            revision: e.revision,
            config: e.config,
            client_requirement: e.client_requirement,
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct EvidenceDto {
    pub id: i64,
    pub endpoint_id: String,
    pub service: EndpointService,
    #[serde(flatten)]
    pub evidence: EndpointEvidence,
    pub current: bool,
    pub expired: bool,
    pub revision_matches: bool,
    pub suggested_action: String,
}
impl From<EndpointEvidenceRecord> for EvidenceDto {
    fn from(e: EndpointEvidenceRecord) -> Self {
        Self {
            id: e.id,
            endpoint_id: e.endpoint_id,
            service: e.service,
            evidence: e.evidence,
            current: e.current,
            expired: e.expired,
            revision_matches: e.revision_matches,
            suggested_action: e.suggested_action.into(),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct EndpointSummary {
    pub endpoint: EndpointDto,
    pub evidence: Vec<EvidenceDto>,
}
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct ProjectOption {
    pub id: i64,
    pub name: String,
}
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct CertificateOption {
    pub id: i64,
    pub site_id: i64,
    pub name: String,
}
#[derive(Serialize, ToSchema)]
pub struct ConnectivityOverview {
    pub can_manage: bool,
    pub endpoints: Vec<EndpointSummary>,
    pub projects: Vec<ProjectOption>,
    pub certificates: Vec<CertificateOption>,
    pub page: u32,
    pub per_page: u32,
    pub total: i64,
    pub total_pages: i64,
    pub service_counts: std::collections::BTreeMap<String, i64>,
}
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in=Query)]
pub struct ConnectivityQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub service: Option<EndpointService>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceEndpoint {
    pub expected_revision: i64,
    pub config: EndpointConfig,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteEndpoint {
    pub expected_revision: i64,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnoseEndpoint {
    pub expected_revision: i64,
}
#[derive(Serialize, ToSchema)]
pub struct ConnectivityProblem {
    #[serde(rename = "type")]
    pub problem_type: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
}
pub struct Error {
    status: StatusCode,
    body: Box<ConnectivityProblem>,
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            self.status,
            [(axum::http::header::CONTENT_TYPE, "application/problem+json")],
            Json(*self.body),
        )
            .into_response()
    }
}
pub(crate) fn json_error(rejection: JsonRejection) -> Error {
    problem(
        rejection.status(),
        "Isian alamat tidak valid. Periksa hostname/IP, port, jenis layanan, lingkup dan revisi konfigurasi.",
    )
}
pub(crate) fn problem(status: StatusCode, detail: impl Into<String>) -> Error {
    Error {
        status,
        body: Box::new(ConnectivityProblem {
            problem_type: "about:blank".into(),
            title: "Konfigurasi konektivitas gagal".into(),
            status: status.as_u16(),
            detail: detail.into(),
        }),
    }
}
pub(crate) fn db_error(error: DbError) -> Error {
    match error {
        DbError::Conflict(message) => problem(StatusCode::CONFLICT, message),
        DbError::NotFound(message) => problem(StatusCode::NOT_FOUND, message),
        DbError::Validation(message) => problem(StatusCode::UNPROCESSABLE_ENTITY, message),
        _ => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Data konektivitas gagal dimuat atau disimpan. Coba kembali.",
        ),
    }
}
fn validate_id(id: &str) -> Result<(), Error> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| problem(StatusCode::BAD_REQUEST, "ID endpoint tidak valid."))
}

#[utoipa::path(get, path="/api/v1/connectivity", params(ConnectivityQuery), responses((status=200,body=ConnectivityOverview),(status=401,description="Session required")), tag="connectivity")]
pub async fn overview(
    State(state): State<AppState>,
    RequireAuth(actor): RequireAuth,
    Query(query): Query<ConnectivityQuery>,
) -> Result<Json<ConnectivityOverview>, Error> {
    let repo = SqliteConnectivityRepository::new(state.pool.clone());
    let page = query.page.unwrap_or(1);
    let per_page = query.per_page.unwrap_or(25);
    if page == 0 || !(1..=100).contains(&per_page) {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            "Gunakan halaman mulai 1 dan ukuran halaman 1 sampai 100.",
        ));
    }
    let service_counts = repo.service_counts(actor.id).await.map_err(db_error)?;
    let total = query
        .service
        .map(|service| service_counts.get(service.as_str()).copied().unwrap_or(0))
        .unwrap_or_else(|| service_counts.values().sum());
    let mut endpoints = Vec::new();
    for endpoint in repo
        .list_page(
            actor.id,
            query.service,
            per_page,
            u64::from(page - 1) * u64::from(per_page),
        )
        .await
        .map_err(db_error)?
    {
        let evidence = repo
            .evidence(actor.id, &endpoint.id)
            .await
            .map_err(db_error)?;
        endpoints.push(EndpointSummary {
            endpoint: endpoint.into(),
            evidence: evidence.into_iter().map(Into::into).collect(),
        });
    }
    let admin = actor.role == "admin";
    let projects = sqlx::query_as::<_, ProjectOption>("SELECT s.id,COALESCE((SELECT d.name FROM domains d WHERE d.site_id=s.id ORDER BY d.is_primary DESC,d.id LIMIT 1),'Situs #' || s.id) AS name FROM sites s JOIN users u ON u.id=s.user_id WHERE u.status='active' AND s.status!='suspended' AND (? OR s.user_id=?) ORDER BY name,s.id")
        .bind(admin).bind(actor.id).fetch_all(&state.pool).await.map_err(DbError::from).map_err(db_error)?;
    let certificates = if admin {
        sqlx::query_as::<_, CertificateOption>("SELECT c.id,c.site_id,c.primary_domain AS name FROM certificates c JOIN sites s ON s.id=c.site_id JOIN users u ON u.id=s.user_id WHERE u.status='active' AND s.status!='suspended' ORDER BY c.id")
            .fetch_all(&state.pool).await.map_err(DbError::from).map_err(db_error)?
    } else {
        Vec::new()
    };
    Ok(Json(ConnectivityOverview {
        can_manage: admin,
        endpoints,
        projects,
        certificates,
        page,
        per_page,
        total,
        total_pages: (total + i64::from(per_page) - 1) / i64::from(per_page),
        service_counts,
    }))
}

#[utoipa::path(get, path="/api/v1/connectivity/{id}", params(("id"=String,Path,description="Endpoint UUID")), responses((status=200,body=EndpointDto),(status=404,body=ConnectivityProblem,content_type="application/problem+json")), tag="connectivity")]
pub async fn find(
    State(state): State<AppState>,
    RequireAuth(actor): RequireAuth,
    Path(id): Path<String>,
) -> Result<Json<EndpointDto>, Error> {
    validate_id(&id)?;
    let endpoint = SqliteConnectivityRepository::new(state.pool.clone())
        .find(actor.id, &id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| {
            problem(
                StatusCode::NOT_FOUND,
                "Endpoint tidak ditemukan atau tidak dapat diakses.",
            )
        })?;
    Ok(Json(endpoint.into()))
}

#[utoipa::path(post, path="/api/v1/connectivity", params(("Idempotency-Key"=Option<String>,Header,description="Client UUID for persistent create retries")), request_body=EndpointConfig, responses((status=200,description="Existing create replay",body=EndpointDto),(status=201,body=EndpointDto),(status=403,description="Admin/CSRF required"),(status=409,body=ConnectivityProblem,content_type="application/problem+json"),(status=422,body=ConnectivityProblem,content_type="application/problem+json")), tag="connectivity")]
pub async fn create(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    headers: HeaderMap,
    payload: Result<Json<EndpointConfig>, JsonRejection>,
) -> Result<(StatusCode, Json<EndpointDto>), Error> {
    let Json(config) = payload.map_err(json_error)?;
    let keys: Vec<_> = headers.get_all("Idempotency-Key").iter().collect();
    if keys.len() > 1 {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            "Kunci permintaan tidak valid.",
        ));
    }
    let key = keys
        .first()
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| uuid::Uuid::parse_str(value).ok())
                .ok_or_else(|| {
                    problem(
                        StatusCode::BAD_REQUEST,
                        "Kunci permintaan harus berupa UUID.",
                    )
                })
        })
        .transpose()?;
    let (endpoint, created) = SqliteConnectivityRepository::new(state.pool.clone())
        .create_once(actor.id, &config, key)
        .await
        .map_err(db_error)?;
    if created {
        audit(
            &state,
            actor.id,
            &actor.username,
            "connectivity.create",
            &endpoint.id,
        );
    }
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(endpoint.into()),
    ))
}

#[utoipa::path(put, path="/api/v1/connectivity/{id}", params(("id"=String,Path,description="Endpoint UUID")), request_body=ReplaceEndpoint, responses((status=200,body=EndpointDto),(status=403,description="Admin/CSRF required"),(status=404,body=ConnectivityProblem,content_type="application/problem+json"),(status=409,body=ConnectivityProblem,content_type="application/problem+json"),(status=422,body=ConnectivityProblem,content_type="application/problem+json")), tag="connectivity")]
pub async fn replace(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    payload: Result<Json<ReplaceEndpoint>, JsonRejection>,
) -> Result<Json<EndpointDto>, Error> {
    let Json(body) = payload.map_err(json_error)?;
    validate_id(&id)?;
    let endpoint = SqliteConnectivityRepository::new(state.pool.clone())
        .update(actor.id, &id, body.expected_revision, &body.config)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        actor.id,
        &actor.username,
        "connectivity.update",
        &id,
    );
    Ok(Json(endpoint.into()))
}

#[utoipa::path(delete, path="/api/v1/connectivity/{id}", params(("id"=String,Path,description="Endpoint UUID")), request_body=DeleteEndpoint, responses((status=204,description="Metadata deleted; no provider effects"),(status=403,description="Admin/CSRF required"),(status=409,body=ConnectivityProblem,content_type="application/problem+json")), tag="connectivity")]
pub async fn remove(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    payload: Result<Json<DeleteEndpoint>, JsonRejection>,
) -> Result<StatusCode, Error> {
    let Json(body) = payload.map_err(json_error)?;
    validate_id(&id)?;
    SqliteConnectivityRepository::new(state.pool.clone())
        .delete(actor.id, &id, body.expected_revision)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        actor.id,
        &actor.username,
        "connectivity.delete",
        &id,
    );
    Ok(StatusCode::NO_CONTENT)
}

fn audit(state: &AppState, actor_id: i64, username: &str, action: &str, id: &str) {
    state.audit_writer.log_async(
        Some(actor_id),
        username,
        action,
        format!("connectivity:{id}"),
        "success",
        None,
        None,
    );
}
#[utoipa::path(post, path="/api/v1/connectivity/{id}/diagnose", params(("id"=String,Path,description="Registered endpoint UUID")), request_body=DiagnoseEndpoint, responses((status=200,body=EndpointSummary),(status=403,description="Admin/CSRF required"),(status=409,body=ConnectivityProblem,content_type="application/problem+json")), tag="connectivity")]
pub async fn diagnose(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    payload: Result<Json<DiagnoseEndpoint>, JsonRejection>,
) -> Result<Json<EndpointSummary>, Error> {
    let Json(body) = payload.map_err(json_error)?;
    validate_id(&id)?;
    let repo = SqliteConnectivityRepository::new(state.pool.clone());
    let token = uuid::Uuid::new_v4().to_string();
    let endpoint = repo
        .claim_diagnosis(actor.id, &id, body.expected_revision, &token)
        .await
        .map_err(db_error)?;
    let reports = crate::connectivity_diagnosis::diagnose(
        &state.agent_client,
        &endpoint.config,
        endpoint.revision,
    )
    .await;
    repo.finish_diagnosis(actor.id, &id, endpoint.revision, &token, &reports)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        actor.id,
        &actor.username,
        "connectivity.diagnose",
        &id,
    );
    // Re-read metadata so a subsequent edit never displays reports as current for
    // an old configuration. The list projection recomputes revision/expiry too.
    let endpoint = repo
        .find(actor.id, &id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| problem(StatusCode::NOT_FOUND, "Endpoint telah dihapus."))?;
    let evidence = repo.evidence(actor.id, &id).await.map_err(db_error)?;
    Ok(Json(EndpointSummary {
        endpoint: endpoint.into(),
        evidence: evidence.into_iter().map(Into::into).collect(),
    }))
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/connectivity", get(overview).post(create))
        .route(
            "/api/v1/connectivity/{id}/diagnose",
            axum::routing::post(diagnose),
        )
        .route(
            "/api/v1/connectivity/{id}",
            get(find).put(replace).delete(remove),
        )
}
