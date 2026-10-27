//! Authoritative execution identity. Panel actors never become Linux users by name guessing.
use crate::{auth::AuthenticatedUser, routes::AppState};
use axum::{Json, http::StatusCode};
use panel_db::{
    models::User,
    repositories::{SqliteSiteRepository, SqliteUserRepository},
};
pub type ContextError = (StatusCode, Json<serde_json::Value>);
fn error(status: StatusCode, message: &str) -> ContextError {
    (status, Json(serde_json::json!({"error": message})))
}
#[derive(Clone, Debug)]
pub struct TenantContext {
    pub user: User,
    pub linux_username: String,
    pub uid: u32,
}

pub fn execution_identity(user: &User) -> Result<(String, u32), (StatusCode, String)> {
    let invalid = || {
        (
            StatusCode::BAD_REQUEST,
            "Tenant belum memiliki identitas Linux yang valid. Provision tenant terlebih dahulu."
                .to_owned(),
        )
    };
    if user.role != "user" || user.status != "active" {
        return Err((
            StatusCode::FORBIDDEN,
            "Target harus tenant aktif, bukan akun administrator.".to_owned(),
        ));
    }
    let username = user.linux_username.as_deref().ok_or_else(invalid)?;
    panel_core::Username::parse(username).map_err(|_| invalid())?;
    let uid = user
        .linux_uid
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| (10000..=59999).contains(id))
        .ok_or_else(invalid)?;
    Ok((username.to_owned(), uid))
}

pub async fn resolve_tenant_context(
    state: &AppState,
    actor: &AuthenticatedUser,
    requested_tenant: Option<&str>,
    requested_site: Option<i64>,
) -> Result<TenantContext, ContextError> {
    let repo = SqliteUserRepository::new(state.pool.clone());
    let db_error = |err: panel_db::error::DbError| {
        tracing::error!(error=%err, "Tenant context lookup failed");
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Gagal memuat konteks tenant.",
        )
    };
    let tenant = if let Some(site_id) = requested_site {
        let site = SqliteSiteRepository::new(state.pool.clone())
            .find_by_id(site_id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "Situs tidak ditemukan."))?;
        if !actor.can_access_user(site.user_id) {
            return Err(error(StatusCode::FORBIDDEN, "Akses tenant ditolak."));
        }
        repo.find_by_id(site.user_id).await.map_err(db_error)?
    } else if let Some(target) = requested_tenant.filter(|s| !s.is_empty()) {
        if !actor.is_admin() && target != actor.username && target != actor.id.to_string() {
            let own = repo.find_by_id(actor.id).await.map_err(db_error)?;
            if own.as_ref().and_then(|u| u.linux_username.as_deref()) != Some(target) {
                return Err(error(StatusCode::FORBIDDEN, "Akses tenant lain ditolak."));
            }
        }
        if let Ok(id) = target.parse::<i64>() {
            repo.find_by_id(id).await.map_err(db_error)?
        } else if let Some(user) = repo.find_by_username(target).await.map_err(db_error)? {
            Some(user)
        } else {
            repo.find_by_linux_username(target)
                .await
                .map_err(db_error)?
        }
    } else {
        if actor.is_admin() {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "Tenant context is required. Pilih tenant atau situs terlebih dahulu.",
            ));
        }
        repo.find_by_id(actor.id).await.map_err(db_error)?
    }
    .ok_or_else(|| error(StatusCode::NOT_FOUND, "Tenant tidak ditemukan."))?;
    if !actor.can_access_user(tenant.id) {
        return Err(error(StatusCode::FORBIDDEN, "Akses tenant ditolak."));
    }
    if let Some(target) = requested_tenant
        && requested_site.is_some()
        && target != tenant.username
        && target != tenant.id.to_string()
        && tenant.linux_username.as_deref() != Some(target)
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Tenant dan situs tidak cocok.",
        ));
    }
    let (linux_username, uid) =
        execution_identity(&tenant).map_err(|(status, message)| error(status, &message))?;
    Ok(TenantContext {
        user: tenant,
        linux_username,
        uid,
    })
}
