//! Session cookie extraction and building.

use axum::http::HeaderMap;

pub const SESSION_COOKIE_NAME: &str = "nusa_session";

/// Extract the session token from the `Cookie` header.
pub fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|pair| {
            let (key, val) = pair.trim().split_once('=')?;
            if key == SESSION_COOKIE_NAME {
                Some(val.to_string())
            } else {
                None
            }
        })
}

/// Create a `Set-Cookie` header value for the session.
pub fn build_session_cookie(token: &str, max_age_secs: u64, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{}={}; HttpOnly{}; SameSite=Strict; Path=/; Max-Age={}",
        SESSION_COOKIE_NAME, token, secure_flag, max_age_secs
    )
}

/// Create a `Set-Cookie` header value that expires/clears the session.
pub fn build_clear_session_cookie(secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{}=; HttpOnly{}; SameSite=Strict; Path=/; Max-Age=0",
        SESSION_COOKIE_NAME, secure_flag
    )
}
