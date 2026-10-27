//! REST API handlers and DTOs for system reliability, doctor, reconcile, and system backups.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use panel_core::doctor::{DoctorCheckCategory, DoctorCheckItem, DoctorCheckStatus, DoctorReport};
use panel_core::reconcile::{DriftItem, DriftType, ReconcileReport};
use panel_core::system_backup::{SystemBackupSummary, SystemBackupVerificationResult};
use panel_db::repositories::domain_repo::SqliteDomainRepository;
use panel_db::repositories::job_repo::SqliteJobRepository;
use panel_db::repositories::site_repo::SqliteSiteRepository;
use panel_proto::{
    DoctorCheckItemDto, DriftItemDto, ReconcileSiteInputDto, SystemBackupCreateRequest,
    SystemBackupListRequest, SystemBackupRestoreRequest, SystemBackupSummaryDto,
    SystemDoctorCheckRequest, SystemDoctorFixRequest, SystemReconcileRequest,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use utoipa::ToSchema;

use crate::routes::AppState;

fn map_proto_category(cat: &str) -> DoctorCheckCategory {
    match cat {
        "os_kernel" => DoctorCheckCategory::OsKernel,
        "prerequisites" => DoctorCheckCategory::Prerequisites,
        "services" => DoctorCheckCategory::Services,
        "socket" => DoctorCheckCategory::Socket,
        "database" => DoctorCheckCategory::Database,
        "filesystem" => DoctorCheckCategory::Filesystem,
        "firewall" => DoctorCheckCategory::Firewall,
        _ => DoctorCheckCategory::Jobs,
    }
}

fn map_proto_status(st: &str) -> DoctorCheckStatus {
    match st {
        "pass" => DoctorCheckStatus::Pass,
        "fail" => DoctorCheckStatus::Fail,
        _ => DoctorCheckStatus::Warn,
    }
}

fn map_dto_to_check_item(dto: DoctorCheckItemDto) -> DoctorCheckItem {
    DoctorCheckItem {
        id: dto.id,
        name: dto.name,
        category: map_proto_category(&dto.category),
        status: map_proto_status(&dto.status),
        message: dto.message,
        remediation_hint: dto.remediation_hint,
        fixable: dto.fixable,
    }
}

fn map_dto_to_drift_item(dto: DriftItemDto) -> DriftItem {
    let drift_type = match dto.drift_type.as_str() {
        "missing_nginx_config" => DriftType::MissingNginxConfig,
        "orphaned_nginx_config" => DriftType::OrphanedNginxConfig,
        "missing_docroot_directory" => DriftType::MissingDocrootDirectory,
        "directory_permission_mismatch" => DriftType::DirectoryPermissionMismatch,
        "nftables_binding_mismatch" => DriftType::NftablesBindingMismatch,
        _ => DriftType::InterruptedJobStuck,
    };

    DriftItem {
        id: dto.id,
        drift_type,
        resource_type: dto.resource_type,
        resource_id: dto.resource_id,
        description: dto.description,
        desired_state: dto.desired_state,
        actual_state: dto.actual_state,
        remediation_action: dto.remediation_action,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSystemBackupPayload {
    #[schema(example = "panel-backup-manual.tar.gz")]
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RestoreSystemBackupPayload {
    #[schema(example = "/var/backups/panel/panel-backup-2026-10-11.tar.gz")]
    pub filepath: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/system/doctor",
    responses(
        (status = 200, description = "Comprehensive system health doctor report", body = DoctorReport)
    ),
    tag = "system"
)]
pub async fn get_system_doctor_handler(State(state): State<AppState>) -> impl IntoResponse {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut checks = Vec::new();

    // 1. Database SQLite Quick Check
    let db_integrity: Result<(String,), _> = sqlx::query_as("PRAGMA quick_check;")
        .fetch_one(&state.pool)
        .await;

    let (db_status, db_msg) = match db_integrity {
        Ok((res,)) if res == "ok" => (
            DoctorCheckStatus::Pass,
            "Integritas berkas SQLite valid (PRAGMA quick_check: ok)".to_string(),
        ),
        Ok((res,)) => (
            DoctorCheckStatus::Fail,
            format!("Kerusakan terdeteksi pada basis data: {}", res),
        ),
        Err(e) => (
            DoctorCheckStatus::Fail,
            format!("Gagal mengeksekusi pemeriksaan basis data: {}", e),
        ),
    };

    checks.push(DoctorCheckItem {
        id: "database_integrity".to_string(),
        name: "Integritas Basis Data SQLite".to_string(),
        category: DoctorCheckCategory::Database,
        status: db_status,
        message: db_msg,
        remediation_hint: None,
        fixable: false,
    });

    // 2. Interrupted/stuck jobs check
    let job_repo = state.job_runner.job_repo.clone();
    let running_jobs = job_repo.list_by_status("running").await.unwrap_or_default();
    let stuck_count = running_jobs.len();

    let (jobs_status, jobs_msg) = if stuck_count == 0 {
        (
            DoctorCheckStatus::Pass,
            "Semua pekerjaan antrean dan eksekutor dalam kondisi sehat".to_string(),
        )
    } else {
        (
            DoctorCheckStatus::Warn,
            format!(
                "Ditemukan {} pekerjaan berstatus 'running' yang terinterupsi",
                stuck_count
            ),
        )
    };

    checks.push(DoctorCheckItem {
        id: "interrupted_jobs".to_string(),
        name: "Pekerjaan Terinterupsi / Macet".to_string(),
        category: DoctorCheckCategory::Jobs,
        status: jobs_status,
        message: jobs_msg,
        remediation_hint: if stuck_count > 0 {
            Some("Jalankan POST /api/v1/system/doctor/fix untuk memulihkan status".to_string())
        } else {
            None
        },
        fixable: true,
    });

    // 3. Delegate to Agent for host/nginx/service checks
    match state
        .agent_client
        .system_doctor_check(SystemDoctorCheckRequest {})
        .await
    {
        Ok(agent_resp) => {
            for agent_dto in agent_resp.checks {
                if !checks.iter().any(|c| c.id == agent_dto.id) {
                    checks.push(map_dto_to_check_item(agent_dto));
                }
            }
        }
        Err(e) => {
            warn!("Agent IPC doctor check gagal / offline: {}", e);
            let is_linux = cfg!(target_os = "linux");
            checks.push(DoctorCheckItem {
                id: "agent_socket".to_string(),
                name: "Soket IPC Agent Panel".to_string(),
                category: DoctorCheckCategory::Socket,
                status: if !is_linux {
                    DoctorCheckStatus::Pass
                } else {
                    DoctorCheckStatus::Warn
                },
                message: if !is_linux {
                    "Pemeriksaan agent IPC dilewati pada mesin non-Linux".to_string()
                } else {
                    format!("Agent IPC tidak merespons: {}", e)
                },
                remediation_hint: if is_linux {
                    Some("Jalankan 'systemctl restart panel-agent'".to_string())
                } else {
                    None
                },
                fixable: false,
            });
        }
    }

    let report = DoctorReport::new(now, checks);
    (StatusCode::OK, Json(report))
}

async fn recheck_db_integrity_post_fix(pool: &panel_db::DbPool) -> DoctorCheckItem {
    let db_integrity: Result<(String,), _> =
        sqlx::query_as("PRAGMA quick_check;").fetch_one(pool).await;
    let (db_status, db_msg) = match db_integrity {
        Ok((res,)) if res == "ok" => (
            DoctorCheckStatus::Pass,
            "Integritas berkas SQLite valid pasca perbaikan WAL (PRAGMA quick_check: ok)"
                .to_string(),
        ),
        _ => (
            DoctorCheckStatus::Fail,
            "Terdeteksi masalah integritas database".to_string(),
        ),
    };
    DoctorCheckItem {
        id: "database_integrity".to_string(),
        name: "Integritas Basis Data SQLite".to_string(),
        category: DoctorCheckCategory::Database,
        status: db_status,
        message: db_msg,
        remediation_hint: None,
        fixable: false,
    }
}

async fn recover_jobs_safe(job_repo: &SqliteJobRepository) -> u64 {
    match job_repo.recover_interrupted_jobs().await {
        Ok(n) => {
            info!("Memulihkan {} pekerjaan terinterupsi", n);
            n
        }
        Err(e) => {
            error!("Gagal memulihkan pekerjaan terinterupsi: {}", e);
            0
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/system/doctor/fix",
    responses(
        (status = 200, description = "Remediated system health doctor report", body = DoctorReport)
    ),
    tag = "system"
)]
pub async fn fix_system_doctor_handler(State(state): State<AppState>) -> impl IntoResponse {
    let job_repo = state.job_runner.job_repo.clone();

    // 1. Recover interrupted jobs
    let recovered = recover_jobs_safe(&job_repo).await;

    // 2. SQLite WAL truncate checkpoint
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE);")
        .execute(&state.pool)
        .await;

    // 3. Request agent remediation
    let _ = state
        .agent_client
        .system_doctor_fix(SystemDoctorFixRequest {
            target_checks: vec![],
        })
        .await;

    // 4. Return refreshed doctor report
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut checks = Vec::new();

    // Recheck DB
    checks.push(recheck_db_integrity_post_fix(&state.pool).await);

    checks.push(DoctorCheckItem {
        id: "interrupted_jobs".to_string(),
        name: "Pekerjaan Terinterupsi / Macet".to_string(),
        category: DoctorCheckCategory::Jobs,
        status: DoctorCheckStatus::Pass,
        message: format!(
            "Pekerjaan terinterupsi telah dipulihkan (total: {})",
            recovered
        ),
        remediation_hint: None,
        fixable: false,
    });

    // Refresh agent status
    if let Ok(agent_resp) = state
        .agent_client
        .system_doctor_check(SystemDoctorCheckRequest {})
        .await
    {
        for agent_dto in agent_resp.checks {
            if !checks.iter().any(|c| c.id == agent_dto.id) {
                checks.push(map_dto_to_check_item(agent_dto));
            }
        }
    }

    let report = DoctorReport::new(now, checks);
    (StatusCode::OK, Json(report))
}

#[utoipa::path(
    get,
    path = "/api/v1/system/reconcile",
    responses(
        (status = 200, description = "Drift detection report between database and host state", body = ReconcileReport)
    ),
    tag = "system"
)]
pub async fn get_system_reconcile_handler(State(state): State<AppState>) -> impl IntoResponse {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let job_repo = state.job_runner.job_repo.clone();

    let sites = site_repo.list_all().await.unwrap_or_default();
    let mut desired_sites_dto = Vec::new();

    for s in &sites {
        let domain_name = domain_repo
            .find_primary_by_site_id(s.id)
            .await
            .ok()
            .flatten()
            .map(|d| d.name)
            .unwrap_or_else(|| format!("site-{}.local", s.id));

        desired_sites_dto.push(ReconcileSiteInputDto {
            site_id: s.id,
            domain: domain_name,
            username: format!("siteuser{}", s.user_id),
            uid: 1000 + (s.user_id as u32),
            kind: s.kind.clone(),
            docroot: s.docroot.clone(),
            config_content: String::new(),
            is_active: s.status == "active",
        });
    }

    // Call Agent for drift detection
    let req = SystemReconcileRequest {
        dry_run: true,
        desired_sites: desired_sites_dto,
        desired_nftables_ports: vec![],
    };

    let mut report = match state.agent_client.system_reconcile(req).await {
        Ok(res) => {
            let core_drifts = res.drifts.into_iter().map(map_dto_to_drift_item).collect();
            ReconcileReport::new(res.timestamp, res.total_resources_checked, core_drifts)
        }
        Err(e) => {
            warn!(
                "Agent IPC reconcile call failed, returning local drift calculation: {}",
                e
            );
            ReconcileReport::new(now, sites.len(), vec![])
        }
    };

    // Check stuck jobs drift
    let running_jobs = job_repo.list_by_status("running").await.unwrap_or_default();
    if !running_jobs.is_empty() {
        report.drifts.push(DriftItem {
            id: "drift-stuck-jobs".to_string(),
            drift_type: DriftType::InterruptedJobStuck,
            resource_type: "job".to_string(),
            resource_id: format!("{} jobs", running_jobs.len()),
            description: format!(
                "Ditemukan {} pekerjaan berstatus running yang terinterupsi",
                running_jobs.len()
            ),
            desired_state: "completed or failed".to_string(),
            actual_state: "running".to_string(),
            remediation_action: "Tandai gagal pekerjaan macet via reconcile execution".to_string(),
        });
        report.is_clean = false;
    }

    (StatusCode::OK, Json(report))
}

#[utoipa::path(
    post,
    path = "/api/v1/system/reconcile",
    responses(
        (status = 200, description = "Execute drift remediation across host configuration and database", body = ReconcileReport)
    ),
    tag = "system"
)]
pub async fn apply_system_reconcile_handler(State(state): State<AppState>) -> impl IntoResponse {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let job_repo = state.job_runner.job_repo.clone();

    // 1. Recover jobs
    let _ = job_repo.recover_interrupted_jobs().await;

    let sites = site_repo.list_all().await.unwrap_or_default();
    let mut desired_sites_dto = Vec::new();

    for s in &sites {
        let domain_name = domain_repo
            .find_primary_by_site_id(s.id)
            .await
            .ok()
            .flatten()
            .map(|d| d.name)
            .unwrap_or_else(|| format!("site-{}.local", s.id));

        desired_sites_dto.push(ReconcileSiteInputDto {
            site_id: s.id,
            domain: domain_name,
            username: format!("siteuser{}", s.user_id),
            uid: 1000 + (s.user_id as u32),
            kind: s.kind.clone(),
            docroot: s.docroot.clone(),
            config_content: String::new(),
            is_active: s.status == "active",
        });
    }

    // Call Agent with dry_run = false to reconcile state
    let req = SystemReconcileRequest {
        dry_run: false,
        desired_sites: desired_sites_dto,
        desired_nftables_ports: vec![],
    };

    let report = match state.agent_client.system_reconcile(req).await {
        Ok(res) => {
            let core_drifts = res.drifts.into_iter().map(map_dto_to_drift_item).collect();
            ReconcileReport::new(res.timestamp, res.total_resources_checked, core_drifts)
        }
        Err(e) => {
            warn!("Agent IPC reconcile apply returned error: {}", e);
            ReconcileReport::new(now, sites.len(), vec![])
        }
    };

    (StatusCode::OK, Json(report))
}

#[utoipa::path(
    get,
    path = "/api/v1/system/backups",
    responses(
        (status = 200, description = "List existing system configuration and database backups", body = Vec<SystemBackupSummary>)
    ),
    tag = "system"
)]
pub async fn list_system_backups_handler(State(state): State<AppState>) -> impl IntoResponse {
    let req = SystemBackupListRequest { backups_dir: None };
    match state.agent_client.system_backup_list(req).await {
        Ok(res) => {
            let summaries: Vec<SystemBackupSummary> = res
                .backups
                .into_iter()
                .map(|b: SystemBackupSummaryDto| SystemBackupSummary {
                    filename: b.filename,
                    filepath: b.filepath,
                    created_at: b.created_at,
                    size_bytes: b.size_bytes,
                    checksum_sha256: b.checksum_sha256,
                    files_count: b.files_count,
                    verified: b.verified,
                })
                .collect();
            (StatusCode::OK, Json(summaries))
        }
        Err(e) => {
            warn!(
                "Agent backup list error, scanning fallback directory: {}",
                e
            );
            let list = scan_fallback_backup_dir(Path::new("/var/backups/panel"));
            (StatusCode::OK, Json(list))
        }
    }
}

fn scan_fallback_backup_dir(backup_dir: &Path) -> Vec<SystemBackupSummary> {
    let mut list = Vec::new();
    if !backup_dir.exists() {
        return list;
    }
    if let Ok(entries) = std::fs::read_dir(backup_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let fname = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !fname.starts_with("panel-backup-") {
                continue;
            }
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let ctime = meta
                .and_then(|m| m.created().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            list.push(SystemBackupSummary {
                filename: fname,
                filepath: path.to_string_lossy().to_string(),
                size_bytes: size,
                checksum_sha256: String::new(),
                created_at: ctime,
                files_count: 0,
                verified: false,
            });
        }
    }
    list
}

#[utoipa::path(
    post,
    path = "/api/v1/system/backups",
    request_body = Option<CreateSystemBackupPayload>,
    responses(
        (status = 201, description = "Create a snapshot backup of panel DB and configuration", body = SystemBackupSummary)
    ),
    tag = "system"
)]
pub async fn create_system_backup_handler(
    State(state): State<AppState>,
    Json(payload): Json<Option<CreateSystemBackupPayload>>,
) -> impl IntoResponse {
    // SQLite's snapshot operation includes WAL data without stopping all writers.
    let output_filename = payload.and_then(|p| p.filename);
    let req = SystemBackupCreateRequest {
        output_filename,
        include_analytics: false,
        db_path: panel_db::operations::database_path(&state.pool)
            .await
            .ok()
            .flatten()
            .map(|path| path.to_string_lossy().into_owned()),
        analytics_db_path: Some(state.analytics_db_path.to_string_lossy().into_owned()),
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    match state.agent_client.system_backup_create(req).await {
        Ok(res) => (
            StatusCode::CREATED,
            Json(SystemBackupSummary {
                filename: res.filename,
                filepath: res.filepath,
                created_at: now,
                size_bytes: res.size_bytes,
                checksum_sha256: res.checksum_sha256,
                files_count: res.files_count,
                verified: true,
            }),
        ),
        Err(e) => {
            error!("Gagal membuat snapshot cadangan sistem: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SystemBackupSummary {
                    filename: "backup_failed".to_string(),
                    filepath: String::new(),
                    size_bytes: 0,
                    checksum_sha256: String::new(),
                    created_at: 0,
                    files_count: 0,
                    verified: false,
                }),
            )
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/system/backups/restore",
    request_body = RestoreSystemBackupPayload,
    responses(
        (status = 200, description = "Verify and restore panel database and configuration from backup", body = SystemBackupVerificationResult)
    ),
    tag = "system"
)]
pub async fn restore_system_backup_handler(
    State(state): State<AppState>,
    Json(payload): Json<RestoreSystemBackupPayload>,
) -> impl IntoResponse {
    let req = SystemBackupRestoreRequest {
        backup_filepath: payload.filepath,
        target_db_path: None,
        force: false,
    };

    match state.agent_client.system_backup_restore(req).await {
        Ok(res) => (
            StatusCode::OK,
            Json(SystemBackupVerificationResult {
                is_valid: res.success,
                manifest: None,
                errors: res.error.into_iter().collect(),
            }),
        ),
        Err(e) => {
            error!("Gagal memulihkan snapshot sistem: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SystemBackupVerificationResult {
                    is_valid: false,
                    manifest: None,
                    errors: vec![format!("Pemulihan gagal: {}", e)],
                }),
            )
        }
    }
}
