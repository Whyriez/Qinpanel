//! Encrypted environment variables management and `.env` parsing.
//!
//! Complies with PRD Section 6 & Phase 8 (P8-T1):
//! - Encryption/decryption strictly handled inside the agent.
//! - Mask / reveal support (default masked `••••••••`).
//! - `.env` file importing with comments, quotes, multiline values, and export prefixes.
//! - Restart required banner/flag tracking and reset on process restart.
//! - Plaintext secrets never stored in `panel.db`, audit logs, or system logs.

use crate::AppState;
use crate::auth::AuthenticatedUser;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_db::repositories::site_repo::SqliteSiteRepository;
use panel_proto::{
    NodeSiteActionRequest, NodeSiteActionType, PythonSiteActionRequest, PythonSiteActionType,
    SiteEnvApplyRequest, SiteEnvDecryptRequest, SiteEnvEncryptRequest,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use utoipa::ToSchema;

pub const MASKED_PLACEHOLDER: &str = "••••••••";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct EnvVariableDto {
    pub key: String,
    pub value: String,
    pub is_masked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteEnvListResponse {
    pub site_id: i64,
    pub variables: Vec<EnvVariableDto>,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSiteEnvRequest {
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportDotEnvRequest {
    pub content: String,
    pub merge: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteEnvActionResponse {
    pub success: bool,
    pub message: String,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SiteEnvQuery {
    pub reveal: Option<bool>,
}

/// Validates POSIX environment variable name.
pub fn validate_env_key(key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("Nama environment variable tidak boleh kosong".to_string());
    }
    let first = trimmed.chars().next().unwrap_or('_');
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(format!(
            "Nama variabel '{}' harus diawali dengan huruf atau underscore",
            trimmed
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!(
            "Nama variabel '{}' hanya boleh mengandung huruf, angka, dan underscore",
            trimmed
        ));
    }
    Ok(())
}

/// Parses `.env` format content into a map of key-value pairs.
/// Supports comments (`#`), `export ` prefixes, single quotes, double quotes with escapes,
/// and multiline values.
pub fn parse_dotenv(content: &str) -> Result<BTreeMap<String, String>, String> {
    let mut result = BTreeMap::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();
        i += 1;

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Strip optional `export `
        let line_clean = if let Some(stripped) = line.strip_prefix("export ") {
            stripped.trim_start()
        } else {
            line
        };

        if line_clean.starts_with('#') {
            continue;
        }

        let eq_idx = match line_clean.find('=') {
            Some(idx) => idx,
            None => {
                return Err(format!(
                    "Format .env tidak valid pada baris: '{}' (tidak ada '=')",
                    line_clean
                ));
            }
        };

        let key = line_clean[..eq_idx].trim();
        validate_env_key(key)?;

        let val_part = line_clean[eq_idx + 1..].trim();

        if let Some(stripped) = val_part.strip_prefix('\'') {
            // Single-quoted literal
            let mut val = String::new();
            if let Some(closing_idx) = stripped.find('\'') {
                val.push_str(&stripped[..closing_idx]);
            } else {
                // Multiline single-quote
                val.push_str(stripped);
                let mut found_closing = false;
                while i < lines.len() {
                    let next_line = lines[i];
                    i += 1;
                    if let Some(c_idx) = next_line.find('\'') {
                        val.push('\n');
                        val.push_str(&next_line[..c_idx]);
                        found_closing = true;
                        break;
                    } else {
                        val.push('\n');
                        val.push_str(next_line);
                    }
                }
                if !found_closing {
                    return Err(format!(
                        "Tanda petik tunggal penutup tidak ditemukan untuk key '{}'",
                        key
                    ));
                }
            }
            result.insert(key.to_string(), val);
        } else if let Some(stripped) = val_part.strip_prefix('"') {
            // Double-quoted string with escapes
            let mut raw_val = String::new();
            let mut found_closing = false;

            // Check if closing quote is on this line (not escaped)
            let mut is_escaped = false;
            let mut closing_pos = None;
            for (idx, ch) in stripped.char_indices() {
                if is_escaped {
                    is_escaped = false;
                } else if ch == '\\' {
                    is_escaped = true;
                } else if ch == '"' {
                    closing_pos = Some(idx);
                    break;
                }
            }

            if let Some(pos) = closing_pos {
                raw_val.push_str(&stripped[..pos]);
                found_closing = true;
            } else {
                // Multiline double-quote
                raw_val.push_str(stripped);
                while i < lines.len() {
                    let next_line = lines[i];
                    i += 1;
                    let mut line_closing = None;
                    let mut esc = false;
                    for (idx, ch) in next_line.char_indices() {
                        if esc {
                            esc = false;
                        } else if ch == '\\' {
                            esc = true;
                        } else if ch == '"' {
                            line_closing = Some(idx);
                            break;
                        }
                    }
                    if let Some(pos) = line_closing {
                        raw_val.push('\n');
                        raw_val.push_str(&next_line[..pos]);
                        found_closing = true;
                        break;
                    } else {
                        raw_val.push('\n');
                        raw_val.push_str(next_line);
                    }
                }
            }

            if !found_closing {
                return Err(format!(
                    "Tanda petik ganda penutup tidak ditemukan untuk key '{}'",
                    key
                ));
            }

            // Interpret escape sequences: \n, \t, \r, \", \\
            let unescaped = unescape_double_quoted(&raw_val);
            result.insert(key.to_string(), unescaped);
        } else {
            // Unquoted: strip inline comments if separated by whitespace
            let mut val_tokens = val_part;
            if let Some(comment_pos) = val_part
                .find('#')
                .filter(|&pos| pos == 0 || val_part[..pos].ends_with(|c: char| c.is_whitespace()))
            {
                val_tokens = val_part[..comment_pos].trim_end();
            }
            result.insert(key.to_string(), val_tokens.to_string());
        }
    }

    Ok(result)
}

fn unescape_double_quoted(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('$') => out.push('$'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

async fn verify_site_access(
    pool: &SqlitePool,
    auth_user: &AuthenticatedUser,
    site_id: i64,
) -> Result<(panel_db::models::Site, panel_db::models::User), (StatusCode, String)> {
    let site_repo = SqliteSiteRepository::new(pool.clone());
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

/// GET /api/v1/sites/{id}/env
/// Lists site environment variables (masked by default, or revealed if query ?reveal=true).
pub async fn get_site_env_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<SiteEnvQuery>,
) -> Response {
    let (site, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let settings: serde_json::Value = serde_json::from_str(&site.settings_json).unwrap_or_default();

    let restart_required = settings
        .get("env_restart_required")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Read encrypted_env_vars map from settings_json (fallback to env_vars)
    let mut encrypted_map: BTreeMap<String, String> = BTreeMap::new();
    if let Some(obj) = settings
        .get("encrypted_env_vars")
        .or_else(|| settings.get("env_vars"))
        .and_then(|v| v.as_object())
    {
        for (k, v) in obj {
            if let Some(val_str) = v.as_str() {
                encrypted_map.insert(k.clone(), val_str.to_string());
            }
        }
    }

    let reveal = query.reveal.unwrap_or(false);

    let variables: Vec<EnvVariableDto> = if !reveal {
        // Return masked values
        encrypted_map
            .into_keys()
            .map(|k| EnvVariableDto {
                key: k,
                value: MASKED_PLACEHOLDER.to_string(),
                is_masked: true,
            })
            .collect()
    } else {
        // Query agent to decrypt values
        let dec_req = SiteEnvDecryptRequest {
            site_id: format!("site_{}", site.id),
            encrypted_vars: encrypted_map,
        };
        match state.agent_client.decrypt_site_env(dec_req).await {
            Ok(dec_res) => dec_res
                .env_vars
                .into_iter()
                .map(|(k, v)| EnvVariableDto {
                    key: k,
                    value: v,
                    is_masked: false,
                })
                .collect(),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Dekripsi variabel lingkungan via agent gagal: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    };

    (
        StatusCode::OK,
        Json(SiteEnvListResponse {
            site_id: site.id,
            variables,
            restart_required,
        }),
    )
        .into_response()
}

/// PUT /api/v1/sites/{id}/env
/// Updates environment variables: encrypts via agent, saves encrypted values to DB,
/// marks restart_required = true, and applies decrypted env file on host.
pub async fn update_site_env_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<UpdateSiteEnvRequest>,
) -> Response {
    let (mut site, tenant_user) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    // Validate all keys
    for k in payload.variables.keys() {
        if let Err(err_msg) = validate_env_key(k) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": err_msg })),
            )
                .into_response();
        }
    }

    // Call agent to encrypt variables
    let enc_req = SiteEnvEncryptRequest {
        site_id: format!("site_{}", site.id),
        env_vars: payload.variables.clone(),
    };
    let enc_res = match state.agent_client.encrypt_site_env(enc_req).await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Enkripsi variabel lingkungan via agent gagal: {}", e)
                })),
            )
                .into_response();
        }
    };

    // Update settings_json in DB with encrypted vars & mark restart_required = true
    let mut settings: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&site.settings_json).unwrap_or_default();
    settings.insert(
        "encrypted_env_vars".to_string(),
        serde_json::json!(enc_res.encrypted_vars),
    );
    settings.insert("env_restart_required".to_string(), serde_json::json!(true));

    let updated_settings_json = serde_json::to_string(&settings).unwrap_or_default();
    site.settings_json = updated_settings_json.clone();

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let update_dto = panel_db::models::UpdateSite {
        kind: None,
        docroot: None,
        runtime: site.runtime.clone(),
        version: site.version.clone(),
        status: None,
        settings_json: Some(updated_settings_json),
    };

    if let Err(e) = site_repo.update(site.id, &update_dto).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal menyimpan variabel lingkungan ke database: {}", e)
            })),
        )
            .into_response();
    }

    // Apply env file to host via agent
    let (linux_username, _linux_uid) = match crate::tenant_context::execution_identity(&tenant_user)
    {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error": message}))).into_response();
        }
    };
    let app_req = SiteEnvApplyRequest {
        site_id: format!("site_{}", site.id),
        username: linux_username.clone(),
        env_file_path: None,
        encrypted_vars: enc_res.encrypted_vars,
    };
    if let Err(err) = state.agent_client.apply_site_env(app_req).await {
        tracing::error!(error=%err, "Apply tenant environment failed");
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error":"Environment tersimpan, tetapi gagal diterapkan pada runtime. Coba terapkan kembali."}))).into_response();
    }

    // Audit log (masked keys only, NO plaintext values)
    let keys_list: Vec<&String> = payload.variables.keys().collect();
    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.env_update",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!(
            "Memperbarui {} variabel lingkungan ({:?})",
            keys_list.len(),
            keys_list
        )),
    );

    (
        StatusCode::OK,
        Json(SiteEnvActionResponse {
            success: true,
            message: "Variabel lingkungan terenkripsi berhasil disimpan. Restart aplikasi untuk menerapkan perubahan.".to_string(),
            restart_required: true,
        }),
    )
        .into_response()
}

/// POST /api/v1/sites/{id}/env/import
/// Parses a `.env` file content and saves the resulting variables (encrypted via agent).
pub async fn import_site_env_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<ImportDotEnvRequest>,
) -> Response {
    let (mut site, tenant_user) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let parsed_vars = match parse_dotenv(&payload.content) {
        Ok(vars) => vars,
        Err(err_msg) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": err_msg })),
            )
                .into_response();
        }
    };

    if parsed_vars.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Konten .env kosong atau tidak ada variabel valid yang ditemukan"
            })),
        )
            .into_response();
    }

    let mut target_vars = BTreeMap::new();
    let merge = payload.merge.unwrap_or(false);

    if merge {
        let settings: serde_json::Value =
            serde_json::from_str(&site.settings_json).unwrap_or_default();
        if let Some(obj) = settings
            .get("encrypted_env_vars")
            .or_else(|| settings.get("env_vars"))
            .and_then(|v| v.as_object())
        {
            let mut existing_enc = BTreeMap::new();
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    existing_enc.insert(k.clone(), s.to_string());
                }
            }
            // Decrypt existing
            let dec_req = SiteEnvDecryptRequest {
                site_id: format!("site_{}", site.id),
                encrypted_vars: existing_enc,
            };
            if let Ok(dec_res) = state.agent_client.decrypt_site_env(dec_req).await {
                target_vars = dec_res.env_vars;
            }
        }
    }

    // Merge or replace
    for (k, v) in parsed_vars {
        target_vars.insert(k, v);
    }

    // Encrypt via agent
    let enc_req = SiteEnvEncryptRequest {
        site_id: format!("site_{}", site.id),
        env_vars: target_vars.clone(),
    };
    let enc_res = match state.agent_client.encrypt_site_env(enc_req).await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Enkripsi variabel lingkungan via agent gagal: {}", e)
                })),
            )
                .into_response();
        }
    };

    // Save to DB
    let mut settings: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&site.settings_json).unwrap_or_default();
    settings.insert(
        "encrypted_env_vars".to_string(),
        serde_json::json!(enc_res.encrypted_vars),
    );
    settings.insert("env_restart_required".to_string(), serde_json::json!(true));

    let updated_settings_json = serde_json::to_string(&settings).unwrap_or_default();
    site.settings_json = updated_settings_json.clone();

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let update_dto = panel_db::models::UpdateSite {
        kind: None,
        docroot: None,
        runtime: site.runtime.clone(),
        version: site.version.clone(),
        status: None,
        settings_json: Some(updated_settings_json),
    };

    if let Err(e) = site_repo.update(site.id, &update_dto).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Gagal menyimpan variabel lingkungan ke database: {}", e)
            })),
        )
            .into_response();
    }

    // Apply on host
    let (linux_username, _linux_uid) = match crate::tenant_context::execution_identity(&tenant_user)
    {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error": message}))).into_response();
        }
    };
    let app_req = SiteEnvApplyRequest {
        site_id: format!("site_{}", site.id),
        username: linux_username.clone(),
        env_file_path: None,
        encrypted_vars: enc_res.encrypted_vars,
    };
    if let Err(err) = state.agent_client.apply_site_env(app_req).await {
        tracing::error!(error=%err, "Apply tenant environment failed");
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error":"Environment tersimpan, tetapi gagal diterapkan pada runtime. Coba terapkan kembali."}))).into_response();
    }

    // Audit log (count only, NO plaintext values)
    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.env_import",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!(
            "Import {} variabel dari .env (merge={})",
            target_vars.len(),
            merge
        )),
    );

    (
        StatusCode::OK,
        Json(SiteEnvActionResponse {
            success: true,
            message: format!(
                "Berhasil mengimpor {} variabel lingkungan. Perlu restart agar berlaku.",
                target_vars.len()
            ),
            restart_required: true,
        }),
    )
        .into_response()
}

/// POST /api/v1/sites/{id}/env/restart
/// Restarts the application service and clears the restart_required flag.
pub async fn restart_site_env_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    let (mut site, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    // Perform runtime restart
    let runtime_str = site.runtime.clone().unwrap_or_else(|| site.kind.clone());

    if runtime_str == "nodejs" {
        let act_req = NodeSiteActionRequest {
            site_id: format!("site_{}", site.id),
            action: NodeSiteActionType::Restart,
        };
        let _ = state.agent_client.action_node_site(act_req).await;
    } else if runtime_str == "python" {
        let act_req = PythonSiteActionRequest {
            site_id: format!("site_{}", site.id),
            action: PythonSiteActionType::Restart,
        };
        let _ = state.agent_client.action_python_site(act_req).await;
    } else {
        // PHP / static / proxy: re-apply baseline or reload nginx
        let _ = crate::sites::apply_site_config(&state, &site).await;
    }

    // Clear env_restart_required = false in DB
    let mut settings: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&site.settings_json).unwrap_or_default();
    settings.insert("env_restart_required".to_string(), serde_json::json!(false));

    let updated_settings_json = serde_json::to_string(&settings).unwrap_or_default();
    site.settings_json = updated_settings_json.clone();

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let update_dto = panel_db::models::UpdateSite {
        kind: None,
        docroot: None,
        runtime: site.runtime.clone(),
        version: site.version.clone(),
        status: None,
        settings_json: Some(updated_settings_json),
    };
    let _ = site_repo.update(site.id, &update_dto).await;

    // Audit log
    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.env_restart",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!(
            "Restart situs {} untuk menerapkan variabel lingkungan",
            site.id
        )),
    );

    (
        StatusCode::OK,
        Json(SiteEnvActionResponse {
            success: true,
            message: "Situs berhasil dimulai ulang dan variabel lingkungan aktif".to_string(),
            restart_required: false,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dotenv_basic() {
        let raw = r#"
# Database Configuration
DB_HOST=127.0.0.1
DB_PORT=5432
DB_USER=postgres
DB_PASS=secret123

# App Settings
export APP_NAME=VpanelApp
PORT=3000
DEBUG=true
"#;
        let vars = parse_dotenv(raw).expect("parse dotenv");
        assert_eq!(vars.get("DB_HOST").map(|s| s.as_str()), Some("127.0.0.1"));
        assert_eq!(vars.get("DB_PORT").map(|s| s.as_str()), Some("5432"));
        assert_eq!(vars.get("DB_USER").map(|s| s.as_str()), Some("postgres"));
        assert_eq!(vars.get("DB_PASS").map(|s| s.as_str()), Some("secret123"));
        assert_eq!(vars.get("APP_NAME").map(|s| s.as_str()), Some("VpanelApp"));
        assert_eq!(vars.get("PORT").map(|s| s.as_str()), Some("3000"));
        assert_eq!(vars.get("DEBUG").map(|s| s.as_str()), Some("true"));
    }

    #[test]
    fn test_parse_dotenv_quotes_and_escapes() {
        let raw = r#"
SINGLE_QUOTED='hello $world with "quotes"'
DOUBLE_QUOTED="first line\nsecond line\twith tab"
WITH_COMMENT=my_value # this is an inline comment
"#;
        let vars = parse_dotenv(raw).expect("parse quotes");
        assert_eq!(
            vars.get("SINGLE_QUOTED").map(|s| s.as_str()),
            Some("hello $world with \"quotes\"")
        );
        assert_eq!(
            vars.get("DOUBLE_QUOTED").map(|s| s.as_str()),
            Some("first line\nsecond line\twith tab")
        );
        assert_eq!(
            vars.get("WITH_COMMENT").map(|s| s.as_str()),
            Some("my_value")
        );
    }

    #[test]
    fn test_parse_dotenv_multiline_double_quotes() {
        let raw = "CERT_KEY=\"-----BEGIN PRIVATE KEY-----\nABCDEF123456\n-----END PRIVATE KEY-----\"\nANOTHER_KEY=val";
        let vars = parse_dotenv(raw).expect("multiline quotes");
        assert_eq!(
            vars.get("CERT_KEY").map(|s| s.as_str()),
            Some("-----BEGIN PRIVATE KEY-----\nABCDEF123456\n-----END PRIVATE KEY-----")
        );
        assert_eq!(vars.get("ANOTHER_KEY").map(|s| s.as_str()), Some("val"));
    }

    #[test]
    fn test_parse_dotenv_invalid_key_or_syntax() {
        assert!(parse_dotenv("INVALID NO EQUALS").is_err());
        assert!(parse_dotenv("123BAD_KEY=val").is_err());
        assert!(parse_dotenv("DASH-NOT-ALLOWED=val").is_err());
    }
}
