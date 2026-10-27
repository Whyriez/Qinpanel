//! Administrator-only encrypted archive transfer and typed migration jobs.
mod tasks;
use crate::{
    auth::middleware::{RequireAdmin, RequireAdminCsrf},
    routes::AppState,
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use panel_proto::{ServerBackupJob, ServerBackupOperation, ServerBackupRequest};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io::SeekFrom, path::PathBuf};
pub use tasks::start_observer;
pub(crate) use tasks::{apply_task_snapshot, read_task_snapshot, task_view};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

const INCOMING: &str = "/var/lib/panel/server-transfers";
const EXPORTS: &str = "/var/lib/qonpanel-migration/exports";
const LIMIT: u64 = 512 * 1024 * 1024 * 1024;
static UPLOAD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
type Error = (StatusCode, Json<Value>);
fn problem(status: StatusCode, message: &str) -> Error {
    (status, Json(json!({"detail":message})))
}
fn id(value: &str) -> Result<(), Error> {
    if !uuid::Uuid::parse_str(value).is_ok_and(|uuid| uuid.to_string() == value) {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Identitas backup tidak valid.",
        ));
    }
    Ok(())
}
fn incoming(value: &str, extension: &str) -> PathBuf {
    PathBuf::from(INCOMING).join(format!("{value}.{extension}"))
}
async fn command(state: &AppState, operation: ServerBackupRequest) -> Result<Json<Value>, Error> {
    state.agent_client.server_backup(operation).await.map(Json).map_err(|_|problem(StatusCode::CONFLICT,"Pekerjaan tidak dapat dijadwalkan. Muat status, periksa prasyarat, dan pastikan tidak ada pekerjaan lain."))
}
#[utoipa::path(get,path="/api/v1/system/server-backup",responses((status=200,description="Encrypted server backup jobs and catalog")),tag="system")]
pub async fn status(
    State(state): State<AppState>,
    RequireAdmin(_actor): RequireAdmin,
) -> Result<Json<Value>, Error> {
    let result = command(&state, ServerBackupRequest::Status).await?;
    tasks::observe(&state, &result.0, None).await;
    Ok(result)
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub id: String,
    pub operation: ServerBackupOperation,
    pub archive_id: Option<String>,
    pub digest: Option<String>,
    pub new_ipv4: Option<String>,
    pub passphrase: String,
    pub admin_password: Option<String>,
    #[serde(default)]
    pub confirm_restore: bool,
}
#[utoipa::path(post,path="/api/v1/system/server-backup/jobs",responses((status=202,description="Durable root worker accepted")),tag="system")]
pub async fn start(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Json(body): Json<Submit>,
) -> Result<(StatusCode, Json<Value>), Error> {
    id(&body.id)?;
    if !(16..=256).contains(&body.passphrase.len())
        || body
            .passphrase
            .chars()
            .any(|c| matches!(c, '\r' | '\n' | '\0'))
    {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Passphrase harus 16–256 byte tanpa baris baru.",
        ));
    }
    if !matches!(body.operation, ServerBackupOperation::Create) {
        id(body.archive_id.as_deref().unwrap_or(""))?;
    }
    if matches!(body.operation, ServerBackupOperation::Restore) {
        if !body.confirm_restore
            || !body
                .digest
                .as_deref()
                .is_some_and(panel_core::update::valid_digest)
        {
            return Err(problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Periksa arsip dan konfirmasikan restore terlebih dahulu.",
            ));
        }
        let user = panel_db::repositories::SqliteUserRepository::new(state.pool.clone())
            .find_by_id(actor.id)
            .await
            .map_err(|_| {
                problem(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Akun tidak dapat diperiksa.",
                )
            })?
            .ok_or_else(|| problem(StatusCode::UNAUTHORIZED, "Akun tidak tersedia."))?;
        let hash = user.password_hash;
        let password = body.admin_password.unwrap_or_default();
        if password.len() > 1024 {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "Password administrator tidak sesuai.",
            ));
        }
        let verified = tokio::task::spawn_blocking(move || {
            panel_core::auth::verify_password(&password, &hash)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
        if !verified {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "Password administrator tidak sesuai.",
            ));
        }
    }
    let operation = body.operation;
    let job_id = body.id.clone();
    let response = command(
        &state,
        ServerBackupRequest::Start(ServerBackupJob {
            id: body.id,
            operation,
            archive_id: body.archive_id,
            digest: body.digest,
            new_ipv4: body.new_ipv4,
            passphrase: body.passphrase,
        }),
    )
    .await?;
    tasks::observe(&state, &response.0, Some(actor.id)).await;
    state.audit_writer.log_async(
        Some(actor.id),
        &actor.username,
        "server.backup.job",
        &job_id,
        "accepted",
        Some(format!("{operation:?}")),
        None,
    );
    Ok((StatusCode::ACCEPTED, response))
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Upload {
    pub offset: u64,
    pub total: u64,
}
#[utoipa::path(put,path="/api/v1/system/server-backup/uploads/{id}",request_body(content=String,content_type="application/octet-stream"),responses((status=200,description="Bounded resumable archive chunk")),tag="system")]
pub async fn upload(
    RequireAdminCsrf(_actor): RequireAdminCsrf,
    Path(value): Path<String>,
    Query(query): Query<Upload>,
    body: Bytes,
) -> Result<Json<Value>, Error> {
    id(&value)?;
    if query.total == 0
        || query.total > LIMIT
        || body.is_empty()
        || body.len() > 8 * 1024 * 1024
        || query
            .offset
            .checked_add(body.len() as u64)
            .is_none_or(|end| end > query.total)
    {
        return Err(problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Ukuran atau posisi potongan backup tidak valid.",
        ));
    }
    let _guard = UPLOAD_LOCK.lock().await;
    tokio::fs::create_dir_all(INCOMING).await.map_err(|_| {
        problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "Direktori transfer belum tersedia.",
        )
    })?;
    let completed = incoming(&value, "qpb");
    if completed.exists() {
        let length = tokio::fs::metadata(completed)
            .await
            .map_err(|_| problem(StatusCode::CONFLICT, "Unggahan tidak tersedia."))?
            .len();
        return Ok(Json(
            json!({"offset":length,"total":length,"complete":true}),
        ));
    }
    let partial = incoming(&value, "part");
    if query.offset == 0 && !partial.exists() {
        let mut entries = tokio::fs::read_dir(INCOMING)
            .await
            .map_err(|_| problem(StatusCode::SERVICE_UNAVAILABLE, "Transfer belum tersedia."))?;
        let mut count = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "part" || ext == "qpb")
            {
                count += 1;
            }
        }
        if count >= 2 {
            return Err(problem(
                StatusCode::CONFLICT,
                "Maksimal dua unggahan. Hapus transfer lama sebelum mengunggah lagi.",
            ));
        }
    }
    if let Ok(metadata) = tokio::fs::symlink_metadata(&partial).await
        && !metadata.is_file()
    {
        return Err(problem(
            StatusCode::CONFLICT,
            "Target transfer tidak valid.",
        ));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&partial)
        .await
        .map_err(|_| {
            problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Unggahan tidak dapat disimpan.",
            )
        })?;
    let length = file
        .metadata()
        .await
        .map_err(|_| problem(StatusCode::SERVICE_UNAVAILABLE, "Unggahan tidak tersedia."))?
        .len();
    if length != query.offset {
        return Ok(Json(
            json!({"offset":length,"total":query.total,"complete":false}),
        ));
    }
    file.seek(SeekFrom::Start(query.offset))
        .await
        .map_err(|_| problem(StatusCode::SERVICE_UNAVAILABLE, "Posisi unggahan gagal."))?;
    file.write_all(&body).await.map_err(|_| {
        problem(
            StatusCode::INSUFFICIENT_STORAGE,
            "Ruang disk transfer tidak cukup.",
        )
    })?;
    file.sync_all().await.map_err(|_| {
        problem(
            StatusCode::INSUFFICIENT_STORAGE,
            "Unggahan belum tersimpan.",
        )
    })?;
    let offset = length + body.len() as u64;
    drop(file);
    if offset == query.total {
        tokio::fs::rename(partial, completed).await.map_err(|_| {
            problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Unggahan belum dapat diselesaikan.",
            )
        })?;
    }
    Ok(Json(
        json!({"offset":offset,"total":query.total,"complete":offset==query.total}),
    ))
}
pub async fn remove_upload(
    State(state): State<AppState>,
    RequireAdminCsrf(_actor): RequireAdminCsrf,
    Path(value): Path<String>,
) -> Result<StatusCode, Error> {
    id(&value)?;
    let _ = command(
        &state,
        ServerBackupRequest::DeleteUpload { id: value.clone() },
    )
    .await?;
    let _guard = UPLOAD_LOCK.lock().await;
    for ext in ["part", "qpb"] {
        match tokio::fs::remove_file(incoming(&value, ext)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(problem(
                    StatusCode::CONFLICT,
                    "Transfer belum dapat dihapus.",
                ));
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/v1/system/server-backup/archives/{id}",responses((status=200,description="Encrypted archive download")),tag="system")]
pub async fn download(
    RequireAdmin(_actor): RequireAdmin,
    Path(value): Path<String>,
) -> Result<Response, Error> {
    id(&value)?;
    let path = PathBuf::from(EXPORTS).join(format!("{value}.qpb"));
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| problem(StatusCode::NOT_FOUND, "Backup tidak ditemukan."))?;
    if !metadata.is_file() {
        return Err(problem(StatusCode::NOT_FOUND, "Backup tidak tersedia."));
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| problem(StatusCode::NOT_FOUND, "Backup tidak dapat dibaca."))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_LENGTH, metadata.len().to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"Qinpanel-backup-{value}.qpb\""),
            ),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}
pub async fn remove(
    State(state): State<AppState>,
    RequireAdminCsrf(_actor): RequireAdminCsrf,
    Path(value): Path<String>,
) -> Result<Json<Value>, Error> {
    id(&value)?;
    command(&state, ServerBackupRequest::Delete { id: value }).await
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/system/server-backup", get(status))
        .route("/api/v1/system/server-backup/jobs", post(start))
        .route(
            "/api/v1/system/server-backup/uploads/{id}",
            put(upload).delete(remove_upload),
        )
        .route(
            "/api/v1/system/server-backup/archives/{id}",
            get(download).delete(remove),
        )
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
}
