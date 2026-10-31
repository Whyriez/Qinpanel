//! Reverse proxy site API handlers: settings management, SSRF guard enforcement,
//! and upstream probing via privileged agent RPC.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_db::{
    SqliteDomainRepository, SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository,
    models::{Site, UpdateSite},
};
use panel_proto::ProxyUpstreamProbeRequest;
use panel_webserver::template::{
    HeaderConfig, validate_header_config, validate_reverse_proxy_upstream_url,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::{IntoParams, ToSchema};

use crate::auth::middleware::AuthenticatedUser;
use crate::routes::AppState;
use crate::sites::{fetch_and_authorize_site, recompile_and_apply_site_vhost};

/// Configuration for a custom header in reverse proxy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct ProxyHeaderConfig {
    pub name: String,
    pub value: String,
}

impl From<ProxyHeaderConfig> for HeaderConfig {
    fn from(cfg: ProxyHeaderConfig) -> Self {
        HeaderConfig {
            name: cfg.name,
            value: cfg.value,
        }
    }
}

impl From<HeaderConfig> for ProxyHeaderConfig {
    fn from(cfg: HeaderConfig) -> Self {
        ProxyHeaderConfig {
            name: cfg.name,
            value: cfg.value,
        }
    }
}

/// Settings DTO for a reverse proxy website.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ProxySiteSettingsDto {
    pub site_id: i64,
    pub upstream_url: String,
    pub preserve_host: bool,
    pub websocket_support: bool,
    pub proxy_connect_timeout: Option<String>,
    pub proxy_send_timeout: Option<String>,
    pub proxy_read_timeout: Option<String>,
    pub proxy_buffering: Option<bool>,
    pub custom_request_headers: Vec<ProxyHeaderConfig>,
    pub custom_response_headers: Vec<ProxyHeaderConfig>,
    pub allow_external_upstream: bool,
}

/// Request payload to update reverse proxy site settings.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, Default)]
pub struct UpdateProxySiteSettingsRequest {
    pub upstream_url: Option<String>,
    pub preserve_host: Option<bool>,
    pub websocket_support: Option<bool>,
    pub proxy_connect_timeout: Option<String>,
    pub proxy_send_timeout: Option<String>,
    pub proxy_read_timeout: Option<String>,
    pub proxy_buffering: Option<bool>,
    pub custom_request_headers: Option<Vec<ProxyHeaderConfig>>,
    pub custom_response_headers: Option<Vec<ProxyHeaderConfig>>,
}

/// Query parameters for testing upstream health.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ProxyProbeQuery {
    pub upstream_url: Option<String>,
    pub timeout_ms: Option<u64>,
}

/// Checks whether a tenant user is authorized by their plan to use external upstreams.
pub async fn check_user_allow_external_upstream(
    pool: &SqlitePool,
    user_id: i64,
    caller_is_admin: bool,
) -> bool {
    if caller_is_admin {
        return true;
    }
    let user_repo = SqliteUserRepository::new(pool.clone());
    let Ok(Some(user)) = user_repo.find_by_id(user_id).await else {
        return false;
    };
    if user.role == "admin" {
        return true;
    }
    let Some(plan_id) = user.plan_id else {
        return false;
    };
    let plan_repo = SqlitePlanRepository::new(pool.clone());
    if let Ok(Some(plan)) = plan_repo.find_by_id(plan_id).await {
        plan.allow_external_upstream
    } else {
        false
    }
}

pub fn extract_proxy_settings_dto(site: &Site, allow_external: bool) -> ProxySiteSettingsDto {
    let settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));

    let upstream_url = settings
        .get("upstream_url")
        .and_then(|v| v.as_str())
        .unwrap_or("http://127.0.0.1:8080")
        .to_string();

    let preserve_host = settings
        .get("preserve_host")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let websocket_support = settings
        .get("websocket_support")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let proxy_connect_timeout = settings
        .get("proxy_connect_timeout")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let proxy_send_timeout = settings
        .get("proxy_send_timeout")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let proxy_read_timeout = settings
        .get("proxy_read_timeout")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let proxy_buffering = settings.get("proxy_buffering").and_then(|v| v.as_bool());

    let custom_request_headers: Vec<ProxyHeaderConfig> = settings
        .get("custom_request_headers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let custom_response_headers: Vec<ProxyHeaderConfig> = settings
        .get("custom_response_headers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    ProxySiteSettingsDto {
        site_id: site.id,
        upstream_url,
        preserve_host,
        websocket_support,
        proxy_connect_timeout,
        proxy_send_timeout,
        proxy_read_timeout,
        proxy_buffering,
        custom_request_headers,
        custom_response_headers,
        allow_external_upstream: allow_external,
    }
}

/// GET /api/v1/sites/{id}/proxy/settings
/// Retrieves reverse proxy settings for a site.
pub async fn get_proxy_site_settings_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    if site.kind != "proxy" && site.runtime.as_deref() != Some("proxy") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Situs ini bukan situs reverse proxy" })),
        )
            .into_response();
    }

    let allow_external =
        check_user_allow_external_upstream(&state.pool, site.user_id, auth_user.is_admin()).await;
    let dto = extract_proxy_settings_dto(&site, allow_external);

    (StatusCode::OK, Json(dto)).into_response()
}

/// PUT /api/v1/sites/{id}/proxy/settings
/// Updates reverse proxy settings for a site and recompiles Nginx vhost atomically.
#[allow(clippy::cognitive_complexity)]
pub async fn update_proxy_site_settings_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<UpdateProxySiteSettingsRequest>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    if site.kind != "proxy" && site.runtime.as_deref() != Some("proxy") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Situs ini bukan situs reverse proxy" })),
        )
            .into_response();
    }

    let allow_external =
        check_user_allow_external_upstream(&state.pool, site.user_id, auth_user.is_admin()).await;

    let mut settings_map: serde_json::Map<String, serde_json::Value> =
        match serde_json::from_str(&site.settings_json) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };

    // 1. Upstream URL with SSRF Guard validation
    if let Some(new_upstream) = payload.upstream_url {
        let trimmed = new_upstream.trim();
        if trimmed.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Upstream URL tidak boleh kosong" })),
            )
                .into_response();
        }
        if let Err(e) = validate_reverse_proxy_upstream_url(trimmed, allow_external) {
            let err_str = e.to_string();
            let status = if err_str.contains("not permit") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            };
            return (status, Json(serde_json::json!({ "error": err_str }))).into_response();
        }
        if let Err((status, message)) =
            crate::proxy_policy::admit(&state.pool, site.user_id, trimmed).await
        {
            return (status, Json(serde_json::json!({"error":message}))).into_response();
        }
        settings_map.insert(
            "upstream_url".to_string(),
            serde_json::json!(trimmed.to_string()),
        );
    }

    // 2. Flags
    if let Some(preserve) = payload.preserve_host {
        settings_map.insert("preserve_host".to_string(), serde_json::json!(preserve));
    }
    if let Some(ws) = payload.websocket_support {
        settings_map.insert("websocket_support".to_string(), serde_json::json!(ws));
    }

    // 3. Timeouts
    if let Some(t) = payload.proxy_connect_timeout {
        if let Err(e) = validate_timeout_input(&t) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
        settings_map.insert("proxy_connect_timeout".to_string(), serde_json::json!(t));
    }
    if let Some(t) = payload.proxy_send_timeout {
        if let Err(e) = validate_timeout_input(&t) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
        settings_map.insert("proxy_send_timeout".to_string(), serde_json::json!(t));
    }
    if let Some(t) = payload.proxy_read_timeout {
        if let Err(e) = validate_timeout_input(&t) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
        settings_map.insert("proxy_read_timeout".to_string(), serde_json::json!(t));
    }

    // 4. Buffering
    if let Some(b) = payload.proxy_buffering {
        settings_map.insert("proxy_buffering".to_string(), serde_json::json!(b));
    }

    // 5. Custom headers
    let current_req_headers: Vec<ProxyHeaderConfig> = settings_map
        .get("custom_request_headers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let current_resp_headers: Vec<ProxyHeaderConfig> = settings_map
        .get("custom_response_headers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let target_req_headers = payload
        .custom_request_headers
        .unwrap_or(current_req_headers);
    let target_resp_headers = payload
        .custom_response_headers
        .unwrap_or(current_resp_headers);

    for h in &target_req_headers {
        let hc = HeaderConfig {
            name: h.name.clone(),
            value: h.value.clone(),
        };
        if let Err(e) = validate_header_config(&hc) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    }
    for h in &target_resp_headers {
        let hc = HeaderConfig {
            name: h.name.clone(),
            value: h.value.clone(),
        };
        if let Err(e) = validate_header_config(&hc) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    }

    settings_map.insert(
        "custom_request_headers".to_string(),
        serde_json::json!(target_req_headers),
    );
    settings_map.insert(
        "custom_response_headers".to_string(),
        serde_json::json!(target_resp_headers),
    );

    let new_settings_json = serde_json::Value::Object(settings_map).to_string();

    let updated_site = match site_repo
        .update(
            site_id,
            &UpdateSite {
                kind: None,
                docroot: None,
                runtime: None,
                version: None,
                status: None,
                settings_json: Some(new_settings_json),
            },
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Recompile and apply Nginx configuration atomically if site is active
    if updated_site.status == "active" {
        let domain_repo = SqliteDomainRepository::new(state.pool.clone());
        let domains = match domain_repo.find_by_site_id(site_id).await {
            Ok(d) => d,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

        let user_repo = SqliteUserRepository::new(state.pool.clone());
        let owner = match user_repo.find_by_id(updated_site.user_id).await {
            Ok(Some(u)) => u,
            _ => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "Pemilik situs tidak ditemukan" })),
                )
                    .into_response();
            }
        };

        if let Err((code, err)) =
            recompile_and_apply_site_vhost(&state, &updated_site, &owner, &domains).await
        {
            return (code, Json(serde_json::json!({ "error": err }))).into_response();
        }
    }

    let dto = extract_proxy_settings_dto(&updated_site, allow_external);
    (StatusCode::OK, Json(dto)).into_response()
}

/// GET /api/v1/sites/{id}/proxy/probe
/// Probes reverse proxy upstream health, measuring latency and response status code.
pub async fn probe_proxy_upstream_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<ProxyProbeQuery>,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    if site.kind != "proxy" && site.runtime.as_deref() != Some("proxy") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Situs ini bukan situs reverse proxy" })),
        )
            .into_response();
    }

    let allow_external =
        check_user_allow_external_upstream(&state.pool, site.user_id, auth_user.is_admin()).await;

    let target_upstream = if let Some(ref candidate) = query.upstream_url {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            let settings: serde_json::Value =
                serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
            settings
                .get("upstream_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://127.0.0.1:8080")
                .to_string()
        } else {
            trimmed.to_string()
        }
    } else {
        let settings: serde_json::Value =
            serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
        settings
            .get("upstream_url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://127.0.0.1:8080")
            .to_string()
    };

    // Strict SSRF guard check before dispatching probe
    if let Err(e) = validate_reverse_proxy_upstream_url(&target_upstream, allow_external) {
        let err_str = e.to_string();
        let status = if err_str.contains("not permit") {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::BAD_REQUEST
        };
        return (status, Json(serde_json::json!({ "error": err_str }))).into_response();
    }

    let admitted =
        match crate::proxy_policy::admit(&state.pool, site.user_id, &target_upstream).await {
            Ok(admitted) => admitted,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let probe_req = ProxyUpstreamProbeRequest {
        upstream_url: admitted.probe_url(),
        timeout_ms: query.timeout_ms,
    };

    match state.agent_client.probe_proxy_upstream(probe_req).await {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Probe failed: {e}") })),
        )
            .into_response(),
    }
}

fn validate_timeout_input(val: &str) -> Result<(), String> {
    if val.contains('\n')
        || val.contains('\r')
        || val.contains(';')
        || val.contains('{')
        || val.contains('}')
    {
        return Err("Timeout mengandung karakter terlarang".to_string());
    }
    let trimmed = val.trim();
    if trimmed.is_empty() || trimmed.len() > 10 {
        return Err("Format timeout tidak valid".to_string());
    }
    Ok(())
}
