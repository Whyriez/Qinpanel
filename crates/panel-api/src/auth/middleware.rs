//! Authentication, RBAC, and CSRF extractors and middleware.

use axum::{
    Json,
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use panel_core::CoreError;
use panel_core::auth::hash_token;
use panel_core::rbac::{Action, Role, authorize};
use panel_db::repositories::{SqliteSessionRepository, SqliteUserRepository};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use super::csrf::{CSRF_HEADER_NAME, generate_csrf_token, verify_csrf_token};
use super::session::extract_session_token;
use crate::routes::AppState;

/// Default-deny boundary for the API, including legacy handlers that predate the
/// CSRF extractors. Public entrypoints have their own credential verification.
pub async fn api_security_boundary(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if !path.starts_with("/api/v1/") {
        return next.run(request).await;
    }
    let public_post = request.method() == axum::http::Method::POST
        && (matches!(
            path,
            "/api/v1/auth/login" | "/api/v1/auth/setup" | "/api/v1/auth/setup/validate"
        ) || path
            .strip_prefix("/api/v1/webhooks/deploy/")
            .is_some_and(|id| id.parse::<i64>().is_ok_and(|id| id > 0)));
    if public_post {
        return next.run(request).await;
    }
    // These HTML tools use their own form CSRF tokens. The gateway still
    // authenticates, checks the database owner and validates browser origin.
    let database_tool = path.strip_prefix("/api/v1/databases/").is_some_and(|rest| {
        rest.split_once('/').is_some_and(|(id, rest)| {
            id.parse::<i64>().is_ok_and(|id| id > 0) && rest.starts_with("web/")
        })
    });
    let safe = matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let (mut parts, body) = request.into_parts();
    let result = if safe || database_tool {
        RequireAuth::from_request_parts(&mut parts, &state)
            .await
            .map(|_| ())
    } else {
        RequireCsrf::from_request_parts(&mut parts, &state)
            .await
            .map(|_| ())
    };
    match result {
        Ok(()) => next.run(Request::from_parts(parts, body)).await,
        Err(response) => response,
    }
}

/// Represents a verified authenticated user in a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub email: Option<String>,
    pub totp_enabled: bool,
    pub session_id: String,
    pub csrf_token: String,
    pub impersonated_by: Option<String>,
}

impl AuthenticatedUser {
    /// Returns the parsed `Role` enum.
    pub fn role_enum(&self) -> Result<Role, CoreError> {
        self.role.parse()
    }

    /// Checks if this user is an administrator.
    pub fn is_admin(&self) -> bool {
        self.role.eq_ignore_ascii_case("admin")
    }

    /// Checks whether this user is allowed to access target user's resources.
    /// Admins can access any user, regular users can only access their own.
    pub fn can_access_user(&self, target_user_id: i64) -> bool {
        self.is_admin() || self.id == target_user_id
    }

    /// Evaluates permission against the centralized policy engine.
    pub fn authorize(&self, action: &Action) -> bool {
        let role = self.role_enum().unwrap_or(Role::User);
        authorize(role, self.id, action)
    }

    /// Enforces permission against the centralized policy engine, returning 403 if forbidden.
    pub fn require_permission(&self, action: &Action) -> Result<(), (StatusCode, &'static str)> {
        if self.authorize(action) {
            Ok(())
        } else {
            Err((
                StatusCode::FORBIDDEN,
                "Forbidden: Not authorized to perform action on requested resource",
            ))
        }
    }
}

/// Extractor that requires any authenticated, active user.
#[derive(Debug, Clone)]
pub struct RequireAuth(pub AuthenticatedUser);

/// Type alias for RequireAuth emphasizing user role.
pub type RequireUser = RequireAuth;

/// Extractor that requires any authenticated user and a valid CSRF token.
#[derive(Debug, Clone)]
pub struct RequireCsrf(pub AuthenticatedUser);

/// Type alias for RequireCsrf emphasizing user role.
pub type RequireUserCsrf = RequireCsrf;

/// Extractor that requires an authenticated user with administrator role.
#[derive(Debug, Clone)]
pub struct RequireAdmin(pub AuthenticatedUser);

/// Extractor that requires an administrator user and a valid CSRF token.
#[derive(Debug, Clone)]
pub struct RequireAdminCsrf(pub AuthenticatedUser);

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = match extract_session_token(&parts.headers) {
            Some(t) => t,
            None => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "Authentication required: missing session cookie"
                    })),
                )
                    .into_response());
            }
        };

        let token_hash = hash_token(&token);
        let session_repo = SqliteSessionRepository::new(state.pool.clone());
        let session = match session_repo.find_by_token_hash(&token_hash).await {
            Ok(Some(s)) => s,
            _ => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "Invalid or expired session"
                    })),
                )
                    .into_response());
            }
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if session.expires_at <= now {
            let _ = session_repo.delete(&session.id).await;
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Session has expired"
                })),
            )
                .into_response());
        }

        let user_repo = SqliteUserRepository::new(state.pool.clone());
        let user = match user_repo.find_by_id(session.user_id).await {
            Ok(Some(u)) => u,
            _ => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "User not found"
                    })),
                )
                    .into_response());
            }
        };

        if user.status != "active" {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "Account is inactive or suspended"
                })),
            )
                .into_response());
        }

        let csrf_token = generate_csrf_token(&session.id);
        let impersonated_by = session
            .user_agent
            .as_deref()
            .and_then(|ua| ua.strip_prefix("impersonated_by:"))
            .map(|s| s.to_string());

        Ok(AuthenticatedUser {
            id: user.id,
            username: user.username,
            role: user.role,
            email: user.email,
            totp_enabled: user.totp_enabled,
            session_id: session.id,
            csrf_token,
            impersonated_by,
        })
    }
}

impl FromRequestParts<AppState> for RequireAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;
        Ok(RequireAuth(user))
    }
}

impl FromRequestParts<AppState> for RequireCsrf {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;

        let candidate_csrf = parts
            .headers
            .get(CSRF_HEADER_NAME)
            .and_then(|h| h.to_str().ok());

        match candidate_csrf {
            Some(candidate) if verify_csrf_token(&user.session_id, candidate) => {
                Ok(RequireCsrf(user))
            }
            _ => Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "CSRF token missing or invalid"
                })),
            )
                .into_response()),
        }
    }
}

impl FromRequestParts<AppState> for RequireAdmin {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;
        if !user.is_admin() {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "Forbidden: Administrator privileges required"
                })),
            )
                .into_response());
        }
        Ok(RequireAdmin(user))
    }
}

impl FromRequestParts<AppState> for RequireAdminCsrf {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let RequireCsrf(user) = RequireCsrf::from_request_parts(parts, state).await?;
        if !user.is_admin() {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "Forbidden: Administrator privileges required"
                })),
            )
                .into_response());
        }
        Ok(RequireAdminCsrf(user))
    }
}

/// Helper function to validate CSRF token from headers against authenticated user.
pub fn validate_csrf_header(headers: &HeaderMap, user: &AuthenticatedUser) -> bool {
    let candidate_csrf = headers.get(CSRF_HEADER_NAME).and_then(|h| h.to_str().ok());

    match candidate_csrf {
        Some(candidate) => verify_csrf_token(&user.session_id, candidate),
        None => false,
    }
}
