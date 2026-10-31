//! HTTP handlers for authentication and session management.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header::SET_COOKIE},
    response::{IntoResponse, Response},
};
use panel_core::auth::{hash_password, hash_token, verify_password};
use panel_core::totp::{
    generate_recovery_codes, generate_totp_qr_base64, generate_totp_secret, generate_totp_uri,
    hash_recovery_code, verify_totp_code,
};
use panel_db::models::CreateSession;
use panel_db::repositories::{
    SqliteRecoveryCodeRepository, SqliteSessionRepository, SqliteUserRepository,
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use utoipa::ToSchema;

use super::csrf::generate_csrf_token;
use super::middleware::{RequireAuth, RequireCsrf};
use super::session::{build_clear_session_cookie, build_session_cookie};
use crate::routes::AppState;

// Bound Argon2's memory/CPU usage even when requests arrive concurrently.
static LOGIN_VERIFICATIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
static LOGIN_REQUESTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    #[schema(example = "admin")]
    pub username: String,
    #[schema(example = "Secret123!")]
    pub password: String,
    #[schema(example = "123456")]
    pub code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, Clone)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub email: Option<String>,
    pub totp_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impersonated_by: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csrf_token: Option<String>,
    pub requires_2fa: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    #[schema(example = "OldPassword123!")]
    #[serde(alias = "old_password")]
    pub current_password: String,
    #[schema(example = "NewPassword456!")]
    pub new_password: Option<String>,
    /// Optional administrator login name; never changes a tenant Linux identity.
    pub username: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CsrfResponse {
    pub csrf_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TwoFactorSetupResponse {
    pub secret: String,
    pub otpauth_url: String,
    pub qr_code_base64: String,
    pub recovery_codes: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TwoFactorEnableRequest {
    #[schema(example = "123456")]
    pub code: String,
    pub secret: String,
    pub recovery_codes: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TwoFactorDisableRequest {
    #[schema(example = "123456")]
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TwoFactorStatusResponse {
    pub enabled: bool,
    pub remaining_recovery_codes: i64,
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful or 2FA challenge required", body = LoginResponse),
        (status = 401, description = "Invalid credentials"),
        (status = 423, description = "Account locked due to brute force attempts")
    ),
    tag = "auth"
)]
pub async fn login_handler(
    State(state): State<AppState>,
    req_headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Response {
    if payload.username.is_empty()
        || payload.username.len() > 32
        || payload.password.len() > 4096
        || payload.code.as_ref().is_some_and(|code| code.len() > 128)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"Format kredensial tidak valid"})),
        )
            .into_response();
    }
    let _request_permit = match LOGIN_REQUESTS.try_acquire() {
        Ok(permit) => permit,
        Err(_) => return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({"error":"Terlalu banyak proses login. Coba kembali sebentar lagi."}))).into_response(),
    };
    let permit =
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            LOGIN_VERIFICATIONS.acquire(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            _ => return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(
                    serde_json::json!({"error":"Antrean login penuh. Coba kembali sebentar lagi."}),
                ),
            )
                .into_response(),
        };
    let client_ip = crate::audit::extract_client_ip(&req_headers);

    // 1. Check account lockout status
    if let Err(remaining_secs) = state.lockout_manager.check_lockout(&payload.username) {
        let _ = state
            .audit_writer
            .log(
                None,
                &payload.username,
                "auth.login",
                "auth",
                "failure",
                client_ip.clone(),
                Some(r#"{"reason":"account_locked"}"#.to_string()),
            )
            .await;
        return (
            StatusCode::LOCKED,
            Json(serde_json::json!({
                "error": "Account is temporarily locked due to too many failed login attempts",
                "remaining_seconds": remaining_secs
            })),
        )
            .into_response();
    }

    // 2. Fetch user from database
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_username(&payload.username).await {
        Ok(Some(u)) => u,
        _ => {
            let lockout = state.lockout_manager.record_failure(&payload.username);
            let _ = state
                .audit_writer
                .log(
                    None,
                    &payload.username,
                    "auth.login",
                    "auth",
                    "failure",
                    client_ip.clone(),
                    Some(r#"{"reason":"user_not_found"}"#.to_string()),
                )
                .await;
            let status = if lockout.is_some() {
                StatusCode::LOCKED
            } else {
                StatusCode::UNAUTHORIZED
            };
            return (
                status,
                Json(serde_json::json!({
                    "error": "Invalid username or password"
                })),
            )
                .into_response();
        }
    };

    // 3. Verify password hash using Argon2id
    let password = payload.password;
    let hash = user.password_hash.clone();
    let is_valid = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        verify_password(&password, &hash).unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !is_valid {
        let lockout = state.lockout_manager.record_failure(&payload.username);
        let _ = state
            .audit_writer
            .log(
                Some(user.id),
                &user.username,
                "auth.login",
                "auth",
                "failure",
                client_ip.clone(),
                Some(r#"{"reason":"invalid_password"}"#.to_string()),
            )
            .await;
        let status = if lockout.is_some() {
            StatusCode::LOCKED
        } else {
            StatusCode::UNAUTHORIZED
        };
        return (
            status,
            Json(serde_json::json!({
                "error": "Invalid username or password"
            })),
        )
            .into_response();
    }

    // 4. Check account status
    if user.status != "active" {
        let _ = state
            .audit_writer
            .log(
                Some(user.id),
                &user.username,
                "auth.login",
                "auth",
                "failure",
                client_ip.clone(),
                Some(r#"{"reason":"inactive_account"}"#.to_string()),
            )
            .await;
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Account is inactive or suspended"
            })),
        )
            .into_response();
    }

    // 5. Check 2FA if enabled
    if user.totp_enabled {
        match payload.code.as_deref() {
            None => {
                return (
                    StatusCode::OK,
                    Json(LoginResponse {
                        user: None,
                        csrf_token: None,
                        requires_2fa: true,
                    }),
                )
                    .into_response();
            }
            Some(submitted_code) => {
                let code_trimmed = submitted_code.trim();
                let secret = user.totp_secret.as_deref().unwrap_or("");
                let is_totp_valid = verify_totp_code(secret, code_trimmed, &user.username);

                let is_valid_2fa = if is_totp_valid {
                    true
                } else {
                    let recovery_repo = SqliteRecoveryCodeRepository::new(state.pool.clone());
                    let code_hash = hash_recovery_code(code_trimmed);
                    recovery_repo
                        .verify_and_consume(user.id, &code_hash)
                        .await
                        .unwrap_or(false)
                };

                if !is_valid_2fa {
                    let lockout = state.lockout_manager.record_failure(&payload.username);
                    let _ = state
                        .audit_writer
                        .log(
                            Some(user.id),
                            &user.username,
                            "auth.login",
                            "auth",
                            "failure",
                            client_ip.clone(),
                            Some(r#"{"reason":"invalid_2fa"}"#.to_string()),
                        )
                        .await;
                    let status = if lockout.is_some() {
                        StatusCode::LOCKED
                    } else {
                        StatusCode::UNAUTHORIZED
                    };
                    return (
                        status,
                        Json(serde_json::json!({
                            "error": "Invalid two-factor authentication code or recovery code"
                        })),
                    )
                        .into_response();
                }
            }
        }
    }

    // 6. Successful login: reset lockout counter
    state.lockout_manager.record_success(&payload.username);

    // 7. Create session in database
    let session_id = uuid::Uuid::new_v4().to_string();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let token_hash = hash_token(&token);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let expires_at = now + state.auth_config.session_duration_secs as i64;

    let session_repo = SqliteSessionRepository::new(state.pool.clone());
    match session_repo
        .create_if_credentials_match(&CreateSession {
            id: session_id.clone(),
            user_id: user.id,
            token_hash,
            ip_address: client_ip.clone(),
            user_agent: None,
            expires_at,
        }, &user)
        .await
    {
        Ok(true) => {},
        Ok(false) => return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"Kredensial akun telah berubah. Silakan masuk kembali."})),
        ).into_response(),
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to create session: {}", e) })),
        )
            .into_response(),
    }

    // Record successful login audit log
    let _ = state
        .audit_writer
        .log(
            Some(user.id),
            &user.username,
            "auth.login",
            "auth",
            "success",
            client_ip,
            None,
        )
        .await;

    // 8. Generate CSRF token and Set-Cookie header
    let csrf_token = generate_csrf_token(&session_id);
    let is_secure = state
        .auth_config
        .cookie_secure
        .unwrap_or(state.environment != "development");
    let cookie_val =
        build_session_cookie(&token, state.auth_config.session_duration_secs, is_secure);

    let mut headers = HeaderMap::new();
    if let Ok(cookie_header) = cookie_val.parse() {
        headers.insert(SET_COOKIE, cookie_header);
    }

    let response_body = LoginResponse {
        user: Some(UserInfo {
            id: user.id,
            username: user.username,
            role: user.role,
            email: user.email,
            totp_enabled: user.totp_enabled,
            impersonated_by: None,
        }),
        csrf_token: Some(csrf_token),
        requires_2fa: false,
    };

    (headers, Json(response_body)).into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    responses(
        (status = 200, description = "Logged out successfully", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Invalid CSRF token")
    ),
    tag = "auth"
)]
pub async fn logout_handler(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
) -> Response {
    let session_repo = SqliteSessionRepository::new(state.pool.clone());
    let _ = session_repo.delete(&user.session_id).await;

    let _ = state
        .audit_writer
        .log(
            Some(user.id),
            &user.username,
            "auth.logout",
            "auth",
            "success",
            None,
            None,
        )
        .await;

    let is_secure = state
        .auth_config
        .cookie_secure
        .unwrap_or(state.environment != "development");
    let cookie_val = build_clear_session_cookie(is_secure);

    let mut headers = HeaderMap::new();
    if let Ok(cookie_header) = cookie_val.parse() {
        headers.insert(SET_COOKIE, cookie_header);
    }

    (
        headers,
        Json(MessageResponse {
            message: "Logged out successfully".to_string(),
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current authenticated user profile", body = LoginResponse),
        (status = 401, description = "Not authenticated")
    ),
    tag = "auth"
)]
pub async fn me_handler(RequireAuth(user): RequireAuth) -> impl IntoResponse {
    Json(LoginResponse {
        user: Some(UserInfo {
            id: user.id,
            username: user.username,
            role: user.role,
            email: user.email,
            totp_enabled: user.totp_enabled,
            impersonated_by: user.impersonated_by,
        }),
        csrf_token: Some(user.csrf_token),
        requires_2fa: false,
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/csrf",
    responses(
        (status = 200, description = "Current session CSRF token", body = CsrfResponse)
    ),
    tag = "auth"
)]
pub async fn csrf_handler(RequireAuth(user): RequireAuth) -> impl IntoResponse {
    Json(CsrfResponse {
        csrf_token: user.csrf_token,
    })
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/password",
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Credentials updated atomically; other sessions and API tokens revoked", body = LoginResponse),
        (status = 400, description = "Incorrect current password or invalid new password"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Invalid CSRF token")
    ),
    tag = "auth"
)]
pub async fn change_password_handler(
    State(state): State<AppState>,
    RequireCsrf(auth_user): RequireCsrf,
    Json(payload): Json<ChangePasswordRequest>,
) -> Response {
    let fail = |status: StatusCode, message: &str| {
        (status, Json(serde_json::json!({"error":message}))).into_response()
    };
    if auth_user.impersonated_by.is_some() {
        return fail(
            StatusCode::FORBIDDEN,
            "Akhiri impersonasi sebelum mengubah kredensial.",
        );
    }
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_id(auth_user.id).await {
        Ok(Some(u)) => u,
        _ => return fail(StatusCode::NOT_FOUND, "Akun tidak ditemukan."),
    };
    if payload.current_password.len() > 4096 {
        return fail(StatusCode::BAD_REQUEST, "Kata sandi terlalu panjang.");
    }
    let verification_key = format!("credentials:{}", user.id);
    if state
        .lockout_manager
        .check_lockout(&verification_key)
        .is_err()
    {
        return fail(
            StatusCode::LOCKED,
            "Terlalu banyak percobaan. Tunggu sebelum mencoba kembali.",
        );
    }
    let current = payload.current_password;
    let hash = user.password_hash.clone();
    let verified = tokio::task::spawn_blocking(move || verify_password(&current, &hash))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    if !verified {
        let locked = state
            .lockout_manager
            .record_failure(&verification_key)
            .is_some();
        state.audit_writer.log_async(
            Some(user.id),
            &user.username,
            "auth.credentials_change",
            "auth",
            "failure",
            None,
            None,
        );
        return fail(
            if locked {
                StatusCode::LOCKED
            } else {
                StatusCode::BAD_REQUEST
            },
            "Kata sandi saat ini tidak benar.",
        );
    }
    state.lockout_manager.record_success(&verification_key);
    let username = payload.username.as_deref().unwrap_or(&user.username);
    if username != user.username && user.role != "admin" {
        return fail(
            StatusCode::FORBIDDEN,
            "Perubahan username mandiri tersedia untuk administrator.",
        );
    }
    if !(3..=32).contains(&username.len())
        || !username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return fail(
            StatusCode::BAD_REQUEST,
            "Username harus 3-32 karakter: huruf, angka, garis bawah, atau tanda hubung.",
        );
    }
    if payload.new_password.is_none() && username == user.username {
        return fail(
            StatusCode::BAD_REQUEST,
            "Ubah username atau masukkan kata sandi baru.",
        );
    }
    let new_hash = if let Some(password) = payload.new_password {
        if !(8..=4096).contains(&password.len()) {
            return fail(
                StatusCode::BAD_REQUEST,
                "Kata sandi baru harus 8-4096 byte.",
            );
        }
        match tokio::task::spawn_blocking(move || hash_password(&password)).await {
            Ok(Ok(hash)) => hash,
            _ => {
                return fail(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Gagal mengamankan kata sandi.",
                );
            }
        }
    } else {
        user.password_hash.clone()
    };
    let new_session_id = uuid::Uuid::new_v4().to_string();
    let new_token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // Compare the verified hash and session again under the write lock. A concurrent
    // password reset or credential change cannot be overwritten by a stale request.
    let update = async {
        let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query("UPDATE users SET username=?, password_hash=?, updated_at=? WHERE id=? AND password_hash=? AND status='active' AND EXISTS (SELECT 1 FROM sessions WHERE id=? AND user_id=users.id AND expires_at>?)")
            .bind(username).bind(new_hash).bind(now).bind(user.id).bind(&user.password_hash).bind(&auth_user.session_id).bind(now)
            .execute(&mut *tx).await?;
        if result.rows_affected() != 1 { return Ok::<bool, sqlx::Error>(false); }
        sqlx::query("DELETE FROM sessions WHERE user_id=?").bind(user.id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM api_tokens WHERE user_id=?").bind(user.id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO sessions (id,user_id,token_hash,expires_at,created_at) VALUES (?,?,?,?,?)")
            .bind(&new_session_id).bind(user.id).bind(hash_token(&new_token))
            .bind(now + state.auth_config.session_duration_secs as i64).bind(now).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }.await;
    match update {
        Ok(true) => {}
        Ok(false) => {
            return fail(
                StatusCode::CONFLICT,
                "Akun atau sesi telah berubah. Masuk kembali sebelum mencoba lagi.",
            );
        }
        Err(ref e)
            if e.as_database_error()
                .is_some_and(|e| e.is_unique_violation()) =>
        {
            return fail(StatusCode::CONFLICT, "Username sudah digunakan.");
        }
        Err(e) => {
            tracing::error!(error=%e, "Credential transaction failed");
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Perubahan akun dibatalkan. Silakan coba lagi.",
            );
        }
    }
    let new_csrf = generate_csrf_token(&new_session_id);
    let secure = state
        .auth_config
        .cookie_secure
        .unwrap_or(state.environment != "development");
    let mut headers = HeaderMap::new();
    if let Ok(value) =
        build_session_cookie(&new_token, state.auth_config.session_duration_secs, secure).parse()
    {
        headers.insert(SET_COOKIE, value);
    }
    state.lockout_manager.record_success(username);
    state.audit_writer.log_async(
        Some(user.id),
        username,
        "auth.credentials_change",
        "auth",
        "success",
        None,
        None,
    );
    (
        headers,
        Json(LoginResponse {
            user: Some(UserInfo {
                id: user.id,
                username: username.to_string(),
                role: user.role,
                email: user.email,
                totp_enabled: user.totp_enabled,
                impersonated_by: None,
            }),
            csrf_token: Some(new_csrf),
            requires_2fa: false,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/2fa/setup",
    responses(
        (status = 200, description = "Generated TOTP secret, QR code, and recovery codes", body = TwoFactorSetupResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Invalid CSRF token")
    ),
    tag = "auth"
)]
pub async fn two_factor_setup_handler(RequireCsrf(user): RequireCsrf) -> Response {
    let secret = generate_totp_secret();
    let otpauth_url = match generate_totp_uri(&secret, &user.username, None) {
        Ok(url) => url,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to generate TOTP URI: {}", e) })),
            )
                .into_response();
        }
    };

    let qr_code_base64 = match generate_totp_qr_base64(&secret, &user.username, None) {
        Ok(qr) => qr,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to generate QR code: {}", e) })),
            )
                .into_response();
        }
    };

    let recovery_codes = generate_recovery_codes(8);

    Json(TwoFactorSetupResponse {
        secret,
        otpauth_url,
        qr_code_base64,
        recovery_codes,
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/2fa/enable",
    request_body = TwoFactorEnableRequest,
    responses(
        (status = 200, description = "2FA enabled successfully", body = MessageResponse),
        (status = 400, description = "Invalid verification code or recovery codes"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Invalid CSRF token")
    ),
    tag = "auth"
)]
pub async fn two_factor_enable_handler(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Json(payload): Json<TwoFactorEnableRequest>,
) -> Response {
    if user.totp_enabled {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"2FA sudah aktif. Verifikasi dan nonaktifkan faktor lama sebelum menggantinya."}))).into_response();
    }
    // 1. Verify code against provided secret
    if !verify_totp_code(&payload.secret, &payload.code, &user.username) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid verification code" })),
        )
            .into_response();
    }

    if payload.recovery_codes.len() != 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Must provide exactly 8 recovery codes" })),
        )
            .into_response();
    }

    // 2. Hash recovery codes and save to DB
    let recovery_hashes: Vec<String> = payload
        .recovery_codes
        .iter()
        .map(|code| hash_recovery_code(code))
        .collect();

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    match user_repo.enable_totp_if_disabled(user.id, &user.session_id, &payload.secret, &recovery_hashes).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"Faktor atau sesi telah berubah. Masuk kembali sebelum melanjutkan."}))).into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"Aktivasi 2FA dibatalkan; silakan coba kembali."}))).into_response(),
    }

    state.audit_writer.log_async(
        Some(user.id),
        &user.username,
        "auth.2fa_enable",
        "auth",
        "success",
        None,
        None,
    );

    Json(MessageResponse {
        message: "Two-factor authentication enabled successfully".to_string(),
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/2fa/disable",
    request_body = TwoFactorDisableRequest,
    responses(
        (status = 200, description = "2FA disabled successfully", body = MessageResponse),
        (status = 400, description = "Invalid code or 2FA not enabled"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Invalid CSRF token")
    ),
    tag = "auth"
)]
pub async fn two_factor_disable_handler(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Json(payload): Json<TwoFactorDisableRequest>,
) -> Response {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let db_user = match user_repo.find_by_id(user.id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "User not found" })),
            )
                .into_response();
        }
    };

    if !db_user.totp_enabled {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Two-factor authentication is not enabled" })),
        )
            .into_response();
    }

    let code_trimmed = payload.code.trim();
    let secret = db_user.totp_secret.as_deref().unwrap_or("");
    let is_totp_valid = verify_totp_code(secret, code_trimmed, &db_user.username);

    let is_valid = if is_totp_valid {
        true
    } else {
        let recovery_repo = SqliteRecoveryCodeRepository::new(state.pool.clone());
        let code_hash = hash_recovery_code(code_trimmed);
        recovery_repo
            .verify_and_consume(user.id, &code_hash)
            .await
            .unwrap_or(false)
    };

    if !is_valid {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid verification code or recovery code" })),
        )
            .into_response();
    }

    // Disable TOTP on user
    if let Err(e) = user_repo.update_totp(user.id, None, false).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to disable TOTP: {}", e) })),
        )
            .into_response();
    }

    // Delete recovery codes
    let recovery_repo = SqliteRecoveryCodeRepository::new(state.pool.clone());
    let _ = recovery_repo.delete_by_user_id(user.id).await;

    state.audit_writer.log_async(
        Some(user.id),
        &user.username,
        "auth.2fa_disable",
        "auth",
        "success",
        None,
        None,
    );

    Json(MessageResponse {
        message: "Two-factor authentication disabled successfully".to_string(),
    })
    .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/2fa/status",
    responses(
        (status = 200, description = "Two-factor authentication status", body = TwoFactorStatusResponse),
        (status = 401, description = "Not authenticated")
    ),
    tag = "auth"
)]
pub async fn two_factor_status_handler(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
) -> Response {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let db_user = match user_repo.find_by_id(user.id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "User not found" })),
            )
                .into_response();
        }
    };

    let remaining = if db_user.totp_enabled {
        let recovery_repo = SqliteRecoveryCodeRepository::new(state.pool.clone());
        recovery_repo.count_unused(user.id).await.unwrap_or(0)
    } else {
        0
    };

    Json(TwoFactorStatusResponse {
        enabled: db_user.totp_enabled,
        remaining_recovery_codes: remaining,
    })
    .into_response()
}
