//! PHP runtime management endpoints.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_db::models::UpdateSite;
use panel_db::repositories::{
    SqliteDomainRepository, SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository,
};
use panel_runtime::php::{CURATED_PHP_EXTENSIONS, PhpVersion, find_curated_extension};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{RequireAdminCsrf, RequireAuth, RequireCsrf};
use crate::jobs::JobItem;
use crate::routes::AppState;
use crate::sites::{fetch_and_authorize_site, recompile_and_apply_site_vhost};

/// Status information of a specific PHP runtime version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpVersionStatusItem {
    pub version: String,
    pub latest_release: Option<String>,
    pub installed: bool,
    pub fpm_binary_exists: bool,
    pub cli_binary_exists: bool,
    pub service_masked: bool,
    pub status: String,
    pub fpm_binary_path: String,
    pub cli_binary_path: String,
    pub service_name: String,
}

/// Response containing all PHP runtime versions and PPA configuration status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpRuntimeListApiResponse {
    pub ppa_configured: bool,
    pub versions: Vec<PhpVersionStatusItem>,
}

impl From<panel_proto::PhpVersionStatusDto> for PhpVersionStatusItem {
    fn from(dto: panel_proto::PhpVersionStatusDto) -> Self {
        Self {
            version: dto.version,
            latest_release: None,
            installed: dto.installed,
            fpm_binary_exists: dto.fpm_binary_exists,
            cli_binary_exists: dto.cli_binary_exists,
            service_masked: dto.service_masked,
            status: dto.status,
            fpm_binary_path: dto.fpm_binary_path,
            cli_binary_path: dto.cli_binary_path,
            service_name: dto.service_name,
        }
    }
}

impl From<panel_proto::PhpRuntimeListResponse> for PhpRuntimeListApiResponse {
    fn from(res: panel_proto::PhpRuntimeListResponse) -> Self {
        Self {
            ppa_configured: res.ppa_configured,
            versions: res.versions.into_iter().map(Into::into).collect(),
        }
    }
}

/// Overview information for a tenant's PHP-FPM master instance.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpFpmMasterOverviewDto {
    pub username: String,
    pub version: String,
    pub service_name: String,
    pub is_active: bool,
    pub substate: String,
    pub socket_exists: bool,
    pub pools_count: usize,
}

/// Detailed status information for a specific master.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpFpmMasterStatusDto {
    pub service_name: String,
    pub is_active: bool,
    pub substate: String,
    pub socket_exists: bool,
    pub pools_count: usize,
}

/// Generic message response for PHP-FPM actions (reload, restart).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpFpmActionResponseDto {
    pub service_name: String,
    pub action: String,
    pub success: bool,
    pub message: String,
}

/// Request payload to switch a website's PHP version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SwitchPhpVersionRequest {
    pub version: String,
}

/// Response returned after successfully switching a website's PHP version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SwitchPhpVersionResponse {
    pub site_id: i64,
    pub previous_version: String,
    pub current_version: String,
    pub status: String,
    pub message: String,
}

/// Response returned for a site's PHP ini and runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SitePhpSettingsResponse {
    pub site_id: i64,
    pub php_version: String,
    pub settings: panel_core::php::PhpSettings,
    pub limits: panel_core::php::PhpSettingsLimits,
    pub admin_ini: Option<String>,
    pub is_admin: bool,
}

/// Request payload to update curated and admin PHP settings.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSitePhpSettingsInput {
    pub settings: panel_core::php::PhpSettings,
    #[serde(default)]
    pub admin_ini: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/php/versions",
    responses(
        (status = 200, description = "List available and installed PHP runtime versions", body = PhpRuntimeListApiResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Failed to communicate with agent")
    ),
    tag = "runtime"
)]
pub async fn list_php_versions_handler(
    State(state): State<AppState>,
    RequireAuth(_user): RequireAuth,
) -> Response {
    match state.agent_client.list_php_runtimes().await {
        Ok(res) => {
            let mut response = PhpRuntimeListApiResponse::from(res);
            if let Ok(cache) = crate::runtime_catalog::php_cache(&state.pool).await
                && cache.fresh(chrono::Utc::now().timestamp())
            {
                // The agent's PHP whitelist includes compatibility runtimes such as 7.4.
                // Upstream active releases enrich patch metadata, not version availability.
                for item in &mut response.versions {
                    item.latest_release = cache
                        .data
                        .iter()
                        .find(|r| r.branch == item.version)
                        .map(|r| r.latest.clone());
                }
            }
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal memuat status runtime PHP: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/php/repo/setup",
    responses(
        (status = 202, description = "PHP Sury/PPA repository setup job enqueued", body = JobItem),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin privileges and CSRF required)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn setup_php_repo_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(admin): RequireAdminCsrf,
) -> Response {
    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(admin.id),
            None,
            "php.repo.setup".to_string(),
            "{}".to_string(),
            Some(3),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal membuat antrean job: {}", e) })),
            )
                .into_response();
        }
    };

    state.job_runner.notify.notify_one();

    state.audit_writer.log_async(
        Some(admin.id),
        &admin.username,
        "php.repo.setup",
        "system:apt".to_string(),
        "success",
        None,
        Some("Setup OS-specific Sury PHP repository".to_string()),
    );

    (StatusCode::ACCEPTED, Json(JobItem::from(job))).into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/php/versions/{version}/install",
    params(
        ("version" = String, Path, description = "PHP version (7.4, 8.0, 8.1, 8.2, 8.3, 8.4)")
    ),
    responses(
        (status = 202, description = "PHP install job enqueued", body = JobItem),
        (status = 400, description = "Invalid PHP version"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin privileges and CSRF required)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn install_php_version_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(admin): RequireAdminCsrf,
    Path(version): Path<String>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }

    let payload = serde_json::json!({ "version": version });
    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(admin.id),
            None,
            "php.install".to_string(),
            payload.to_string(),
            Some(3),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal membuat antrean job instalasi: {}", e) })),
            )
                .into_response();
        }
    };

    state.job_runner.notify.notify_one();

    state.audit_writer.log_async(
        Some(admin.id),
        &admin.username,
        "php.install",
        format!("php:{}", version),
        "success",
        None,
        Some(format!("Memulai instalasi PHP {}", version)),
    );

    (StatusCode::ACCEPTED, Json(JobItem::from(job))).into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/php/versions/{version}/remove",
    params(
        ("version" = String, Path, description = "PHP version (7.4, 8.0, 8.1, 8.2, 8.3, 8.4)")
    ),
    responses(
        (status = 202, description = "PHP remove job enqueued", body = JobItem),
        (status = 400, description = "Invalid PHP version or version currently in use by sites"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin privileges and CSRF required)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn remove_php_version_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(admin): RequireAdminCsrf,
    Path(version): Path<String>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }

    // Pre-check: Ensure no sites are currently using this PHP version
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    match site_repo
        .count_by_runtime_and_version("php", &version)
        .await
    {
        Ok(count) if count > 0 => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Versi PHP {} sedang digunakan oleh {} situs dan tidak dapat dihapus", version, count)
                })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal memeriksa database situs: {}", e) })),
            )
                .into_response();
        }
        Ok(_) => {}
    }

    let payload = serde_json::json!({ "version": version });
    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(admin.id),
            None,
            "php.remove".to_string(),
            payload.to_string(),
            Some(3),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal membuat antrean job penghapusan: {}", e) })),
            )
                .into_response();
        }
    };

    state.job_runner.notify.notify_one();

    state.audit_writer.log_async(
        Some(admin.id),
        &admin.username,
        "php.remove",
        format!("php:{}", version),
        "success",
        None,
        Some(format!("Memulai penghapusan PHP {}", version)),
    );

    (StatusCode::ACCEPTED, Json(JobItem::from(job))).into_response()
}

/// Detailed information about an individual PHP extension.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpExtensionItem {
    pub name: String,
    pub version: String,
    pub package_name: String,
    pub installed: bool,
    pub enabled: bool,
    pub is_base: bool,
    pub category: String,
    pub description: String,
}

impl From<panel_proto::PhpExtensionDto> for PhpExtensionItem {
    fn from(dto: panel_proto::PhpExtensionDto) -> Self {
        Self {
            name: dto.name,
            version: dto.version,
            package_name: dto.package_name,
            installed: dto.installed,
            enabled: dto.enabled,
            is_base: dto.is_base,
            category: dto.category,
            description: dto.description,
        }
    }
}

/// Response containing all curated extensions for a specific PHP version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpExtensionListApiResponse {
    pub version: String,
    pub extensions: Vec<PhpExtensionItem>,
}

impl From<panel_proto::PhpExtensionListResponse> for PhpExtensionListApiResponse {
    fn from(res: panel_proto::PhpExtensionListResponse) -> Self {
        Self {
            version: res.version,
            extensions: res.extensions.into_iter().map(Into::into).collect(),
        }
    }
}

/// Payload for toggling an extension's enabled/disabled state.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToggleExtensionRequest {
    pub enabled: bool,
}

/// Response for extension actions (toggle, etc).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpExtensionActionApiResponse {
    pub version: String,
    pub extension: String,
    pub success: bool,
    pub message: String,
}

/// Response containing per-site PHP extensions configuration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SitePhpExtensionsResponse {
    pub site_id: i64,
    pub php_version: String,
    pub extensions: Vec<String>,
    pub available_extensions: Vec<String>,
}

/// Input payload to update per-site PHP extension overrides.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSitePhpExtensionsInput {
    pub extensions: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/php/versions/{version}/extensions",
    params(
        ("version" = String, Path, description = "PHP version (7.4, 8.0, 8.1, 8.2, 8.3, 8.4)")
    ),
    responses(
        (status = 200, description = "List extensions for PHP version", body = PhpExtensionListApiResponse),
        (status = 400, description = "Invalid PHP version"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Failed to communicate with agent")
    ),
    tag = "runtime"
)]
pub async fn list_php_extensions_handler(
    State(state): State<AppState>,
    RequireAuth(_user): RequireAuth,
    Path(version): Path<String>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }

    match state.agent_client.list_php_extensions(&version).await {
        Ok(res) => (StatusCode::OK, Json(PhpExtensionListApiResponse::from(res))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal memuat ekstensi PHP: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/php/versions/{version}/extensions/{extension}/install",
    params(
        ("version" = String, Path, description = "PHP version"),
        ("extension" = String, Path, description = "Curated PHP extension name")
    ),
    responses(
        (status = 202, description = "Extension install job enqueued", body = JobItem),
        (status = 400, description = "Invalid version or extension name"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin privileges and CSRF required)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn install_php_extension_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(admin): RequireAdminCsrf,
    Path((version, extension)): Path<(String, String)>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }
    let ext_def = match find_curated_extension(&extension) {
        Ok(def) => def,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let payload = serde_json::json!({
        "version": version,
        "extension": ext_def.name,
    });
    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(admin.id),
            None,
            "php.extension.install".to_string(),
            payload.to_string(),
            Some(3),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal membuat antrean job instalasi: {}", e) })),
            )
                .into_response();
        }
    };

    state.job_runner.notify.notify_one();

    state.audit_writer.log_async(
        Some(admin.id),
        &admin.username,
        "php.extension.install",
        format!("php{}:{}", version, ext_def.name),
        "success",
        None,
        Some(format!(
            "Memulai instalasi ekstensi {} untuk PHP {}",
            ext_def.name, version
        )),
    );

    (StatusCode::ACCEPTED, Json(JobItem::from(job))).into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/php/versions/{version}/extensions/{extension}/toggle",
    params(
        ("version" = String, Path, description = "PHP version"),
        ("extension" = String, Path, description = "Curated PHP extension name")
    ),
    request_body = ToggleExtensionRequest,
    responses(
        (status = 200, description = "Extension toggled successfully", body = PhpExtensionActionApiResponse),
        (status = 400, description = "Invalid version or extension name"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin privileges and CSRF required)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn toggle_php_extension_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(admin): RequireAdminCsrf,
    Path((version, extension)): Path<(String, String)>,
    Json(body): Json<ToggleExtensionRequest>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }
    let ext_def = match find_curated_extension(&extension) {
        Ok(def) => def,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    match state
        .agent_client
        .toggle_php_extension(&version, ext_def.name, body.enabled)
        .await
    {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(admin.id),
                &admin.username,
                "php.extension.toggle",
                format!("php{}:{}", version, ext_def.name),
                "success",
                None,
                Some(format!(
                    "Mengubah status ekstensi {} PHP {} menjadi {}",
                    ext_def.name,
                    version,
                    if body.enabled { "aktif" } else { "nonaktif" }
                )),
            );

            (
                StatusCode::OK,
                Json(PhpExtensionActionApiResponse {
                    version: res.version,
                    extension: res.extension,
                    success: res.success,
                    message: res.message,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal mengubah status ekstensi: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/php/versions/{version}/extensions/{extension}/remove",
    params(
        ("version" = String, Path, description = "PHP version"),
        ("extension" = String, Path, description = "Curated PHP extension name")
    ),
    responses(
        (status = 202, description = "Extension remove job enqueued", body = JobItem),
        (status = 400, description = "Invalid version or base extension cannot be removed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin privileges and CSRF required)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn remove_php_extension_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(admin): RequireAdminCsrf,
    Path((version, extension)): Path<(String, String)>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }
    let ext_def = match find_curated_extension(&extension) {
        Ok(def) => def,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if ext_def.is_base {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Ekstensi dasar '{}' tidak dapat dihapus karena merupakan modul inti PHP", ext_def.name)
            })),
        )
            .into_response();
    }

    let payload = serde_json::json!({
        "version": version,
        "extension": ext_def.name,
    });
    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(admin.id),
            None,
            "php.extension.remove".to_string(),
            payload.to_string(),
            Some(3),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal membuat antrean job penghapusan: {}", e) })),
            )
                .into_response();
        }
    };

    state.job_runner.notify.notify_one();

    state.audit_writer.log_async(
        Some(admin.id),
        &admin.username,
        "php.extension.remove",
        format!("php{}:{}", version, ext_def.name),
        "success",
        None,
        Some(format!(
            "Memulai penghapusan ekstensi {} untuk PHP {}",
            ext_def.name, version
        )),
    );

    (StatusCode::ACCEPTED, Json(JobItem::from(job))).into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/php/extensions",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "Per-site PHP extensions preferences", body = SitePhpExtensionsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Site not found")
    ),
    tag = "runtime"
)]
pub async fn get_site_php_extensions_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
    Path(id): Path<i64>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let extensions = serde_json::from_str::<serde_json::Value>(&site.settings_json)
        .ok()
        .and_then(|val| {
            val.get("php")
                .and_then(|p| p.get("extensions"))
                .and_then(|arr| arr.as_array().cloned())
        })
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();

    let available_extensions = CURATED_PHP_EXTENSIONS
        .iter()
        .map(|e| e.name.to_string())
        .collect();

    (
        StatusCode::OK,
        Json(SitePhpExtensionsResponse {
            site_id: site.id,
            php_version: site.version.unwrap_or_else(|| "8.3".to_string()),
            extensions,
            available_extensions,
        }),
    )
        .into_response()
}

#[utoipa::path(
    put,
    path = "/api/v1/sites/{id}/php/extensions",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    request_body = UpdateSitePhpExtensionsInput,
    responses(
        (status = 200, description = "Per-site PHP extensions updated", body = SitePhpExtensionsResponse),
        (status = 400, description = "Invalid extension in list"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Site not found")
    ),
    tag = "runtime"
)]
pub async fn update_site_php_extensions_handler(
    State(state): State<AppState>,
    RequireCsrf(auth_user): RequireCsrf,
    Path(id): Path<i64>,
    Json(input): Json<UpdateSitePhpExtensionsInput>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    // Validate that every requested extension is in CURATED_PHP_EXTENSIONS
    let mut normalized_extensions = Vec::with_capacity(input.extensions.len());
    for ext_name in &input.extensions {
        match find_curated_extension(ext_name) {
            Ok(def) => {
                if !normalized_extensions.contains(&def.name.to_string()) {
                    normalized_extensions.push(def.name.to_string());
                }
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
    }

    // Merge into settings_json
    let mut root: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or_else(|_| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    if let Some(map) = root.as_object_mut() {
        let php_obj = map.entry("php").or_insert_with(|| serde_json::json!({}));
        if !php_obj.is_object() {
            *php_obj = serde_json::json!({});
        }
        if let Some(php_map) = php_obj.as_object_mut() {
            php_map.insert(
                "extensions".to_string(),
                serde_json::json!(normalized_extensions),
            );
        }
    }

    let updated_json = root.to_string();
    if let Err(e) = site_repo
        .update(
            site.id,
            &UpdateSite {
                settings_json: Some(updated_json),
                ..Default::default()
            },
        )
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({ "error": format!("Gagal menyimpan pengaturan situs: {}", e) }),
            ),
        )
            .into_response();
    }

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.php.extensions_update",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!(
            "Memperbarui ekstensi PHP situs: {:?}",
            normalized_extensions
        )),
    );

    let available_extensions = CURATED_PHP_EXTENSIONS
        .iter()
        .map(|e| e.name.to_string())
        .collect();

    (
        StatusCode::OK,
        Json(SitePhpExtensionsResponse {
            site_id: site.id,
            php_version: site.version.unwrap_or_else(|| "8.3".to_string()),
            extensions: normalized_extensions,
            available_extensions,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/php/settings",
    params(
        ("id" = i64, Path, description = "ID Situs")
    ),
    responses(
        (status = 200, description = "Pengaturan PHP situs dan batas paket", body = SitePhpSettingsResponse),
        (status = 400, description = "Situs bukan berjenis PHP"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Situs tidak ditemukan")
    ),
    tag = "runtime"
)]
pub async fn get_site_php_settings_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
    Path(id): Path<i64>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());

    let site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    if site.kind != "php" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Situs ini bukan berjenis PHP" })),
        )
            .into_response();
    }

    let owner = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let limits = if let Some(plan_id) = owner.plan_id {
        match plan_repo.find_by_id(plan_id).await {
            Ok(Some(plan)) => panel_core::php::PhpSettingsLimits::from_plan(
                plan.memory_limit_bytes,
                plan.max_upload_bytes,
            ),
            _ => panel_core::php::PhpSettingsLimits::default(),
        }
    } else {
        panel_core::php::PhpSettingsLimits::default()
    };

    let root: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or_else(|_| serde_json::json!({}));
    let php_obj = root.get("php");

    let settings = php_obj
        .and_then(|p| p.get("settings"))
        .and_then(|s| serde_json::from_value::<panel_core::php::PhpSettings>(s.clone()).ok())
        .unwrap_or_else(|| panel_core::php::PhpSettings::for_limits(&limits));

    let is_admin = auth_user.role == "admin";
    let admin_ini = if is_admin {
        php_obj
            .and_then(|p| p.get("admin_ini"))
            .and_then(|a| a.as_str().map(ToString::to_string))
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(SitePhpSettingsResponse {
            site_id: site.id,
            php_version: site.version.unwrap_or_else(|| "8.3".to_string()),
            settings,
            limits,
            admin_ini,
            is_admin,
        }),
    )
        .into_response()
}

#[utoipa::path(
    put,
    path = "/api/v1/sites/{id}/php/settings",
    params(
        ("id" = i64, Path, description = "ID Situs")
    ),
    request_body = UpdateSitePhpSettingsInput,
    responses(
        (status = 200, description = "Pengaturan PHP situs berhasil diperbarui", body = SitePhpSettingsResponse),
        (status = 400, description = "Validasi direktif PHP atau batas paket gagal"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Bukan admin saat mengirim admin_ini)"),
        (status = 404, description = "Situs tidak ditemukan")
    ),
    tag = "runtime"
)]
pub async fn update_site_php_settings_handler(
    State(state): State<AppState>,
    RequireCsrf(auth_user): RequireCsrf,
    Path(id): Path<i64>,
    Json(input): Json<UpdateSitePhpSettingsInput>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());

    let site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    if site.kind != "php" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Situs ini bukan berjenis PHP" })),
        )
            .into_response();
    }

    let owner = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let limits = if let Some(plan_id) = owner.plan_id {
        match plan_repo.find_by_id(plan_id).await {
            Ok(Some(plan)) => panel_core::php::PhpSettingsLimits::from_plan(
                plan.memory_limit_bytes,
                plan.max_upload_bytes,
            ),
            _ => panel_core::php::PhpSettingsLimits::default(),
        }
    } else {
        panel_core::php::PhpSettingsLimits::default()
    };

    // 1. Validate curated settings against tenant plan limits
    if let Err(e) = input.settings.validate(&limits) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let is_admin = auth_user.role == "admin";

    // 2. Validate raw admin_ini if supplied
    let mut updated_admin_ini: Option<String> = None;
    if let Some(ref ini_raw) = input.admin_ini {
        if !ini_raw.trim().is_empty() {
            if !is_admin {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Hanya administrator yang memiliki izin untuk menyunting direktif php.ini mentah"
                    })),
                )
                    .into_response();
            }
            if let Err(e) = panel_core::php::parse_admin_ini(ini_raw) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            updated_admin_ini = Some(ini_raw.clone());
        } else if is_admin {
            updated_admin_ini = None;
        }
    }

    // 3. Update settings_json
    let mut root: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or_else(|_| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    if let Some(map) = root.as_object_mut() {
        let php_obj = map.entry("php").or_insert_with(|| serde_json::json!({}));
        if !php_obj.is_object() {
            *php_obj = serde_json::json!({});
        }
        if let Some(php_map) = php_obj.as_object_mut() {
            php_map.insert(
                "settings".to_string(),
                serde_json::to_value(&input.settings).unwrap_or_default(),
            );
            if is_admin {
                match updated_admin_ini.as_deref() {
                    Some(ini_str) => {
                        php_map.insert("admin_ini".to_string(), serde_json::json!(ini_str));
                    }
                    None => {
                        php_map.remove("admin_ini");
                    }
                }
            } else if let Some(existing_ini) = php_map.get("admin_ini").and_then(|v| v.as_str()) {
                updated_admin_ini = Some(existing_ini.to_string());
            }
        }
    }

    let updated_json = root.to_string();
    if let Err(e) = site_repo
        .update(
            site.id,
            &UpdateSite {
                settings_json: Some(updated_json),
                ..Default::default()
            },
        )
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal menyimpan pengaturan situs: {e}") })),
        )
            .into_response();
    }

    // 4. Re-apply PHP-FPM pool via Agent IPC
    let (resolved_linux_name, resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&owner) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let linux_uname = resolved_linux_name.as_str();
    let owner_uid = resolved_linux_uid;
    let php_version = site.version.as_deref().unwrap_or("8.3");

    let pool_req = panel_proto::PhpFpmPoolApplyRequest {
        username: linux_uname.to_string(),
        uid: owner_uid,
        version: php_version.to_string(),
        pool_name: format!("site_{}", site.id),
        docroot: site.docroot.clone(),
        socket_path: None,
        max_children: None,
        process_idle_timeout_secs: None,
        max_requests: None,
        open_basedir: None,
        disable_functions: None,
        settings: Some(input.settings.clone()),
        admin_ini: updated_admin_ini.clone(),
        reload: true,
    };

    if let Err(e) = state.agent_client.apply_php_fpm_pool(pool_req).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Pengaturan tersimpan tetapi gagal menerapkan konfigurasi pool: {e}")
            })),
        )
            .into_response();
    }

    // 5. Audit Log
    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.php.settings_update",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!(
            "Pengaturan PHP situs {} diperbarui (memory_limit: {}M, max_execution_time: {}s)",
            site.id, input.settings.memory_limit_mb, input.settings.max_execution_time
        )),
    );

    (
        StatusCode::OK,
        Json(SitePhpSettingsResponse {
            site_id: site.id,
            php_version: php_version.to_string(),
            settings: input.settings,
            limits,
            admin_ini: if is_admin { updated_admin_ini } else { None },
            is_admin,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/php/masters",
    responses(
        (status = 200, description = "Daftar master PHP-FPM tenant", body = Vec<PhpFpmMasterOverviewDto>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn list_php_fpm_masters_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
) -> Response {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let users = if auth_user.is_admin() {
        user_repo.list_all().await.unwrap_or_default()
    } else {
        match user_repo.find_by_id(auth_user.id).await {
            Ok(Some(u)) => vec![u],
            _ => vec![],
        }
    };

    let supported_versions = PhpVersion::all_supported();
    let mut masters = Vec::new();

    for user in users {
        let linux_user = match user.linux_username.as_deref() {
            Some(u) if !u.is_empty() => u,
            _ => continue,
        };

        for ver in supported_versions {
            let ver_str = ver.as_str();
            let status = match state
                .agent_client
                .get_php_fpm_master_status(linux_user, ver_str)
                .await
            {
                Ok(s) => s,
                Err(_) => continue,
            };

            if status.is_active || status.pools_count > 0 {
                masters.push(PhpFpmMasterOverviewDto {
                    username: linux_user.to_string(),
                    version: ver_str.to_string(),
                    service_name: status.service_name,
                    is_active: status.is_active,
                    substate: status.substate,
                    socket_exists: status.socket_exists,
                    pools_count: status.pools_count,
                });
            }
        }
    }

    (StatusCode::OK, Json(masters)).into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/php/masters/{username}/{version}/status",
    params(
        ("username" = String, Path, description = "Username Linux tenant"),
        ("version" = String, Path, description = "Versi PHP (mis. 8.3)")
    ),
    responses(
        (status = 200, description = "Status master PHP-FPM", body = PhpFpmMasterStatusDto),
        (status = 400, description = "Versi PHP tidak valid"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Bukan pemilik akun)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn get_php_fpm_master_status_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
    Path((username, version)): Path<(String, String)>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }

    let context = match crate::tenant_context::resolve_tenant_context(
        &state,
        &auth_user,
        Some(&username),
        None,
    )
    .await
    {
        Ok(context) => context,
        Err(err) => return err.into_response(),
    };
    let username = context.linux_username;

    match state
        .agent_client
        .get_php_fpm_master_status(&username, &version)
        .await
    {
        Ok(res) => (
            StatusCode::OK,
            Json(PhpFpmMasterStatusDto {
                service_name: res.service_name,
                is_active: res.is_active,
                substate: res.substate,
                socket_exists: res.socket_exists,
                pools_count: res.pools_count,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal membaca status master FPM: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/php/masters/{username}/{version}/reload",
    params(
        ("username" = String, Path, description = "Username Linux tenant"),
        ("version" = String, Path, description = "Versi PHP (mis. 8.3)")
    ),
    responses(
        (status = 200, description = "PHP-FPM master berhasil di-reload", body = PhpFpmActionResponseDto),
        (status = 400, description = "Versi PHP tidak valid"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Bukan pemilik atau CSRF tidak valid)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn reload_php_fpm_master_handler(
    State(state): State<AppState>,
    RequireCsrf(auth_user): RequireCsrf,
    Path((username, version)): Path<(String, String)>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }

    let context = match crate::tenant_context::resolve_tenant_context(
        &state,
        &auth_user,
        Some(&username),
        None,
    )
    .await
    {
        Ok(context) => context,
        Err(err) => return err.into_response(),
    };
    let username = context.linux_username;

    match state
        .agent_client
        .reload_php_fpm_master(&username, &version)
        .await
    {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "php.fpm.reload",
                format!("{}-{}", username, version),
                "success",
                None,
                Some(format!("Reload master FPM {}-{}", username, version)),
            );

            (
                StatusCode::OK,
                Json(PhpFpmActionResponseDto {
                    service_name: res.service_name,
                    action: res.action,
                    success: res.success,
                    message: res.message,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal me-reload master FPM: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/php/masters/{username}/{version}/restart",
    params(
        ("username" = String, Path, description = "Username Linux tenant"),
        ("version" = String, Path, description = "Versi PHP (mis. 8.3)")
    ),
    responses(
        (status = 200, description = "PHP-FPM master berhasil di-restart", body = PhpFpmActionResponseDto),
        (status = 400, description = "Versi PHP tidak valid"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Bukan pemilik atau CSRF tidak valid)"),
        (status = 500, description = "Internal error")
    ),
    tag = "runtime"
)]
pub async fn restart_php_fpm_master_handler(
    State(state): State<AppState>,
    RequireCsrf(auth_user): RequireCsrf,
    Path((username, version)): Path<(String, String)>,
) -> Response {
    if let Err(e) = PhpVersion::parse(&version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {}", e) })),
        )
            .into_response();
    }

    let context = match crate::tenant_context::resolve_tenant_context(
        &state,
        &auth_user,
        Some(&username),
        None,
    )
    .await
    {
        Ok(context) => context,
        Err(err) => return err.into_response(),
    };
    let username = context.linux_username;

    match state
        .agent_client
        .restart_php_fpm_master(&username, &version)
        .await
    {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(auth_user.id),
                &auth_user.username,
                "php.fpm.restart",
                format!("{}-{}", username, version),
                "success",
                None,
                Some(format!("Restart master FPM {}-{}", username, version)),
            );

            (
                StatusCode::OK,
                Json(PhpFpmActionResponseDto {
                    service_name: res.service_name,
                    action: res.action,
                    success: res.success,
                    message: res.message,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal me-restart master FPM: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/sites/{id}/php/version",
    params(
        ("id" = i64, Path, description = "ID Situs")
    ),
    request_body = SwitchPhpVersionRequest,
    responses(
        (status = 200, description = "Versi PHP berhasil diganti tanpa downtime", body = SwitchPhpVersionResponse),
        (status = 400, description = "Permintaan tidak valid, versi tidak terpasang, atau versi sama"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Situs tidak ditemukan"),
        (status = 500, description = "Gagal migrasi dan otomatis di-rollback")
    ),
    tag = "runtime"
)]
#[allow(clippy::cognitive_complexity)]
pub async fn switch_site_php_version_handler(
    State(state): State<AppState>,
    RequireCsrf(auth_user): RequireCsrf,
    Path(id): Path<i64>,
    Json(payload): Json<SwitchPhpVersionRequest>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());

    // 1. Fetch site and authorize
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    // 2. Must be a PHP site
    if site.kind != "php" && site.runtime.as_deref() != Some("php") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Situs ini bukan situs PHP" })),
        )
            .into_response();
    }

    // 3. Parse and validate target version
    let target_ver = match PhpVersion::parse(&payload.version) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Versi PHP tidak valid: {e}") })),
            )
                .into_response();
        }
    };
    let target_ver_str = target_ver.as_str();
    let old_ver_str = site.version.as_deref().unwrap_or("8.3");

    if old_ver_str == target_ver_str {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Situs sudah menggunakan versi PHP {old_ver_str}")
            })),
        )
            .into_response();
    }

    // 4. Verify target version is installed on host
    let runtimes = match state.agent_client.list_php_runtimes().await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Gagal memeriksa versi PHP yang terpasang di host: {e}")
                })),
            )
                .into_response();
        }
    };

    let is_installed = runtimes
        .versions
        .iter()
        .any(|v| v.version == target_ver_str && v.installed);

    if !is_installed {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Versi PHP {target_ver_str} belum terpasang di server host. Silakan pasang terlebih dahulu via menu Runtime.")
            })),
        )
            .into_response();
    }

    // 5. Resolve site owner
    let owner = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };
    let (resolved_linux_name, resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&owner) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let linux_uname = resolved_linux_name.as_str();
    let owner_uid = resolved_linux_uid;
    let site_id_str = format!("site_{id}");

    let domains = match domain_repo.find_by_site_id(id).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal membaca domain situs: {e}") })),
            )
                .into_response();
        }
    };

    // 6. Zero-downtime Cutover with Automatic Rollback Flow:
    // Step A: Ensure target PHP-FPM master exists and is ready
    if let Err(e) = state
        .agent_client
        .ensure_php_fpm_master(linux_uname, owner_uid, target_ver_str)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal memastikan master PHP-FPM {target_ver_str} aktif: {e}")
            })),
        )
            .into_response();
    }

    // Step B: Apply pool in target master
    let (site_settings, site_admin_ini) = {
        let root: serde_json::Value =
            serde_json::from_str(&site.settings_json).unwrap_or_else(|_| serde_json::json!({}));
        let php_obj = root.get("php");
        let s = php_obj.and_then(|p| p.get("settings")).and_then(|val| {
            serde_json::from_value::<panel_core::php::PhpSettings>(val.clone()).ok()
        });
        let a = php_obj
            .and_then(|p| p.get("admin_ini"))
            .and_then(|val| val.as_str().map(ToString::to_string));
        (s, a)
    };

    let target_pool_req = panel_proto::PhpFpmPoolApplyRequest {
        username: linux_uname.to_string(),
        uid: owner_uid,
        version: target_ver_str.to_string(),
        pool_name: site_id_str.clone(),
        docroot: site.docroot.clone(),
        socket_path: None,
        max_children: Some(5),
        process_idle_timeout_secs: Some(10),
        max_requests: Some(500),
        open_basedir: None,
        disable_functions: None,
        settings: site_settings,
        admin_ini: site_admin_ini,
        reload: true,
    };

    if let Err(e) = state.agent_client.apply_php_fpm_pool(target_pool_req).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal menerapkan konfigurasi pool pada PHP {target_ver_str}: {e}")
            })),
        )
            .into_response();
    }

    // Step C: Atomically update Nginx virtual host to point to new target version UDS socket
    let mut new_site = site.clone();
    new_site.version = Some(target_ver_str.to_string());

    if let Err((code, msg)) =
        recompile_and_apply_site_vhost(&state, &new_site, &owner, &domains).await
    {
        // Rollback target pool if Nginx update fails
        let _ = state
            .agent_client
            .remove_php_fpm_pool(linux_uname, target_ver_str, site_id_str.clone(), true)
            .await;
        return (
            code,
            Json(serde_json::json!({
                "error": format!("Gagal menerapkan vhost Nginx untuk PHP {target_ver_str}: {msg}. Pool tujuan telah dibersihkan.")
            })),
        )
            .into_response();
    }

    // Step D: Post-cutover Health Check on target master & socket
    let health_check = state
        .agent_client
        .check_php_fpm_health(linux_uname, target_ver_str, &site_id_str)
        .await;

    let is_healthy = match health_check {
        Ok(hc) => hc.healthy,
        Err(_) => false,
    };

    if !is_healthy {
        tracing::warn!(
            "Health check failed after switching site {} to PHP {}. Initiating automatic rollback to PHP {}",
            site.id,
            target_ver_str,
            old_ver_str
        );

        // --- AUTOMATIC ROLLBACK ---
        // 1. Revert Nginx vhost back to old PHP version
        let mut rollback_site = site.clone();
        rollback_site.version = Some(old_ver_str.to_string());
        let _ = recompile_and_apply_site_vhost(&state, &rollback_site, &owner, &domains).await;

        // 2. Remove pool from target master
        let _ = state
            .agent_client
            .remove_php_fpm_pool(linux_uname, target_ver_str, site_id_str.clone(), true)
            .await;

        // 3. Log rollback audit event
        state.audit_writer.log_async(
            Some(auth_user.id),
            &auth_user.username,
            "site.php_switch_rollback",
            format!("site:{}", site.id),
            "failure",
            None,
            Some(format!(
                "Health check PHP {target_ver_str} gagal. Konfigurasi otomatis di-rollback ke PHP {old_ver_str}"
            )),
        );

        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!(
                    "Health check pada master PHP {target_ver_str} gagal. Sistem otomatis melakukan rollback ke versi {old_ver_str}."
                ),
                "rolled_back_to": old_ver_str
            })),
        )
            .into_response();
    }

    // Step E: Health check passed! Clean up old pool from old master
    let _ = state
        .agent_client
        .remove_php_fpm_pool(linux_uname, old_ver_str, site_id_str, true)
        .await;

    // Step F: Persist new version to DB
    if let Err(e) = site_repo
        .update(
            site.id,
            &UpdateSite {
                version: Some(target_ver_str.to_string()),
                ..Default::default()
            },
        )
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Versi PHP berhasil dialihkan, tetapi gagal menyimpan ke database: {e}")
            })),
        )
            .into_response();
    }

    // Step G: Audit Log
    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.php_version_changed",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!(
            "Berhasil mengubah versi PHP dari {old_ver_str} ke {target_ver_str} tanpa downtime"
        )),
    );

    (
        StatusCode::OK,
        Json(SwitchPhpVersionResponse {
            site_id: site.id,
            previous_version: old_ver_str.to_string(),
            current_version: target_ver_str.to_string(),
            status: "success".to_string(),
            message: format!("Versi PHP situs berhasil diubah ke {target_ver_str} tanpa downtime"),
        }),
    )
        .into_response()
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct PhpFpmStatusMetricsDto {
    pub pool: String,
    pub process_manager: String,
    pub start_time: u64,
    pub start_since: u64,
    pub accepted_conn: u64,
    pub listen_queue: u32,
    pub max_listen_queue: u32,
    pub listen_queue_len: u32,
    pub idle_processes: u32,
    pub active_processes: u32,
    pub total_processes: u32,
    pub max_active_processes: u32,
    pub max_children_reached: u32,
    pub slow_requests: u64,
}

impl From<panel_proto::PhpFpmStatusMetrics> for PhpFpmStatusMetricsDto {
    fn from(m: panel_proto::PhpFpmStatusMetrics) -> Self {
        Self {
            pool: m.pool,
            process_manager: m.process_manager,
            start_time: m.start_time,
            start_since: m.start_since,
            accepted_conn: m.accepted_conn,
            listen_queue: m.listen_queue,
            max_listen_queue: m.max_listen_queue,
            listen_queue_len: m.listen_queue_len,
            idle_processes: m.idle_processes,
            active_processes: m.active_processes,
            total_processes: m.total_processes,
            max_active_processes: m.max_active_processes,
            max_children_reached: m.max_children_reached,
            slow_requests: m.slow_requests,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SitePhpStatusResponse {
    pub site_id: i64,
    pub primary_domain: String,
    pub php_version: String,
    pub service_name: String,
    pub is_active: bool,
    pub socket_exists: bool,
    pub metrics: Option<PhpFpmStatusMetricsDto>,
}

#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/php/status",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "Status runtime PHP-FPM situs", body = SitePhpStatusResponse),
        (status = 400, description = "Bukan situs PHP"),
        (status = 401, description = "Tidak terautentikasi"),
        (status = 403, description = "Akses ditolak"),
        (status = 404, description = "Situs tidak ditemukan")
    ),
    tag = "php"
)]
pub async fn get_site_php_status_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
    Path(id): Path<i64>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {e}") })),
            )
                .into_response();
        }
    };

    if auth_user.role != "admin" && site.user_id != auth_user.id {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Anda tidak memiliki akses ke situs ini" })),
        )
            .into_response();
    }

    if site.kind != "php" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Situs ini bukan situs bertipe PHP" })),
        )
            .into_response();
    }

    let owner = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let (resolved_linux_name, _resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&owner) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let linux_uname = resolved_linux_name.as_str();
    let version = site.version.as_deref().unwrap_or("8.3");
    let pool_name = format!("site_{id}");

    let domains = domain_repo.find_by_site_id(id).await.unwrap_or_default();
    let primary_domain = domains
        .into_iter()
        .find(|d| d.is_primary)
        .map(|d| d.name)
        .unwrap_or_else(|| site.docroot.clone());

    match state
        .agent_client
        .get_php_fpm_pool_status(linux_uname, version, &pool_name)
        .await
    {
        Ok(res) => (
            StatusCode::OK,
            Json(SitePhpStatusResponse {
                site_id: site.id,
                primary_domain,
                php_version: version.to_string(),
                service_name: res.service_name,
                is_active: res.is_active,
                socket_exists: res.socket_exists,
                metrics: res.metrics.map(Into::into),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal membaca status FPM: {e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct SitePhpLogsQuery {
    pub r#type: Option<String>,
    pub lines: Option<usize>,
    pub q: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SitePhpLogsResponse {
    pub site_id: i64,
    pub primary_domain: String,
    pub php_version: String,
    pub log_type: String,
    pub lines: Vec<String>,
    pub total_matched: usize,
}

#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/php/logs",
    params(
        ("id" = i64, Path, description = "Site ID"),
        SitePhpLogsQuery
    ),
    responses(
        (status = 200, description = "Baris log PHP situs", body = SitePhpLogsResponse),
        (status = 400, description = "Bukan situs PHP atau tipe log tidak valid"),
        (status = 401, description = "Tidak terautentikasi"),
        (status = 403, description = "Akses ditolak"),
        (status = 404, description = "Situs tidak ditemukan")
    ),
    tag = "php"
)]
pub async fn get_site_php_logs_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
    Path(id): Path<i64>,
    Query(query): Query<SitePhpLogsQuery>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());

    let site = match site_repo.find_by_id(id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Situs tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {e}") })),
            )
                .into_response();
        }
    };

    if auth_user.role != "admin" && site.user_id != auth_user.id {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Anda tidak memiliki akses ke situs ini" })),
        )
            .into_response();
    }

    if site.kind != "php" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Situs ini bukan situs bertipe PHP" })),
        )
            .into_response();
    }

    let owner = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let raw_type = query
        .r#type
        .unwrap_or_else(|| "error".to_string())
        .to_lowercase();
    let log_type = match raw_type.as_str() {
        "error" | "php_error" | "php-error" => "error",
        "slow" | "php_slow" | "php-slow" => "slow",
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Tipe log tidak valid. Gunakan 'error' atau 'slow'" })),
            )
                .into_response();
        }
    };

    let (resolved_linux_name, _resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&owner) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let linux_uname = resolved_linux_name.as_str();
    let version = site.version.as_deref().unwrap_or("8.3");
    let pool_name = format!("site_{id}");

    let domains = domain_repo.find_by_site_id(id).await.unwrap_or_default();
    let primary_domain = domains
        .into_iter()
        .find(|d| d.is_primary)
        .map(|d| d.name)
        .unwrap_or_else(|| site.docroot.clone());

    let req = panel_proto::PhpLogsReadRequest {
        username: linux_uname.to_string(),
        version: version.to_string(),
        pool_name: Some(pool_name),
        log_type: log_type.to_string(),
        lines: query.lines.unwrap_or(100).clamp(1, 1000),
        query: query.q,
        log_base_dir: None,
    };

    match state.agent_client.read_php_logs(req).await {
        Ok(res) => (
            StatusCode::OK,
            Json(SitePhpLogsResponse {
                site_id: site.id,
                primary_domain,
                php_version: version.to_string(),
                log_type: log_type.to_string(),
                lines: res.lines,
                total_matched: res.total_matched,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal membaca log PHP: {e}") })),
        )
            .into_response(),
    }
}
