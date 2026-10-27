//! Security Analytics and Temporary IP Banning with TTL.
//!
//! Provides endpoints to inspect probing behavior, brute force incidents,
//! top client IPs with risk evaluation, and manage Nginx-backed IP bans with TTL.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use chrono::Utc;
use panel_core::site_rules::{IpAccessRule, IpAction, SiteRulesConfig};
use panel_db::models::{CreateSiteBannedIpInput, SiteBannedIpModel, UpdateSite};
use panel_db::repositories::{SqliteSiteBannedIpRepository, SqliteSiteRepository};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;
use crate::site_rules::verify_site_access;
use crate::sites::recompile_and_apply_site_vhost;

// ==========================================
// DTO Definitions
// ==========================================

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct SecurityOverviewQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BanIpRequest {
    pub ip: String,
    /// Duration in seconds. None indicates permanent ban.
    pub ttl_secs: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BannedIpDto {
    pub id: String,
    pub site_id: i64,
    pub ip_address: String,
    pub reason: String,
    pub banned_at: i64,
    pub expires_at: Option<i64>,
    pub remaining_ttl_secs: Option<i64>,
    pub is_active: bool,
    pub banned_by: String,
}

impl From<(SiteBannedIpModel, i64)> for BannedIpDto {
    fn from((model, now): (SiteBannedIpModel, i64)) -> Self {
        let remaining_ttl_secs = model.expires_at.map(|exp| (exp - now).max(0));
        Self {
            id: model.id,
            site_id: model.site_id,
            ip_address: model.ip_address,
            reason: model.reason,
            banned_at: model.banned_at,
            expires_at: model.expires_at,
            remaining_ttl_secs,
            is_active: model.is_active,
            banned_by: model.banned_by,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TopSecurityIpDto {
    pub ip: String,
    pub total_requests: u64,
    pub error_4xx: u64,
    pub error_5xx: u64,
    pub probing_count: u64,
    pub auth_failures: u64,
    pub country: Option<String>,
    pub top_path: Option<String>,
    pub risk_level: String,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProbingPathStatDto {
    pub path: String,
    pub category: String,
    pub count: u64,
    pub unique_ips: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BruteForceIncidentDto {
    pub ip: String,
    pub target_path: String,
    pub failed_attempts: u64,
    pub first_attempt_ts: i64,
    pub last_attempt_ts: i64,
    pub country: Option<String>,
    pub risk_score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SecurityOverviewResponse {
    pub site_id: i64,
    pub from: i64,
    pub to: i64,
    pub total_probing_requests: u64,
    pub total_brute_force_attempts: u64,
    pub unique_suspicious_ips: u64,
    pub top_ips: Vec<TopSecurityIpDto>,
    pub probing_paths: Vec<ProbingPathStatDto>,
    pub brute_force_incidents: Vec<BruteForceIncidentDto>,
    pub active_banned_ips_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UnbanIpResponse {
    pub success: bool,
    pub message: String,
    pub unbanned_count: u64,
}

// ==========================================
// Helper Utilities
// ==========================================

/// Validates whether a given string is a valid IPv4/IPv6 address or CIDR notation.
pub fn is_valid_ip_or_cidr(val: &str) -> bool {
    let val = val.trim();
    if val.is_empty() {
        return false;
    }
    if val.parse::<IpAddr>().is_ok() {
        return true;
    }
    if let Some((ip_part, mask_part)) = val.split_once('/') {
        return match (ip_part.parse::<IpAddr>(), mask_part.parse::<u8>()) {
            (Ok(IpAddr::V4(_)), Ok(mask)) => mask <= 32,
            (Ok(IpAddr::V6(_)), Ok(mask)) => mask <= 128,
            _ => false,
        };
    }
    false
}

// ==========================================
// API Handlers
// ==========================================

/// GET /api/v1/sites/{id}/analytics/security/overview
/// Retrieves security metrics summary including top suspicious IPs, probing paths, and brute force incidents.
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/security/overview",
    params(
        ("id" = i64, Path, description = "Site ID"),
        SecurityOverviewQuery
    ),
    responses(
        (status = 200, description = "Security analytics overview", body = SecurityOverviewResponse),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn get_site_security_overview_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<SecurityOverviewQuery>,
) -> Response {
    let (_site, _user, _domains) = match verify_site_access(&state.pool, &auth_user, site_id).await
    {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let now = Utc::now().timestamp();
    let from_ts = query.from.unwrap_or(now - 7 * 86400);
    let to_ts = query.to.unwrap_or(now);
    let limit = query.limit.unwrap_or(10).clamp(1, 100);

    let banned_repo = SqliteSiteBannedIpRepository::new(state.pool.clone());
    let active_bans = banned_repo
        .list_active_by_site(site_id, now)
        .await
        .unwrap_or_default();

    let storage_guard = state.analytics_storage.read().await;
    let summary = if let Some(storage) = storage_guard.as_ref() {
        match storage
            .query_security_overview(site_id, from_ts, to_ts, limit)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "Failed to query security overview from storage: {}, returning empty overview",
                    e
                );
                panel_analytics::SecurityOverviewSummary::default()
            }
        }
    } else {
        panel_analytics::SecurityOverviewSummary::default()
    };

    let top_ips = summary
        .top_ips
        .into_iter()
        .map(|ip| TopSecurityIpDto {
            ip: ip.ip,
            total_requests: ip.total_requests,
            error_4xx: ip.error_4xx,
            error_5xx: ip.error_5xx,
            probing_count: ip.probing_count,
            auth_failures: ip.auth_failures,
            country: ip.country,
            top_path: ip.top_path,
            risk_level: ip.risk_level.as_str().to_string(),
            last_seen: ip.last_seen,
        })
        .collect();

    let probing_paths = summary
        .probing_paths
        .into_iter()
        .map(|p| ProbingPathStatDto {
            path: p.path,
            category: p.category,
            count: p.count,
            unique_ips: p.unique_ips,
        })
        .collect();

    let brute_force_incidents = summary
        .brute_force_incidents
        .into_iter()
        .map(|bf| BruteForceIncidentDto {
            ip: bf.ip,
            target_path: bf.target_path,
            failed_attempts: bf.failed_attempts,
            first_attempt_ts: bf.first_attempt_ts,
            last_attempt_ts: bf.last_attempt_ts,
            country: bf.country,
            risk_score: bf.risk_score,
        })
        .collect();

    Json(SecurityOverviewResponse {
        site_id,
        from: from_ts,
        to: to_ts,
        total_probing_requests: summary.total_probing_requests,
        total_brute_force_attempts: summary.total_brute_force_attempts,
        unique_suspicious_ips: summary.unique_suspicious_ips,
        top_ips,
        probing_paths,
        brute_force_incidents,
        active_banned_ips_count: active_bans.len(),
    })
    .into_response()
}

/// GET /api/v1/sites/{id}/security/banned-ips
/// Retrieves the list of currently active banned IP addresses with remaining TTL countdown.
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/security/banned-ips",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "List of active banned IPs", body = Vec<BannedIpDto>),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "security"
)]
pub async fn get_site_banned_ips_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    let (site, user, domains) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let now = Utc::now().timestamp();
    let banned_repo = SqliteSiteBannedIpRepository::new(state.pool.clone());

    // 1. Deactivate expired bans
    let expired = banned_repo
        .deactivate_expired(now)
        .await
        .unwrap_or_default();
    if !expired.is_empty() {
        // Synchronize with site rules to remove expired deny rules from Nginx
        let mut settings: serde_json::Value =
            serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
        let mut rules = settings
            .get("site_rules")
            .and_then(|v| serde_json::from_value::<SiteRulesConfig>(v.clone()).ok())
            .unwrap_or_default();

        let expired_ips: Vec<&str> = expired.iter().map(|e| e.ip_address.as_str()).collect();
        let prev_len = rules.ip_access.rules.len();
        rules
            .ip_access
            .rules
            .retain(|r| !expired_ips.contains(&r.cidr_or_ip.as_str()));

        if rules.ip_access.rules.len() != prev_len {
            match serde_json::to_value(&rules) {
                Ok(rules_val) => {
                    settings["site_rules"] = rules_val;
                    if let Ok(new_json) = serde_json::to_string(&settings) {
                        let site_repo = SqliteSiteRepository::new(state.pool.clone());
                        let update_input = UpdateSite {
                            kind: None,
                            docroot: None,
                            runtime: None,
                            version: None,
                            status: None,
                            settings_json: Some(new_json),
                        };
                        if let Ok(updated_site) = site_repo.update(site.id, &update_input).await {
                            let _ = recompile_and_apply_site_vhost(
                                &state,
                                &updated_site,
                                &user,
                                &domains,
                            )
                            .await;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to serialize site rules on ban sync: {}", e);
                }
            }
        }
    }

    // 2. Fetch active bans
    let active = match banned_repo.list_active_by_site(site_id, now).await {
        Ok(list) => list,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal memuat daftar IP terblokir: {}", e) })),
            )
                .into_response();
        }
    };

    let dtos: Vec<BannedIpDto> = active
        .into_iter()
        .map(|m| BannedIpDto::from((m, now)))
        .collect();
    Json(dtos).into_response()
}

/// POST /api/v1/sites/{id}/security/ban-ip
/// Blocks an IP address with optional TTL, updates database records, and applies Nginx deny rule atomically.
#[utoipa::path(
    post,
    path = "/api/v1/sites/{id}/security/ban-ip",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    request_body = BanIpRequest,
    responses(
        (status = 200, description = "IP banned successfully", body = BannedIpDto),
        (status = 400, description = "Invalid IP format or TTL"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "security"
)]
pub async fn ban_ip_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<BanIpRequest>,
) -> Response {
    let (site, user, domains) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let ip_clean = payload.ip.trim();
    if !is_valid_ip_or_cidr(ip_clean) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Format alamat IP atau CIDR tidak valid (contoh: 192.168.1.1 atau 10.0.0.0/24)"
            })),
        )
            .into_response();
    }

    if payload.ttl_secs.is_some_and(|ttl| ttl <= 0) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Nilai TTL harus lebih besar dari 0 detik" })),
        )
            .into_response();
    }

    let now = Utc::now().timestamp();
    let expires_at = payload.ttl_secs.map(|ttl| now + ttl);
    let reason = payload
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Diblokir manual via Security Analytics");

    let banned_repo = SqliteSiteBannedIpRepository::new(state.pool.clone());
    let create_input = CreateSiteBannedIpInput {
        id: Some(Uuid::new_v4().to_string()),
        site_id,
        ip_address: ip_clean.to_string(),
        reason: reason.to_string(),
        banned_at: now,
        expires_at,
        banned_by: auth_user.username.clone(),
    };

    let created_record = match banned_repo.create(&create_input).await {
        Ok(rec) => rec,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal menyimpan catatan pemblokiran: {}", e) })),
            )
                .into_response();
        }
    };

    // Synchronize deny rule to Nginx configuration
    let mut settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
    let mut rules = settings
        .get("site_rules")
        .and_then(|v| serde_json::from_value::<SiteRulesConfig>(v.clone()).ok())
        .unwrap_or_default();

    // Enable IP access module and upsert deny rule
    rules.ip_access.enabled = true;
    rules.ip_access.rules.retain(|r| r.cidr_or_ip != ip_clean);
    rules.ip_access.rules.push(IpAccessRule {
        id: Uuid::new_v4().to_string(),
        action: IpAction::Deny,
        cidr_or_ip: ip_clean.to_string(),
        description: Some(format!("Banned: {}", reason)),
    });

    let rules_val = match serde_json::to_value(&rules) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal serialisasi aturan situs: {}", e) })),
            )
                .into_response();
        }
    };
    settings["site_rules"] = rules_val;

    let new_settings_json = match serde_json::to_string(&settings) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal menyimpan pengaturan: {}", e) })),
            )
                .into_response();
        }
    };

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let update_input = UpdateSite {
        kind: None,
        docroot: None,
        runtime: None,
        version: None,
        status: None,
        settings_json: Some(new_settings_json),
    };

    let updated_site = match site_repo.update(site.id, &update_input).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal memperbarui situs di basis data: {}", e) })),
            )
                .into_response();
        }
    };

    // Apply atomically via privileged agent (syntax tested via nginx -t)
    if let Err((status, msg)) =
        recompile_and_apply_site_vhost(&state, &updated_site, &user, &domains).await
    {
        // Rollback DB record if agent application fails
        let rollback_input = UpdateSite {
            kind: None,
            docroot: None,
            runtime: None,
            version: None,
            status: None,
            settings_json: Some(site.settings_json.clone()),
        };
        let _ = site_repo.update(site.id, &rollback_input).await;
        let _ = banned_repo.unban(site_id, ip_clean).await;

        return (
            status,
            Json(serde_json::json!({
                "error": format!("Gagal menerapkan konfigurasi Nginx pemblokiran IP: {}", msg)
            })),
        )
            .into_response();
    }

    Json(BannedIpDto::from((created_record, now))).into_response()
}

/// DELETE /api/v1/sites/{id}/security/ban-ip/{ip}
/// Unbans an IP address, removes deny rule from Nginx, and recompiles vhost configuration atomically.
#[utoipa::path(
    delete,
    path = "/api/v1/sites/{id}/security/ban-ip/{ip}",
    params(
        ("id" = i64, Path, description = "Site ID"),
        ("ip" = String, Path, description = "IP address or CIDR to unban")
    ),
    responses(
        (status = 200, description = "IP unbanned successfully", body = UnbanIpResponse),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "security"
)]
pub async fn unban_ip_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, ip_to_unban)): Path<(i64, String)>,
) -> Response {
    let (site, user, domains) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let ip_clean = ip_to_unban.trim();
    let banned_repo = SqliteSiteBannedIpRepository::new(state.pool.clone());
    let unbanned_count = match banned_repo.unban(site_id, ip_clean).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal membuka blokir IP di basis data: {}", e) })),
            )
                .into_response();
        }
    };

    // Remove deny rule from Nginx configuration
    let mut settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
    let mut rules = settings
        .get("site_rules")
        .and_then(|v| serde_json::from_value::<SiteRulesConfig>(v.clone()).ok())
        .unwrap_or_default();

    let prev_len = rules.ip_access.rules.len();
    rules.ip_access.rules.retain(|r| r.cidr_or_ip != ip_clean);

    if rules.ip_access.rules.len() != prev_len {
        let rules_val = match serde_json::to_value(&rules) {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Gagal serialisasi aturan situs: {}", e) })),
                )
                    .into_response();
            }
        };
        settings["site_rules"] = rules_val;

        let new_settings_json = match serde_json::to_string(&settings) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Gagal menyimpan pengaturan: {}", e) })),
                )
                    .into_response();
            }
        };

        let site_repo = SqliteSiteRepository::new(state.pool.clone());
        let update_input = UpdateSite {
            kind: None,
            docroot: None,
            runtime: None,
            version: None,
            status: None,
            settings_json: Some(new_settings_json),
        };

        let updated_site = match site_repo.update(site.id, &update_input).await {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Gagal memperbarui situs: {}", e) })),
                )
                    .into_response();
            }
        };

        if let Err((status, msg)) =
            recompile_and_apply_site_vhost(&state, &updated_site, &user, &domains).await
        {
            return (
                status,
                Json(serde_json::json!({
                    "error": format!("Gagal menerapkan konfigurasi Nginx setelah buka blokir: {}", msg)
                })),
            )
                .into_response();
        }
    }

    Json(UnbanIpResponse {
        success: true,
        message: format!(
            "Alamat IP '{}' berhasil dibuka dari daftar blokir",
            ip_clean
        ),
        unbanned_count,
    })
    .into_response()
}
