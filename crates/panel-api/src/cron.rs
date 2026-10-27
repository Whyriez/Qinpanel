//! Cron job management endpoints (systemd timers, run-now, logs, and human builder).

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_core::{
    cron_to_human_en, cron_to_human_id, cron_to_systemd_calendar, validate_cron_expression,
};
use panel_db::{
    CreateAuditLog, CreateCronJob, CreateCronRun, CronJob, CronRun, SqliteAuditLogRepository,
    SqliteCronRepository, SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository,
    UpdateCronJob,
};
use panel_proto::{
    CronJobApplyRequest, CronJobLogsReadRequest, CronJobRemoveRequest, CronJobRunNowRequest,
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CronJobDto {
    pub id: i64,
    pub site_id: i64,
    pub name: String,
    pub command: String,
    pub schedule: String,
    pub systemd_calendar: String,
    pub human_description: String,
    pub human_description_en: String,
    pub description: Option<String>,
    pub work_dir: Option<String>,
    pub timeout_secs: i64,
    pub enabled: bool,
    pub status: String,
    pub last_run_at: Option<i64>,
    pub last_duration_ms: Option<i64>,
    pub last_exit_code: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<CronJob> for CronJobDto {
    fn from(job: CronJob) -> Self {
        let human_id = cron_to_human_id(&job.schedule);
        let human_en = cron_to_human_en(&job.schedule);
        Self {
            id: job.id,
            site_id: job.site_id,
            name: job.name,
            command: job.command,
            schedule: job.schedule,
            systemd_calendar: job.systemd_calendar,
            human_description: human_id,
            human_description_en: human_en,
            description: job.description,
            work_dir: job.work_dir,
            timeout_secs: job.timeout_secs,
            enabled: job.enabled,
            status: job.status,
            last_run_at: job.last_run_at,
            last_duration_ms: job.last_duration_ms,
            last_exit_code: job.last_exit_code,
            created_at: job.created_at,
            updated_at: job.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CronQuotaDto {
    pub used: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CronJobListResponse {
    pub jobs: Vec<CronJobDto>,
    pub quota: CronQuotaDto,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateCronJobRequest {
    pub name: String,
    pub command: String,
    pub schedule: String,
    pub description: Option<String>,
    pub work_dir: Option<String>,
    pub timeout_secs: Option<i64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateCronJobRequest {
    pub name: Option<String>,
    pub command: Option<String>,
    pub schedule: Option<String>,
    pub description: Option<String>,
    pub work_dir: Option<String>,
    pub timeout_secs: Option<i64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CronRunDto {
    pub id: i64,
    pub cron_job_id: i64,
    pub status: String,
    pub exit_code: Option<i64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub duration_ms: Option<i64>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

impl From<CronRun> for CronRunDto {
    fn from(r: CronRun) -> Self {
        Self {
            id: r.id,
            cron_job_id: r.cron_job_id,
            status: r.status,
            exit_code: r.exit_code,
            stdout: r.stdout,
            stderr: r.stderr,
            duration_ms: r.duration_ms,
            started_at: r.started_at,
            finished_at: r.finished_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ValidateCronRequest {
    pub expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ValidateCronResponse {
    pub valid: bool,
    pub systemd_calendar: Option<String>,
    pub human_id: Option<String>,
    pub human_en: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunsQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogsQuery {
    pub lines: Option<usize>,
}

/// Helper to verify user permissions for a given site.
async fn verify_site_access(
    pool: &panel_db::DbPool,
    auth_user: &AuthenticatedUser,
    site_id: i64,
) -> Result<(panel_db::models::Site, panel_db::models::User), (StatusCode, String)> {
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let user_repo = SqliteUserRepository::new(pool.clone());

    let site = site_repo
        .find_by_id(site_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Situs tidak ditemukan".to_string()))?;

    if !auth_user.is_admin() && site.user_id != auth_user.id {
        return Err((
            StatusCode::FORBIDDEN,
            "Anda tidak memiliki izin untuk mengelola situs ini".to_string(),
        ));
    }

    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Pengguna pemilik situs tidak ditemukan".to_string(),
            )
        })?;

    crate::tenant_context::execution_identity(&user)?;
    Ok((site, user))
}

/// POST /api/v1/sites/{id}/cron/validate
/// Validates a cron expression and returns human translations without persisting.
pub async fn validate_cron_handler(
    Json(payload): Json<ValidateCronRequest>,
) -> Result<Json<ValidateCronResponse>, (StatusCode, Json<serde_json::Value>)> {
    let trimmed = payload.expression.trim();
    match validate_cron_expression(trimmed) {
        Ok(_) => {
            let calendar = cron_to_systemd_calendar(trimmed).ok();
            let human_id = cron_to_human_id(trimmed);
            let human_en = cron_to_human_en(trimmed);
            Ok(Json(ValidateCronResponse {
                valid: true,
                systemd_calendar: calendar,
                human_id: Some(human_id),
                human_en: Some(human_en),
                error: None,
            }))
        }
        Err(e) => Ok(Json(ValidateCronResponse {
            valid: false,
            systemd_calendar: None,
            human_id: None,
            human_en: None,
            error: Some(e.to_string()),
        })),
    }
}

/// GET /api/v1/sites/{id}/cron
/// Lists all cron jobs for the site with plan quota status.
pub async fn list_site_cron_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    let (_site, user) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let cron_repo = SqliteCronRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());

    let jobs = match cron_repo.list_jobs_for_site(site_id).await {
        Ok(list) => list,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let used = match cron_repo.count_jobs_for_user(user.id).await {
        Ok(cnt) => cnt,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let plan_id = user.plan_id.unwrap_or(1);
    let plan_limit = match plan_repo.find_by_id(plan_id).await {
        Ok(Some(p)) => p.max_cron_jobs,
        _ => 10,
    };

    let job_dtos: Vec<CronJobDto> = jobs.into_iter().map(CronJobDto::from).collect();

    (
        StatusCode::OK,
        Json(CronJobListResponse {
            jobs: job_dtos,
            quota: CronQuotaDto {
                used,
                limit: plan_limit,
            },
        }),
    )
        .into_response()
}

/// POST /api/v1/sites/{id}/cron
/// Creates a new cron job, applies systemd timer via agent, and records DB entry.
pub async fn create_site_cron_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<CreateCronJobRequest>,
) -> Response {
    let (site, user) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let cron_repo = SqliteCronRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());

    // 1. Quota check against user's hosting plan
    let used_jobs = match cron_repo.count_jobs_for_user(user.id).await {
        Ok(cnt) => cnt,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let plan_id = user.plan_id.unwrap_or(1);
    let plan_limit = match plan_repo.find_by_id(plan_id).await {
        Ok(Some(p)) => p.max_cron_jobs,
        _ => 10,
    };

    if used_jobs >= plan_limit {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": format!(
                    "Batas kuota cron job tercapai ({}/{}). Tingkatkan paket hosting untuk menambah cron job.",
                    used_jobs, plan_limit
                )
            })),
        )
            .into_response();
    }

    // 2. Validate input fields
    let trimmed_name = payload.name.trim();
    if trimmed_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Nama cron job tidak boleh kosong" })),
        )
            .into_response();
    }

    let trimmed_cmd = payload.command.trim();
    if trimmed_cmd.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Command cron job tidak boleh kosong" })),
        )
            .into_response();
    }
    if trimmed_cmd.contains('\n') || trimmed_cmd.contains('\r') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Command tidak boleh mengandung karakter baris baru" })),
        )
            .into_response();
    }

    let trimmed_schedule = payload.schedule.trim();
    if let Err(e) = validate_cron_expression(trimmed_schedule) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Ekspresi cron tidak valid: {}", e) })),
        )
            .into_response();
    }

    let systemd_calendar = match cron_to_systemd_calendar(trimmed_schedule) {
        Ok(cal) => cal,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Gagal mengonversi jadwal ke systemd timer: {}", e) })),
            )
                .into_response();
        }
    };

    let timeout_secs = payload.timeout_secs.unwrap_or(3600).clamp(10, 86400);
    let enabled = payload.enabled.unwrap_or(true);
    let work_dir = payload
        .work_dir
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| site.docroot.clone());

    // 3. Create database entry
    let created_job = match cron_repo
        .create_job(&CreateCronJob {
            site_id,
            name: trimmed_name.to_string(),
            command: trimmed_cmd.to_string(),
            schedule: trimmed_schedule.to_string(),
            systemd_calendar: systemd_calendar.clone(),
            description: payload.description,
            work_dir: Some(work_dir.clone()),
            timeout_secs: Some(timeout_secs),
            enabled: Some(enabled),
        })
        .await
    {
        Ok(job) => job,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal menyimpan cron job ke database: {}", e) })),
            )
                .into_response();
        }
    };

    // 4. Apply systemd timer and service unit via agent
    let (linux_user, linux_uid) = match crate::tenant_context::execution_identity(&user) {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error":message}))).into_response();
        }
    };

    let agent_req = CronJobApplyRequest {
        site_id: format!("site_{}", site.id),
        job_id: created_job.id,
        name: created_job.name.clone(),
        username: linux_user,
        uid: linux_uid,
        command: created_job.command.clone(),
        schedule: created_job.schedule.clone(),
        systemd_calendar,
        work_dir: Some(work_dir),
        timeout_secs: timeout_secs as u64,
        enabled,
        unit_dir: None,
        env_file_path: None,
    };

    if let Err(e) = state.agent_client.apply_cron_job(agent_req).await {
        // Rollback DB record if agent configuration fails
        let _ = cron_repo.delete_job(created_job.id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal memasang systemd timer di sistem melalui agent: {}", e)
            })),
        )
            .into_response();
    }

    // 5. Audit log
    let audit_repo = SqliteAuditLogRepository::new(state.operations_pool.clone());

    let _ = audit_repo
        .create(&CreateAuditLog {
            user_id: Some(auth_user.id),
            actor_username: auth_user.username.clone(),
            action: "cron.create".to_string(),
            target_resource: format!("site:{}:cron:{}", site_id, created_job.id),
            details_json: Some(
                serde_json::json!({
                    "site_id": site_id,
                    "name": created_job.name,
                    "schedule": created_job.schedule,
                    "enabled": enabled,
                })
                .to_string(),
            ),
            ip_address: None,
            status: "success".to_string(),
        })
        .await;

    (StatusCode::CREATED, Json(CronJobDto::from(created_job))).into_response()
}

/// GET /api/v1/sites/{id}/cron/{job_id}
/// Gets detailed information for a specific cron job.
pub async fn get_site_cron_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, job_id)): Path<(i64, i64)>,
) -> Response {
    if let Err((code, msg)) = verify_site_access(&state.pool, &auth_user, site_id).await {
        return (code, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    let cron_repo = SqliteCronRepository::new(state.pool.clone());
    match cron_repo.find_job_by_id(job_id).await {
        Ok(Some(job)) if job.site_id == site_id => {
            (StatusCode::OK, Json(CronJobDto::from(job))).into_response()
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Cron job tidak ditemukan" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// PUT /api/v1/sites/{id}/cron/{job_id}
/// Updates a cron job configuration and updates its systemd timer.
pub async fn update_site_cron_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, job_id)): Path<(i64, i64)>,
    Json(payload): Json<UpdateCronJobRequest>,
) -> Response {
    let (site, user) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let cron_repo = SqliteCronRepository::new(state.pool.clone());
    let existing = match cron_repo.find_job_by_id(job_id).await {
        Ok(Some(job)) if job.site_id == site_id => job,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Cron job tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let mut systemd_calendar = existing.systemd_calendar.clone();
    if let Some(ref sched) = payload.schedule {
        let trimmed_sched = sched.trim();
        if let Err(e) = validate_cron_expression(trimmed_sched) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Ekspresi cron tidak valid: {}", e) })),
            )
                .into_response();
        }
        systemd_calendar = match cron_to_systemd_calendar(trimmed_sched) {
            Ok(cal) => cal,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("Gagal mengonversi jadwal ke systemd timer: {}", e) })),
                )
                    .into_response();
            }
        };
    }

    let updated_job = match cron_repo
        .update_job(
            job_id,
            &UpdateCronJob {
                name: payload.name.map(|s| s.trim().to_string()),
                command: payload.command.map(|s| s.trim().to_string()),
                schedule: payload.schedule.map(|s| s.trim().to_string()),
                systemd_calendar: Some(systemd_calendar.clone()),
                description: payload.description,
                work_dir: payload.work_dir,
                timeout_secs: payload.timeout_secs,
                enabled: payload.enabled,
                ..Default::default()
            },
        )
        .await
    {
        Ok(job) => job,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal memperbarui cron job: {}", e) })),
            )
                .into_response();
        }
    };

    // Update systemd timer via agent
    let (linux_user, linux_uid) = match crate::tenant_context::execution_identity(&user) {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error":message}))).into_response();
        }
    };

    let agent_req = CronJobApplyRequest {
        site_id: format!("site_{}", site.id),
        job_id: updated_job.id,
        name: updated_job.name.clone(),
        username: linux_user,
        uid: linux_uid,
        command: updated_job.command.clone(),
        schedule: updated_job.schedule.clone(),
        systemd_calendar,
        work_dir: updated_job.work_dir.clone(),
        timeout_secs: updated_job.timeout_secs as u64,
        enabled: updated_job.enabled,
        unit_dir: None,
        env_file_path: None,
    };

    if let Err(e) = state.agent_client.apply_cron_job(agent_req).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal memperbarui timer systemd melalui agent: {}", e)
            })),
        )
            .into_response();
    }

    (StatusCode::OK, Json(CronJobDto::from(updated_job))).into_response()
}

/// DELETE /api/v1/sites/{id}/cron/{job_id}
/// Deletes a cron job, disables and removes its systemd timer/service.
pub async fn delete_site_cron_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, job_id)): Path<(i64, i64)>,
) -> Response {
    let (site, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let cron_repo = SqliteCronRepository::new(state.pool.clone());
    let existing = match cron_repo.find_job_by_id(job_id).await {
        Ok(Some(job)) if job.site_id == site_id => job,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Cron job tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Remove systemd timer/service via agent
    let rem_req = CronJobRemoveRequest {
        site_id: format!("site_{}", site.id),
        job_id: existing.id,
        unit_dir: None,
    };

    let _ = state.agent_client.remove_cron_job(rem_req).await;

    // Delete from database
    if let Err(e) = cron_repo.delete_job(job_id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal menghapus cron job: {}", e) })),
        )
            .into_response();
    }

    // Audit log
    let audit_repo = SqliteAuditLogRepository::new(state.operations_pool.clone());

    let _ = audit_repo
        .create(&CreateAuditLog {
            user_id: Some(auth_user.id),
            actor_username: auth_user.username.clone(),
            action: "cron.delete".to_string(),
            target_resource: format!("site:{}:cron:{}", site_id, job_id),
            details_json: Some(
                serde_json::json!({
                    "site_id": site_id,
                    "name": existing.name,
                })
                .to_string(),
            ),
            ip_address: None,
            status: "success".to_string(),
        })
        .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({ "message": "Cron job berhasil dihapus" })),
    )
        .into_response()
}

/// POST /api/v1/sites/{id}/cron/{job_id}/run
/// Triggers instant execution ("Run Now") via agent and records run history.
pub async fn run_site_cron_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, job_id)): Path<(i64, i64)>,
) -> Response {
    let (site, user) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let cron_repo = SqliteCronRepository::new(state.pool.clone());
    let existing = match cron_repo.find_job_by_id(job_id).await {
        Ok(Some(job)) if job.site_id == site_id => job,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Cron job tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let (linux_user, linux_uid) = match crate::tenant_context::execution_identity(&user) {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error":message}))).into_response();
        }
    };

    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // 1. Set job status to running in DB
    let _ = cron_repo
        .update_job(
            job_id,
            &UpdateCronJob {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .await;

    // 2. Call agent to run the service
    let run_req = CronJobRunNowRequest {
        site_id: format!("site_{}", site.id),
        job_id: existing.id,
        username: linux_user,
        uid: linux_uid,
        command: existing.command.clone(),
        work_dir: existing.work_dir.clone(),
        timeout_secs: existing.timeout_secs as u64,
        env_file_path: None,
        unit_dir: None,
    };

    let run_res = match state.agent_client.run_now_cron_job(run_req).await {
        Ok(res) => res,
        Err(e) => {
            let _ = cron_repo
                .update_job(
                    job_id,
                    &UpdateCronJob {
                        status: Some("failed".to_string()),
                        last_run_at: Some(now_ts),
                        last_exit_code: Some(-1),
                        ..Default::default()
                    },
                )
                .await;

            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Eksekusi cron job via agent gagal: {}", e)
                })),
            )
                .into_response();
        }
    };

    let finish_status = if run_res.success && run_res.exit_code == 0 {
        "success"
    } else {
        "failed"
    };

    // 3. Record in cron_runs table
    let run_record = match cron_repo
        .create_run(&CreateCronRun {
            cron_job_id: existing.id,
            status: finish_status.to_string(),
            exit_code: Some(run_res.exit_code as i64),
            stdout: Some(run_res.stdout.clone()),
            stderr: Some(run_res.stderr.clone()),
            duration_ms: Some(run_res.duration_ms as i64),
            started_at: now_ts,
            finished_at: Some(now_ts + (run_res.duration_ms / 1000) as i64),
        })
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Gagal mencatat riwayat run cron: {}", e)
                })),
            )
                .into_response();
        }
    };

    // 4. Update cron_jobs table with last execution metrics
    let _ = cron_repo
        .update_job(
            job_id,
            &UpdateCronJob {
                status: Some(finish_status.to_string()),
                last_run_at: Some(now_ts),
                last_duration_ms: Some(run_res.duration_ms as i64),
                last_exit_code: Some(run_res.exit_code as i64),
                ..Default::default()
            },
        )
        .await;

    (StatusCode::OK, Json(CronRunDto::from(run_record))).into_response()
}

/// GET /api/v1/sites/{id}/cron/{job_id}/runs
/// Returns recent execution history for a cron job.
pub async fn get_site_cron_runs_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, job_id)): Path<(i64, i64)>,
    Query(query): Query<RunsQuery>,
) -> Response {
    if let Err((code, msg)) = verify_site_access(&state.pool, &auth_user, site_id).await {
        return (code, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    let cron_repo = SqliteCronRepository::new(state.pool.clone());
    let limit = query.limit.unwrap_or(20).clamp(1, 100);

    match cron_repo.list_runs_for_job(job_id, limit).await {
        Ok(runs) => {
            let dtos: Vec<CronRunDto> = runs.into_iter().map(CronRunDto::from).collect();
            (StatusCode::OK, Json(dtos)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/v1/sites/{id}/cron/{job_id}/logs
/// Reads journalctl logs for the cron job's service unit.
pub async fn get_site_cron_logs_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, job_id)): Path<(i64, i64)>,
    Query(query): Query<LogsQuery>,
) -> Response {
    let (site, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let cron_repo = SqliteCronRepository::new(state.pool.clone());
    let _existing = match cron_repo.find_job_by_id(job_id).await {
        Ok(Some(job)) if job.site_id == site_id => job,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Cron job tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let lines = query.lines.unwrap_or(50).clamp(10, 500);

    let logs_req = CronJobLogsReadRequest {
        site_id: format!("site_{}", site.id),
        job_id,
        lines,
    };

    match state.agent_client.read_cron_job_logs(logs_req).await {
        Ok(res) => (
            StatusCode::OK,
            Json(serde_json::json!({ "logs": res.logs })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal membaca log systemd dari agent: {}", e)
            })),
        )
            .into_response(),
    }
}
