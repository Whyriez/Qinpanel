//! Git Deploy endpoints: configuration, deploy key Ed25519, manual deploy,
//! atomic release swap, rollback, deployment history & logs, and webhook receiver.

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;
use crate::sites;
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use panel_core::git_deploy::{
    generate_release_id, parse_github_push_payload, parse_gitlab_push_payload,
    validate_docroot_subpath, validate_git_branch, validate_git_repo_url, verify_github_signature,
    verify_gitlab_token,
};
use panel_db::models::{
    CreateAuditLog, CreateSiteDeployment, SiteDeployment, UpdateSite, UpdateSiteDeployment,
    UpsertSiteGitConfig,
};
use panel_db::{
    SqliteAuditLogRepository, SqliteDomainRepository, SqliteGitDeployRepository,
    SqliteSiteRepository, SqliteUserRepository,
};
use panel_proto::{SiteDeployKeyGenerateRequest, SiteGitDeployRequest, SiteGitRollbackRequest};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteGitConfigResponse {
    pub id: Option<i64>,
    pub site_id: i64,
    pub repo_url: String,
    pub branch: String,
    pub build_cmd: Option<String>,
    pub post_deploy_cmd: Option<String>,
    pub docroot_subpath: Option<String>,
    pub auto_deploy: bool,
    pub keep_releases: i64,
    pub deploy_key_public: Option<String>,
    pub webhook_url: String,
    pub webhook_secret: Option<String>,
    pub has_webhook_secret: bool,
    pub current_release_id: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertGitConfigRequest {
    pub repo_url: String,
    pub branch: String,
    pub build_cmd: Option<String>,
    pub post_deploy_cmd: Option<String>,
    pub docroot_subpath: Option<String>,
    pub auto_deploy: Option<bool>,
    pub keep_releases: Option<i64>,
    pub webhook_secret: Option<String>,
    pub rotate_webhook_secret: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TriggerDeployRequest {
    pub branch: Option<String>,
    pub commit_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RollbackDeployRequest {
    pub target_release_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteDeploymentDto {
    pub id: i64,
    pub site_id: i64,
    pub release_id: String,
    pub commit_hash: Option<String>,
    pub commit_author: Option<String>,
    pub commit_message: Option<String>,
    pub branch: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub trigger_type: String,
    pub log: Option<String>,
    pub error_message: Option<String>,
    pub is_current: bool,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

impl SiteDeploymentDto {
    pub fn from_deployment(d: SiteDeployment, current_release_id: Option<&str>) -> Self {
        let is_current = current_release_id == Some(&d.release_id);
        Self {
            id: d.id,
            site_id: d.site_id,
            release_id: d.release_id,
            commit_hash: d.commit_hash,
            commit_author: d.commit_author,
            commit_message: d.commit_message,
            branch: d.branch,
            status: d.status,
            duration_ms: d.duration_ms,
            trigger_type: d.trigger_type,
            log: d.log_output,
            error_message: None,
            is_current,
            created_at: d.created_at,
            finished_at: d.finished_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DeploymentListQuery {
    pub limit: Option<i64>,
}

/// Helper to verify user access to site.
async fn verify_site_access(
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

    let domains = domain_repo
        .find_by_site_id(site.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((site, user, domains))
}

/// Extracts base directory of site, e.g. `/home/user/sites/example.com`.
fn derive_site_base_dir(docroot: &str) -> String {
    if docroot.contains("/sites/") {
        let parts: Vec<&str> = docroot.split("/sites/").collect();
        if parts.len() == 2 {
            let site_folder = parts[1].split('/').next().unwrap_or("");
            return format!("{}/sites/{}", parts[0], site_folder);
        }
    }
    // Fallback: parent dir or docroot itself
    if let Some(parent) = std::path::Path::new(docroot).parent()
        && parent.to_string_lossy() != "/"
    {
        return parent.to_string_lossy().to_string();
    }
    docroot.to_string()
}

/// Generates a random 32-character hex secret string.
fn generate_random_hex_secret() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:016x}{:016x}", now, std::process::id())
}

/// GET /api/v1/sites/{id}/git
/// Retrieves Git deployment configuration for the specified site.
pub async fn get_git_config_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    if let Err((status, msg)) = verify_site_access(&state.pool, &auth_user, site_id).await {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    let repo = SqliteGitDeployRepository::new(state.pool.clone());
    let config = match repo.get_config(site_id).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let webhook_url = format!("/api/v1/webhooks/deploy/{}", site_id);

    match config {
        Some(c) => {
            let res = SiteGitConfigResponse {
                id: Some(c.id),
                site_id: c.site_id,
                repo_url: c.repo_url,
                branch: c.branch,
                build_cmd: c.build_command,
                post_deploy_cmd: c.post_deploy_hook,
                docroot_subpath: if c.docroot_subpath.is_empty() {
                    None
                } else {
                    Some(c.docroot_subpath)
                },
                auto_deploy: c.auto_deploy_enabled,
                keep_releases: c.releases_to_keep,
                deploy_key_public: c.deploy_key_pub,
                webhook_url,
                webhook_secret: c.webhook_secret.clone(),
                has_webhook_secret: c.webhook_secret.is_some(),
                current_release_id: c.current_release_id,
                created_at: Some(c.created_at),
                updated_at: Some(c.updated_at),
            };
            (StatusCode::OK, Json(res)).into_response()
        }
        None => {
            // Return empty configuration template
            let res = SiteGitConfigResponse {
                id: None,
                site_id,
                repo_url: String::new(),
                branch: "main".to_string(),
                build_cmd: None,
                post_deploy_cmd: None,
                docroot_subpath: None,
                auto_deploy: true,
                keep_releases: 5,
                deploy_key_public: None,
                webhook_url,
                webhook_secret: None,
                has_webhook_secret: false,
                current_release_id: None,
                created_at: None,
                updated_at: None,
            };
            (StatusCode::OK, Json(res)).into_response()
        }
    }
}

/// PUT /api/v1/sites/{id}/git
/// Creates or updates Git deployment configuration for the site.
pub async fn upsert_git_config_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<UpsertGitConfigRequest>,
) -> Response {
    if let Err((status, msg)) = verify_site_access(&state.pool, &auth_user, site_id).await {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    if let Err(e) = validate_git_repo_url(&payload.repo_url) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    if let Err(e) = validate_git_branch(&payload.branch) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    if let Some(sub) = &payload.docroot_subpath
        && let Err(e) = validate_docroot_subpath(sub)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let repo = SqliteGitDeployRepository::new(state.pool.clone());
    let existing = match repo.get_config(site_id).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let webhook_secret = if payload.rotate_webhook_secret.unwrap_or(false) {
        Some(generate_random_hex_secret())
    } else if payload.webhook_secret.is_some() {
        payload.webhook_secret
    } else {
        existing
            .as_ref()
            .and_then(|c| c.webhook_secret.clone())
            .or_else(|| Some(generate_random_hex_secret()))
    };

    let upsert_input = UpsertSiteGitConfig {
        site_id,
        repo_url: payload.repo_url,
        branch: payload.branch,
        deploy_key_pub: existing.as_ref().and_then(|c| c.deploy_key_pub.clone()),
        deploy_key_priv_path: existing
            .as_ref()
            .and_then(|c| c.deploy_key_priv_path.clone()),
        build_command: payload.build_cmd,
        post_deploy_hook: payload.post_deploy_cmd,
        docroot_subpath: payload.docroot_subpath,
        releases_to_keep: payload.keep_releases,
        webhook_secret,
        auto_deploy_enabled: payload.auto_deploy,
        current_release_id: existing.as_ref().and_then(|c| c.current_release_id.clone()),
    };

    let updated = match repo.upsert_config(&upsert_input).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Audit log
    let audit_repo = SqliteAuditLogRepository::new(state.operations_pool.clone());
    let _ = audit_repo
        .create(&CreateAuditLog {
            user_id: Some(auth_user.id),
            actor_username: auth_user.username.clone(),
            action: "site.git_config_update".to_string(),
            target_resource: format!("site:{}", site_id),
            details_json: Some(serde_json::json!({ "branch": updated.branch }).to_string()),
            ip_address: None,
            status: "success".to_string(),
        })
        .await;

    let webhook_url = format!("/api/v1/webhooks/deploy/{}", site_id);
    let res = SiteGitConfigResponse {
        id: Some(updated.id),
        site_id: updated.site_id,
        repo_url: updated.repo_url,
        branch: updated.branch,
        build_cmd: updated.build_command,
        post_deploy_cmd: updated.post_deploy_hook,
        docroot_subpath: if updated.docroot_subpath.is_empty() {
            None
        } else {
            Some(updated.docroot_subpath)
        },
        auto_deploy: updated.auto_deploy_enabled,
        keep_releases: updated.releases_to_keep,
        deploy_key_public: updated.deploy_key_pub,
        webhook_url,
        webhook_secret: updated.webhook_secret.clone(),
        has_webhook_secret: updated.webhook_secret.is_some(),
        current_release_id: updated.current_release_id,
        created_at: Some(updated.created_at),
        updated_at: Some(updated.updated_at),
    };

    (StatusCode::OK, Json(res)).into_response()
}

/// POST /api/v1/sites/{id}/git/key/generate
/// Generates an Ed25519 deploy keypair for the site via the privileged agent.
pub async fn generate_deploy_key_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Response {
    let (site, user, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((status, msg)) => {
            return (status, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let (linux_username, _linux_uid) = match crate::tenant_context::execution_identity(&user) {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error": message}))).into_response();
        }
    };
    let req = SiteDeployKeyGenerateRequest {
        site_id: site.id.to_string(),
        username: linux_username.clone(),
        key_dir: None,
    };

    let agent_res = match state.agent_client.generate_deploy_key(req).await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal membuat deploy key di agent: {}", e) })),
            )
                .into_response();
        }
    };

    let repo = SqliteGitDeployRepository::new(state.pool.clone());
    let upsert = match repo.get_config(site_id).await {
        Ok(Some(c)) => UpsertSiteGitConfig {
            site_id,
            repo_url: c.repo_url,
            branch: c.branch,
            deploy_key_pub: Some(agent_res.public_key.clone()),
            deploy_key_priv_path: Some(agent_res.private_key_path.clone()),
            build_command: c.build_command,
            post_deploy_hook: c.post_deploy_hook,
            docroot_subpath: Some(c.docroot_subpath),
            releases_to_keep: Some(c.releases_to_keep),
            webhook_secret: c.webhook_secret,
            auto_deploy_enabled: Some(c.auto_deploy_enabled),
            current_release_id: c.current_release_id,
        },
        Ok(None) => UpsertSiteGitConfig {
            site_id,
            repo_url: String::new(),
            branch: "main".to_string(),
            deploy_key_pub: Some(agent_res.public_key.clone()),
            deploy_key_priv_path: Some(agent_res.private_key_path.clone()),
            build_command: None,
            post_deploy_hook: None,
            docroot_subpath: None,
            releases_to_keep: Some(5),
            webhook_secret: None,
            auto_deploy_enabled: Some(true),
            current_release_id: None,
        },
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if let Err(e) = repo.upsert_config(&upsert).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let audit_repo = SqliteAuditLogRepository::new(state.operations_pool.clone());
    let _ = audit_repo
        .create(&CreateAuditLog {
            user_id: Some(auth_user.id),
            actor_username: auth_user.username.clone(),
            action: "site.git_key_generate".to_string(),
            target_resource: format!("site:{}", site_id),
            details_json: Some(
                serde_json::json!({
                    "action": "generated Ed25519 deploy key"
                })
                .to_string(),
            ),
            ip_address: None,
            status: "success".to_string(),
        })
        .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "site_id": site_id,
            "public_key": agent_res.public_key,
        })),
    )
        .into_response()
}

/// POST /api/v1/sites/{id}/git/deploy
/// Triggers a manual deployment for the site.
pub async fn trigger_deploy_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<Option<TriggerDeployRequest>>,
) -> Response {
    let (site, user, domains) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((status, msg)) => {
            return (status, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let git_repo = SqliteGitDeployRepository::new(state.pool.clone());
    let config = match git_repo.get_config(site_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({ "error": "Konfigurasi Git belum diatur untuk situs ini" }),
                ),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if config.repo_url.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "URL repositori Git belum diatur" })),
        )
            .into_response();
    }

    let branch = payload
        .as_ref()
        .and_then(|p| p.branch.clone())
        .filter(|b| !b.trim().is_empty())
        .unwrap_or(config.branch.clone());

    let commit_hash_req = payload.as_ref().and_then(|p| p.commit_hash.clone());
    let release_id = generate_release_id(commit_hash_req.as_deref());
    let base_dir = derive_site_base_dir(&site.docroot);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Create initial deployment record
    let create_deploy = CreateSiteDeployment {
        site_id,
        release_id: release_id.clone(),
        commit_hash: commit_hash_req.clone(),
        commit_message: None,
        commit_author: None,
        branch: branch.clone(),
        trigger_type: "manual".to_string(),
        status: "running".to_string(),
        log_output: Some(format!(
            "Deployment diawali oleh pengguna {}",
            auth_user.username
        )),
        duration_ms: None,
        created_at: now,
        finished_at: None,
    };

    let deployment = match git_repo.create_deployment(&create_deploy).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let deploy_key_path = format!("/etc/panel/keys/site_{}_deploy_key", site.id);
    let (linux_username, linux_uid) = match crate::tenant_context::execution_identity(&user) {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error": message}))).into_response();
        }
    };
    let agent_deploy_req = SiteGitDeployRequest {
        site_id: site.id.to_string(),
        username: linux_username.clone(),
        uid: linux_uid,
        repo_url: config.repo_url.clone(),
        branch: branch.clone(),
        commit_hash: commit_hash_req,
        base_dir: base_dir.clone(),
        docroot_subpath: if config.docroot_subpath.is_empty() {
            None
        } else {
            Some(config.docroot_subpath.clone())
        },
        build_cmd: config.build_command.clone(),
        post_deploy_cmd: config.post_deploy_hook.clone(),
        release_id: release_id.clone(),
        keep_releases: config.releases_to_keep as u32,
        deploy_key_path: Some(deploy_key_path),
        env_file_path: Some(format!("/etc/panel/env/site-{}.env", site.id)),
    };

    let deploy_res = state.agent_client.git_deploy(agent_deploy_req).await;
    let finish_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    match deploy_res {
        Ok(agent_out) => {
            let status_str = if agent_out.success {
                "success"
            } else {
                "failed"
            };
            let _ = git_repo
                .update_deployment(
                    deployment.id,
                    &UpdateSiteDeployment {
                        status: status_str.to_string(),
                        commit_hash: Some(agent_out.commit_hash.clone()),
                        commit_message: Some(agent_out.commit_message.clone()),
                        commit_author: Some(agent_out.commit_author.clone()),
                        log_output: Some(agent_out.log.clone()),
                        duration_ms: Some(agent_out.duration_ms as i64),
                        finished_at: Some(finish_time),
                    },
                )
                .await;

            if agent_out.success {
                let _ = git_repo.update_current_release(site_id, &release_id).await;

                // Update site docroot to `<base_dir>/current` if needed
                let current_symlink = format!("{}/current", base_dir);
                if site.docroot != current_symlink {
                    let site_repo = SqliteSiteRepository::new(state.pool.clone());
                    let update_site = UpdateSite {
                        kind: None,
                        docroot: Some(current_symlink),
                        runtime: None,
                        version: None,
                        status: None,
                        settings_json: None,
                    };
                    if let Ok(updated_site) = site_repo.update(site.id, &update_site).await {
                        let _ = sites::recompile_and_apply_site_vhost(
                            &state,
                            &updated_site,
                            &user,
                            &domains,
                        )
                        .await;
                    }
                }
            }

            let updated_dep = git_repo
                .get_deployment(site_id, deployment.id)
                .await
                .ok()
                .flatten()
                .unwrap_or(deployment);

            let dto = SiteDeploymentDto::from_deployment(
                updated_dep,
                if agent_out.success {
                    Some(&release_id)
                } else {
                    config.current_release_id.as_deref()
                },
            );

            (StatusCode::OK, Json(dto)).into_response()
        }
        Err(e) => {
            let err_text = e.to_string();
            let _ = git_repo
                .update_deployment(
                    deployment.id,
                    &UpdateSiteDeployment {
                        status: "failed".to_string(),
                        commit_hash: None,
                        commit_message: None,
                        commit_author: None,
                        log_output: Some(format!("Agent error: {}", err_text)),
                        duration_ms: Some(0),
                        finished_at: Some(finish_time),
                    },
                )
                .await;

            let updated_dep = git_repo
                .get_deployment(site_id, deployment.id)
                .await
                .ok()
                .flatten()
                .unwrap_or(deployment);

            let dto = SiteDeploymentDto::from_deployment(
                updated_dep,
                config.current_release_id.as_deref(),
            );

            (StatusCode::OK, Json(dto)).into_response()
        }
    }
}

/// POST /api/v1/sites/{id}/git/rollback
/// Instantly rolls back the active symlink to a previous release directory.
pub async fn rollback_deploy_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<RollbackDeployRequest>,
) -> Response {
    let (site, user, _) = match verify_site_access(&state.pool, &auth_user, site_id).await {
        Ok(res) => res,
        Err((status, msg)) => {
            return (status, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let target_release = payload.target_release_id.trim();
    if target_release.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Target release ID tidak boleh kosong" })),
        )
            .into_response();
    }

    let base_dir = derive_site_base_dir(&site.docroot);
    let (linux_username, linux_uid) = match crate::tenant_context::execution_identity(&user) {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error": message}))).into_response();
        }
    };
    let agent_req = SiteGitRollbackRequest {
        site_id: site.id.to_string(),
        username: linux_username.clone(),
        uid: linux_uid,
        base_dir: base_dir.clone(),
        target_release_id: target_release.to_string(),
    };

    let agent_res = match state.agent_client.git_rollback(agent_req).await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Rollback gagal di agent: {}", e) })),
            )
                .into_response();
        }
    };

    let git_repo = SqliteGitDeployRepository::new(state.pool.clone());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Create a deployment record for the rollback operation
    let rollback_record = CreateSiteDeployment {
        site_id,
        release_id: target_release.to_string(),
        commit_hash: None,
        commit_message: Some(format!("Rollback ke release {}", target_release)),
        commit_author: None,
        branch: "rollback".to_string(),
        trigger_type: "rollback".to_string(),
        status: "success".to_string(),
        log_output: Some(agent_res.log),
        duration_ms: Some(0),
        created_at: now,
        finished_at: Some(now),
    };

    let deployment = match git_repo.create_deployment(&rollback_record).await {
        Ok(d) => {
            let _ = git_repo
                .update_current_release(site_id, target_release)
                .await;
            d
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let audit_repo = SqliteAuditLogRepository::new(state.operations_pool.clone());
    let _ = audit_repo
        .create(&CreateAuditLog {
            user_id: Some(auth_user.id),
            actor_username: auth_user.username.clone(),
            action: "site.git_rollback".to_string(),
            target_resource: format!("site:{}", site_id),
            details_json: Some(
                serde_json::json!({
                    "target_release": target_release
                })
                .to_string(),
            ),
            ip_address: None,
            status: "success".to_string(),
        })
        .await;

    let dto = SiteDeploymentDto::from_deployment(deployment, Some(target_release));
    (StatusCode::OK, Json(dto)).into_response()
}

/// GET /api/v1/sites/{id}/git/deployments
/// Retrieves deployment history for the site.
pub async fn list_deployments_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<DeploymentListQuery>,
) -> Response {
    if let Err((status, msg)) = verify_site_access(&state.pool, &auth_user, site_id).await {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    let repo = SqliteGitDeployRepository::new(state.pool.clone());
    let config = repo.get_config(site_id).await.ok().flatten();
    let current_rel = config
        .as_ref()
        .and_then(|c| c.current_release_id.as_deref());

    let limit = query.limit.unwrap_or(20);
    match repo.list_deployments(site_id, limit, 0).await {
        Ok(list) => {
            let dtos: Vec<SiteDeploymentDto> = list
                .into_iter()
                .map(|d| SiteDeploymentDto::from_deployment(d, current_rel))
                .collect();
            (StatusCode::OK, Json(dtos)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/v1/sites/{id}/git/deployments/{deploy_id}
/// Retrieves details and full logs for a specific deployment.
pub async fn get_deployment_detail_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, deploy_id)): Path<(i64, i64)>,
) -> Response {
    if let Err((status, msg)) = verify_site_access(&state.pool, &auth_user, site_id).await {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    let repo = SqliteGitDeployRepository::new(state.pool.clone());
    let config = repo.get_config(site_id).await.ok().flatten();
    let current_rel = config
        .as_ref()
        .and_then(|c| c.current_release_id.as_deref());

    match repo.get_deployment(site_id, deploy_id).await {
        Ok(Some(d)) => (
            StatusCode::OK,
            Json(SiteDeploymentDto::from_deployment(d, current_rel)),
        )
            .into_response(),
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Catatan deployment tidak ditemukan" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /api/v1/webhooks/deploy/{site_id}
/// Public endpoint for GitHub (`X-Hub-Signature-256`) and GitLab (`X-Gitlab-Token`) webhook delivery.
#[allow(clippy::cognitive_complexity)]
pub async fn webhook_deploy_handler(
    State(state): State<AppState>,
    Path(site_id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match site_repo.find_by_id(site_id).await {
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
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let git_repo = SqliteGitDeployRepository::new(state.pool.clone());
    let config = match git_repo.get_config(site_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Konfigurasi Git belum diatur" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Legacy or cleared secrets must never turn a public route into an
    // unauthenticated deployment trigger.
    let Some(expected_secret) = config
        .webhook_secret
        .as_deref()
        .filter(|secret| !secret.trim().is_empty())
    else {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error":"Webhook dinonaktifkan sampai secret diatur ulang melalui konfigurasi Git."}))).into_response();
    };
    {
        let is_github = headers.get("X-Hub-Signature-256").is_some();
        let is_gitlab = headers.get("X-Gitlab-Token").is_some();

        let verified = if is_github {
            let sig_val = headers
                .get("X-Hub-Signature-256")
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default();
            verify_github_signature(&body, expected_secret, sig_val)
        } else if is_gitlab {
            let token_val = headers
                .get("X-Gitlab-Token")
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default();
            verify_gitlab_token(token_val, expected_secret)
        } else {
            false
        };

        if !verified {
            warn!("Webhook verification failed for site {}", site_id);
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Tanda tangan atau token webhook tidak valid" })),
            )
                .into_response();
        }
    }

    // Parse payload to check matching branch
    let pushed_branch = parse_github_push_payload(&body)
        .map(|p| p.branch)
        .or_else(|_| parse_gitlab_push_payload(&body).map(|p| p.branch))
        .ok();

    if let Some(branch) = &pushed_branch
        && branch != &config.branch
    {
        info!(
            "Webhook event for site {} ignored: pushed branch '{}' does not match configured branch '{}'",
            site_id, branch, config.branch
        );
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ignored",
                "reason": format!("Branch '{}' does not match configured '{}'", branch, config.branch)
            })),
        )
            .into_response();
    }

    if !config.auto_deploy_enabled {
        info!(
            "Webhook for site {} ignored: auto_deploy is disabled",
            site_id
        );
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ignored",
                "reason": "Auto deploy is disabled"
            })),
        )
            .into_response();
    }

    // Trigger deployment in background task
    let release_id = generate_release_id(None);
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Pengguna pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let base_dir = derive_site_base_dir(&site.docroot);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let trigger_type = if headers.get("X-Hub-Signature-256").is_some() {
        "webhook_github".to_string()
    } else {
        "webhook_gitlab".to_string()
    };

    let create_deploy = CreateSiteDeployment {
        site_id,
        release_id: release_id.clone(),
        commit_hash: None,
        commit_message: Some("Triggered via Webhook push event".to_string()),
        commit_author: None,
        branch: config.branch.clone(),
        trigger_type,
        status: "running".to_string(),
        log_output: Some("Deployment dipicu secara otomatis oleh webhook push event".to_string()),
        duration_ms: None,
        created_at: now,
        finished_at: None,
    };

    let deployment = match git_repo.create_deployment(&create_deploy).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Execute deploy
    let deploy_key_path = format!("/etc/panel/keys/site_{}_deploy_key", site.id);
    let (linux_username, linux_uid) = match crate::tenant_context::execution_identity(&user) {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error": message}))).into_response();
        }
    };
    let agent_deploy_req = SiteGitDeployRequest {
        site_id: site.id.to_string(),
        username: linux_username.clone(),
        uid: linux_uid,
        repo_url: config.repo_url.clone(),
        branch: config.branch.clone(),
        commit_hash: None,
        base_dir: base_dir.clone(),
        docroot_subpath: if config.docroot_subpath.is_empty() {
            None
        } else {
            Some(config.docroot_subpath.clone())
        },
        build_cmd: config.build_command.clone(),
        post_deploy_cmd: config.post_deploy_hook.clone(),
        release_id: release_id.clone(),
        keep_releases: config.releases_to_keep as u32,
        deploy_key_path: Some(deploy_key_path),
        env_file_path: Some(format!("/etc/panel/env/site-{}.env", site.id)),
    };

    let state_clone = state.clone();
    let site_clone = site.clone();
    let user_clone = user.clone();
    let release_id_clone = release_id.clone();
    let deploy_id = deployment.id;

    tokio::spawn(async move {
        let deploy_res = state_clone.agent_client.git_deploy(agent_deploy_req).await;
        let finish_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let git_repo = SqliteGitDeployRepository::new(state_clone.pool.clone());

        match deploy_res {
            Ok(agent_out) => {
                let status_str = if agent_out.success {
                    "success"
                } else {
                    "failed"
                };
                let _ = git_repo
                    .update_deployment(
                        deploy_id,
                        &UpdateSiteDeployment {
                            status: status_str.to_string(),
                            commit_hash: Some(agent_out.commit_hash.clone()),
                            commit_message: Some(agent_out.commit_message.clone()),
                            commit_author: Some(agent_out.commit_author.clone()),
                            log_output: Some(agent_out.log.clone()),
                            duration_ms: Some(agent_out.duration_ms as i64),
                            finished_at: Some(finish_time),
                        },
                    )
                    .await;

                if agent_out.success {
                    let _ = git_repo
                        .update_current_release(site_id, &release_id_clone)
                        .await;

                    let current_symlink = format!("{}/current", base_dir);
                    if site_clone.docroot != current_symlink {
                        let site_repo = SqliteSiteRepository::new(state_clone.pool.clone());
                        let domain_repo = SqliteDomainRepository::new(state_clone.pool.clone());
                        let domains = domain_repo
                            .find_by_site_id(site_id)
                            .await
                            .unwrap_or_default();
                        let update_site = UpdateSite {
                            kind: None,
                            docroot: Some(current_symlink),
                            runtime: None,
                            version: None,
                            status: None,
                            settings_json: None,
                        };
                        if let Ok(updated_site) =
                            site_repo.update(site_clone.id, &update_site).await
                        {
                            let _ = sites::recompile_and_apply_site_vhost(
                                &state_clone,
                                &updated_site,
                                &user_clone,
                                &domains,
                            )
                            .await;
                        }
                    }
                }
            }
            Err(e) => {
                error!("Webhook deploy failed for site {}: {}", site_id, e);
                let _ = git_repo
                    .update_deployment(
                        deploy_id,
                        &UpdateSiteDeployment {
                            status: "failed".to_string(),
                            commit_hash: None,
                            commit_message: None,
                            commit_author: None,
                            log_output: Some(format!("Agent error: {}", e)),
                            duration_ms: Some(0),
                            finished_at: Some(finish_time),
                        },
                    )
                    .await;
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "status": "deploy_triggered",
            "release_id": release_id,
            "deployment_id": deployment.id,
        })),
    )
        .into_response()
}
