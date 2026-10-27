//! Password hashing and verification using Argon2id.

use crate::CoreError;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

/// Hashes a plaintext password using Argon2id.
pub fn hash_password(password: &str) -> Result<String, CoreError> {
    if password.len() < 8 {
        return Err(CoreError::Validation(
            "Password must be at least 8 characters long".to_string(),
        ));
    }
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| CoreError::Validation(format!("Failed to hash password: {}", e)))?;
    Ok(hash.to_string())
}

/// Verifies a plaintext password against an Argon2id password hash.
pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, CoreError> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|e| CoreError::Validation(format!("Invalid password hash: {}", e)))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Computes SHA-256 hash of a token (e.g. for session tokens or API tokens).
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Hashes a plaintext password using bcrypt for Nginx HTTP Basic Auth htpasswd files.
pub fn hash_basic_auth_password(password: &str) -> Result<String, CoreError> {
    if password.is_empty() {
        return Err(CoreError::Validation(
            "Password must not be empty".to_string(),
        ));
    }
    bcrypt::hash(password, 10)
        .map_err(|e| CoreError::Validation(format!("Failed to hash bcrypt password: {}", e)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify_password() {
        let password = "SuperSecretPassword123!";
        let hash = hash_password(password).expect("hash password");
        assert!(hash.starts_with("$argon2id$"));

        let is_valid = verify_password(password, &hash).expect("verify password");
        assert!(is_valid);

        let is_invalid = verify_password("WrongPassword!", &hash).expect("verify password");
        assert!(!is_invalid);
    }

    #[test]
    fn test_short_password_rejected() {
        assert!(hash_password("short").is_err());
    }
}
