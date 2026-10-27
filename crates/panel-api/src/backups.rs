//! REST API endpoints for site backups, snapshots, restorations, and automated schedules.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use panel_core::backup::{
    BackupFormat, generate_backup_filename, validate_backup_label, validate_backup_schedule,
    validate_restore_destination, validate_retention_count, validate_s3_config,
};
use panel_db::models::{CreateBackupInput, SiteBackupModel, UpsertBackupScheduleInput};
use panel_db::repositories::{
    SqliteBackupRepository, SqliteDatabaseRepository, SqliteDomainRepository, SqliteSiteRepository,
    SqliteUserRepository,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use crate::auth::middleware::{RequireAuth, RequireCsrf};
use crate::routes::AppState;

/// Request DTO for creating a new site backup.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateBackupRequest {
    pub label: Option<String>,
    #[serde(default = "default_true")]
    pub include_files: bool,
    #[serde(default = "default_true")]
    pub include_databases: bool,
    pub format: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Request DTO for restoring a backup.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RestoreBackupRequest {
    pub target_subpath: Option<String>,
    #[serde(default = "default_true")]
    pub restore_files: bool,
    #[serde(default = "default_true")]
    pub restore_databases: bool,
}

/// Request DTO for updating backup schedule.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateScheduleRequest {
    pub is_enabled: bool,
    pub frequency: String,
    pub time_hour: i64,
    pub time_minute: i64,
    pub retention_count: i64,
    pub include_files: bool,
    pub include_databases: bool,
    pub storage_destination: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,
    pub s3_prefix: Option<String>,
}

/// Response DTO for a single backup item.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BackupItemResponse {
    pub id: i64,
    pub site_id: i64,
    pub user_id: i64,
    pub filename: String,
    pub filepath: String,
    pub size_bytes: i64,
    pub label: Option<String>,
    pub includes_files: bool,
    pub includes_databases: bool,
    pub status: String,
    pub error_message: Option<String>,
    pub storage_type: String,
    pub s3_key: Option<String>,
    pub checksum_sha256: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<SiteBackupModel> for BackupItemResponse {
    fn from(m: SiteBackupModel) -> Self {
        Self {
            id: m.id,
            site_id: m.site_id,
            user_id: m.user_id,
            filename: m.filename,
            filepath: m.filepath,
            size_bytes: m.size_bytes,
            label: m.label,
            includes_files: m.includes_files,
            includes_databases: m.includes_databases,
            status: m.status,
            error_message: m.error_message,
            storage_type: m.storage_type,
            s3_key: m.s3_key,
            checksum_sha256: m.checksum_sha256,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Response DTO containing backups list and quota metrics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BackupListResponse {
    pub backups: Vec<BackupItemResponse>,
    pub total_bytes_used: i64,
    pub count: usize,
}

/// Lists all backups for a specific site.
pub async fn list_site_backups(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path(site_id): Path<i64>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if !user.can_access_user(site.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let backups = match backup_repo.list_backups_by_site(site_id).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let total_bytes_used = backup_repo
        .get_total_backup_bytes_by_site(site_id)
        .await
        .unwrap_or(0);

    let count = backups.len();
    let backup_dtos: Vec<BackupItemResponse> =
        backups.into_iter().map(BackupItemResponse::from).collect();
    (
        StatusCode::OK,
        Json(BackupListResponse {
            backups: backup_dtos,
            total_bytes_used,
            count,
        }),
    )
        .into_response()
}

/// Enqueues a manual backup snapshot creation job.
pub async fn create_site_backup(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(site_id): Path<i64>,
    Json(payload): Json<CreateBackupRequest>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if !user.can_access_user(site.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    if let Some(Err(e)) = payload.label.as_deref().map(validate_backup_label) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
    }

    let tenant_user = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let (resolved_linux_name, resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&tenant_user) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let tenant_username = resolved_linux_name;
    let tenant_uid = resolved_linux_uid;

    let format_str = payload.format.as_deref().unwrap_or("tar.zst");
    let format = BackupFormat::from_str_loose(format_str);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let primary_domain = domain_repo
        .find_primary_by_site_id(site_id)
        .await
        .ok()
        .flatten()
        .map(|d| d.name)
        .unwrap_or_else(|| format!("site-{}", site_id));
    let filename = generate_backup_filename(&primary_domain, now, format);
    let tenant_home = format!("/home/{}", tenant_username);
    let backup_dir = format!("{}/backups", tenant_home);
    let filepath = format!("{}/{}", backup_dir, filename);
    let site_dir = if site.docroot.ends_with("/public") {
        site.docroot.trim_end_matches("/public").to_string()
    } else {
        site.docroot.clone()
    };

    // Collect databases for site
    let site_databases = db_repo.list_by_site(site_id).await.unwrap_or_default();
    let db_infos: Vec<serde_json::Value> = site_databases
        .into_iter()
        .map(|db| {
            json!({
                "engine": db.engine,
                "name": db.name,
                "username": db.db_user,
            })
        })
        .collect();

    // 1. Create pending record in DB
    let backup_rec = match backup_repo
        .create_backup(&CreateBackupInput {
            site_id,
            user_id: site.user_id,
            filename: filename.clone(),
            filepath: filepath.clone(),
            size_bytes: 0,
            label: payload.label.clone(),
            includes_files: payload.include_files,
            includes_databases: payload.include_databases,
            status: "pending".to_string(),
            storage_type: "local".to_string(),
            s3_key: None,
            checksum_sha256: None,
        })
        .await
    {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Gagal mencatat data cadangan: {e}") })),
            )
                .into_response();
        }
    };

    // Check retention count from schedule if configured
    let schedule = backup_repo.get_schedule(site_id).await.unwrap_or(None);
    let retention_count = schedule.map(|s| s.retention_count).unwrap_or(0);

    // 2. Enqueue background job
    let job_payload = json!({
        "site_id": site_id,
        "backup_id": backup_rec.id,
        "tenant_username": tenant_username,
        "tenant_uid": tenant_uid,
        "site_dir": site_dir,
        "backup_dir": backup_dir,
        "filename": filename,
        "include_files": payload.include_files,
        "include_databases": payload.include_databases,
        "databases": db_infos,
        "format": format.extension(),
        "retention_count": retention_count,
    });

    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(user.id),
            None,
            "backup.create".to_string(),
            job_payload.to_string(),
            Some(1),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Gagal mendaftarkan job cadangan: {e}") })),
            )
                .into_response();
        }
    };

    (
        StatusCode::ACCEPTED,
        Json(json!({
            "message": "Pembuatan cadangan dimulai di latar belakang",
            "job_id": job.id,
            "backup": backup_rec,
        })),
    )
        .into_response()
}

/// Retrieves single backup details.
pub async fn get_site_backup(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path((site_id, backup_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if !user.can_access_user(site.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    match backup_repo.find_backup_by_id(backup_id).await {
        Ok(Some(b)) if b.site_id == site_id => (StatusCode::OK, Json(b)).into_response(),
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Berkas cadangan tidak ditemukan" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Deletes a backup archive and its database record.
pub async fn delete_site_backup(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path((site_id, backup_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if !user.can_access_user(site.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let backup = match backup_repo.find_backup_by_id(backup_id).await {
        Ok(Some(b)) if b.site_id == site_id => b,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Berkas cadangan tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let context =
        match crate::tenant_context::resolve_tenant_context(&state, &user, None, Some(site_id))
            .await
        {
            Ok(context) => context,
            Err(err) => return err.into_response(),
        };
    let tenant_username = context.linux_username;

    // 1. Delete on host via agent
    let _ = state
        .agent_client
        .backup_delete(panel_proto::BackupDeleteRequest {
            filepath: backup.filepath,
            tenant_username,
        })
        .await;

    // 2. Delete database record
    if let Err(e) = backup_repo.delete_backup(backup_id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal menghapus entri cadangan: {e}") })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({ "message": "Berkas cadangan berhasil dihapus" })),
    )
        .into_response()
}

/// Enqueues a restore job from a backup archive.
pub async fn restore_site_backup(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path((site_id, backup_id)): Path<(i64, i64)>,
    Json(payload): Json<RestoreBackupRequest>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if !user.can_access_user(site.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let backup = match backup_repo.find_backup_by_id(backup_id).await {
        Ok(Some(b)) if b.site_id == site_id => b,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Berkas cadangan tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let tenant_user = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let (resolved_linux_name, resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&tenant_user) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let tenant_username = resolved_linux_name;
    let tenant_uid = resolved_linux_uid;
    let tenant_home = format!("/home/{}", tenant_username);

    // Determine target restore site dir
    let target_site_dir = if let Some(sub) = payload.target_subpath.as_deref() {
        let sub_clean = sub.trim().trim_start_matches('/').trim_end_matches('/');
        let dest = format!("{}/{}", tenant_home, sub_clean);
        if let Err(e) = validate_restore_destination(&dest, &tenant_home) {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
        dest
    } else if site.docroot.ends_with("/public") {
        site.docroot.trim_end_matches("/public").to_string()
    } else {
        site.docroot.clone()
    };

    // Databases associated with site
    let site_databases = db_repo.list_by_site(site_id).await.unwrap_or_default();
    let db_infos: Vec<serde_json::Value> = site_databases
        .into_iter()
        .map(|db| {
            json!({
                "engine": db.engine,
                "name": db.name,
                "username": db.db_user,
            })
        })
        .collect();

    let job_payload = json!({
        "site_id": site_id,
        "backup_id": backup_id,
        "tenant_username": tenant_username,
        "tenant_uid": tenant_uid,
        "backup_filepath": backup.filepath,
        "target_site_dir": target_site_dir,
        "restore_files": payload.restore_files,
        "restore_databases": payload.restore_databases,
        "databases": db_infos,
    });

    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(user.id),
            None,
            "backup.restore".to_string(),
            job_payload.to_string(),
            Some(1),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Gagal mendaftarkan job pemulihan: {e}") })),
            )
                .into_response();
        }
    };

    (
        StatusCode::ACCEPTED,
        Json(json!({
            "message": "Pemulihan cadangan dimulai di latar belakang",
            "job_id": job.id,
        })),
    )
        .into_response()
}

/// Downloads a backup archive.
pub async fn download_site_backup(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path((site_id, backup_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if !user.can_access_user(site.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let backup = match backup_repo.find_backup_by_id(backup_id).await {
        Ok(Some(b)) if b.site_id == site_id => b,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Berkas cadangan tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let bytes = match tokio::fs::read(&backup.filepath).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Berkas cadangan fisik tidak ditemukan di host" })),
            )
                .into_response();
        }
    };

    (
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{}\"", backup.filename),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// Gets backup schedule configuration for a site.
pub async fn get_backup_schedule(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path(site_id): Path<i64>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if !user.can_access_user(site.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let schedule = backup_repo.get_schedule(site_id).await.unwrap_or(None);
    (StatusCode::OK, Json(schedule)).into_response()
}

/// Updates backup schedule configuration and S3 remote settings for a site.
pub async fn update_backup_schedule(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(site_id): Path<i64>,
    Json(payload): Json<UpdateScheduleRequest>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(site_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if !user.can_access_user(site.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    if let Err(e) = validate_backup_schedule(
        &payload.frequency,
        payload.time_hour as u32,
        payload.time_minute as u32,
    ) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
    }

    if let Err(e) = validate_retention_count(payload.retention_count as u32) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
    }

    let storage_destination = payload
        .storage_destination
        .unwrap_or_else(|| "local".to_string());
    if storage_destination == "s3" {
        if let (Some(ep), Some(b), Some(reg), Some(ak), Some(sk)) = (
            payload.s3_endpoint.as_deref(),
            payload.s3_bucket.as_deref(),
            payload.s3_region.as_deref(),
            payload.s3_access_key.as_deref(),
            payload.s3_secret_key.as_deref(),
        ) {
            if let Err(e) = validate_s3_config(ep, b, reg, ak, sk) {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
            }
        } else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Konfigurasi S3 (endpoint, bucket, region, access key, secret key) wajib diisi lengkap" })),
            )
                .into_response();
        }
    }

    let schedule = match backup_repo
        .upsert_schedule(&UpsertBackupScheduleInput {
            site_id,
            is_enabled: payload.is_enabled,
            frequency: payload.frequency,
            time_hour: payload.time_hour,
            time_minute: payload.time_minute,
            retention_count: payload.retention_count,
            include_files: payload.include_files,
            include_databases: payload.include_databases,
            storage_destination,
            s3_endpoint: payload.s3_endpoint,
            s3_bucket: payload.s3_bucket,
            s3_region: payload.s3_region,
            s3_access_key: payload.s3_access_key,
            s3_secret_key_enc: payload.s3_secret_key,
            s3_prefix: payload.s3_prefix,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Gagal menyimpan jadwal cadangan: {e}") })),
            )
                .into_response();
        }
    };

    (StatusCode::OK, Json(schedule)).into_response()
}

/// Builds the Axum router for site backups.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/sites/{id}/backups",
            get(list_site_backups).post(create_site_backup),
        )
        .route(
            "/api/v1/sites/{id}/backups/schedule",
            get(get_backup_schedule).put(update_backup_schedule),
        )
        .route(
            "/api/v1/sites/{id}/backups/{backup_id}",
            get(get_site_backup).delete(delete_site_backup),
        )
        .route(
            "/api/v1/sites/{id}/backups/{backup_id}/restore",
            post(restore_site_backup),
        )
        .route(
            "/api/v1/sites/{id}/backups/{backup_id}/download",
            get(download_site_backup),
        )
}
