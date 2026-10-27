//! Server Optimizer REST API handlers, DTO schemas, and admin operations.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use panel_core::capacity::{
    CapacityError, PackageDemand, PackagePreset, PackageUse, recommend_package,
};
use panel_proto::{
    AppRuntimeHints, NginxTuningConfig, OptimizerApplyRequest, OptimizerProfile,
    OptimizerProfileType, OptimizerRollbackRequest, PhpFpmTuningConfig, ServerHardwareSpecs,
    SysctlTuningConfig,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;

/// Hardware specifications of host server.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServerHardwareSpecsDto {
    pub vcpu_count: u32,
    pub ram_total_mb: u64,
    pub ram_free_mb: u64,
    pub swap_total_mb: u64,
    pub swap_free_mb: u64,
    pub kernel_version: String,
    pub os_distro: String,
}

impl From<ServerHardwareSpecs> for ServerHardwareSpecsDto {
    fn from(s: ServerHardwareSpecs) -> Self {
        Self {
            vcpu_count: s.vcpu_count,
            ram_total_mb: s.ram_total_mb,
            ram_free_mb: s.ram_free_mb,
            swap_total_mb: s.swap_total_mb,
            swap_free_mb: s.swap_free_mb,
            kernel_version: s.kernel_version,
            os_distro: s.os_distro,
        }
    }
}

/// Nginx performance tuning parameters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NginxTuningConfigDto {
    pub worker_processes: String,
    pub worker_connections: u32,
    pub worker_rlimit_nofile: u32,
    pub multi_accept: bool,
    pub keepalive_timeout_secs: u32,
    pub client_body_buffer_size_kb: u32,
    pub fastcgi_buffer_size_kb: u32,
    pub fastcgi_buffers: String,
    pub open_file_cache_max: u32,
    pub open_file_cache_inactive_secs: u32,
}

impl From<NginxTuningConfig> for NginxTuningConfigDto {
    fn from(c: NginxTuningConfig) -> Self {
        Self {
            worker_processes: c.worker_processes,
            worker_connections: c.worker_connections,
            worker_rlimit_nofile: c.worker_rlimit_nofile,
            multi_accept: c.multi_accept,
            keepalive_timeout_secs: c.keepalive_timeout_secs,
            client_body_buffer_size_kb: c.client_body_buffer_size_kb,
            fastcgi_buffer_size_kb: c.fastcgi_buffer_size_kb,
            fastcgi_buffers: c.fastcgi_buffers,
            open_file_cache_max: c.open_file_cache_max,
            open_file_cache_inactive_secs: c.open_file_cache_inactive_secs,
        }
    }
}

/// Kernel sysctl tuning parameters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SysctlTuningConfigDto {
    pub somaxconn: u32,
    pub tcp_max_syn_backlog: u32,
    pub tcp_tw_reuse: u32,
    pub ip_local_port_range: String,
    pub swappiness: u32,
    pub file_max: u64,
    pub netdev_max_backlog: u32,
}

impl From<SysctlTuningConfig> for SysctlTuningConfigDto {
    fn from(c: SysctlTuningConfig) -> Self {
        Self {
            somaxconn: c.somaxconn,
            tcp_max_syn_backlog: c.tcp_max_syn_backlog,
            tcp_tw_reuse: c.tcp_tw_reuse,
            ip_local_port_range: c.ip_local_port_range,
            swappiness: c.swappiness,
            file_max: c.file_max,
            netdev_max_backlog: c.netdev_max_backlog,
        }
    }
}

/// PHP-FPM pool tuning parameters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpFpmTuningConfigDto {
    pub pm_type: String,
    pub max_children: u32,
    pub start_servers: Option<u32>,
    pub min_spare_servers: Option<u32>,
    pub max_spare_servers: Option<u32>,
    pub process_idle_timeout_secs: u32,
    pub max_requests: u32,
}

impl From<PhpFpmTuningConfig> for PhpFpmTuningConfigDto {
    fn from(c: PhpFpmTuningConfig) -> Self {
        Self {
            pm_type: c.pm_type,
            max_children: c.max_children,
            start_servers: c.start_servers,
            min_spare_servers: c.min_spare_servers,
            max_spare_servers: c.max_spare_servers,
            process_idle_timeout_secs: c.process_idle_timeout_secs,
            max_requests: c.max_requests,
        }
    }
}

/// Hints for Node.js and Python runtimes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppRuntimeHintsDto {
    pub recommended_node_workers: u32,
    pub recommended_node_max_old_space_mb: u32,
    pub recommended_python_workers: u32,
}

impl From<AppRuntimeHints> for AppRuntimeHintsDto {
    fn from(h: AppRuntimeHints) -> Self {
        Self {
            recommended_node_workers: h.recommended_node_workers,
            recommended_node_max_old_space_mb: h.recommended_node_max_old_space_mb,
            recommended_python_workers: h.recommended_python_workers,
        }
    }
}

/// Complete optimizer profile settings.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OptimizerProfileDto {
    pub profile_type: String,
    pub display_name: String,
    pub description: String,
    pub nginx: NginxTuningConfigDto,
    pub sysctl: SysctlTuningConfigDto,
    pub php_fpm: PhpFpmTuningConfigDto,
    pub hints: AppRuntimeHintsDto,
}

impl From<OptimizerProfile> for OptimizerProfileDto {
    fn from(p: OptimizerProfile) -> Self {
        Self {
            profile_type: p.profile_type.as_str().to_string(),
            display_name: p.display_name,
            description: p.description,
            nginx: p.nginx.into(),
            sysctl: p.sysctl.into(),
            php_fpm: p.php_fpm.into(),
            hints: p.hints.into(),
        }
    }
}

/// Status and configuration overview of server optimizer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OptimizerStatusResponseDto {
    pub hardware: ServerHardwareSpecsDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<panel_core::capacity::CapacitySnapshot>,
    /// Present only when a deterministic capacity preview was requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_preset: Option<PackagePreset>,
    pub recommended_profile: String,
    pub active_profile: Option<String>,
    pub applied_at: Option<u64>,
    pub current_settings: OptimizerProfileDto,
    pub recommended_settings: OptimizerProfileDto,
    pub has_backup: bool,
    pub last_backup_at: Option<u64>,
}

/// Request to apply an automated or custom server tuning profile.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApplyOptimizerRequestDto {
    /// Profile type: "micro", "standard", "high_performance", "dedicated_large", "custom"
    pub profile: String,
}

/// Action response after applying or rolling back tuning profile.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OptimizerActionResponseDto {
    pub success: bool,
    pub profile: Option<String>,
    pub message: String,
    pub nginx_reloaded: bool,
    pub sysctl_applied: bool,
    pub timestamp: u64,
}

/// Helper to parse profile string to OptimizerProfileType enum.
fn parse_profile_type(s: &str) -> Result<OptimizerProfileType, String> {
    match s.to_lowercase().replace('-', "_").as_str() {
        "micro" | "vps_micro" | "low" => Ok(OptimizerProfileType::Micro),
        "standard" | "std" | "balanced" => Ok(OptimizerProfileType::Standard),
        "high_performance" | "perf" | "high" => Ok(OptimizerProfileType::HighPerformance),
        "dedicated_large" | "dedicated" | "large" => Ok(OptimizerProfileType::DedicatedLarge),
        "custom" => Ok(OptimizerProfileType::Custom),
        _ => Err(format!(
            "Profil optimizer tidak dikenal: '{s}'. Nilai valid: micro, standard, high_performance, dedicated_large"
        )),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CapacityQuery {
    /// Read-only package preview. Does not invoke an AI provider or apply limits.
    pub capacity_usage: Option<PackageUse>,
    pub projects_per_tenant: Option<u32>,
    pub tenant_slots: Option<u32>,
}

impl CapacityQuery {
    fn demand(&self) -> Result<Option<PackageDemand>, CapacityError> {
        let Some(usage) = self.capacity_usage else {
            return if self.projects_per_tenant.is_some() || self.tenant_slots.is_some() {
                Err(CapacityError::InvalidDemand)
            } else {
                Ok(None)
            };
        };
        let demand = PackageDemand {
            usage,
            projects_per_tenant: self.projects_per_tenant.unwrap_or(
                if usage == PackageUse::MultiProject {
                    2
                } else {
                    1
                },
            ),
            tenant_slots: self
                .tenant_slots
                .unwrap_or(if usage == PackageUse::MultiTenant {
                    2
                } else {
                    1
                }),
        };
        demand.validate()?;
        Ok(Some(demand))
    }
}

fn capacity_error(error: CapacityError) -> Response {
    let (status, code) = match &error {
        CapacityError::InvalidDemand => (StatusCode::BAD_REQUEST, "invalid_capacity_demand"),
        CapacityError::Stale => (StatusCode::CONFLICT, "capacity_snapshot_stale"),
        CapacityError::Insufficient => (StatusCode::CONFLICT, "capacity_insufficient"),
        CapacityError::InvalidMeasurement => (StatusCode::SERVICE_UNAVAILABLE, "capacity_invalid"),
        CapacityError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "capacity_unavailable"),
    };
    (
        status,
        Json(serde_json::json!({"code":code,"detail":error.to_string()})),
    )
        .into_response()
}

/// Get server hardware specs, tuning profile status, and recommendations.
#[utoipa::path(
    get,
    path = "/api/v1/system/optimizer",
    params(CapacityQuery),
    responses(
        (status = 200, description = "Status and recommendations of server optimizer", body = OptimizerStatusResponseDto),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Admin access required"),
        (status = 400, description = "Invalid capacity demand"),
        (status = 409, description = "Insufficient capacity or stale snapshot"),
        (status = 503, description = "Verified capacity unavailable"),
        (status = 502, description = "Bad Gateway: Agent communication failure")
    ),
    tag = "system"
)]
pub async fn get_optimizer_status_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<CapacityQuery>,
) -> Response {
    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Forbidden",
                "status": 403,
                "detail": "Hanya administrator yang berhak mengakses pengaturan optimizer server."
            })),
        )
            .into_response();
    }

    let demand = match query.demand() {
        Ok(demand) => demand,
        Err(error) => return capacity_error(error),
    };
    let status = match state.agent_client.optimizer_status().await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Agent Error",
                    "status": 502,
                    "detail": format!("Gagal menghubungi agent optimizer: {e}")
                })),
            )
                .into_response();
        }
    };

    let package_preset = if let Some(demand) = demand {
        let Some(snapshot) = &status.capacity else {
            return capacity_error(CapacityError::Unavailable);
        };
        let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(value) => value.as_secs(),
            Err(_) => return capacity_error(CapacityError::Stale),
        };
        match recommend_package(snapshot, demand, now) {
            Ok(preset) => Some(preset),
            Err(error) => return capacity_error(error),
        }
    } else {
        None
    };
    let dto = OptimizerStatusResponseDto {
        hardware: status.hardware.into(),
        capacity: status.capacity,
        package_preset,
        recommended_profile: status.recommended_profile.as_str().to_string(),
        active_profile: status.active_profile.map(|p| p.as_str().to_string()),
        applied_at: status.applied_at,
        current_settings: status.current_settings.into(),
        recommended_settings: status.recommended_settings.into(),
        has_backup: status.has_backup,
        last_backup_at: status.last_backup_at,
    };

    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(dto),
    )
        .into_response()
}

/// Apply a specific performance tuning profile to Nginx and kernel sysctl.
#[utoipa::path(
    post,
    path = "/api/v1/system/optimizer/apply",
    request_body = ApplyOptimizerRequestDto,
    responses(
        (status = 200, description = "Tuning profile applied successfully", body = OptimizerActionResponseDto),
        (status = 400, description = "Bad Request: Validation failed or rollback triggered"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Admin access required"),
        (status = 502, description = "Bad Gateway: Agent communication failure")
    ),
    tag = "system"
)]
pub async fn apply_optimizer_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<ApplyOptimizerRequestDto>,
) -> Response {
    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Forbidden",
                "status": 403,
                "detail": "Hanya administrator yang berhak menerapkan tuning profil server."
            })),
        )
            .into_response();
    }

    let profile_type = match parse_profile_type(&payload.profile) {
        Ok(p) => p,
        Err(msg) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Validation Error",
                    "status": 400,
                    "detail": msg
                })),
            )
                .into_response();
        }
    };

    let req = OptimizerApplyRequest {
        profile: profile_type,
        custom_tuning: None,
    };

    let result = match state.agent_client.optimizer_apply(req).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Optimizer Apply Error",
                    "status": 400,
                    "detail": format!("{e}")
                })),
            )
                .into_response();
        }
    };

    // Record audit log asynchronously
    state.audit_writer.log_async(
        Some(auth_user.id),
        auth_user.username.clone(),
        "optimizer.apply",
        "system",
        "success",
        None,
        Some(format!(
            "Applied server tuning profile: {}",
            result.applied_profile.as_str()
        )),
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let dto = OptimizerActionResponseDto {
        success: result.success,
        profile: Some(result.applied_profile.as_str().to_string()),
        message: result.message,
        nginx_reloaded: result.nginx_reloaded,
        sysctl_applied: result.sysctl_applied,
        timestamp: now,
    };

    (StatusCode::OK, Json(dto)).into_response()
}

/// Rollback server tuning configurations to the previous state.
#[utoipa::path(
    post,
    path = "/api/v1/system/optimizer/rollback",
    responses(
        (status = 200, description = "Configuration successfully rolled back", body = OptimizerActionResponseDto),
        (status = 400, description = "Bad Request: No backup found to rollback"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Admin access required"),
        (status = 502, description = "Bad Gateway: Agent communication failure")
    ),
    tag = "system"
)]
pub async fn rollback_optimizer_handler(
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
                "detail": "Hanya administrator yang berhak melakukan rollback profil server."
            })),
        )
            .into_response();
    }

    let result = match state
        .agent_client
        .optimizer_rollback(OptimizerRollbackRequest {})
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Optimizer Rollback Error",
                    "status": 400,
                    "detail": format!("{e}")
                })),
            )
                .into_response();
        }
    };

    // Record audit log asynchronously
    state.audit_writer.log_async(
        Some(auth_user.id),
        auth_user.username.clone(),
        "optimizer.rollback",
        "system",
        "success",
        None,
        Some(format!(
            "Rolled back server tuning configuration to: {:?}",
            result.restored_profile.map(|p| p.as_str())
        )),
    );

    let dto = OptimizerActionResponseDto {
        success: result.success,
        profile: result.restored_profile.map(|p| p.as_str().to_string()),
        message: result.message,
        nginx_reloaded: true,
        sysctl_applied: true,
        timestamp: result.restored_at,
    };

    (StatusCode::OK, Json(dto)).into_response()
}
