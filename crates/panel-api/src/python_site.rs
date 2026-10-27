//! Python site API handlers: runtime version management, process lifecycle actions
//! (start, stop, restart, redeploy), socket health querying, tenant job execution,
//! and settings management.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_proto::{
    PythonRuntimeInstallRequest, PythonRuntimeListRequest, PythonRuntimeRemoveRequest,
    PythonSiteActionRequest, PythonSiteActionType, PythonSiteJobRequest, PythonSiteJobType,
    PythonSiteStatusRequest,
};
use panel_runtime::python::{
    DEFAULT_PYTHON_SOCKET_DIR, DEFAULT_PYTHON_VERSION, calculate_default_python_workers,
    get_curated_python_catalog, normalize_python_version,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use utoipa::ToSchema;

use crate::auth::{RequireAdminCsrf, middleware::AuthenticatedUser};
use crate::routes::AppState;

/// Request payload to install a Python runtime version.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct InstallPythonVersionRequest {
    pub version: String,
}

/// Request payload to trigger a Python process action.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct PythonSiteActionApiRequest {
    pub action: String, // "start", "stop", "restart", "redeploy"
}

/// Request payload to run a tenant job (uv venv, uv pip install, custom script).
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct PythonSiteJobApiRequest {
    pub job_type: String, // "uv_venv", "uv_pip_install", "uv_pip_pyproject", "custom"
    pub req_file: Option<String>,
    pub script_name: Option<String>,
    pub timeout_secs: Option<u64>,
}

/// Settings for a Python website process.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct PythonSiteSettingsDto {
    pub site_id: i64,
    pub python_version: String,
    pub framework_preset: String, // "django", "flask", "fastapi", "generic"
    pub server_type: String,      // "gunicorn_wsgi", "uvicorn_asgi", "gunicorn_uvicorn_worker"
    pub entrypoint_type: String,  // "wsgi", "asgi", "custom"
    pub entrypoint_value: String, // e.g. "wsgi:application", "app:app", "main:app"
    pub workers: u32,
    pub app_dir: String,
    pub venv_dir: String,
    pub socket_path: String,
    pub static_url: Option<String>,
    pub static_dir: Option<String>,
    pub media_url: Option<String>,
    pub media_dir: Option<String>,
    pub auto_restart: bool,
    pub env_vars: BTreeMap<String, String>,
}

/// Request payload to update Python site settings.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct UpdatePythonSiteSettingsRequest {
    pub python_version: Option<String>,
    pub framework_preset: Option<String>,
    pub server_type: Option<String>,
    pub entrypoint_type: Option<String>,
    pub entrypoint_value: Option<String>,
    pub workers: Option<u32>,
    pub app_dir: Option<String>,
    pub venv_dir: Option<String>,
    pub static_url: Option<String>,
    pub static_dir: Option<String>,
    pub media_url: Option<String>,
    pub media_dir: Option<String>,
    pub auto_restart: Option<bool>,
    pub env_vars: Option<BTreeMap<String, String>>,
}

/// Python site runtime status response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PythonSiteStatusApiResponse {
    pub site_id: String,
    pub service_name: String,
    pub active: bool,
    pub sub_state: String,
    pub pid: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub cpu_usage_usec: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub socket_path: String,
    pub socket_exists: bool,
    pub healthy: bool,
    pub probe_message: Option<String>,
}

/// Python runtime version presentation DTO.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PythonVersionSummaryDto {
    pub version: String,
    pub major_minor: String,
    pub release_date: Option<String>,
    pub is_default: bool,
    pub is_lts_or_stable: bool,
    pub description: String,
    pub is_installed: bool,
    pub install_path: Option<String>,
    pub bin_path: Option<String>,
}

/// Verifies access to site and returns Site & User models.
async fn verify_site_access(
    pool: &SqlitePool,
    auth_user: &AuthenticatedUser,
    site_id: i64,
) -> Result<(panel_db::models::Site, panel_db::models::User), (StatusCode, String)> {
    let site_repo = panel_db::repositories::site_repo::SqliteSiteRepository::new(pool.clone());
    let user_repo = panel_db::repositories::user_repo::SqliteUserRepository::new(pool.clone());

    let site = site_repo
        .find_by_id(site_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Situs tidak ditemukan".to_string()))?;

    if auth_user.role != "admin" && site.user_id != auth_user.id {
        return Err((
            StatusCode::FORBIDDEN,
            "Anda tidak memiliki izin untuk mengakses situs ini".to_string(),
        ));
    }

    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Pengguna situs tidak ditemukan".to_string(),
            )
        })?;

    Ok((site, user))
}

/// GET /api/v1/runtimes/python/versions
/// Returns curated Python releases combined with installed status on the host system.
pub async fn list_python_versions_handler(
    State(state): State<AppState>,
    _auth_user: AuthenticatedUser,
) -> Response {
    let cache = match crate::runtime_catalog::python_cache(&state.pool).await {
        Ok(cache) => cache,
        Err(error) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":error})),
            )
                .into_response();
        }
    };
    let catalog_ready = cache.fresh(chrono::Utc::now().timestamp());
    let curated = if catalog_ready {
        cache.data
    } else {
        get_curated_python_catalog()
    };

    let installed_res = state
        .agent_client
        .list_python_runtimes(PythonRuntimeListRequest { base_dir: None })
        .await;

    let installed_map: BTreeMap<String, panel_proto::PythonInstalledVersionDto> =
        match installed_res {
            Ok(list) => list
                .installed
                .into_iter()
                .map(|item| (item.version.clone(), item))
                .collect(),
            Err(e) => {
                return (StatusCode::SERVICE_UNAVAILABLE,Json(serde_json::json!({"error":format!("Gagal memeriksa Python terpasang: {e}")}))).into_response();
            }
        };

    let mut versions: Vec<PythonVersionSummaryDto> = curated
        .into_iter()
        .map(|entry| {
            let installed = installed_map.get(&entry.version);
            PythonVersionSummaryDto {
                version: entry.version.clone(),
                major_minor: entry.major_minor,
                release_date: entry.release_date,
                is_default: entry.is_default,
                is_lts_or_stable: entry.is_lts_or_stable,
                description: entry.description,
                is_installed: installed.is_some(),
                install_path: installed.map(|i| i.install_path.clone()),
                bin_path: installed.map(|i| i.bin_path.clone()),
            }
        })
        .collect();

    // Add any installed versions not in curated catalog
    for (ver, inst) in installed_map {
        if !versions.iter().any(|v| v.version == ver) {
            versions.push(PythonVersionSummaryDto {
                version: ver.clone(),
                major_minor: ver.split('.').take(2).collect::<Vec<&str>>().join("."),
                release_date: None,
                is_default: ver == DEFAULT_PYTHON_VERSION,
                is_lts_or_stable: false,
                description: format!("Python {} (kustom terpasang)", ver),
                is_installed: true,
                install_path: Some(inst.install_path),
                bin_path: Some(inst.bin_path),
            });
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "versions": versions, "catalog_ready": catalog_ready })),
    )
        .into_response()
}

/// POST /api/v1/runtimes/python/install
/// Installs a specified Python version via `uv` on the host system.
pub async fn install_python_version_handler(
    State(state): State<AppState>,
    admin: RequireAdminCsrf,
    Json(payload): Json<InstallPythonVersionRequest>,
) -> Response {
    let norm_ver = normalize_python_version(&payload.version);
    if let Err(error) = panel_runtime::python::validate_python_version(&norm_ver) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":error.to_string()})),
        )
            .into_response();
    }
    let req = PythonRuntimeInstallRequest {
        version: norm_ver.clone(),
        base_dir: None,
    };

    let payload_json = match serde_json::to_string(&req) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":error.to_string()})),
            )
                .into_response();
        }
    };
    let input = panel_db::models::CreateJob {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: Some(admin.0.id),
        name: "python.install".into(),
        payload_json,
        max_retries: 1,
    };
    match state.job_runner.job_repo.create_or_find_active(&input).await {
        Ok((job,_)) => {
            state.job_runner.notify.notify_one();
            state.audit_writer.log_async(Some(admin.0.id),&admin.0.username,"python.runtime_install",&norm_ver,"success",None,Some(format!("Tugas pemasangan Python dibuat: {}",job.id)));
            (StatusCode::ACCEPTED,Json(crate::jobs::JobItem::from(job))).into_response()
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal menginstal Python {}: {}", norm_ver, e) })),
        )
            .into_response(),
    }
}

/// DELETE /api/v1/runtimes/python/{version}
/// Removes an installed Python runtime.
/// PRD rule: Cannot delete a runtime version currently in use by active sites (409 Conflict).
pub async fn remove_python_version_handler(
    State(state): State<AppState>,
    _admin: RequireAdminCsrf,
    Path(version): Path<String>,
) -> Response {
    let norm_ver = normalize_python_version(&version);
    if let Err(error) = panel_runtime::python::validate_python_version(&norm_ver) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":error.to_string()})),
        )
            .into_response();
    }

    // Verify whether any site in DB is using this version
    let rows = sqlx::query(
        "SELECT id, version, settings_json FROM sites WHERE kind = 'python' OR runtime = 'python'",
    )
    .fetch_all(&state.pool)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"Gagal memeriksa penggunaan runtime Python"})),
            )
                .into_response();
        }
    };
    {
        let in_use = rows.iter().any(|row| {
            let ver: Option<String> = sqlx::Row::try_get(row, "version").ok();
            if ver
                .map(|v| normalize_python_version(&v) == norm_ver)
                .unwrap_or(false)
            {
                return true;
            }
            sqlx::Row::try_get::<String, _>(row, "settings_json")
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|val| {
                    val.get("python_version").and_then(|p| {
                        p.as_str()
                            .map(|pv| normalize_python_version(pv) == norm_ver)
                    })
                })
                .unwrap_or(false)
        });

        if in_use {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!("Versi Python '{}' sedang digunakan oleh situs lain dan tidak dapat dihapus", norm_ver)
                })),
            )
                .into_response();
        }
    }

    let req = PythonRuntimeRemoveRequest {
        version: norm_ver.clone(),
        base_dir: None,
    };

    match state.agent_client.remove_python_runtime(req).await {
        Ok(res) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": res.message,
                "version": res.version,
                "path": res.path,
                "success": res.success
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal menghapus Python {}: {}", norm_ver, e) })),
        )
            .into_response(),
    }
}

/// GET /api/v1/sites/{id}/python/status
/// Queries systemd service status and Unix domain socket existence.
pub async fn get_python_site_status_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    let (site, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let settings: serde_json::Value = serde_json::from_str(&site.settings_json).unwrap_or_default();
    let socket_path = settings
        .get("socket_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}/site_{}/app.sock", DEFAULT_PYTHON_SOCKET_DIR, site.id));

    let probe_req = PythonSiteStatusRequest {
        site_id: format!("site_{}", site.id),
        socket_path: Some(socket_path.clone()),
    };

    match state.agent_client.status_python_site(probe_req).await {
        Ok(status) => (
            StatusCode::OK,
            Json(PythonSiteStatusApiResponse {
                site_id: status.site_id,
                service_name: status.service_name,
                active: status.active,
                sub_state: status.sub_state,
                pid: status.pid,
                memory_bytes: status.memory_bytes,
                cpu_usage_usec: status.cpu_usage_usec,
                uptime_secs: status.uptime_secs,
                socket_path,
                socket_exists: status.socket_exists,
                healthy: status.healthy,
                probe_message: status.probe_message,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal membaca status service dari agent: {}", e)
            })),
        )
            .into_response(),
    }
}

/// POST /api/v1/sites/{id}/python/action
/// Triggers a process action (start, stop, restart, redeploy).
pub async fn action_python_site_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<PythonSiteActionApiRequest>,
) -> Response {
    let (site, _user) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let action_type = match payload.action.trim().to_lowercase().as_str() {
        "start" => PythonSiteActionType::Start,
        "stop" => PythonSiteActionType::Stop,
        "restart" => PythonSiteActionType::Restart,
        "redeploy" => PythonSiteActionType::Redeploy,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Aksi '{}' tidak didukung. Pilihan: start, stop, restart, redeploy", other)
                })),
            )
                .into_response();
        }
    };

    // If redeploy requested, reapply site config first
    if action_type == PythonSiteActionType::Redeploy {
        match crate::sites::apply_site_config(&state, &site).await {
            Ok(()) => {}
            Err((code, msg)) => {
                return (code, Json(serde_json::json!({ "error": msg }))).into_response();
            }
        }
    }

    let act_req = PythonSiteActionRequest {
        site_id: format!("site_{}", site.id),
        action: action_type,
    };

    match state.agent_client.action_python_site(act_req).await {
        Ok(res) => {
            if action_type == PythonSiteActionType::Restart {
                let mut settings_map: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_str(&site.settings_json).unwrap_or_default();
                if settings_map.get("env_restart_required") == Some(&serde_json::json!(true)) {
                    settings_map
                        .insert("env_restart_required".to_string(), serde_json::json!(false));
                    let updated_json = serde_json::to_string(&settings_map).unwrap_or_default();
                    let repo = panel_db::repositories::site_repo::SqliteSiteRepository::new(
                        state.pool.clone(),
                    );
                    let _ = repo
                        .update(
                            site.id,
                            &panel_db::models::UpdateSite {
                                kind: None,
                                docroot: None,
                                runtime: site.runtime.clone(),
                                version: site.version.clone(),
                                status: None,
                                settings_json: Some(updated_json),
                            },
                        )
                        .await;
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": res.success,
                    "action": res.action,
                    "message": res.message
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal menjalankan aksi proses: {}", e) })),
        )
            .into_response(),
    }
}

/// POST /api/v1/sites/{id}/python/job
/// Runs a tenant job (uv venv, uv pip install, custom script) under tenant UID in tenant slice.
pub async fn run_python_site_job_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<PythonSiteJobApiRequest>,
) -> Response {
    let (site, user) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let (resolved_linux_name, resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&user) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let linux_uname = resolved_linux_name.as_str();
    let settings: serde_json::Value = serde_json::from_str(&site.settings_json).unwrap_or_default();
    let python_ver = site
        .version
        .clone()
        .or_else(|| {
            settings
                .get("python_version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| DEFAULT_PYTHON_VERSION.to_string());

    let app_dir = settings
        .get("app_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(&site.docroot)
        .to_string();

    let venv_dir = settings
        .get("venv_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}/.venv", app_dir));

    let job_type = match payload.job_type.as_str() {
        "uv_venv" => PythonSiteJobType::UvVenvCreate,
        "uv_pip_install" => PythonSiteJobType::UvPipInstallRequirements {
            req_file: payload.req_file,
        },
        "uv_pip_pyproject" => PythonSiteJobType::UvPipInstallPyproject,
        "custom" => {
            let script_name = match payload.script_name {
                Some(name) => name,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": "script_name wajib diisi untuk job kustom" })),
                    )
                        .into_response();
                }
            };
            PythonSiteJobType::CustomScript { script_name }
        }
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("job_type '{}' tidak valid. Pilihan: uv_venv, uv_pip_install, uv_pip_pyproject, custom", other)
                })),
            )
                .into_response();
        }
    };

    let job_req = PythonSiteJobRequest {
        site_id: format!("site_{}", site.id),
        username: linux_uname.to_string(),
        uid: resolved_linux_uid,
        python_version: python_ver,
        app_dir,
        venv_dir,
        job_type,
        timeout_secs: payload.timeout_secs,
        base_runtime_dir: None,
    };

    match state.agent_client.run_python_site_job(job_req).await {
        Ok(res) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "site_id": res.site_id,
                "command": res.command,
                "exit_code": res.exit_code,
                "success": res.success,
                "stdout": res.stdout,
                "stderr": res.stderr,
                "duration_ms": res.duration_ms
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal mengeksekusi job Python: {}", e) })),
        )
            .into_response(),
    }
}

/// GET /api/v1/sites/{id}/python/settings
/// Retrieves the Python configuration settings for a site.
pub async fn get_python_site_settings_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    let (site, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let settings: serde_json::Value = serde_json::from_str(&site.settings_json).unwrap_or_default();
    let dto = build_python_site_settings_dto(&site, &settings);
    (StatusCode::OK, Json(dto)).into_response()
}

/// Helper function to construct PythonSiteSettingsDto from Site model and settings JSON.
fn build_python_site_settings_dto(
    site: &panel_db::models::Site,
    settings: &serde_json::Value,
) -> PythonSiteSettingsDto {
    let python_version = site
        .version
        .clone()
        .or_else(|| {
            settings
                .get("python_version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| DEFAULT_PYTHON_VERSION.to_string());

    let framework_preset = settings
        .get("framework_preset")
        .or_else(|| settings.get("framework"))
        .and_then(|v| v.as_str())
        .unwrap_or("generic")
        .to_string();

    let server_type = settings
        .get("server_type")
        .and_then(|v| v.as_str())
        .unwrap_or("gunicorn_wsgi")
        .to_string();

    let entrypoint_type = settings
        .get("entrypoint_type")
        .and_then(|v| v.as_str())
        .unwrap_or("wsgi")
        .to_string();

    let entrypoint_value = settings
        .get("entrypoint_value")
        .or_else(|| settings.get("entrypoint"))
        .and_then(|v| v.as_str())
        .unwrap_or("app:app")
        .to_string();

    let workers = settings
        .get("workers")
        .and_then(|v| v.as_u64())
        .map(|w| w as u32)
        .unwrap_or_else(|| calculate_default_python_workers(1, None));

    let app_dir = settings
        .get("app_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(&site.docroot)
        .to_string();

    let venv_dir = settings
        .get("venv_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}/.venv", app_dir));

    let socket_path = settings
        .get("socket_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}/site_{}/app.sock", DEFAULT_PYTHON_SOCKET_DIR, site.id));

    let static_url = settings
        .get("static_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let static_dir = settings
        .get("static_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let media_url = settings
        .get("media_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let media_dir = settings
        .get("media_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let auto_restart = settings
        .get("auto_restart")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let mut env_vars = BTreeMap::new();
    if let Some(env_obj) = settings.get("env_vars").and_then(|v| v.as_object()) {
        for (k, v) in env_obj {
            if let Some(val_str) = v.as_str() {
                env_vars.insert(k.clone(), val_str.to_string());
            }
        }
    }

    PythonSiteSettingsDto {
        site_id: site.id,
        python_version,
        framework_preset,
        server_type,
        entrypoint_type,
        entrypoint_value,
        workers,
        app_dir,
        venv_dir,
        socket_path,
        static_url,
        static_dir,
        media_url,
        media_dir,
        auto_restart,
        env_vars,
    }
}

/// PUT /api/v1/sites/{id}/python/settings
/// Updates Python settings for a site and reapplies configuration if active.
pub async fn update_python_site_settings_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<UpdatePythonSiteSettingsRequest>,
) -> Response {
    let (mut site, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let mut settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));

    if let Some(ref ver) = payload.python_version {
        let norm_ver = normalize_python_version(ver);
        site.version = Some(norm_ver.clone());
        settings["python_version"] = serde_json::Value::String(norm_ver);
    }
    if let Some(ref fw) = payload.framework_preset {
        settings["framework_preset"] = serde_json::Value::String(fw.clone());
    }
    if let Some(ref st) = payload.server_type {
        settings["server_type"] = serde_json::Value::String(st.clone());
    }
    if let Some(ref et) = payload.entrypoint_type {
        settings["entrypoint_type"] = serde_json::Value::String(et.clone());
    }
    if let Some(ref ev) = payload.entrypoint_value {
        settings["entrypoint_value"] = serde_json::Value::String(ev.clone());
    }
    if let Some(w) = payload.workers {
        settings["workers"] = serde_json::Value::Number(serde_json::Number::from(w));
    }
    if let Some(ref dir) = payload.app_dir {
        settings["app_dir"] = serde_json::Value::String(dir.clone());
    }
    if let Some(ref venv) = payload.venv_dir {
        settings["venv_dir"] = serde_json::Value::String(venv.clone());
    }
    if let Some(ref su) = payload.static_url {
        settings["static_url"] = serde_json::Value::String(su.clone());
    }
    if let Some(ref sd) = payload.static_dir {
        settings["static_dir"] = serde_json::Value::String(sd.clone());
    }
    if let Some(ref mu) = payload.media_url {
        settings["media_url"] = serde_json::Value::String(mu.clone());
    }
    if let Some(ref md) = payload.media_dir {
        settings["media_dir"] = serde_json::Value::String(md.clone());
    }
    if let Some(ar) = payload.auto_restart {
        settings["auto_restart"] = serde_json::Value::Bool(ar);
    }
    if let Some(ref env) = payload.env_vars {
        settings["env_vars"] = serde_json::to_value(env).unwrap_or_default();
    }

    let updated_settings_json = serde_json::to_string(&settings).unwrap_or_default();
    site.settings_json = updated_settings_json.clone();

    let site_repo =
        panel_db::repositories::site_repo::SqliteSiteRepository::new(state.pool.clone());
    let update_dto = panel_db::models::UpdateSite {
        kind: None,
        docroot: None,
        runtime: Some("python".to_string()),
        version: site.version.clone(),
        status: None,
        settings_json: Some(updated_settings_json),
    };
    if let Err(e) = site_repo.update(site.id, &update_dto).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal memperbarui pengaturan situs: {}", e) })),
        )
            .into_response();
    }

    // Reapply configuration on host if site is active
    if site.status == "active" {
        match crate::sites::apply_site_config(&state, &site).await {
            Ok(()) => {}
            Err((code, msg)) => {
                return (code, Json(serde_json::json!({ "error": msg }))).into_response();
            }
        }
    }

    let dto = build_python_site_settings_dto(&site, &settings);
    (StatusCode::OK, Json(dto)).into_response()
}
