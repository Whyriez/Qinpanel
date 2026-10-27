//! File manager HTTP REST API endpoints and handlers.
//!
//! Complies with PRD Section 5.3 and Task P5-T3:
//! - Directory listing with filtering, sorting, and pagination
//! - Stat entry metadata
//! - Create directory (mkdir)
//! - Rename and move files/directories
//! - Copy files/directories
//! - Trash lifecycle: move to trash, list trash, restore from trash, empty trash
//! - Permanent deletion
//! - Tenant isolation and RBAC policy enforcement

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{
        HeaderMap, StatusCode,
        header::{self, HeaderValue},
    },
    response::{IntoResponse, Response},
};
use panel_proto::{
    ArchiveFormatDto, ArchiveResultDto, FsArchiveCompressRequest, FsArchiveExtractRequest,
    FsChmodRecursiveRequest, FsChmodRequest, FsChownRequest, FsCopyRequest, FsDeleteRequest,
    FsEntryDto, FsListRequest, FsListResponse, FsMkdirRequest, FsReadFileRequest,
    FsReadRangeRequest, FsRenameRequest, FsSearchRequest, FsStatRequest, FsTrashEmptyRequest,
    FsTrashListRequest, FsTrashRequest, FsTrashRestoreRequest, FsUploadAppendChunkRequest,
    FsUploadGetSessionRequest, FsUploadInitRequest, FsWriteEditorRequest, FsZipDirectoryRequest,
    QuotaGetRequest, TrashedItemDto, UploadMetadataDto,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{AuthenticatedUser, RequireCsrf};
use crate::routes::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FsEntryItem {
    pub name: String,
    pub relative_path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
    pub created_at: Option<i64>,
    pub mode: u32,
    pub mode_octal: String,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub mime_type: Option<String>,
}

impl From<FsEntryDto> for FsEntryItem {
    fn from(dto: FsEntryDto) -> Self {
        Self {
            name: dto.name,
            relative_path: dto.relative_path,
            is_dir: dto.is_dir,
            is_file: dto.is_file,
            is_symlink: dto.is_symlink,
            symlink_target: dto.symlink_target,
            size_bytes: dto.size_bytes,
            modified_at: dto.modified_at,
            created_at: dto.created_at,
            mode: dto.mode,
            mode_octal: dto.mode_octal,
            uid: dto.uid,
            gid: dto.gid,
            mime_type: dto.mime_type,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FsListApiResponse {
    pub entries: Vec<FsEntryItem>,
    pub total_count: usize,
    pub offset: usize,
    pub limit: usize,
    pub has_more: bool,
}

impl From<FsListResponse> for FsListApiResponse {
    fn from(res: FsListResponse) -> Self {
        Self {
            entries: res.entries.into_iter().map(Into::into).collect(),
            total_count: res.total_count,
            offset: res.offset,
            limit: res.limit,
            has_more: res.has_more,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TrashedItemApiResponse {
    pub trash_id: String,
    pub original_path: String,
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub trashed_at: i64,
}

impl From<TrashedItemDto> for TrashedItemApiResponse {
    fn from(dto: TrashedItemDto) -> Self {
        Self {
            trash_id: dto.trash_id,
            original_path: dto.original_path,
            name: dto.name,
            is_dir: dto.is_dir,
            size_bytes: dto.size_bytes,
            trashed_at: dto.trashed_at,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct FileListQuery {
    pub path: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub show_hidden: Option<bool>,
    pub search: Option<String>,
    pub extension: Option<String>,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct FileStatQuery {
    pub path: String,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MkdirPayload {
    pub path: String,
    pub recursive: Option<bool>,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RenamePayload {
    pub source: String,
    pub destination: String,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CopyPayload {
    pub source: String,
    pub destination: String,
    pub recursive: Option<bool>,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TrashPayload {
    pub path: String,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct TrashListQuery {
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TrashRestorePayload {
    pub trash_id: String,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct TrashEmptyQuery {
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeletePayload {
    pub path: String,
    pub recursive: Option<bool>,
    pub tenant: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FileActionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TrashEmptyApiResponse {
    pub success: bool,
    pub purged_count: usize,
    pub bytes_freed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UploadMetadataApiResponse {
    pub upload_id: String,
    pub file_name: String,
    pub target_path: String,
    pub total_size: u64,
    pub current_offset: u64,
    pub created_at: i64,
    pub completed: bool,
}

impl From<UploadMetadataDto> for UploadMetadataApiResponse {
    fn from(dto: UploadMetadataDto) -> Self {
        Self {
            upload_id: dto.upload_id,
            file_name: dto.file_name,
            target_path: dto.target_path,
            total_size: dto.total_size,
            current_offset: dto.current_offset,
            created_at: dto.created_at,
            completed: dto.completed,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UploadInitPayload {
    pub target_path: String,
    pub file_name: String,
    pub total_size: u64,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct UploadStatusQuery {
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct UploadChunkQuery {
    pub offset: u64,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DownloadFileQuery {
    pub path: String,
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DownloadZipQuery {
    pub path: String,
    pub tenant: Option<String>,
}

async fn resolve_tenant(
    state: &AppState,
    user: &AuthenticatedUser,
    requested_tenant: Option<&str>,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    Ok(
        crate::tenant_context::resolve_tenant_context(state, user, requested_tenant, None)
            .await?
            .linux_username,
    )
}

/// List files and directories within a confined folder.
#[utoipa::path(
    get,
    path = "/api/v1/files/list",
    params(FileListQuery),
    responses(
        (status = 200, description = "Directory listing result", body = FsListApiResponse),
        (status = 400, description = "Invalid path or parameters"),
        (status = 403, description = "Forbidden")
    ),
    tag = "files"
)]
pub async fn list_files_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<FileListQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;

    let req = FsListRequest {
        username: tenant,
        path: query.path.unwrap_or_default(),
        limit: query.limit.unwrap_or(100),
        offset: query.offset.unwrap_or(0),
        sort_by: query.sort_by.unwrap_or_else(|| "name".to_string()),
        sort_order: query.sort_order.unwrap_or_else(|| "asc".to_string()),
        show_hidden: query.show_hidden.unwrap_or(false),
        search_query: query.search,
        extension: query.extension,
    };

    match state.agent_client.fs_list(req).await {
        Ok(res) => Ok((StatusCode::OK, Json(FsListApiResponse::from(res)))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Retrieve detailed metadata for a file or directory.
#[utoipa::path(
    get,
    path = "/api/v1/files/stat",
    params(FileStatQuery),
    responses(
        (status = 200, description = "File metadata", body = FsEntryItem),
        (status = 404, description = "File not found"),
        (status = 403, description = "Forbidden")
    ),
    tag = "files"
)]
pub async fn stat_file_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<FileStatQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;

    let req = FsStatRequest {
        username: tenant,
        path: query.path,
    };

    match state.agent_client.fs_stat(req).await {
        Ok(res) => Ok((StatusCode::OK, Json(FsEntryItem::from(res.entry)))),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Create a new directory within the tenant's root jail.
#[utoipa::path(
    post,
    path = "/api/v1/files/mkdir",
    request_body = MkdirPayload,
    responses(
        (status = 201, description = "Directory created", body = FileActionResponse),
        (status = 400, description = "Failed to create directory")
    ),
    tag = "files"
)]
pub async fn mkdir_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<MkdirPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    let req = FsMkdirRequest {
        username: tenant.clone(),
        path: payload.path.clone(),
        recursive: payload.recursive.unwrap_or(true),
    };

    match state.agent_client.fs_mkdir(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.mkdir",
                format!("{}:{}", tenant, payload.path),
                "success",
                None,
                Some(serde_json::json!({ "recursive": payload.recursive }).to_string()),
            );
            Ok((
                StatusCode::CREATED,
                Json(FileActionResponse {
                    success: res.success,
                    message: res.message,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Rename or move a file or directory.
#[utoipa::path(
    post,
    path = "/api/v1/files/rename",
    request_body = RenamePayload,
    responses(
        (status = 200, description = "Renamed successfully", body = FileActionResponse),
        (status = 400, description = "Failed to rename")
    ),
    tag = "files"
)]
pub async fn rename_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<RenamePayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    let req = FsRenameRequest {
        username: tenant.clone(),
        source: payload.source.clone(),
        destination: payload.destination.clone(),
    };

    match state.agent_client.fs_rename(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.rename",
                format!("{}:{} -> {}", tenant, payload.source, payload.destination),
                "success",
                None,
                Some(
                    serde_json::json!({
                        "source": payload.source,
                        "destination": payload.destination
                    })
                    .to_string(),
                ),
            );
            Ok((
                StatusCode::OK,
                Json(FileActionResponse {
                    success: res.success,
                    message: res.message,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Copy a file or directory within the tenant's root jail.
#[utoipa::path(
    post,
    path = "/api/v1/files/copy",
    request_body = CopyPayload,
    responses(
        (status = 200, description = "Copied successfully", body = FileActionResponse),
        (status = 400, description = "Failed to copy")
    ),
    tag = "files"
)]
pub async fn copy_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<CopyPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    let req = FsCopyRequest {
        username: tenant.clone(),
        source: payload.source.clone(),
        destination: payload.destination.clone(),
        recursive: payload.recursive.unwrap_or(true),
    };

    match state.agent_client.fs_copy(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.copy",
                format!("{}:{} -> {}", tenant, payload.source, payload.destination),
                "success",
                None,
                Some(
                    serde_json::json!({
                        "source": payload.source,
                        "destination": payload.destination
                    })
                    .to_string(),
                ),
            );
            Ok((
                StatusCode::OK,
                Json(FileActionResponse {
                    success: res.success,
                    message: res.message,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Move a file or directory into `.panel-trash`.
#[utoipa::path(
    post,
    path = "/api/v1/files/trash",
    request_body = TrashPayload,
    responses(
        (status = 200, description = "Moved to trash", body = TrashedItemApiResponse),
        (status = 400, description = "Failed to move to trash")
    ),
    tag = "files"
)]
pub async fn trash_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<TrashPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    let req = FsTrashRequest {
        username: tenant.clone(),
        path: payload.path.clone(),
    };

    match state.agent_client.fs_trash(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.trash",
                format!("{}:{}", tenant, payload.path),
                "success",
                None,
                Some(serde_json::json!({ "trash_id": res.item.trash_id }).to_string()),
            );
            Ok((StatusCode::OK, Json(TrashedItemApiResponse::from(res.item))))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// List all items currently in `.panel-trash`.
#[utoipa::path(
    get,
    path = "/api/v1/files/trash",
    params(TrashListQuery),
    responses(
        (status = 200, description = "List of trashed items", body = Vec<TrashedItemApiResponse>),
        (status = 400, description = "Failed to list trash")
    ),
    tag = "files"
)]
pub async fn list_trash_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<TrashListQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;

    let req = FsTrashListRequest { username: tenant };

    match state.agent_client.fs_trash_list(req).await {
        Ok(res) => {
            let items: Vec<TrashedItemApiResponse> =
                res.items.into_iter().map(Into::into).collect();
            Ok((StatusCode::OK, Json(items)))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Restore a trashed item back to its original location.
#[utoipa::path(
    post,
    path = "/api/v1/files/trash/restore",
    request_body = TrashRestorePayload,
    responses(
        (status = 200, description = "Restored successfully", body = TrashedItemApiResponse),
        (status = 400, description = "Failed to restore")
    ),
    tag = "files"
)]
pub async fn restore_trash_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<TrashRestorePayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    let req = FsTrashRestoreRequest {
        username: tenant.clone(),
        trash_id: payload.trash_id.clone(),
    };

    match state.agent_client.fs_trash_restore(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.trash_restore",
                format!("{}:{}", tenant, payload.trash_id),
                "success",
                None,
                Some(serde_json::json!({ "original_path": res.item.original_path }).to_string()),
            );
            Ok((StatusCode::OK, Json(TrashedItemApiResponse::from(res.item))))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Permanently empty `.panel-trash`, purging all files.
#[utoipa::path(
    delete,
    path = "/api/v1/files/trash/empty",
    params(TrashEmptyQuery),
    responses(
        (status = 200, description = "Trash emptied", body = TrashEmptyApiResponse),
        (status = 400, description = "Failed to empty trash")
    ),
    tag = "files"
)]
pub async fn empty_trash_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<TrashEmptyQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;

    let req = FsTrashEmptyRequest {
        username: tenant.clone(),
    };

    match state.agent_client.fs_trash_empty(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.trash_empty",
                &tenant,
                "success",
                None,
                Some(
                    serde_json::json!({
                        "purged_count": res.purged_count,
                        "bytes_freed": res.bytes_freed
                    })
                    .to_string(),
                ),
            );
            Ok((
                StatusCode::OK,
                Json(TrashEmptyApiResponse {
                    success: true,
                    purged_count: res.purged_count,
                    bytes_freed: res.bytes_freed,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Permanently delete a file or directory without moving to trash.
#[utoipa::path(
    delete,
    path = "/api/v1/files/permanent",
    request_body = DeletePayload,
    responses(
        (status = 200, description = "Deleted permanently", body = FileActionResponse),
        (status = 400, description = "Failed to delete")
    ),
    tag = "files"
)]
pub async fn delete_permanent_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<DeletePayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    let req = FsDeleteRequest {
        username: tenant.clone(),
        path: payload.path.clone(),
        recursive: payload.recursive.unwrap_or(false),
    };

    match state.agent_client.fs_delete(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.delete_permanent",
                format!("{}:{}", tenant, payload.path),
                "success",
                None,
                Some(serde_json::json!({ "recursive": payload.recursive }).to_string()),
            );
            Ok((
                StatusCode::OK,
                Json(FileActionResponse {
                    success: res.success,
                    message: res.message,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Initialize a chunked resumable upload session with pre-flight quota check.
#[utoipa::path(
    post,
    path = "/api/v1/files/upload/init",
    request_body = UploadInitPayload,
    responses(
        (status = 201, description = "Upload session initialized", body = UploadMetadataApiResponse),
        (status = 400, description = "Failed to initialize upload session"),
        (status = 413, description = "Pre-flight quota exceeded")
    ),
    tag = "files"
)]
pub async fn upload_init_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<UploadInitPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let context = crate::tenant_context::resolve_tenant_context(
        &state,
        &auth_user,
        payload.tenant.as_deref(),
        None,
    )
    .await?;
    let tenant = context.linux_username;
    let uid = context.uid;
    {
        let quota_res = state
            .agent_client
            .get_quota(QuotaGetRequest {
                uid,
                username: tenant.clone(),
                mount_point: None,
            })
            .await;

        if let Ok(quota) = quota_res {
            let exceeds_limit = quota.bytes_limit > 0
                && quota.bytes_used.saturating_add(payload.total_size) > quota.bytes_limit;
            if exceeds_limit {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Json(serde_json::json!({
                        "error": format!(
                            "Pre-flight quota exceeded: file size ({} bytes) + used space ({} bytes) exceeds disk limit ({} bytes)",
                            payload.total_size, quota.bytes_used, quota.bytes_limit
                        )
                    })),
                ));
            }
        }
    }

    let req = FsUploadInitRequest {
        username: tenant,
        target_path: payload.target_path,
        file_name: payload.file_name,
        total_size: payload.total_size,
    };

    match state.agent_client.fs_upload_init(req).await {
        Ok(res) => Ok((
            StatusCode::CREATED,
            Json(UploadMetadataApiResponse::from(res.metadata)),
        )),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Get status and current offset of an upload session.
#[utoipa::path(
    get,
    path = "/api/v1/files/upload/{upload_id}",
    params(
        ("upload_id" = String, Path, description = "Upload session UUID"),
        UploadStatusQuery
    ),
    responses(
        (status = 200, description = "Upload session status retrieved", body = UploadMetadataApiResponse),
        (status = 404, description = "Upload session not found")
    ),
    tag = "files"
)]
pub async fn upload_status_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(upload_id): Path<String>,
    Query(query): Query<UploadStatusQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;

    let req = FsUploadGetSessionRequest {
        username: tenant,
        upload_id,
    };

    match state.agent_client.fs_upload_get_session(req).await {
        Ok(res) => Ok((
            StatusCode::OK,
            Json(UploadMetadataApiResponse::from(res.metadata)),
        )),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Append a binary chunk to an upload session.
#[utoipa::path(
    post,
    path = "/api/v1/files/upload/{upload_id}/chunk",
    params(
        ("upload_id" = String, Path, description = "Upload session UUID"),
        UploadChunkQuery
    ),
    request_body(content = String, content_type = "application/octet-stream", description = "Raw binary chunk payload"),
    responses(
        (status = 200, description = "Chunk appended successfully", body = UploadMetadataApiResponse),
        (status = 409, description = "Offset mismatch conflict"),
        (status = 400, description = "Failed to append chunk")
    ),
    tag = "files"
)]
pub async fn upload_chunk_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(upload_id): Path<String>,
    Query(query): Query<UploadChunkQuery>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;

    let req = FsUploadAppendChunkRequest {
        username: tenant.clone(),
        upload_id,
        offset: query.offset,
        data: body.to_vec(),
    };

    match state.agent_client.fs_upload_append_chunk(req).await {
        Ok(res) => {
            if res.metadata.completed {
                state.audit_writer.log_async(
                    Some(auth_user.id),
                    &auth_user.username,
                    "file.upload",
                    format!("{}:{}", tenant, res.metadata.target_path),
                    "success",
                    None,
                    Some(
                        serde_json::json!({
                            "total_size": res.metadata.total_size,
                            "file_name": res.metadata.file_name
                        })
                        .to_string(),
                    ),
                );
            }
            Ok((
                StatusCode::OK,
                Json(UploadMetadataApiResponse::from(res.metadata)),
            ))
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.to_lowercase().contains("offset mismatch") {
                Err((
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": err_str })),
                ))
            } else {
                Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": err_str })),
                ))
            }
        }
    }
}

/// Download a file with HTTP Range and ETag support.
#[utoipa::path(
    get,
    path = "/api/v1/files/download",
    params(DownloadFileQuery),
    responses(
        (status = 200, description = "File payload stream"),
        (status = 206, description = "Partial content range stream"),
        (status = 304, description = "Not modified ETag match"),
        (status = 404, description = "File not found")
    ),
    tag = "files"
)]
pub async fn download_file_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<DownloadFileQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;
    let stat = state
        .agent_client
        .fs_stat(FsStatRequest {
            username: tenant.clone(),
            path: query.path.clone(),
        })
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error":format!("File not found: {e}")})),
            )
        })?;
    if !stat.entry.is_file {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"Only regular files can be downloaded directly"})),
        ));
    }
    let size = stat.entry.size_bytes;
    let mime = stat
        .entry
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".into());
    let etag = format!("\"{:x}-{:x}\"", stat.entry.modified_at.unwrap_or(0), size);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str())
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        private_file_headers(response.headers_mut());
        if let Ok(value) = HeaderValue::from_str(&etag) {
            response.headers_mut().insert(header::ETAG, value);
        }
        return Ok(response);
    }
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes="));
    let (start, length, status) = if let Some(spec) = range {
        match download_range(spec, size) {
            Some((start, end)) => (start, end - start + 1, StatusCode::PARTIAL_CONTENT),
            None => {
                let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                private_file_headers(response.headers_mut());
                if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                    response.headers_mut().insert(header::CONTENT_RANGE, value);
                }
                return Ok(response);
            }
        }
    } else {
        (0, size, StatusCode::OK)
    };
    let name = std::path::Path::new(&query.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let body = file_download_body(
        std::sync::Arc::clone(&state.agent_client),
        tenant,
        query.path,
        start,
        length,
    )
    .await?;
    let mut response = (status, body).into_response();
    let output = response.headers_mut();
    private_file_headers(output);
    output.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(value) = HeaderValue::from_str(&etag) {
        output.insert(header::ETAG, value);
    }
    if let Ok(value) = HeaderValue::from_str(&mime) {
        output.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
        output.insert(header::CONTENT_LENGTH, value);
    }
    if status == StatusCode::PARTIAL_CONTENT
        && let Ok(value) =
            HeaderValue::from_str(&format!("bytes {start}-{}/{}", start + length - 1, size))
    {
        output.insert(header::CONTENT_RANGE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        output.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

fn private_file_headers(headers: &mut HeaderMap) {
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

fn download_range(spec: &str, size: u64) -> Option<(u64, u64)> {
    if size == 0 || spec.contains(',') {
        return None;
    }
    let (first, last) = spec.split_once('-')?;
    if first.is_empty() {
        let suffix = last.parse::<u64>().ok().filter(|value| *value > 0)?;
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start = first.parse::<u64>().ok().filter(|start| *start < size)?;
    let end = if last.is_empty() {
        size - 1
    } else {
        last.parse::<u64>().ok()?.min(size - 1)
    };
    (end >= start).then_some((start, end))
}

pub const FILE_DOWNLOAD_CHUNK: u64 = 1024 * 1024;

async fn file_download_body(
    client: std::sync::Arc<crate::agent_client::AgentClient>,
    tenant: String,
    path: String,
    start: u64,
    length: u64,
) -> Result<axum::body::Body, (StatusCode, Json<serde_json::Value>)> {
    // Prefetch under the tenant identity before sending success headers, then honor HTTP
    // backpressure. Neither the agent transport nor API buffers the complete download.
    let first = client
        .fs_read_range(FsReadRangeRequest {
            username: tenant.clone(),
            path: path.clone(),
            start,
            length: length.min(FILE_DOWNLOAD_CHUNK),
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?
        .data;
    let stream = futures_util::stream::try_unfold(
        (client, tenant, path, start, length, Some(first)),
        |(client, tenant, path, next, remaining, first)| async move {
            if remaining == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            let count = remaining.min(FILE_DOWNLOAD_CHUNK);
            let bytes = if let Some(bytes) = first {
                bytes
            } else {
                client
                    .fs_read_range(FsReadRangeRequest {
                        username: tenant.clone(),
                        path: path.clone(),
                        start: next,
                        length: count,
                    })
                    .await
                    .map_err(std::io::Error::other)?
                    .data
            };
            if bytes.len() as u64 != count {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "File changed during download",
                ));
            }
            Ok(Some((
                bytes,
                (client, tenant, path, next + count, remaining - count, None),
            )))
        },
    );
    Ok(axum::body::Body::from_stream(stream))
}

/// Download a ZIP archive (generation is bounded by worker IPC, not a streaming snapshot).
#[utoipa::path(
    get,
    path = "/api/v1/files/download-zip",
    params(DownloadZipQuery),
    responses(
        (status = 200, description = "ZIP archive stream"),
        (status = 400, description = "Failed to archive directory")
    ),
    tag = "files"
)]
pub async fn download_zip_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<DownloadZipQuery>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;

    let folder_name = {
        let trimmed = query.path.trim_matches('/');
        if trimmed.is_empty() {
            "root-archive".to_string()
        } else {
            std::path::Path::new(trimmed)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("archive")
                .to_string()
        }
    };

    let data = state
        .agent_client
        .fs_zip_directory(FsZipDirectoryRequest {
            username: tenant,
            path: query.path,
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?
        .data;

    let mut res = (StatusCode::OK, data).into_response();
    let headers_mut = res.headers_mut();
    private_file_headers(headers_mut);
    headers_mut.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    let disp = format!("attachment; filename=\"{}.zip\"", folder_name);
    if let Ok(disp_val) = HeaderValue::from_str(&disp) {
        headers_mut.insert(header::CONTENT_DISPOSITION, disp_val);
    }
    Ok(res)
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ArchiveCompressPayload {
    pub source_paths: Vec<String>,
    pub destination: String,
    pub format: Option<String>,
    pub tenant: Option<String>,
    pub background: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ArchiveExtractPayload {
    pub archive_path: String,
    pub destination_dir: String,
    pub format: Option<String>,
    pub tenant: Option<String>,
    pub background: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ArchiveResultApiResponse {
    pub entries_processed: usize,
    pub total_bytes: u64,
    pub target_path: String,
}

impl From<ArchiveResultDto> for ArchiveResultApiResponse {
    fn from(dto: ArchiveResultDto) -> Self {
        Self {
            entries_processed: dto.entries_processed,
            total_bytes: dto.total_bytes,
            target_path: dto.target_path,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ArchiveActionResponse {
    pub success: bool,
    pub message: String,
    pub result: Option<ArchiveResultApiResponse>,
    pub job_id: Option<String>,
}

/// Compress files or directories into an archive (ZIP / TAR.GZ).
#[utoipa::path(
    post,
    path = "/api/v1/files/archive/compress",
    request_body = ArchiveCompressPayload,
    responses(
        (status = 200, description = "Archive compressed successfully", body = ArchiveActionResponse),
        (status = 202, description = "Archive compression enqueued as background job", body = ArchiveActionResponse),
        (status = 400, description = "Invalid request or compression failure")
    ),
    tag = "files"
)]
pub async fn archive_compress_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<ArchiveCompressPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    if payload.source_paths.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "source_paths cannot be empty" })),
        ));
    }

    let format_dto = match payload.format.as_deref() {
        Some("targz") | Some("tar.gz") | Some("tgz") => ArchiveFormatDto::TarGz,
        Some("zip") => ArchiveFormatDto::Zip,
        _ => {
            if payload.destination.ends_with(".tar.gz") || payload.destination.ends_with(".tgz") {
                ArchiveFormatDto::TarGz
            } else {
                ArchiveFormatDto::Zip
            }
        }
    };

    if payload.background == Some(true) {
        let job_payload = serde_json::json!({
            "username": tenant,
            "source_paths": payload.source_paths,
            "destination": payload.destination,
            "format": match format_dto {
                ArchiveFormatDto::Zip => "zip",
                ArchiveFormatDto::TarGz => "tar.gz",
            },
        });

        let (job, _) = state
            .job_runner
            .enqueue(
                Some(auth_user.id),
                None,
                "files.archive.compress".to_string(),
                job_payload.to_string(),
                Some(3),
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed to enqueue archive job: {e}") })),
                )
            })?;

        state.audit_writer.log_async(
            Some(auth_user.id),
            &auth_user.username,
            "file.archive_compress",
            format!("{}:{}", tenant, payload.destination),
            "enqueued",
            None,
            Some(serde_json::json!({ "job_id": job.id }).to_string()),
        );

        return Ok((
            StatusCode::ACCEPTED,
            Json(ArchiveActionResponse {
                success: true,
                message: "Archive compression job enqueued".to_string(),
                result: None,
                job_id: Some(job.id),
            }),
        ));
    }

    let req = FsArchiveCompressRequest {
        username: tenant.clone(),
        source_paths: payload.source_paths.clone(),
        destination: payload.destination.clone(),
        format: format_dto,
    };

    match state.agent_client.fs_archive_compress(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.archive_compress",
                format!("{}:{}", tenant, payload.destination),
                "success",
                None,
                Some(serde_json::to_string(&res.result).unwrap_or_default()),
            );
            Ok((
                StatusCode::OK,
                Json(ArchiveActionResponse {
                    success: true,
                    message: "Archive created successfully".to_string(),
                    result: Some(ArchiveResultApiResponse::from(res.result)),
                    job_id: None,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Safely extract an archive file with anti-zip-slip, anti-symlink, and anti-zip-bomb guardrails.
#[utoipa::path(
    post,
    path = "/api/v1/files/archive/extract",
    request_body = ArchiveExtractPayload,
    responses(
        (status = 200, description = "Archive extracted successfully", body = ArchiveActionResponse),
        (status = 202, description = "Archive extraction enqueued as background job", body = ArchiveActionResponse),
        (status = 400, description = "Invalid request or extraction failure")
    ),
    tag = "files"
)]
pub async fn archive_extract_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<ArchiveExtractPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    let format_dto = match payload.format.as_deref() {
        Some("zip") => Some(ArchiveFormatDto::Zip),
        Some("targz") | Some("tar.gz") | Some("tgz") => Some(ArchiveFormatDto::TarGz),
        _ => None,
    };

    // Pre-flight check file size: if > 10MB or background requested -> background job
    let stat_res = state
        .agent_client
        .fs_stat(FsStatRequest {
            username: tenant.clone(),
            path: payload.archive_path.clone(),
        })
        .await;

    let archive_size = stat_res.map(|r| r.entry.size_bytes).unwrap_or(0);
    let should_background = payload.background == Some(true) || archive_size > 10 * 1024 * 1024;

    if should_background {
        let job_payload = serde_json::json!({
            "username": tenant,
            "archive_path": payload.archive_path,
            "destination_dir": payload.destination_dir,
            "format": payload.format,
        });

        let (job, _) = state
            .job_runner
            .enqueue(
                Some(auth_user.id),
                None,
                "files.archive.extract".to_string(),
                job_payload.to_string(),
                Some(3),
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed to enqueue extraction job: {e}") })),
                )
            })?;

        state.audit_writer.log_async(
            Some(auth_user.id),
            &auth_user.username,
            "file.archive_extract",
            format!("{}:{}", tenant, payload.archive_path),
            "enqueued",
            None,
            Some(serde_json::json!({ "job_id": job.id, "size_bytes": archive_size }).to_string()),
        );

        return Ok((
            StatusCode::ACCEPTED,
            Json(ArchiveActionResponse {
                success: true,
                message: "Archive extraction job enqueued".to_string(),
                result: None,
                job_id: Some(job.id),
            }),
        ));
    }

    let req = FsArchiveExtractRequest {
        username: tenant.clone(),
        archive_path: payload.archive_path.clone(),
        destination_dir: payload.destination_dir.clone(),
        format: format_dto,
    };

    match state.agent_client.fs_archive_extract(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.archive_extract",
                format!("{}:{}", tenant, payload.archive_path),
                "success",
                None,
                Some(serde_json::to_string(&res.result).unwrap_or_default()),
            );
            Ok((
                StatusCode::OK,
                Json(ArchiveActionResponse {
                    success: true,
                    message: "Archive extracted successfully".to_string(),
                    result: Some(ArchiveResultApiResponse::from(res.result)),
                    job_id: None,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChmodPayload {
    pub path: String,
    pub mode: String,
    pub tenant: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChmodApiResponse {
    pub path: String,
    pub mode: u32,
    pub mode_octal: String,
    pub warning: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChmodRecursivePayload {
    pub path: String,
    pub file_mode: String,
    pub dir_mode: String,
    pub tenant: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChmodRecursiveApiResponse {
    pub path: String,
    pub file_mode: u32,
    pub file_mode_octal: String,
    pub dir_mode: u32,
    pub dir_mode_octal: String,
    pub affected_count: usize,
    pub warning: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChownPayload {
    pub path: String,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub tenant: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChownApiResponse {
    pub path: String,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

fn parse_octal_mode(input: &str) -> Result<u32, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Mode string tidak boleh kosong".to_string());
    }
    let raw = trimmed.strip_prefix("0o").unwrap_or(trimmed);
    let val = u32::from_str_radix(raw, 8)
        .map_err(|e| format!("Format oktal '{trimmed}' tidak valid: {e}"))?;
    if val > 0o7777 {
        return Err(format!("Mode oktal '{trimmed}' melebihi batas POSIX 07777"));
    }
    Ok(val)
}

/// Change permissions (chmod) on a single file or directory.
#[utoipa::path(
    post,
    path = "/api/v1/files/chmod",
    request_body = ChmodPayload,
    responses(
        (status = 200, description = "Permissions updated successfully", body = ChmodApiResponse),
        (status = 400, description = "Invalid mode or path error")
    ),
    tag = "files"
)]
pub async fn chmod_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<ChmodPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;
    let mode = parse_octal_mode(&payload.mode).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Mode oktal tidak valid: {e}") })),
        )
    })?;

    let req = FsChmodRequest {
        username: tenant.clone(),
        path: payload.path.clone(),
        mode,
    };

    match state.agent_client.fs_chmod(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.chmod",
                format!("{}:{}:{:04o}", tenant, payload.path, mode),
                "success",
                None,
                res.warning
                    .as_ref()
                    .map(|w| serde_json::json!({ "warning": w }).to_string()),
            );
            Ok((
                StatusCode::OK,
                Json(ChmodApiResponse {
                    path: res.path,
                    mode: res.mode,
                    mode_octal: res.mode_octal,
                    warning: res.warning,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Recursively change permissions on files and directories with separate modes.
#[utoipa::path(
    post,
    path = "/api/v1/files/chmod-recursive",
    request_body = ChmodRecursivePayload,
    responses(
        (status = 200, description = "Recursive permissions updated successfully", body = ChmodRecursiveApiResponse),
        (status = 400, description = "Invalid mode or path error")
    ),
    tag = "files"
)]
pub async fn chmod_recursive_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<ChmodRecursivePayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;
    let file_mode = parse_octal_mode(&payload.file_mode).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Mode file oktal tidak valid: {e}") })),
        )
    })?;
    let dir_mode = parse_octal_mode(&payload.dir_mode).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Mode folder oktal tidak valid: {e}") })),
        )
    })?;

    let req = FsChmodRecursiveRequest {
        username: tenant.clone(),
        path: payload.path.clone(),
        file_mode,
        dir_mode,
    };

    match state.agent_client.fs_chmod_recursive(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.chmod_recursive",
                format!(
                    "{}:{}:{:04o}:{:04o}",
                    tenant, payload.path, file_mode, dir_mode
                ),
                "success",
                None,
                res.warning
                    .as_ref()
                    .map(|w| serde_json::json!({ "warning": w }).to_string()),
            );
            Ok((
                StatusCode::OK,
                Json(ChmodRecursiveApiResponse {
                    path: res.path,
                    file_mode: res.file_mode,
                    file_mode_octal: res.file_mode_octal,
                    dir_mode: res.dir_mode,
                    dir_mode_octal: res.dir_mode_octal,
                    affected_count: res.affected_count,
                    warning: res.warning,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Change ownership of a file or directory (restricted, tenants cannot change owner).
#[utoipa::path(
    post,
    path = "/api/v1/files/chown",
    request_body = ChownPayload,
    responses(
        (status = 200, description = "Ownership updated successfully", body = ChownApiResponse),
        (status = 403, description = "Tenants are not allowed to change file ownership"),
        (status = 400, description = "Failed to change ownership")
    ),
    tag = "files"
)]
pub async fn chown_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<ChownPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if !auth_user.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Penyewa tidak diizinkan mengubah pemilik berkas (chown dibatasi)"
            })),
        ));
    }

    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    let req = FsChownRequest {
        username: tenant.clone(),
        path: payload.path.clone(),
        uid: payload.uid,
        gid: payload.gid,
        caller_uid: auth_user.id as u32,
        is_admin: auth_user.is_admin(),
    };

    match state.agent_client.fs_chown(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "file.chown",
                format!(
                    "{}:{}:{:?}:{:?}",
                    tenant, payload.path, payload.uid, payload.gid
                ),
                "success",
                None,
                None,
            );
            Ok((
                StatusCode::OK,
                Json(ChownApiResponse {
                    path: res.path,
                    uid: res.uid,
                    gid: res.gid,
                }),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

pub const MAX_EDITOR_FILE_SIZE: u64 = panel_core::file_revision::MAX_EDITOR_CONTENT;

#[derive(Debug, Deserialize, IntoParams)]
pub struct FileContentQuery {
    pub path: String,
    pub tenant: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FileContentApiResponse {
    pub path: String,
    pub content: String,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
    pub etag: String,
    pub read_only: bool,
    pub is_binary: bool,
    pub mime_type: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FileSavePayload {
    pub path: String,
    pub content: String,
    pub expected_mtime: Option<i64>,
    pub expected_etag: Option<String>,
    pub force: Option<bool>,
    pub tenant: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FileSaveApiResponse {
    pub path: String,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
    pub etag: String,
    pub message: String,
}

/// Read text content of a file for editor preview with size limit guardrails.
#[utoipa::path(
    get,
    path = "/api/v1/files/content",
    params(FileContentQuery),
    responses(
        (status = 200, description = "File content retrieved successfully", body = FileContentApiResponse),
        (status = 404, description = "File not found"),
        (status = 400, description = "Cannot read directory as file")
    ),
    tag = "files"
)]
pub async fn get_file_content_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<FileContentQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, query.tenant.as_deref()).await?;

    let stat_req = FsStatRequest {
        username: tenant.clone(),
        path: query.path.clone(),
    };

    let stat_res = state.agent_client.fs_stat(stat_req).await.map_err(|e| {
        let err_str = e.to_string();
        if err_str.contains("Path traversal")
            || err_str.contains("outside")
            || err_str.contains("forbidden")
            || err_str.contains("prohibited")
            || err_str.contains("Permission denied")
            || err_str.contains("Symlink")
        {
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": format!("Akses ditolak: {e}") })),
            )
        } else {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Berkas tidak ditemukan: {e}") })),
            )
        }
    })?;

    if !stat_res.entry.is_file {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Target bukan berkas reguler" })),
        ));
    }

    let read_only = stat_res.entry.size_bytes > MAX_EDITOR_FILE_SIZE;
    let data = if read_only {
        Vec::new()
    } else {
        state
            .agent_client
            .fs_read_file(FsReadFileRequest {
                username: tenant,
                path: query.path.clone(),
                max_bytes: Some(MAX_EDITOR_FILE_SIZE + 1),
            })
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("Gagal membaca berkas: {e}") })),
                )
            })?
            .data
    };
    let read_only = read_only || data.len() as u64 > MAX_EDITOR_FILE_SIZE;
    let etag = if read_only {
        String::new()
    } else {
        panel_core::file_revision::content_etag(&data)
    };

    let (mut content, is_binary) = match std::str::from_utf8(&data) {
        Ok(valid_text) => (valid_text.to_string(), false),
        Err(_) => (String::new(), true),
    };
    if read_only {
        content.clear();
    }

    Ok((
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        Json(FileContentApiResponse {
            path: query.path,
            content,
            size_bytes: stat_res.entry.size_bytes,
            modified_at: stat_res.entry.modified_at,
            etag,
            read_only,
            is_binary,
            mime_type: stat_res.entry.mime_type,
        }),
    ))
}

/// Save file content with content revisions checked in the serialized tenant worker.
#[utoipa::path(
    put,
    path = "/api/v1/files/content",
    request_body = FileSavePayload,
    responses(
        (status = 200, description = "File saved successfully", body = FileSaveApiResponse),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Invalid CSRF token or forbidden tenant"),
        (status = 409, description = "Conflict detected (file changed on server)"),
        (status = 428, description = "A current content revision is required"),
        (status = 413, description = "Editor size limit exceeded"),
        (status = 400, description = "Failed to save file")
    ),
    tag = "files"
)]
pub async fn save_file_content_handler(
    State(state): State<AppState>,
    RequireCsrf(auth_user): RequireCsrf,
    Json(payload): Json<FileSavePayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;

    if payload.content.len() as u64 > MAX_EDITOR_FILE_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error":"Batas editor adalah 5 MiB."})),
        ));
    }
    let expected_etag = if payload.force == Some(true) {
        None
    } else {
        let revision = payload.expected_etag.as_deref().filter(|value| panel_core::file_revision::is_content_etag(value))
            .ok_or_else(|| (StatusCode::PRECONDITION_REQUIRED, Json(serde_json::json!({"error":"Buka ulang berkas untuk memperoleh revisi sebelum menyimpan."}))))?;
        Some(revision.to_owned())
    };
    // A failed stat must never silently turn an edit into an unconditional create/write.
    let current = state
        .agent_client
        .fs_stat(FsStatRequest {
            username: tenant.clone(),
            path: payload.path.clone(),
        })
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":format!("Berkas tidak dapat diperiksa: {e}")})),
            )
        })?;
    if !current.entry.is_file {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"Target bukan berkas reguler."})),
        ));
    }
    if current.entry.size_bytes > MAX_EDITOR_FILE_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({"error":"Berkas melebihi batas editor dan hanya dapat dibaca."}),
            ),
        ));
    }
    let new_etag = panel_core::file_revision::content_etag(payload.content.as_bytes());
    let write_req = FsWriteEditorRequest {
        username: tenant.clone(),
        path: payload.path.clone(),
        data: payload.content.into_bytes(),
        expected_etag,
    };

    let write_res = state
        .agent_client
        .fs_write_editor(write_req)
        .await
        .map_err(|e| {
            if e.to_string().contains("File content conflict:") {
                // Compatibility metadata only; the worker's content revision is
                // authoritative, never this preflight timestamp.
                (StatusCode::CONFLICT, Json(serde_json::json!({"error":"Konflik penyimpanan: Berkas telah dimodifikasi oleh proses lain di server sejak Anda membukanya.","conflict":true,"current_mtime":current.entry.modified_at})))
            } else {
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":format!("Gagal menyimpan berkas: {e}")})))
            }
        })?;

    // Refresh stat after write
    let new_stat = state
        .agent_client
        .fs_stat(FsStatRequest {
            username: tenant.clone(),
            path: payload.path.clone(),
        })
        .await
        .ok();

    let new_mtime = new_stat.as_ref().and_then(|s| s.entry.modified_at);

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "file.edit",
        format!("{}:{}", tenant, payload.path),
        "success",
        None,
        Some(serde_json::json!({ "size_bytes": write_res.bytes_written }).to_string()),
    );

    Ok((
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        Json(FileSaveApiResponse {
            path: payload.path,
            size_bytes: write_res.bytes_written,
            modified_at: new_mtime,
            etag: new_etag,
            message: "Berkas berhasil disimpan".to_string(),
        }),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FileSearchPayload {
    pub path: Option<String>,
    pub query: String,
    pub search_content: Option<bool>,
    pub case_sensitive: Option<bool>,
    pub extension: Option<String>,
    pub max_results: Option<usize>,
    pub timeout_secs: Option<u64>,
    pub as_job: Option<bool>,
    pub tenant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContentMatchApiDto {
    pub line_number: usize,
    pub line_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchResultItemApi {
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
    pub matches: Vec<ContentMatchApiDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FileSearchApiResponse {
    pub items: Vec<SearchResultItemApi>,
    pub total_scanned: usize,
    pub limit_reached: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub job_id: Option<String>,
}

/// Search files by name or content within tenant boundaries.
#[utoipa::path(
    post,
    path = "/api/v1/files/search",
    request_body = FileSearchPayload,
    responses(
        (status = 200, description = "Search completed successfully", body = FileSearchApiResponse),
        (status = 202, description = "Search enqueued as background job", body = FileSearchApiResponse),
        (status = 400, description = "Invalid search parameters"),
        (status = 403, description = "Forbidden path access")
    ),
    tag = "files"
)]
pub async fn search_files_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<FileSearchPayload>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if payload.query.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Query pencarian tidak boleh kosong" })),
        ));
    }

    let tenant = resolve_tenant(&state, &auth_user, payload.tenant.as_deref()).await?;
    let path = payload.path.unwrap_or_default();

    if payload.as_job == Some(true) {
        let job_payload = serde_json::json!({
            "username": tenant,
            "path": path,
            "query": payload.query,
            "search_content": payload.search_content,
            "case_sensitive": payload.case_sensitive,
            "extension": payload.extension,
            "max_results": payload.max_results,
            "timeout_secs": payload.timeout_secs,
        });

        let (job, _) = state
            .job_runner
            .enqueue(
                Some(auth_user.id),
                None,
                "files.search".to_string(),
                job_payload.to_string(),
                Some(1),
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Gagal mendaftarkan job pencarian: {e}") })),
                )
            })?;

        state.audit_writer.log_async(
            Some(auth_user.id),
            &auth_user.username,
            "file.search",
            format!("{}:{}", tenant, path),
            "enqueued",
            None,
            Some(serde_json::json!({ "job_id": job.id, "query": payload.query }).to_string()),
        );

        return Ok((
            StatusCode::ACCEPTED,
            Json(FileSearchApiResponse {
                items: Vec::new(),
                total_scanned: 0,
                limit_reached: false,
                timed_out: false,
                duration_ms: 0,
                job_id: Some(job.id),
            }),
        ));
    }

    let search_req = FsSearchRequest {
        username: tenant.clone(),
        path: path.clone(),
        query: payload.query.clone(),
        search_content: payload.search_content.unwrap_or(false),
        case_sensitive: payload.case_sensitive.unwrap_or(false),
        extension: payload.extension,
        max_results: payload.max_results,
        timeout_secs: payload.timeout_secs,
    };

    let search_res = state
        .agent_client
        .fs_search(search_req)
        .await
        .map_err(|e| {
            let err_str = e.to_string();
            if err_str.contains("Path traversal")
                || err_str.contains("outside")
                || err_str.contains("forbidden")
                || err_str.contains("Permission denied")
            {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({ "error": format!("Akses ditolak: {e}") })),
                )
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("Pencarian gagal: {e}") })),
                )
            }
        })?;

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "file.search",
        format!("{}:{}", tenant, path),
        "success",
        None,
        Some(
            serde_json::json!({
                "query": payload.query,
                "matches_count": search_res.items.len(),
                "duration_ms": search_res.duration_ms
            })
            .to_string(),
        ),
    );

    let api_items = search_res
        .items
        .into_iter()
        .map(|i| SearchResultItemApi {
            path: i.path,
            is_dir: i.is_dir,
            size_bytes: i.size_bytes,
            modified_at: i.modified_at,
            matches: i
                .matches
                .into_iter()
                .map(|m| ContentMatchApiDto {
                    line_number: m.line_number,
                    line_text: m.line_text,
                })
                .collect(),
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(FileSearchApiResponse {
            items: api_items,
            total_scanned: search_res.total_scanned,
            limit_reached: search_res.limit_reached,
            timed_out: search_res.timed_out,
            duration_ms: search_res.duration_ms,
            job_id: None,
        }),
    ))
}
