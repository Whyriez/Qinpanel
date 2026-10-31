//! Node.js site API handlers: port allocation (20000-29999), process lifecycle actions
//! (start, stop, restart, redeploy), health check status querying, tenant job execution,
//! and settings management.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_proto::{
    NodeSiteActionRequest, NodeSiteActionType, NodeSiteJobRequest, NodeSiteJobType,
    NodeSiteStatusRequest,
};
use panel_runtime::node::normalize_node_version;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::{BTreeMap, HashSet};
use std::net::TcpListener;
use utoipa::ToSchema;

use crate::auth::middleware::AuthenticatedUser;
use crate::routes::AppState;

pub const MIN_NODE_PORT: u16 = 20000;
pub const MAX_NODE_PORT: u16 = 29999;

pub(crate) async fn validate_runtime_port(
    pool: &SqlitePool,
    site_id: Option<i64>,
    port: u16,
) -> Result<(), (StatusCode, String)> {
    if !(MIN_NODE_PORT..=MAX_NODE_PORT).contains(&port) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Port aplikasi harus dalam rentang terisolasi 20000-29999".into(),
        ));
    }
    let failure = || {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Pemetaan port tidak dapat diverifikasi".into(),
        )
    };
    let rows: Vec<(i64, i64, String, String)> =
        sqlx::query_as("SELECT id,user_id,COALESCE(runtime,kind),settings_json FROM sites")
            .fetch_all(pool)
            .await
            .map_err(|_| failure())?;
    let current_owner = rows.iter().find_map(|(id, owner, _, raw)| {
        let settings: serde_json::Value = serde_json::from_str(raw).ok()?;
        (Some(*id) == site_id && settings["port"].as_u64() == Some(u64::from(port)))
            .then_some(*owner)
    });
    let conflict = rows.iter().any(|(id, owner, runtime, raw)| {
        if Some(*id) == site_id {
            return false;
        }
        let Ok(settings) = serde_json::from_str::<serde_json::Value>(raw) else {
            return false;
        };
        match runtime.as_str() {
            "nodejs" | "python" => settings["port"].as_u64() == Some(u64::from(port)),
            // Reserve even a stale proxy target. Deleting/changing its original
            // application must never hand its Nginx connection to a new tenant.
            // DNS names also reserve explicit ports, without a second DNS lookup.
            "proxy" => {
                proxy_reserved_port(&settings) == Some(port) && current_owner != Some(*owner)
            }
            _ => false,
        }
    });
    if conflict {
        return Err((
            StatusCode::CONFLICT,
            "Port aplikasi sudah terdaftar atau masih dirujuk proxy project lain".into(),
        ));
    }
    Ok(())
}

fn proxy_reserved_port(settings: &serde_json::Value) -> Option<u16> {
    settings["upstream_url"]
        .as_str()?
        .parse::<axum::http::Uri>()
        .ok()?
        .port_u16()
}

/// Request payload to trigger a Node.js process action.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct NodeSiteActionApiRequest {
    pub action: String, // "start", "stop", "restart", "redeploy"
}

/// Request payload to run a tenant job (npm install, build, script).
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct NodeSiteJobApiRequest {
    pub job_type: String, // "npm_install", "npm_ci", "npm_build", "pnpm_install", "yarn_install", "custom"
    pub script_name: Option<String>,
    pub timeout_secs: Option<u64>,
}

/// Settings for a Node.js website process.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct NodeSiteSettingsDto {
    pub site_id: i64,
    pub node_version: String,
    pub port: u16,
    pub app_dir: String,
    pub entrypoint_type: String, // "script", "npm_start", "npm_custom"
    pub entrypoint_value: Option<String>,
    pub auto_restart: bool,
    pub env_vars: BTreeMap<String, String>,
}

/// Request payload to update Node.js site settings.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct UpdateNodeSiteSettingsRequest {
    pub node_version: Option<String>,
    pub port: Option<u16>,
    pub app_dir: Option<String>,
    pub entrypoint_type: Option<String>,
    pub entrypoint_value: Option<String>,
    pub auto_restart: Option<bool>,
    pub env_vars: Option<BTreeMap<String, String>>,
}

/// Node site runtime status response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NodeSiteStatusApiResponse {
    pub site_id: String,
    pub service_name: String,
    pub active: bool,
    pub sub_state: String,
    pub pid: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub cpu_usage_usec: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub healthy: bool,
    pub port: u16,
    pub probe_status_code: Option<u16>,
    pub probe_message: Option<String>,
}

/// Allocates an unused port in the range 20000..=29999.
/// Checks uniqueness across all database sites and verifies local TCP bind availability.
pub async fn allocate_node_site_port(pool: &SqlitePool) -> Result<u16, String> {
    // Runtime and retained proxy configurations both reserve ports.
    let rows = sqlx::query("SELECT settings_json FROM sites")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Gagal membaca port situs dari database: {e}"))?;
    let mut used_ports = HashSet::new();
    for row in rows {
        let settings = row
            .try_get::<String, _>("settings_json")
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
        if let Some(settings) = settings {
            if let Some(port) = settings["port"]
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
            {
                used_ports.insert(port);
            }
            if let Some(port) = proxy_reserved_port(&settings) {
                used_ports.insert(port);
            }
        }
    }

    // 2. Find first free port in range that can also be bound to 127.0.0.1
    for port in MIN_NODE_PORT..=MAX_NODE_PORT {
        if used_ports.contains(&port) {
            continue;
        }

        // Verify socket is not currently in use locally
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }

    Err(format!(
        "Rentang port {}-{} telah habis teralokasi",
        MIN_NODE_PORT, MAX_NODE_PORT
    ))
}

/// Helper to verify site ownership and permissions.
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
                "Pengguna situs tidak ditemukan".to_string(),
            )
        })?;

    Ok((site, user))
}

/// GET /api/v1/sites/{id}/node/status
/// Queries systemd service status and runs HTTP health check probe on allocated port.
pub async fn get_node_site_status_handler(
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
    let port = settings
        .get("port")
        .and_then(|v| v.as_u64())
        .map(|p| p as u16)
        .unwrap_or(MIN_NODE_PORT);

    let probe_req = NodeSiteStatusRequest {
        site_id: format!("site_{}", site.id),
        port: Some(port),
        probe_path: Some("/".to_string()),
    };

    match state.agent_client.status_node_site(probe_req).await {
        Ok(status) => (
            StatusCode::OK,
            Json(NodeSiteStatusApiResponse {
                site_id: status.site_id,
                service_name: status.service_name,
                active: status.active,
                sub_state: status.sub_state,
                pid: status.pid,
                memory_bytes: status.memory_bytes,
                cpu_usage_usec: status.cpu_usage_usec,
                uptime_secs: status.uptime_secs,
                healthy: status.healthy,
                port,
                probe_status_code: status.probe_status_code,
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

/// POST /api/v1/sites/{id}/node/action
/// Triggers a process action (start, stop, restart, redeploy).
pub async fn action_node_site_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<NodeSiteActionApiRequest>,
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
    let node_ver = site
        .version
        .clone()
        .or_else(|| {
            settings
                .get("node_version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "22.14.0".to_string());
    let app_dir = settings
        .get("app_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(&site.docroot)
        .to_string();

    let action_str = payload.action.trim().to_lowercase();

    if action_str == "redeploy" {
        // Redeploy workflow:
        // 1. Run npm install job
        let job_req = NodeSiteJobRequest {
            site_id: format!("site_{}", site.id),
            username: linux_uname.to_string(),
            uid: resolved_linux_uid,
            node_version: node_ver.clone(),
            app_dir: app_dir.clone(),
            job_type: NodeSiteJobType::NpmInstall,
            timeout_secs: Some(300),
            base_runtime_dir: None,
        };
        let job_res = match state.agent_client.run_node_site_job(job_req).await {
            Ok(res) => res,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Gagal menjalankan npm install saat redeploy: {}", e)
                    })),
                )
                    .into_response();
            }
        };

        if !job_res.success {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Perintah npm install gagal",
                    "stdout": job_res.stdout,
                    "stderr": job_res.stderr,
                    "exit_code": job_res.exit_code
                })),
            )
                .into_response();
        }

        // 2. Restart service
        let action_req = NodeSiteActionRequest {
            site_id: format!("site_{}", site.id),
            action: NodeSiteActionType::Restart,
        };
        match state.agent_client.action_node_site(action_req).await {
            Ok(_res) => {
                state.audit_writer.log_async(
                    Some(auth_user.id),
                    &auth_user.username,
                    "site.node_redeploy",
                    format!("site:{}", site.id),
                    "success",
                    None,
                    Some(format!("Redeploy situs {} berhasil", site.id)),
                );
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "success": true,
                        "action": "redeploy",
                        "message": "Situs berhasil di-deploy ulang dan dimulai kembali",
                        "job_output": job_res.stdout
                    })),
                )
                    .into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Gagal merestart service setelah npm install: {}", e)
                })),
            )
                .into_response(),
        }
    } else {
        let action_type = match action_str.as_str() {
            "start" => NodeSiteActionType::Start,
            "stop" => NodeSiteActionType::Stop,
            "restart" => NodeSiteActionType::Restart,
            "reload" => NodeSiteActionType::Reload,
            other => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Aksi '{}' tidak valid. Pilihan: start, stop, restart, redeploy", other)
                    })),
                )
                    .into_response();
            }
        };

        let action_req = NodeSiteActionRequest {
            site_id: format!("site_{}", site.id),
            action: action_type,
        };

        match state.agent_client.action_node_site(action_req).await {
            Ok(res) => {
                if action_str == "restart" {
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
                state.audit_writer.log_async(
                    Some(auth_user.id),
                    &auth_user.username,
                    format!("site.node_{}", action_str),
                    format!("site:{}", site.id),
                    "success",
                    None,
                    Some(format!(
                        "Aksi '{}' pada situs {} berhasil",
                        action_str, site.id
                    )),
                );
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
                Json(serde_json::json!({
                    "error": format!("Aksi gagal dieksekusi: {}", e)
                })),
            )
                .into_response(),
        }
    }
}

/// POST /api/v1/sites/{id}/node/job
/// Runs a tenant command (`npm install`, `npm run build`, etc.) safely under tenant UID and slice.
pub async fn run_node_site_job_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<NodeSiteJobApiRequest>,
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
    let node_ver = site
        .version
        .clone()
        .or_else(|| {
            settings
                .get("node_version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "22.14.0".to_string());
    let app_dir = settings
        .get("app_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(&site.docroot)
        .to_string();

    let job_type = match payload.job_type.trim().to_lowercase().as_str() {
        "npm_install" | "install" => NodeSiteJobType::NpmInstall,
        "npm_ci" | "ci" => NodeSiteJobType::NpmCi,
        "npm_build" | "build" => NodeSiteJobType::NpmBuild,
        "pnpm_install" => NodeSiteJobType::PnpmInstall,
        "yarn_install" => NodeSiteJobType::YarnInstall,
        "custom" => {
            let script = payload.script_name.unwrap_or_default();
            if script.trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Nama script wajib diisi untuk tipe job custom" })),
                )
                    .into_response();
            }
            NodeSiteJobType::CustomScript {
                script_name: script,
            }
        }
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Tipe job '{}' tidak didukung", other)
                })),
            )
                .into_response();
        }
    };

    let req = NodeSiteJobRequest {
        site_id: format!("site_{}", site.id),
        username: linux_uname.to_string(),
        uid: resolved_linux_uid,
        node_version: node_ver,
        app_dir,
        job_type,
        timeout_secs: payload.timeout_secs.or(Some(300)),
        base_runtime_dir: None,
    };

    match state.agent_client.run_node_site_job(req).await {
        Ok(res) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "site_id": site.id,
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
            Json(serde_json::json!({
                "error": format!("Eksekusi job gagal: {}", e)
            })),
        )
            .into_response(),
    }
}

/// GET /api/v1/sites/{id}/node/settings
/// Fetches current Node.js site settings.
pub async fn get_node_site_settings_handler(
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
    let port = settings
        .get("port")
        .and_then(|v| v.as_u64())
        .map(|p| p as u16)
        .unwrap_or(MIN_NODE_PORT);

    let node_version = site
        .version
        .clone()
        .or_else(|| {
            settings
                .get("node_version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "22.14.0".to_string());

    let app_dir = settings
        .get("app_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(&site.docroot)
        .to_string();

    let entrypoint_type = settings
        .get("entrypoint_type")
        .and_then(|v| v.as_str())
        .unwrap_or("script")
        .to_string();

    let entrypoint_value = settings
        .get("entrypoint_value")
        .or_else(|| settings.get("entrypoint"))
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

    (
        StatusCode::OK,
        Json(NodeSiteSettingsDto {
            site_id: site.id,
            node_version,
            port,
            app_dir,
            entrypoint_type,
            entrypoint_value,
            auto_restart,
            env_vars,
        }),
    )
        .into_response()
}

/// PUT /api/v1/sites/{id}/node/settings
/// Updates Node.js site settings and re-applies configuration.
pub async fn update_node_site_settings_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<UpdateNodeSiteSettingsRequest>,
) -> Response {
    let (mut site, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let mut settings: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&site.settings_json).unwrap_or_default();

    if let Some(ref ver) = payload.node_version {
        let norm = normalize_node_version(ver);
        site.version = Some(norm.clone());
        settings.insert("node_version".to_string(), serde_json::Value::String(norm));
    }
    if let Some(port) = payload.port {
        if let Err((status, message)) =
            validate_runtime_port(&state.pool, Some(site.id), port).await
        {
            return (status, Json(serde_json::json!({"error":message}))).into_response();
        }
        settings.insert("port".to_string(), serde_json::json!(port));
        settings.insert(
            "upstream_url".to_string(),
            serde_json::json!(format!("http://127.0.0.1:{}", port)),
        );
    }
    if let Some(ref dir) = payload.app_dir {
        settings.insert(
            "app_dir".to_string(),
            serde_json::Value::String(dir.clone()),
        );
    }
    if let Some(ref ep_type) = payload.entrypoint_type {
        settings.insert(
            "entrypoint_type".to_string(),
            serde_json::Value::String(ep_type.clone()),
        );
    }
    if let Some(ref ep_val) = payload.entrypoint_value {
        settings.insert(
            "entrypoint_value".to_string(),
            serde_json::Value::String(ep_val.clone()),
        );
    }
    if let Some(restart) = payload.auto_restart {
        settings.insert("auto_restart".to_string(), serde_json::json!(restart));
    }
    if let Some(ref envs) = payload.env_vars {
        settings.insert("env_vars".to_string(), serde_json::json!(envs));
    }

    let updated_settings_json = serde_json::to_string(&settings).unwrap_or_default();
    site.settings_json = updated_settings_json.clone();

    // Update in database
    let site_repo =
        panel_db::repositories::site_repo::SqliteSiteRepository::new(state.pool.clone());
    let update_dto = panel_db::models::UpdateSite {
        kind: None,
        docroot: None,
        runtime: Some("nodejs".to_string()),
        version: site.version.clone(),
        status: None,
        settings_json: Some(updated_settings_json),
    };

    if let Err(e) = site_repo.update(site.id, &update_dto).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({ "error": format!("Gagal menyimpan pengaturan situs: {}", e) }),
            ),
        )
            .into_response();
    }

    // Re-apply site configuration
    if let Err((code, msg)) = crate::sites::apply_site_config(&state, &site).await {
        return (code, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    (
        StatusCode::OK,
        Json(NodeSiteSettingsDto {
            site_id: site.id,
            node_version: site.version.unwrap_or_else(|| "22.14.0".to_string()),
            port: settings
                .get("port")
                .and_then(|v| v.as_u64())
                .map(|p| p as u16)
                .unwrap_or(MIN_NODE_PORT),
            app_dir: settings
                .get("app_dir")
                .and_then(|v| v.as_str())
                .unwrap_or(&site.docroot)
                .to_string(),
            entrypoint_type: settings
                .get("entrypoint_type")
                .and_then(|v| v.as_str())
                .unwrap_or("script")
                .to_string(),
            entrypoint_value: settings
                .get("entrypoint_value")
                .or_else(|| settings.get("entrypoint"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            auto_restart: settings
                .get("auto_restart")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            env_vars: {
                let mut envs = BTreeMap::new();
                if let Some(obj) = settings.get("env_vars").and_then(|v| v.as_object()) {
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            envs.insert(k.clone(), s.to_string());
                        }
                    }
                }
                envs
            },
        }),
    )
        .into_response()
}
