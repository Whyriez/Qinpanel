//! Admin-only self-update control; the root worker survives API restarts.
use crate::{
    auth::{RequireAdmin, RequireAdminCsrf},
    routes::AppState,
};
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
use panel_proto::{PanelUpdateRequest, PanelUpdateStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;
type Error = (StatusCode, Json<Value>);
fn problem(status: StatusCode, detail: impl Into<String>) -> Error {
    (
        status,
        Json(
            json!({"type":"about:blank","title":"Pembaruan panel gagal","status":status.as_u16(),"detail":detail.into()}),
        ),
    )
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyUpdate {
    pub id: String,
    pub version: String,
    pub digest: String,
}
async fn command(
    state: &AppState,
    operation: PanelUpdateRequest,
) -> Result<Json<PanelUpdateStatus>, Error> {
    state
        .agent_client
        .panel_update(operation)
        .await
        .map(Json)
        .map_err(|error| {
            let status = match &error {
                crate::agent_client::AgentClientError::AgentError(_) => StatusCode::CONFLICT,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            // Agent returns only bounded updater diagnostics, never provider bodies.
            problem(
                status,
                match error {
                    crate::agent_client::AgentClientError::AgentError(message) => message,
                    _ => "Layanan update belum tersedia. Coba muat status kembali.".into(),
                },
            )
        })
}
#[utoipa::path(get,path="/api/v1/system/update",responses((status=200,body=PanelUpdateStatus)),tag="system")]
pub async fn overview(
    State(state): State<AppState>,
    RequireAdmin(_actor): RequireAdmin,
) -> Result<Json<PanelUpdateStatus>, Error> {
    command(&state, PanelUpdateRequest::Status).await
}
#[utoipa::path(post,path="/api/v1/system/update/check",responses((status=200,body=PanelUpdateStatus)),tag="system")]
pub async fn check(
    State(state): State<AppState>,
    RequireAdminCsrf(_actor): RequireAdminCsrf,
) -> Result<Json<PanelUpdateStatus>, Error> {
    command(&state, PanelUpdateRequest::Check).await
}
#[utoipa::path(get,path="/api/v1/system/update/logs",responses((status=200,body=PanelUpdateStatus)),tag="system")]
pub async fn logs(
    State(state): State<AppState>,
    RequireAdmin(_actor): RequireAdmin,
) -> Result<Json<PanelUpdateStatus>, Error> {
    command(&state, PanelUpdateRequest::Logs).await
}
#[utoipa::path(post,path="/api/v1/system/update/apply",request_body=ApplyUpdate,responses((status=202,body=PanelUpdateStatus),(status=409,description="Update conflict")),tag="system")]
pub async fn apply(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    body: Result<Json<ApplyUpdate>, JsonRejection>,
) -> Result<(StatusCode, Json<PanelUpdateStatus>), Error> {
    let Json(body) = body.map_err(|e| {
        problem(
            e.status(),
            "Isian pembaruan tidak valid. Periksa versi, digest paket, dan identitas permintaan.",
        )
    })?;
    panel_core::update::release_version(&body.version).map_err(|_| {
        problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Pilih versi stabil yang valid.",
        )
    })?;
    if uuid::Uuid::parse_str(&body.id).is_err() || !panel_core::update::valid_digest(&body.digest) {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Identitas atau digest update tidak valid.",
        ));
    }
    let result = command(
        &state,
        PanelUpdateRequest::Apply {
            id: body.id.clone(),
            version: body.version.clone(),
            digest: body.digest,
        },
    )
    .await?;
    state.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "panel.update.apply",
        &body.id,
        "accepted",
        Some(body.version),
        None,
    );
    Ok((StatusCode::ACCEPTED, result))
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/system/update", get(overview))
        .route("/api/v1/system/update/logs", get(logs))
        .route("/api/v1/system/update/check", post(check))
        .route("/api/v1/system/update/apply", post(apply))
}
