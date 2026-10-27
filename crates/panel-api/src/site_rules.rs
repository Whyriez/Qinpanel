//! API endpoints and handlers for Site Rules (Redirects, Headers, CORS, Hotlink, Basic Auth, IP Access, Maintenance, Custom Error Pages, Nginx Snippets).

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;
use crate::sites;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_core::site_rules::{SiteRulesConfig, validate_nginx_snippet, validate_site_rules};
use panel_db::models::{CreateAuditLog, UpdateSite};
use panel_db::{
    SqliteAuditLogRepository, SqliteDomainRepository, SqliteSiteRepository, SqliteUserRepository,
};
use panel_proto::{SiteHtpasswdDeployRequest, SiteHtpasswdRemoveRequest};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct HashPasswordRequest {
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct HashPasswordResponse {
    pub hash: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ValidateSnippetRequest {
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ValidateSnippetResponse {
    pub valid: bool,
    pub message: Option<String>,
    pub error: Option<String>,
}

/// Helper to verify user permissions for a given site.
pub async fn verify_site_access(
    pool: &panel_db::DbPool,
    auth_user: &AuthenticatedUser,
    site_id: i64,
) -> Result<
    (
        panel_db::models::Site,
        panel_db::models::User,
        Vec<panel_db::models::Domain>,
    ),
    (StatusCode, String),
> {
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let user_repo = SqliteUserRepository::new(pool.clone());
    let domain_repo = SqliteDomainRepository::new(pool.clone());

    let site = site_repo
        .find_by_id(site_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Situs tidak ditemukan".to_string()))?;

    if !auth_user.is_admin() && site.user_id != auth_user.id {
        return Err((
            StatusCode::FORBIDDEN,
            "Anda tidak memiliki izin untuk mengelola aturan situs ini".to_string(),
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

    let domains = domain_repo
        .find_by_site_id(site.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((site, user, domains))
}

/// GET /api/v1/sites/{id}/rules
/// Retrieves the current site rules configuration from site settings.
pub async fn get_site_rules_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    let (site, _, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
    let rules = settings
        .get("site_rules")
        .and_then(|v| serde_json::from_value::<SiteRulesConfig>(v.clone()).ok())
        .unwrap_or_default();

    Json(rules).into_response()
}

/// PUT /api/v1/sites/{id}/rules
/// Updates the site rules configuration, validates all parameters, deploys basic auth htpasswd if needed,
/// compiles the Nginx configuration, and atomically applies it with syntax test and rollback.
pub async fn update_site_rules_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<SiteRulesConfig>,
) -> Response {
    let (site, user, domains) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    // 1. Strict validation of all site rules
    if let Err(err) = validate_site_rules(&payload) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Validasi aturan situs gagal: {}", err) })),
        )
            .into_response();
    }

    // 2. Admin-only check for custom Nginx snippet
    if (payload.nginx_snippet.enabled || !payload.nginx_snippet.content.trim().is_empty())
        && !auth_user.is_admin()
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Hanya administrator yang diizinkan untuk mengonfigurasi snippet Nginx kustom"
            })),
        )
            .into_response();
    }

    // 3. Deploy or remove htpasswd file for basic auth
    let htpasswd_res = if payload.basic_auth.enabled && !payload.basic_auth.users.is_empty() {
        let mut htpasswd_lines = Vec::new();
        for u in &payload.basic_auth.users {
            if u.username.contains(':') || u.username.trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Nama pengguna basic auth tidak valid: '{}'", u.username)
                    })),
                )
                    .into_response();
            }
            if u.password_hash.trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Hash kata sandi untuk pengguna '{}' tidak boleh kosong", u.username)
                    })),
                )
                    .into_response();
            }
            htpasswd_lines.push(format!("{}:{}", u.username.trim(), u.password_hash.trim()));
        }
        let htpasswd_content = format!("{}\n", htpasswd_lines.join("\n"));

        state
            .agent_client
            .deploy_site_htpasswd(SiteHtpasswdDeployRequest {
                site_id: format!("site_{}", site.id),
                htpasswd_content,
                auth_base_dir: None,
            })
            .await
    } else {
        // Clean up any existing htpasswd file if disabled or empty
        let _ = state
            .agent_client
            .remove_site_htpasswd(SiteHtpasswdRemoveRequest {
                site_id: format!("site_{}", site.id),
                auth_base_dir: None,
            })
            .await;
        Ok(panel_proto::SiteHtpasswdDeployedResponse {
            site_id: format!("site_{}", site.id),
            htpasswd_path: String::new(),
        })
    };

    if let Err(e) = htpasswd_res {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal memasang berkas autentikasi htpasswd: {}", e)
            })),
        )
            .into_response();
    }

    // 4. Update site settings in DB
    let mut settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
    let rules_val = match serde_json::to_value(&payload) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal serialisasi aturan: {}", e) })),
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
                Json(serde_json::json!({ "error": format!("Gagal memformat pengaturan: {}", e) })),
            )
                .into_response();
        }
    };

    let update_input = UpdateSite {
        kind: None,
        docroot: None,
        runtime: None,
        version: None,
        status: None,
        settings_json: Some(new_settings_json.clone()),
    };

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let updated_site = match site_repo.update(site.id, &update_input).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal menyimpan pengaturan ke database: {}", e) })),
            )
                .into_response();
        }
    };

    // 5. Atomic recompilation and apply via privileged agent (syntax tested via nginx -t)
    if let Err((status, msg)) =
        sites::recompile_and_apply_site_vhost(&state, &updated_site, &user, &domains).await
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
        return (
            status,
            Json(serde_json::json!({
                "error": format!("Gagal menerapkan konfigurasi web server Nginx: {}", msg)
            })),
        )
            .into_response();
    }

    // 6. Record audit log
    let audit_repo = SqliteAuditLogRepository::new(state.operations_pool.clone());
    let _ = audit_repo
        .create(&CreateAuditLog {
            user_id: Some(auth_user.id),
            actor_username: auth_user.username.clone(),
            action: "site.rules_updated".to_string(),
            target_resource: format!("site:{}", site_id),
            details_json: Some(
                serde_json::json!({
                    "site_id": site_id,
                    "redirects_count": payload.redirects.len(),
                    "custom_headers_count": payload.custom_headers.len(),
                    "basic_auth_enabled": payload.basic_auth.enabled,
                    "maintenance_enabled": payload.maintenance.enabled,
                    "nginx_snippet_enabled": payload.nginx_snippet.enabled,
                })
                .to_string(),
            ),
            ip_address: None,
            status: "success".to_string(),
        })
        .await;

    Json(payload).into_response()
}

/// POST /api/v1/sites/{id}/rules/hash-password
/// Helper endpoint to securely hash a password using bcrypt for Basic Auth htpasswd.
pub async fn hash_basic_auth_password_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<HashPasswordRequest>,
) -> Response {
    if let Err((code, msg)) = verify_site_access(&state.pool, &auth_user, site_id).await {
        return (code, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    let password = payload.password.trim();
    if password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Kata sandi tidak boleh kosong" })),
        )
            .into_response();
    }
    if password.len() > 128 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Kata sandi melebihi batas maksimal 128 karakter" })),
        )
            .into_response();
    }

    match panel_core::auth::hash_basic_auth_password(password) {
        Ok(hash) => Json(HashPasswordResponse { hash }).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal menghasilkan hash kata sandi: {}", e) })),
        )
            .into_response(),
    }
}

/// POST /api/v1/sites/{id}/rules/validate-snippet
/// Admin-only dry run to validate Nginx directives against the strict security denylist.
pub async fn validate_snippet_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<ValidateSnippetRequest>,
) -> Response {
    if let Err((code, msg)) = verify_site_access(&state.pool, &auth_user, site_id).await {
        return (code, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Hanya administrator yang diizinkan untuk memvalidasi snippet Nginx"
            })),
        )
            .into_response();
    }

    match validate_nginx_snippet(&payload.content) {
        Ok(_) => Json(ValidateSnippetResponse {
            valid: true,
            message: Some("Snippet Nginx valid dan memenuhi kriteria keamanan".to_string()),
            error: None,
        })
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ValidateSnippetResponse {
                valid: false,
                message: None,
                error: Some(e.to_string()),
            }),
        )
            .into_response(),
    }
}
