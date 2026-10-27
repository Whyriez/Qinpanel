//! TOTP (Time-based One-Time Password) generation, verification, and recovery codes.

use crate::CoreError;
use crate::auth::hash_token;
use totp_rs::{Algorithm, Secret, TOTP};

/// Default issuer name for Qinpanel TOTP tokens.
pub const DEFAULT_TOTP_ISSUER: &str = "Qinpanel";

/// Generates a random base32 encoded TOTP secret (160 bits / 20 bytes standard).
pub fn generate_totp_secret() -> String {
    Secret::generate_secret().to_encoded().to_string()
}

/// Creates a configured `TOTP` instance with 6 digits, 30s interval, and 1-step skew tolerance.
pub fn create_totp_instance(
    secret_base32: &str,
    username: &str,
    issuer: Option<&str>,
) -> Result<TOTP, CoreError> {
    let secret = Secret::Encoded(secret_base32.to_string())
        .to_bytes()
        .map_err(|e| CoreError::Validation(format!("Invalid base32 TOTP secret: {:?}", e)))?;

    let issuer_str = issuer.unwrap_or(DEFAULT_TOTP_ISSUER).to_string();

    TOTP::new(
        Algorithm::SHA1,
        6,
        1, // 1 step skew tolerance (+/- 30s for clock drift)
        30,
        secret,
        Some(issuer_str),
        username.to_string(),
    )
    .map_err(|e| CoreError::Validation(format!("Failed to create TOTP instance: {}", e)))
}

/// Generates the `otpauth://` URI for enrolling in authenticator apps.
pub fn generate_totp_uri(
    secret_base32: &str,
    username: &str,
    issuer: Option<&str>,
) -> Result<String, CoreError> {
    let totp = create_totp_instance(secret_base32, username, issuer)?;
    Ok(totp.get_url())
}

/// Generates a base64 encoded PNG QR code for easy scanning.
pub fn generate_totp_qr_base64(
    secret_base32: &str,
    username: &str,
    issuer: Option<&str>,
) -> Result<String, CoreError> {
    let totp = create_totp_instance(secret_base32, username, issuer)?;
    totp.get_qr_base64()
        .map_err(|e| CoreError::Validation(format!("Failed to generate QR code: {}", e)))
}

/// Verifies a 6-digit TOTP code against a secret.
pub fn verify_totp_code(secret_base32: &str, code: &str, username: &str) -> bool {
    let trimmed = code.trim();
    if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    match create_totp_instance(secret_base32, username, None) {
        Ok(totp) => totp.check_current(trimmed).unwrap_or(false),
        Err(_) => false,
    }
}

/// Normalizes a recovery code (uppercase, strips dashes/spaces).
pub fn normalize_recovery_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Hashes a recovery code using SHA-256 for secure database storage.
pub fn hash_recovery_code(code: &str) -> String {
    let normalized = normalize_recovery_code(code);
    hash_token(&normalized)
}

/// Generates a list of single-use recovery codes (e.g. 8 codes formatted as `XXXX-XXXX`).
pub fn generate_recovery_codes(count: usize) -> Vec<String> {
    let mut codes = Vec::with_capacity(count);
    for _ in 0..count {
        let u = uuid::Uuid::new_v4().simple().to_string();
        let code = format!("{}-{}", &u[0..4], &u[4..8]).to_ascii_uppercase();
        codes.push(code);
    }
    codes
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_totp_lifecycle() {
        let secret = generate_totp_secret();
        assert!(!secret.is_empty());

        let uri = generate_totp_uri(&secret, "admin", None).expect("uri");
        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains("secret="));

        let qr = generate_totp_qr_base64(&secret, "admin", None).expect("qr");
        assert!(!qr.is_empty());

        // Generate current code
        let totp = create_totp_instance(&secret, "admin", None).expect("totp");
        let current_code = totp.generate_current().expect("generate_current");
        assert_eq!(current_code.len(), 6);

        // Verify valid code
        let is_valid = verify_totp_code(&secret, &current_code, "admin");
        assert!(is_valid);

        // Verify invalid code
        assert!(!verify_totp_code(&secret, "000000", "admin"));
        assert!(!verify_totp_code(&secret, "abc", "admin"));
    }

    #[test]
    fn test_recovery_codes() {
        let codes = generate_recovery_codes(8);
        assert_eq!(codes.len(), 8);

        for code in &codes {
            assert_eq!(code.len(), 9); // XXXX-XXXX
            let hash = hash_recovery_code(code);
            assert_eq!(hash.len(), 64); // SHA-256 hex

            // Normalization is case/dash insensitive
            let lower_without_dash = code.replace('-', "").to_lowercase();
            let hash2 = hash_recovery_code(&lower_without_dash);
            assert_eq!(hash, hash2);
        }
    }
}
