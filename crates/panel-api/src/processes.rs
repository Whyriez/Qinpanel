//! Unified process management API handlers: list and inspect running Node.js and Python processes across sites.

use crate::auth::middleware::AuthenticatedUser;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_db::repositories::site_repo::SqliteSiteRepository;
use panel_proto::{NodeSiteStatusRequest, PythonSiteStatusRequest};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Information for a single running or managed application process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessItemDto {
    pub site_id: i64,
    pub primary_domain: String,
    pub runtime: String,
    pub version: Option<String>,
    pub service_name: String,
    pub active: bool,
    pub sub_state: String,
    pub pid: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub cpu_usage_usec: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub healthy: bool,
    pub port_or_socket: Option<String>,
    pub probe_message: Option<String>,
}

/// Unified process list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessListResponse {
    pub processes: Vec<ProcessItemDto>,
    pub total_active: usize,
    pub total_memory_bytes: u64,
}

/// GET /api/v1/processes
/// Lists all managed application processes (Node.js & Python) for the authenticated user or all sites for admin.
pub async fn list_processes_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());

    let sites = if auth_user.is_admin() {
        match site_repo.list_all().await {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
    } else {
        match site_repo.find_by_user_id(auth_user.id).await {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
    };

    let domain_repo =
        panel_db::repositories::domain_repo::SqliteDomainRepository::new(state.pool.clone());

    let mut process_items = Vec::new();
    let mut total_active = 0;
    let mut total_memory_bytes = 0;

    for site in sites {
        let is_node = site.kind == "nodejs" || site.runtime.as_deref() == Some("nodejs");
        let is_python = site.kind == "python" || site.runtime.as_deref() == Some("python");

        if !is_node && !is_python {
            continue;
        }

        // Fetch primary domain
        let primary_domain = match domain_repo.find_by_site_id(site.id).await {
            Ok(domains) => domains
                .into_iter()
                .find(|d| d.is_primary)
                .map(|d| d.name)
                .unwrap_or_else(|| site.docroot.clone()),
            Err(_) => site.docroot.clone(),
        };

        let settings: serde_json::Value =
            serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));

        if is_node {
            let port = settings
                .get("port")
                .and_then(|v| v.as_u64())
                .map(|p| p as u16);
            let req = NodeSiteStatusRequest {
                site_id: format!("site_{}", site.id),
                port,
                probe_path: None,
            };

            let version = site.version.clone().or_else(|| {
                settings
                    .get("node_version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

            match state.agent_client.status_node_site(req).await {
                Ok(status) => {
                    if status.active {
                        total_active += 1;
                    }
                    if let Some(mem) = status.memory_bytes {
                        total_memory_bytes += mem;
                    }

                    process_items.push(ProcessItemDto {
                        site_id: site.id,
                        primary_domain,
                        runtime: "nodejs".to_string(),
                        version,
                        service_name: status.service_name,
                        active: status.active,
                        sub_state: status.sub_state,
                        pid: status.pid,
                        memory_bytes: status.memory_bytes,
                        cpu_usage_usec: status.cpu_usage_usec,
                        uptime_secs: status.uptime_secs,
                        healthy: status.healthy,
                        port_or_socket: port.map(|p| format!("Port {p}")),
                        probe_message: status.probe_message,
                    });
                }
                Err(e) => {
                    process_items.push(ProcessItemDto {
                        site_id: site.id,
                        primary_domain,
                        runtime: "nodejs".to_string(),
                        version,
                        service_name: format!("site-site_{}.service", site.id),
                        active: false,
                        sub_state: "failed".to_string(),
                        pid: None,
                        memory_bytes: None,
                        cpu_usage_usec: None,
                        uptime_secs: None,
                        healthy: false,
                        port_or_socket: port.map(|p| format!("Port {p}")),
                        probe_message: Some(format!("Agent query error: {e}")),
                    });
                }
            }
        } else if is_python {
            let socket_path = Some(
                settings
                    .get("socket_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("/run/panel/python/site_{}/app.sock", site.id)),
            );

            let req = PythonSiteStatusRequest {
                site_id: format!("site_{}", site.id),
                socket_path: socket_path.clone(),
            };

            let version = site.version.clone().or_else(|| {
                settings
                    .get("python_version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

            match state.agent_client.status_python_site(req).await {
                Ok(status) => {
                    if status.active {
                        total_active += 1;
                    }
                    if let Some(mem) = status.memory_bytes {
                        total_memory_bytes += mem;
                    }

                    process_items.push(ProcessItemDto {
                        site_id: site.id,
                        primary_domain,
                        runtime: "python".to_string(),
                        version,
                        service_name: status.service_name,
                        active: status.active,
                        sub_state: status.sub_state,
                        pid: status.pid,
                        memory_bytes: status.memory_bytes,
                        cpu_usage_usec: status.cpu_usage_usec,
                        uptime_secs: status.uptime_secs,
                        healthy: status.healthy,
                        port_or_socket: socket_path,
                        probe_message: status.probe_message,
                    });
                }
                Err(e) => {
                    process_items.push(ProcessItemDto {
                        site_id: site.id,
                        primary_domain,
                        runtime: "python".to_string(),
                        version,
                        service_name: format!("site-site_{}.service", site.id),
                        active: false,
                        sub_state: "failed".to_string(),
                        pid: None,
                        memory_bytes: None,
                        cpu_usage_usec: None,
                        uptime_secs: None,
                        healthy: false,
                        port_or_socket: socket_path,
                        probe_message: Some(format!("Agent query error: {e}")),
                    });
                }
            }
        }
    }

    (
        StatusCode::OK,
        Json(ProcessListResponse {
            processes: process_items,
            total_active,
            total_memory_bytes,
        }),
    )
        .into_response()
}
