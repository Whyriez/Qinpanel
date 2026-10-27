//! Site logs retrieval and WebSocket live tail streaming for Qinpanel.
//!
//! Provides:
//! - REST API endpoint `GET /api/v1/sites/{id}/logs` with pagination/tail limit and search filters.
//! - WebSocket endpoint `GET /ws/logs/{id}` with Origin validation, session auth, initial backlog,
//!   and real-time live tail streaming.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use panel_db::repositories::{SqliteDomainRepository, SqliteSiteRepository, SqliteUserRepository};
use panel_proto::SiteLogsReadRequest;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};
use utoipa::{IntoParams, ToSchema};

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;
use crate::sites::fetch_and_authorize_site;

#[derive(Debug, Deserialize, IntoParams)]
pub struct SiteLogsQuery {
    /// Log type to inspect: "access" (default) or "error".
    pub r#type: Option<String>,
    /// Number of recent lines to retrieve (default: 100, max: 1000).
    pub lines: Option<usize>,
    /// Optional search term to filter log entries.
    pub q: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SiteLogsResponse {
    pub site_id: i64,
    pub primary_domain: String,
    pub log_type: String,
    pub lines: Vec<String>,
    pub total_matched: usize,
}

#[derive(Debug, Deserialize)]
pub struct SiteLogsWsQuery {
    pub r#type: Option<String>,
    pub lines: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WsLogMessage {
    pub event: String,
    pub site_id: i64,
    pub log_type: String,
    pub lines: Vec<String>,
    pub timestamp: String,
}

/// Validates WebSocket Origin header against cross-site WebSocket hijacking (CSWSH).
pub fn validate_ws_origin(headers: &HeaderMap, secure: bool) -> bool {
    crate::public_url::single(headers, "host")
        .and_then(|host| crate::public_url::PanelOrigin::from_authority(host, secure).ok())
        .is_some_and(|origin| origin.matches_origin(headers, true))
}

#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/logs",
    params(
        ("id" = i64, Path, description = "Site ID"),
        SiteLogsQuery
    ),
    responses(
        (status = 200, description = "Site log lines retrieved successfully", body = SiteLogsResponse),
        (status = 400, description = "Invalid log type or parameters"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Site not found"),
        (status = 500, description = "Failed to read site logs from host agent")
    ),
    tag = "sites"
)]
pub async fn get_site_logs_handler(
    State(state): State<AppState>,
    Path(site_id): Path<i64>,
    Query(query): Query<SiteLogsQuery>,
    auth_user: AuthenticatedUser,
) -> Result<Json<SiteLogsResponse>, (StatusCode, String)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());

    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| (code, msg.to_string()))?;

    let owner = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Site tenant user not found".to_string(),
            )
        })?;

    let domains = domain_repo
        .find_by_site_id(site.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let raw_log_type = query
        .r#type
        .unwrap_or_else(|| "access".to_string())
        .to_lowercase();
    let log_type = match raw_log_type.as_str() {
        "access" => "access",
        "error" => "error",
        "php_error" | "php-error" => "php_error",
        "php_slow" | "php-slow" => "php_slow",
        "app" | "service" | "process" | "node" | "python" => "app",
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid log type. Expected 'access', 'error', 'php_error', 'php_slow', or 'app'"
                    .to_string(),
            ));
        }
    }
    .to_string();

    let lines_limit = query.lines.unwrap_or(100).clamp(1, 1000);
    let (resolved_linux_name, _resolved_linux_uid) =
        crate::tenant_context::execution_identity(&owner)?;
    let username = resolved_linux_name;

    let req = SiteLogsReadRequest {
        username,
        site_domain: primary_domain.clone(),
        log_type: log_type.clone(),
        lines: lines_limit,
        query: query.q,
        log_base_dir: None,
        site_id: Some(format!("site_{}", site.id)),
    };

    let log_res = state
        .agent_client
        .read_site_logs(req)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SiteLogsResponse {
        site_id: site.id,
        primary_domain,
        log_type,
        lines: log_res.lines,
        total_matched: log_res.total_matched,
    }))
}

/// WebSocket live tail endpoint: `GET /ws/logs/{id}`
pub async fn site_logs_ws_handler(
    State(state): State<AppState>,
    Path(site_id): Path<i64>,
    Query(query): Query<SiteLogsWsQuery>,
    headers: HeaderMap,
    auth_user: AuthenticatedUser,
    axum::Extension(context): axum::Extension<crate::public_url::PublicRequest>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    if !validate_ws_origin(&headers, context.origin.secure) {
        warn!(
            "WebSocket rejected due to disallowed Origin header for site {}",
            site_id
        );
        return Err((StatusCode::FORBIDDEN, "Invalid Origin"));
    }

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());

    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id).await?;

    let owner = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::NOT_FOUND, "Site tenant user not found"))?;

    let domains = domain_repo
        .find_by_site_id(site.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let raw_log_type = query
        .r#type
        .unwrap_or_else(|| "access".to_string())
        .to_lowercase();
    let log_type = match raw_log_type.as_str() {
        "access" => "access",
        "error" => "error",
        "php_error" | "php-error" => "php_error",
        "php_slow" | "php-slow" => "php_slow",
        "app" | "service" | "process" | "node" | "python" => "app",
        _ => {
            return Err((StatusCode::BAD_REQUEST, "Invalid log type"));
        }
    }
    .to_string();

    let backlog_lines = query.lines.unwrap_or(50).clamp(1, 200);
    let (resolved_linux_name, _resolved_linux_uid) =
        crate::tenant_context::execution_identity(&owner)
            .map_err(|(status, _)| (status, "Identitas tenant tidak valid"))?;
    let username = resolved_linux_name;

    Ok(ws.on_upgrade(move |socket| {
        handle_site_logs_ws(
            socket,
            state,
            site.id,
            primary_domain,
            username,
            log_type,
            backlog_lines,
        )
    }))
}

async fn send_ws_json<T: Serialize>(socket: &mut WebSocket, msg: &T) -> bool {
    if let Ok(json_str) = serde_json::to_string(msg) {
        socket.send(Message::Text(json_str.into())).await.is_ok()
    } else {
        false
    }
}

#[allow(clippy::cognitive_complexity)]
async fn handle_site_logs_ws(
    mut socket: WebSocket,
    state: AppState,
    site_id: i64,
    domain: String,
    username: String,
    log_type: String,
    backlog_lines: usize,
) {
    info!(
        "WebSocket live tail connected for site {} ({}) type {}",
        site_id, domain, log_type
    );

    // 1. Initial backlog read
    let initial_req = SiteLogsReadRequest {
        username: username.clone(),
        site_domain: domain.clone(),
        log_type: log_type.clone(),
        lines: backlog_lines,
        query: None,
        log_base_dir: None,
        site_id: Some(format!("site_{}", site_id)),
    };

    let mut last_total = match state.agent_client.read_site_logs(initial_req).await {
        Ok(res) => {
            let msg = WsLogMessage {
                event: "backlog".to_string(),
                site_id,
                log_type: log_type.clone(),
                lines: res.lines,
                timestamp: time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default(),
            };
            if !send_ws_json(&mut socket, &msg).await {
                return;
            }
            res.total_matched
        }
        Err(e) => {
            let err_msg = WsLogMessage {
                event: "error".to_string(),
                site_id,
                log_type: log_type.clone(),
                lines: vec![format!("Failed to read initial logs: {e}")],
                timestamp: time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default(),
            };
            let _ = send_ws_json(&mut socket, &err_msg).await;
            0
        }
    };

    // 2. Poll interval loop for live log streaming
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let poll_req = SiteLogsReadRequest {
                    username: username.clone(),
                    site_domain: domain.clone(),
                    log_type: log_type.clone(),
                    lines: 200,
                    query: None,
                    log_base_dir: None,
                    site_id: Some(format!("site_{}", site_id)),
                };

                if let Ok(poll_res) = state.agent_client.read_site_logs(poll_req).await {
                    if poll_res.total_matched > last_total {
                        let new_count = poll_res.total_matched - last_total;
                        let start = poll_res.lines.len().saturating_sub(new_count);
                        let new_lines = poll_res.lines[start..].to_vec();

                        let msg = WsLogMessage {
                            event: "line".to_string(),
                            site_id,
                            log_type: log_type.clone(),
                            lines: new_lines,
                            timestamp: time::OffsetDateTime::now_utc()
                                .format(&time::format_description::well_known::Rfc3339)
                                .unwrap_or_default(),
                        };

                        if !send_ws_json(&mut socket, &msg).await {
                            break;
                        }
                        last_total = poll_res.total_matched;
                    } else if poll_res.total_matched < last_total {
                        // Log file was rotated or truncated
                        last_total = poll_res.total_matched;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket client disconnected for site {}", site_id);
                        break;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error for site {}: {}", site_id, e);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
}
