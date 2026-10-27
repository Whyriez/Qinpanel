//! CSRF token generation and validation.

use panel_core::auth::hash_token;

pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

/// Generates a CSRF token for a given session ID.
pub fn generate_csrf_token(session_id: &str) -> String {
    hash_token(&format!("csrf:{}", session_id))
}

/// Verifies whether candidate token matches the expected CSRF token for the session.
pub fn verify_csrf_token(session_id: &str, candidate: &str) -> bool {
    let expected = generate_csrf_token(session_id);
    expected == candidate
}
