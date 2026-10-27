//! Data lifecycle and retention management REST API and background worker.
//!
//! Adheres to PRD 4.6, 5.6, 11 (P9-T9):
//! - Configurable multi-resolution retention policies (raw 10s, 1m, 1h, 1d)
//! - Security event pruning and stale checkpoint cleanup
//! - Disk size tracking, SQLite PRAGMA wal_checkpoint(TRUNCATE), and VACUUM
//! - Integrity checks and background scheduled maintenance worker

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use panel_analytics::{
    DatabaseStorageStats, IntegrityCheckReport, RetentionConfig, RetentionMaintenanceReport,
};
use panel_db::repositories::SqliteSettingRepository;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info};
use utoipa::ToSchema;

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;

const SETTING_RETENTION_CONFIG_KEY: &str = "analytics.retention_config";
const SETTING_LAST_MAINTENANCE_KEY: &str = "analytics.last_maintenance";

// ==========================================
// DTOs
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct RetentionConfigDto {
    pub raw_retention_secs: i64,
    pub one_minute_retention_secs: i64,
    pub one_hour_retention_secs: i64,
    pub one_day_retention_secs: Option<i64>,
    pub security_events_retention_secs: i64,
    pub max_db_size_bytes: Option<i64>,
    pub auto_vacuum: bool,
}

impl From<RetentionConfig> for RetentionConfigDto {
    fn from(c: RetentionConfig) -> Self {
        Self {
            raw_retention_secs: c.raw_retention_secs,
            one_minute_retention_secs: c.one_minute_retention_secs,
            one_hour_retention_secs: c.one_hour_retention_secs,
            one_day_retention_secs: c.one_day_retention_secs,
            security_events_retention_secs: c.security_events_retention_secs,
            max_db_size_bytes: c.max_db_size_bytes,
            auto_vacuum: c.auto_vacuum,
        }
    }
}

impl From<RetentionConfigDto> for RetentionConfig {
    fn from(d: RetentionConfigDto) -> Self {
        Self {
            raw_retention_secs: d.raw_retention_secs,
            one_minute_retention_secs: d.one_minute_retention_secs,
            one_hour_retention_secs: d.one_hour_retention_secs,
            one_day_retention_secs: d.one_day_retention_secs,
            security_events_retention_secs: d.security_events_retention_secs,
            max_db_size_bytes: d.max_db_size_bytes,
            auto_vacuum: d.auto_vacuum,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct DatabaseTableStatDto {
    pub table_name: String,
    pub row_count: i64,
}

impl From<panel_analytics::DatabaseTableStat> for DatabaseTableStatDto {
    fn from(t: panel_analytics::DatabaseTableStat) -> Self {
        Self {
            table_name: t.table_name,
            row_count: t.row_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct DatabaseStorageStatsDto {
    pub db_size_bytes: i64,
    pub wal_size_bytes: i64,
    pub total_file_size_bytes: i64,
    pub page_size: i64,
    pub page_count: i64,
    pub freelist_count: i64,
    pub free_bytes: i64,
    pub fragmentation_ratio: f64,
    pub max_db_size_bytes: Option<i64>,
    pub is_healthy: bool,
    pub integrity_message: String,
    pub table_stats: Vec<DatabaseTableStatDto>,
}

impl From<DatabaseStorageStats> for DatabaseStorageStatsDto {
    fn from(s: DatabaseStorageStats) -> Self {
        Self {
            db_size_bytes: s.db_size_bytes,
            wal_size_bytes: s.wal_size_bytes,
            total_file_size_bytes: s.total_file_size_bytes,
            page_size: s.page_size,
            page_count: s.page_count,
            freelist_count: s.freelist_count,
            free_bytes: s.free_bytes,
            fragmentation_ratio: s.fragmentation_ratio,
            max_db_size_bytes: s.max_db_size_bytes,
            is_healthy: s.is_healthy,
            integrity_message: s.integrity_message,
            table_stats: s.table_stats.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct RetentionMaintenanceReportDto {
    pub timestamp: i64,
    pub raw_records_pruned: usize,
    pub one_minute_records_pruned: usize,
    pub one_hour_records_pruned: usize,
    pub one_day_records_pruned: usize,
    pub security_events_pruned: usize,
    pub checkpoints_pruned: usize,
    pub total_records_pruned: usize,
    pub wal_checkpoint_executed: bool,
    pub vacuum_executed: bool,
    pub db_size_bytes_before: i64,
    pub db_size_bytes_after: i64,
    pub wal_size_bytes_before: i64,
    pub wal_size_bytes_after: i64,
    pub duration_ms: u64,
    pub emergency_pruning_triggered: bool,
}

impl From<RetentionMaintenanceReport> for RetentionMaintenanceReportDto {
    fn from(r: RetentionMaintenanceReport) -> Self {
        Self {
            timestamp: r.timestamp,
            raw_records_pruned: r.raw_records_pruned,
            one_minute_records_pruned: r.one_minute_records_pruned,
            one_hour_records_pruned: r.one_hour_records_pruned,
            one_day_records_pruned: r.one_day_records_pruned,
            security_events_pruned: r.security_events_pruned,
            checkpoints_pruned: r.checkpoints_pruned,
            total_records_pruned: r.total_records_pruned,
            wal_checkpoint_executed: r.wal_checkpoint_executed,
            vacuum_executed: r.vacuum_executed,
            db_size_bytes_before: r.db_size_bytes_before,
            db_size_bytes_after: r.db_size_bytes_after,
            wal_size_bytes_before: r.wal_size_bytes_before,
            wal_size_bytes_after: r.wal_size_bytes_after,
            duration_ms: r.duration_ms,
            emergency_pruning_triggered: r.emergency_pruning_triggered,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct IntegrityCheckReportDto {
    pub is_healthy: bool,
    pub status: String,
    pub checked_at: i64,
    pub details: Vec<String>,
}

impl From<IntegrityCheckReport> for IntegrityCheckReportDto {
    fn from(r: IntegrityCheckReport) -> Self {
        Self {
            is_healthy: r.is_healthy,
            status: r.status,
            checked_at: r.checked_at,
            details: r.details,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AnalyticsRetentionResponse {
    pub retention: RetentionConfigDto,
    pub stats: DatabaseStorageStatsDto,
    pub last_maintenance: Option<RetentionMaintenanceReportDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRetentionRequest {
    /// Raw metrics retention in hours (1h to 168h / 7 days). Default: 2.
    pub raw_retention_hours: Option<i64>,
    /// 1-minute metrics rollup retention in days (1d to 90d). Default: 14.
    pub one_minute_retention_days: Option<i64>,
    /// 1-hour metrics rollup retention in days (7d to 1000d). Default: 400.
    pub one_hour_retention_days: Option<i64>,
    /// 1-day metrics rollup retention in days (None or null for unlimited).
    pub one_day_retention_days: Option<Option<i64>>,
    /// Security events retention in days (1d to 90d). Default: 7.
    pub security_events_retention_days: Option<i64>,
    /// Maximum SQLite database file size limit in MB (min 50 MB, null for unlimited).
    pub max_db_size_mb: Option<Option<i64>>,
    /// Automatically run SQLite VACUUM during maintenance if fragmented.
    pub auto_vacuum: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct TriggerMaintenanceRequest {
    /// Force database VACUUM regardless of fragmentation threshold.
    pub force_vacuum: Option<bool>,
}

// ==========================================
// Persistence Helpers
// ==========================================

pub async fn load_retention_config(setting_repo: &SqliteSettingRepository) -> RetentionConfig {
    match setting_repo.get(SETTING_RETENTION_CONFIG_KEY).await {
        Ok(Some(raw)) => serde_json::from_str(&raw).unwrap_or_default(),
        _ => RetentionConfig::default(),
    }
}

pub async fn save_retention_config(
    setting_repo: &SqliteSettingRepository,
    config: &RetentionConfig,
) -> Result<(), String> {
    let json_val = serde_json::to_string(config)
        .map_err(|e| format!("Gagal memformat konfigurasi retensi: {e}"))?;
    setting_repo
        .set(SETTING_RETENTION_CONFIG_KEY, &json_val)
        .await
        .map_err(|e| format!("Gagal menyimpan konfigurasi retensi ke database: {e}"))?;
    Ok(())
}

pub async fn load_last_maintenance_report(
    setting_repo: &SqliteSettingRepository,
) -> Option<RetentionMaintenanceReport> {
    match setting_repo.get(SETTING_LAST_MAINTENANCE_KEY).await {
        Ok(Some(raw)) => serde_json::from_str(&raw).ok(),
        _ => None,
    }
}

pub async fn save_last_maintenance_report(
    setting_repo: &SqliteSettingRepository,
    report: &RetentionMaintenanceReport,
) -> Result<(), String> {
    let json_val = serde_json::to_string(report)
        .map_err(|e| format!("Gagal memformat laporan maintenance: {e}"))?;
    setting_repo
        .set(SETTING_LAST_MAINTENANCE_KEY, &json_val)
        .await
        .map_err(|e| format!("Gagal menyimpan laporan maintenance: {e}"))?;
    Ok(())
}

// ==========================================
// REST Handlers
// ==========================================

/// GET /api/v1/system/analytics/retention
///
/// Returns current retention configuration, database storage statistics,
/// and last maintenance summary report. (Administrator only)
#[utoipa::path(
    get,
    path = "/api/v1/system/analytics/retention",
    responses(
        (status = 200, description = "Analytics retention status and storage stats", body = AnalyticsRetentionResponse),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Unauthorized - Admin required")
    ),
    tag = "Analytics"
)]
pub async fn get_analytics_retention_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Response {
    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Forbidden",
                "status": 403,
                "detail": "Hanya administrator yang diizinkan mengakses pengaturan siklus data dan retensi."
            })),
        )
            .into_response();
    }

    let storage = match state.get_analytics().await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (
                code,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Storage Connection Error",
                    "status": code.as_u16(),
                    "detail": msg
                })),
            )
                .into_response();
        }
    };

    let setting_repo = SqliteSettingRepository::new(state.pool.clone());
    let retention = load_retention_config(&setting_repo).await;
    let last_maintenance = load_last_maintenance_report(&setting_repo).await;

    let mut stats = match storage.get_storage_stats().await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Database Query Error",
                    "status": 500,
                    "detail": format!("Gagal mengambil statistik penyimpanan basis data analitik: {e}")
                })),
            )
                .into_response();
        }
    };

    stats.max_db_size_bytes = retention.max_db_size_bytes;

    let stats_dto: DatabaseStorageStatsDto = stats.into();
    let retention_dto: RetentionConfigDto = retention.into();
    let last_maint_dto = last_maintenance.map(Into::into);

    Json(AnalyticsRetentionResponse {
        retention: retention_dto,
        stats: stats_dto,
        last_maintenance: last_maint_dto,
    })
    .into_response()
}

/// PUT /api/v1/system/analytics/retention
///
/// Updates analytics retention policy periods and storage limits. (Administrator only)
#[utoipa::path(
    put,
    path = "/api/v1/system/analytics/retention",
    request_body = UpdateRetentionRequest,
    responses(
        (status = 200, description = "Retention policy successfully updated", body = RetentionConfigDto),
        (status = 400, description = "Invalid retention parameters"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Unauthorized - Admin required")
    ),
    tag = "Analytics"
)]
pub async fn update_analytics_retention_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<UpdateRetentionRequest>,
) -> Response {
    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Forbidden",
                "status": 403,
                "detail": "Hanya administrator yang diizinkan memperbarui pengaturan retensi analitik."
            })),
        )
            .into_response();
    }

    // Guardrail validations
    if matches!(payload.raw_retention_hours, Some(hours) if !(1..=168).contains(&hours)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Invalid Parameter",
                "status": 400,
                "detail": "Durasi retensi metrik mentah harus antara 1 jam hingga 168 jam (7 hari)."
            })),
        )
            .into_response();
    }

    if matches!(payload.one_minute_retention_days, Some(days) if !(1..=90).contains(&days)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Invalid Parameter",
                "status": 400,
                "detail": "Durasi retensi metrik 1 menit harus antara 1 hari hingga 90 hari."
            })),
        )
            .into_response();
    }

    if matches!(payload.one_hour_retention_days, Some(days) if !(7..=1000).contains(&days)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Invalid Parameter",
                "status": 400,
                "detail": "Durasi retensi metrik 1 jam harus antara 7 hari hingga 1000 hari."
            })),
        )
            .into_response();
    }

    if matches!(payload.security_events_retention_days, Some(days) if !(1..=90).contains(&days)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Invalid Parameter",
                "status": 400,
                "detail": "Durasi retensi event keamanan harus antara 1 hari hingga 90 hari."
            })),
        )
            .into_response();
    }

    if matches!(payload.max_db_size_mb, Some(Some(mb)) if mb < 50) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Invalid Parameter",
                "status": 400,
                "detail": "Batas ukuran maksimum database analitik minimal 50 MB."
            })),
        )
            .into_response();
    }

    let setting_repo = SqliteSettingRepository::new(state.pool.clone());
    let mut config = load_retention_config(&setting_repo).await;

    if let Some(hours) = payload.raw_retention_hours {
        config.raw_retention_secs = hours * 3600;
    }
    if let Some(days) = payload.one_minute_retention_days {
        config.one_minute_retention_secs = days * 86400;
    }
    if let Some(days) = payload.one_hour_retention_days {
        config.one_hour_retention_secs = days * 86400;
    }
    if let Some(opt_days) = payload.one_day_retention_days {
        config.one_day_retention_secs = opt_days.map(|d| d * 86400);
    }
    if let Some(days) = payload.security_events_retention_days {
        config.security_events_retention_secs = days * 86400;
    }
    if let Some(opt_mb) = payload.max_db_size_mb {
        config.max_db_size_bytes = opt_mb.map(|mb| mb * 1024 * 1024);
    }
    if let Some(auto_vac) = payload.auto_vacuum {
        config.auto_vacuum = auto_vac;
    }

    if let Err(err) = save_retention_config(&setting_repo, &config).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Save Setting Failed",
                "status": 500,
                "detail": err
            })),
        )
            .into_response();
    }

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "setting.update",
        "retention",
        "success",
        None,
        Some(format!(
            "{{\"raw\":{},\"1m\":{},\"1h\":{},\"sec\":{}}}",
            config.raw_retention_secs / 3600,
            config.one_minute_retention_secs / 86400,
            config.one_hour_retention_secs / 86400,
            config.security_events_retention_secs / 86400
        )),
    );

    Json(RetentionConfigDto::from(config)).into_response()
}

/// POST /api/v1/system/analytics/maintenance
///
/// Triggers manual database pruning, WAL checkpoint truncation, and optional VACUUM.
/// (Administrator only)
#[utoipa::path(
    post,
    path = "/api/v1/system/analytics/maintenance",
    request_body = Option<TriggerMaintenanceRequest>,
    responses(
        (status = 200, description = "Maintenance completed successfully", body = RetentionMaintenanceReportDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Unauthorized - Admin required")
    ),
    tag = "Analytics"
)]
pub async fn trigger_analytics_maintenance_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    body: Option<Json<TriggerMaintenanceRequest>>,
) -> Response {
    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Forbidden",
                "status": 403,
                "detail": "Hanya administrator yang diizinkan menjalankan pemeliharaan basis data analitik."
            })),
        )
            .into_response();
    }

    let storage = match state.get_analytics().await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (
                code,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Storage Connection Error",
                    "status": code.as_u16(),
                    "detail": msg
                })),
            )
                .into_response();
        }
    };

    let force_vacuum = body.and_then(|b| b.force_vacuum).unwrap_or(false);

    let setting_repo = SqliteSettingRepository::new(state.pool.clone());
    let config = load_retention_config(&setting_repo).await;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let report = match storage.run_maintenance(now, &config, force_vacuum).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Maintenance Execution Error",
                    "status": 500,
                    "detail": format!("Gagal mengeksekusi pemeliharaan siklus data: {e}")
                })),
            )
                .into_response();
        }
    };

    let _ = save_last_maintenance_report(&setting_repo, &report).await;

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "analytics.maintenance",
        "analytics.db",
        "success",
        None,
        Some(format!(
            "{{\"pruned\":{},\"wal_checkpoint\":true,\"vacuum\":{}}}",
            report.total_records_pruned, report.vacuum_executed
        )),
    );

    Json(RetentionMaintenanceReportDto::from(report)).into_response()
}

/// POST /api/v1/system/analytics/integrity-check
///
/// Executes SQLite quick_check integrity inspection. (Administrator only)
#[utoipa::path(
    post,
    path = "/api/v1/system/analytics/integrity-check",
    responses(
        (status = 200, description = "Integrity check completed", body = IntegrityCheckReportDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Unauthorized - Admin required")
    ),
    tag = "Analytics"
)]
pub async fn check_analytics_integrity_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Response {
    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Forbidden",
                "status": 403,
                "detail": "Hanya administrator yang diizinkan menguji integritas basis data analitik."
            })),
        )
            .into_response();
    }

    let storage = match state.get_analytics().await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (
                code,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Storage Connection Error",
                    "status": code.as_u16(),
                    "detail": msg
                })),
            )
                .into_response();
        }
    };

    match storage.check_integrity().await {
        Ok(report) => Json(IntegrityCheckReportDto::from(report)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Integrity Check Failed",
                "status": 500,
                "detail": format!("Gagal menjalankan uji integritas: {e}")
            })),
        )
            .into_response(),
    }
}

// ==========================================
// Background Retention Worker
// ==========================================

/// Background scheduled retention maintenance runner.
async fn is_retention_cycle_due(setting_repo: &SqliteSettingRepository, now: i64) -> bool {
    let last_report = load_last_maintenance_report(setting_repo).await;
    match last_report {
        None => true,
        Some(ref rep) => (now - rep.timestamp) >= 24 * 3600,
    }
}

async fn run_maintenance_task(
    storage: Arc<panel_analytics::AnalyticsStorage>,
    config: &panel_analytics::RetentionConfig,
    setting_repo: &SqliteSettingRepository,
    now: i64,
) {
    info!("Executing scheduled 24h retention pruning and WAL checkpoint...");
    match storage.run_maintenance(now, config, false).await {
        Ok(report) => {
            info!(
                "Scheduled retention maintenance completed: {} records pruned, WAL truncated in {}ms",
                report.total_records_pruned, report.duration_ms
            );
            let _ = save_last_maintenance_report(setting_repo, &report).await;
        }
        Err(e) => {
            error!("Error running scheduled retention maintenance: {}", e);
        }
    }
}

async fn execute_retention_cycle(
    pool: &panel_db::pool::DbPool,
    analytics_storage: &Arc<tokio::sync::RwLock<Option<Arc<panel_analytics::AnalyticsStorage>>>>,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let setting_repo = SqliteSettingRepository::new(pool.clone());
    if !is_retention_cycle_due(&setting_repo, now).await {
        return;
    }

    let config = load_retention_config(&setting_repo).await;
    let storage_opt = {
        let lock = analytics_storage.read().await;
        lock.clone()
    };

    if let Some(storage) = storage_opt {
        run_maintenance_task(storage, &config, &setting_repo, now).await;
    }
}

/// Background worker running periodic retention maintenance and WAL checkpoint truncation.
/// Runs once every hour, checking if 24 hours have elapsed since last cycle
/// or if database size limit has been breached.
pub async fn run_retention_worker(
    pool: panel_db::pool::DbPool,
    analytics_storage: Arc<tokio::sync::RwLock<Option<Arc<panel_analytics::AnalyticsStorage>>>>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    info!("Starting background Analytics Retention & Maintenance worker...");

    loop {
        // Sleep for 1 hour or until shutdown signal
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(3600)) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Analytics retention worker received shutdown signal, stopping gracefully.");
                    break;
                }
            }
        }

        if *shutdown_rx.borrow() {
            break;
        }

        execute_retention_cycle(&pool, &analytics_storage).await;
    }
}
