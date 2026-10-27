//! Node.js runtime management endpoints: listing remote & installed versions,
//! installation with SHA256 checksum verification, deletion protection when used by sites,
//! and corepack (pnpm & yarn) management.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_proto::{
    NodeRuntimeCorepackRequest, NodeRuntimeInstallRequest, NodeRuntimeRemoveRequest,
};
use panel_runtime::node::{DEFAULT_NODE_MIRROR, normalize_node_version, raw_node_version_tag};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};

use crate::auth::{RequireAdminCsrf, RequireAuth};
use crate::jobs::JobItem;
use crate::routes::AppState;

/// Query parameters for fetching Node.js versions.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct NodeVersionQuery {
    pub mirror_url: Option<String>,
    pub refresh: Option<bool>,
}

/// Information about a single Node.js runtime version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NodeVersionItemDto {
    pub version: String,
    pub support_status: String,
    pub raw_version: String,
    pub is_installed: bool,
    pub installed_path: Option<String>,
    pub is_lts: bool,
    pub lts_codename: Option<String>,
    pub release_date: Option<String>,
    pub npm_version: Option<String>,
    pub corepack_enabled: bool,
    pub used_by_sites_count: usize,
    pub used_by_domains: Vec<String>,
    pub can_delete: bool,
}

/// API Response containing the list of Node.js versions and metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NodeRuntimeListApiResponse {
    pub mirror_url: String,
    pub versions: Vec<NodeVersionItemDto>,
    pub installed_count: usize,
    pub is_offline_fallback: bool,
}

/// Request payload to install a Node.js version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NodeRuntimeInstallApiRequest {
    pub version: String,
    pub mirror_url: Option<String>,
    pub offline_tarball_path: Option<String>,
    pub offline_checksum: Option<String>,
    pub enable_corepack: Option<bool>,
    pub base_dir: Option<String>,
}

/// Response returned after a Node.js runtime action.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NodeRuntimeActionApiResponse {
    pub version: String,
    pub success: bool,
    pub path: String,
    pub message: String,
}

/// GET /api/v1/runtimes/node/versions
/// Lists all available Node.js versions (from official index.json or offline catalog),
/// indicating local installation status and site usage.
#[utoipa::path(
    get,
    path = "/api/v1/runtimes/node/versions",
    params(NodeVersionQuery),
    responses(
        (status = 200, description = "Daftar versi Node.js berhasil dimuat", body = NodeRuntimeListApiResponse),
        (status = 401, description = "Tidak terautentikasi")
    ),
    security(("session_cookie" = []))
)]
pub async fn get_node_versions_handler(
    State(state): State<AppState>,
    _auth: RequireAuth,
    Query(query): Query<NodeVersionQuery>,
) -> Response {
    let mirror_url = query
        .mirror_url
        .as_deref()
        .unwrap_or(DEFAULT_NODE_MIRROR)
        .trim_end_matches('/')
        .to_string();

    let installed_res = match state.agent_client.list_node_runtimes(None).await {
        Ok(res) => res.versions,
        Err(e) => {
            tracing::warn!("Agent list_node_runtimes error: {}", e);
            Vec::new()
        }
    };

    let sites_usage = fetch_node_sites_usage(&state.pool).await;
    let cache = match crate::runtime_catalog::node_cache(&state.pool).await {
        Ok(cache) => cache,
        Err(error) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": error})),
            )
                .into_response();
        }
    };
    let is_offline_fallback = !cache.fresh(chrono::Utc::now().timestamp());
    let remote_entries = if is_offline_fallback {
        Vec::new()
    } else {
        crate::runtime_catalog::node_choices(&cache.data, chrono::Utc::now().date_naive())
    };
    let installed_count = installed_res.len();
    let mut versions =
        merge_node_catalog_and_installed(&remote_entries, &installed_res, &sites_usage);
    for version in &mut versions {
        version.support_status = if is_offline_fallback {
            "unverified"
        } else {
            crate::runtime_catalog::node_support_status(
                &version.version,
                &cache.data,
                chrono::Utc::now().date_naive(),
            )
        }
        .into();
    }

    (
        StatusCode::OK,
        Json(NodeRuntimeListApiResponse {
            mirror_url,
            versions,
            installed_count,
            is_offline_fallback,
        }),
    )
        .into_response()
}

async fn fetch_node_sites_usage(
    pool: &panel_db::pool::DbPool,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut sites_usage: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    let db_res = sqlx::query(
        "SELECT sites.version as site_version, domains.name as domain_name \
         FROM sites \
         LEFT JOIN domains ON sites.id = domains.site_id AND domains.is_primary = 1 \
         WHERE (sites.kind = 'nodejs' OR sites.runtime = 'nodejs')",
    )
    .fetch_all(pool)
    .await;

    if let Ok(rows) = db_res {
        for row in rows {
            let raw_v: Option<String> = row.get("site_version");
            if let Some(raw_v) = raw_v {
                let norm = normalize_node_version(&raw_v);
                let dom: Option<String> = row.get("domain_name");
                let dom = dom.unwrap_or_else(|| "situs-tanpa-domain".to_string());
                sites_usage.entry(norm).or_default().push(dom);
            }
        }
    }

    sites_usage
}

fn merge_node_catalog_and_installed(
    catalog: &[panel_runtime::node::NodeVersionEntry],
    installed: &[panel_proto::NodeInstalledVersionDto],
    sites_usage: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<NodeVersionItemDto> {
    let mut version_map: std::collections::HashMap<String, NodeVersionItemDto> =
        std::collections::HashMap::new();

    for entry in catalog {
        let norm_v = entry.normalized_version();
        let domains = sites_usage.get(&norm_v).cloned().unwrap_or_default();
        let used_by_sites_count = domains.len();

        version_map.insert(
            norm_v.clone(),
            NodeVersionItemDto {
                version: norm_v.clone(),
                support_status: "unverified".into(),
                raw_version: format!("v{}", norm_v),
                is_installed: false,
                installed_path: None,
                is_lts: entry.is_lts(),
                lts_codename: entry.lts_codename(),
                release_date: entry.date.clone(),
                npm_version: entry.npm.clone(),
                corepack_enabled: false,
                used_by_sites_count,
                used_by_domains: domains,
                can_delete: false,
            },
        );
    }

    for inst in installed {
        let norm_v = normalize_node_version(&inst.version);
        let domains = sites_usage.get(&norm_v).cloned().unwrap_or_default();
        let used_by_sites_count = domains.len();
        let can_delete = used_by_sites_count == 0;

        let entry = version_map
            .entry(norm_v.clone())
            .or_insert_with(|| NodeVersionItemDto {
                version: norm_v.clone(),
                support_status: "unverified".into(),
                raw_version: format!("v{}", norm_v),
                is_installed: true,
                installed_path: Some(inst.path.clone()),
                is_lts: inst.is_lts,
                lts_codename: inst.lts_codename.clone(),
                release_date: None,
                npm_version: None,
                corepack_enabled: inst.corepack_enabled,
                used_by_sites_count,
                used_by_domains: domains.clone(),
                can_delete,
            });

        entry.is_installed = true;
        entry.installed_path = Some(inst.path.clone());
        entry.corepack_enabled = inst.corepack_enabled;
        entry.can_delete = can_delete;
    }

    let mut versions: Vec<NodeVersionItemDto> = version_map.into_values().collect();

    versions.sort_by(|a, b| {
        b.is_installed
            .cmp(&a.is_installed)
            .then_with(|| b.is_lts.cmp(&a.is_lts))
            .then_with(|| {
                crate::runtime_catalog::version_key(&b.version)
                    .cmp(&crate::runtime_catalog::version_key(&a.version))
            })
    });

    versions
}

/// POST /api/v1/runtimes/node/install
/// Enqueues installation with checksum verification and optional corepack.
#[utoipa::path(
    post,
    path = "/api/v1/runtimes/node/install",
    request_body = NodeRuntimeInstallApiRequest,
    responses(
        (status = 202, description = "Tugas pemasangan Node.js dibuat", body = JobItem),
        (status = 400, description = "Permintaan tidak valid"),
        (status = 403, description = "Akses ditolak (hanya admin)")
    ),
    security(("session_cookie" = [], "csrf_token" = []))
)]
pub async fn install_node_version_handler(
    State(state): State<AppState>,
    admin: RequireAdminCsrf,
    Json(payload): Json<NodeRuntimeInstallApiRequest>,
) -> Response {
    let norm_ver = normalize_node_version(&payload.version);
    if norm_ver.is_empty()
        || norm_ver.contains("..")
        || norm_ver.contains('/')
        || norm_ver.contains('\\')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Format versi Node.js '{}' tidak valid", payload.version)
            })),
        )
            .into_response();
    }

    let req = NodeRuntimeInstallRequest {
        version: norm_ver.clone(),
        mirror_url: payload.mirror_url,
        offline_tarball_path: payload.offline_tarball_path,
        offline_checksum: payload.offline_checksum,
        enable_corepack: payload.enable_corepack.unwrap_or(true),
        base_dir: payload.base_dir,
    };
    match crate::runtime_catalog::node_installable(&state.pool,&norm_ver).await {
        Ok(true)=>{},
        Ok(false)=>return (StatusCode::UNPROCESSABLE_ENTITY,Json(serde_json::json!({"error":"Pilih versi Node.js LTS yang masih didukung. Perbarui katalog runtime sebelum memasang."}))).into_response(),
        Err(error)=>return (StatusCode::SERVICE_UNAVAILABLE,Json(serde_json::json!({"error":error}))).into_response(),
    }

    let payload_json = match serde_json::to_string(&req) {
        Ok(json) => json,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Gagal menyiapkan tugas pemasangan Node.js"
                })),
            )
                .into_response();
        }
    };
    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(admin.0.id),
            None,
            "node.install".to_string(),
            payload_json,
            Some(3),
        )
        .await
    {
        Ok(result) => result,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Gagal membuat antrean pemasangan Node.js: {e}")
                })),
            )
                .into_response();
        }
    };
    state.job_runner.notify.notify_one();
    state.audit_writer.log_async(
        Some(admin.0.id),
        &admin.0.username,
        "node.runtime_install",
        raw_node_version_tag(&norm_ver),
        "success",
        None,
        Some(format!(
            "Tugas pemasangan Node.js v{norm_ver} dibuat: {}",
            job.id
        )),
    );
    (StatusCode::ACCEPTED, Json(JobItem::from(job))).into_response()
}

/// DELETE /api/v1/runtimes/node/{version}
/// Deletes an installed Node.js runtime version.
/// Enforces PRD Protection: Rejects with 409 Conflict if the version is in use by any site!
#[utoipa::path(
    delete,
    path = "/api/v1/runtimes/node/{version}",
    params(
        ("version" = String, Path, description = "Versi Node.js yang akan dihapus")
    ),
    responses(
        (status = 200, description = "Node.js berhasil dihapus", body = NodeRuntimeActionApiResponse),
        (status = 409, description = "Versi sedang digunakan oleh situs dan tidak dapat dihapus"),
        (status = 403, description = "Akses ditolak (hanya admin)")
    ),
    security(("session_cookie" = [], "csrf_token" = []))
)]
pub async fn remove_node_version_handler(
    State(state): State<AppState>,
    admin: RequireAdminCsrf,
    AxumPath(version): AxumPath<String>,
) -> Response {
    let norm_ver = normalize_node_version(&version);
    if norm_ver.is_empty()
        || norm_ver.contains("..")
        || norm_ver.contains('/')
        || norm_ver.contains('\\')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Format versi Node.js '{}' tidak valid", version)
            })),
        )
            .into_response();
    }

    let raw_tag = raw_node_version_tag(&norm_ver);

    // Protection Check: Query DB for sites using this version
    let using_sites_res = sqlx::query(
        "SELECT domains.name as domain_name \
         FROM sites \
         LEFT JOIN domains ON sites.id = domains.site_id AND domains.is_primary = 1 \
         WHERE (sites.kind = 'nodejs' OR sites.runtime = 'nodejs') \
           AND (sites.version = ? OR sites.version = ?)",
    )
    .bind(&norm_ver)
    .bind(&raw_tag)
    .fetch_all(&state.pool)
    .await;

    match using_sites_res {
        Ok(sites) if !sites.is_empty() => {
            let site_names: Vec<String> = sites
                .into_iter()
                .map(|s| {
                    let dom: Option<String> = s.get("domain_name");
                    dom.unwrap_or_else(|| "situs".to_string())
                })
                .collect();

            let err_msg = format!(
                "Versi Node.js '{}' sedang digunakan oleh {} situs ({}) dan tidak dapat dihapus.",
                norm_ver,
                site_names.len(),
                site_names.join(", ")
            );

            state.audit_writer.log_async(
                Some(admin.0.id),
                &admin.0.username,
                "node.runtime_remove",
                raw_tag,
                "denied",
                None,
                Some(err_msg.clone()),
            );

            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": err_msg,
                    "used_by_domains": site_names
                })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("Database query error checking Node.js usage: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Gagal memeriksa penggunaan runtime di basis data"
                })),
            )
                .into_response();
        }
        _ => {}
    }

    let req = NodeRuntimeRemoveRequest {
        version: norm_ver.clone(),
        base_dir: None,
    };

    match state.agent_client.remove_node_runtime(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(admin.0.id),
                &admin.0.username,
                "node.runtime_remove",
                raw_tag,
                "success",
                None,
                Some(format!("Berhasil menghapus Node.js v{}", norm_ver)),
            );

            (
                StatusCode::OK,
                Json(NodeRuntimeActionApiResponse {
                    version: res.version,
                    success: res.success,
                    path: res.path,
                    message: res.message,
                }),
            )
                .into_response()
        }
        Err(e) => {
            state.audit_writer.log_async(
                Some(admin.0.id),
                &admin.0.username,
                "node.runtime_remove",
                raw_tag,
                "error",
                None,
                Some(format!("Gagal menghapus Node.js v{}: {}", norm_ver, e)),
            );

            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Gagal menghapus Node.js v{}: {}", norm_ver, e)
                })),
            )
                .into_response()
        }
    }
}

/// POST /api/v1/runtimes/node/{version}/corepack
/// Re-enables or refreshes corepack shims (`pnpm` & `yarn`) for an installed Node.js runtime.
#[utoipa::path(
    post,
    path = "/api/v1/runtimes/node/{version}/corepack",
    params(
        ("version" = String, Path, description = "Versi Node.js yang akan diaktifkan corepack-nya")
    ),
    responses(
        (status = 200, description = "Corepack berhasil diaktifkan", body = NodeRuntimeActionApiResponse),
        (status = 400, description = "Gagal mengaktifkan corepack"),
        (status = 403, description = "Akses ditolak (hanya admin)")
    ),
    security(("session_cookie" = [], "csrf_token" = []))
)]
pub async fn enable_node_corepack_handler(
    State(state): State<AppState>,
    admin: RequireAdminCsrf,
    AxumPath(version): AxumPath<String>,
) -> Response {
    let norm_ver = normalize_node_version(&version);

    let req = NodeRuntimeCorepackRequest {
        version: norm_ver.clone(),
        base_dir: None,
    };

    match state.agent_client.enable_node_corepack(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(admin.0.id),
                &admin.0.username,
                "node.runtime_corepack",
                raw_node_version_tag(&norm_ver),
                "success",
                None,
                Some(format!(
                    "Berhasil mengaktifkan corepack untuk Node.js v{}",
                    norm_ver
                )),
            );

            (
                StatusCode::OK,
                Json(NodeRuntimeActionApiResponse {
                    version: res.version,
                    success: res.success,
                    path: res.path,
                    message: res.message,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Gagal mengaktifkan corepack untuk Node.js v{}: {}", norm_ver, e)
            })),
        )
            .into_response(),
    }
}
