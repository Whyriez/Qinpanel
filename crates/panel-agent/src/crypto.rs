//! Cryptographic module for encrypting tenant environment variables.
//!
//! Complies with PRD Section 6 (Keamanan) & Phase 8 (P8-T1):
//! - Encryption/decryption is strictly performed inside the agent daemon.
//! - Algorithm: XChaCha20-Poly1305 (256-bit key, 192-bit nonce, 128-bit tag).
//! - Master key is stored securely at `/etc/panel/secret.key` (mode 0600 root).
//! - Environment files written to `/etc/panel/env/site-<site_id>.env` (mode 0600 root).
//! - Neither database nor audit logs ever store plaintext secrets.

use crate::AgentError;
use crate::system::SystemRunner;
use base64::prelude::*;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use rand_core::{OsRng, RngCore};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_MASTER_KEY_PATH: &str = "/etc/panel/secret.key";
pub const DEFAULT_ENV_DIR: &str = "/etc/panel/env";
pub const CIPHER_PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
static MASTER_KEY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Loads or creates the 32-byte master key for tenant secret encryption.
pub fn get_or_create_master_key(
    runner: &dyn SystemRunner,
    custom_key_path: Option<&str>,
) -> Result<[u8; 32], AgentError> {
    let _guard = MASTER_KEY_LOCK
        .lock()
        .map_err(|_| AgentError::Internal("Master key lock tidak tersedia".into()))?;
    let key_path_str = custom_key_path.unwrap_or(DEFAULT_MASTER_KEY_PATH);
    let key_path = Path::new(key_path_str);

    if runner.path_exists(key_path) {
        let bytes = runner.read_file(key_path)?;
        if bytes.len() != 32 {
            return Err(AgentError::SecurityViolation(format!(
                "Kunci enkripsi agent pada {:?} tidak valid (panjang {} bytes, diharapkan 32)",
                key_path,
                bytes.len()
            )));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        return Ok(key);
    }

    // Generate fresh 32 bytes using OS CSPRNG
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);

    // Save with mode 0600 (read/write only by root/agent)
    runner.write_file(key_path, &key, 0o600)?;

    Ok(key)
}

/// Encrypts a plaintext string using XChaCha20-Poly1305.
/// Output format: `enc:v1:<base64(24_byte_nonce || ciphertext_with_tag)>`
pub fn encrypt_value(key: &[u8; 32], plaintext: &str) -> Result<String, AgentError> {
    if plaintext.starts_with(CIPHER_PREFIX) {
        // Already encrypted
        return Ok(plaintext.to_string());
    }

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);

    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| AgentError::Internal(format!("Inisialisasi cipher gagal: {}", e)))?;

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| AgentError::Internal(format!("Enkripsi AEAD gagal: {}", e)))?;

    let mut payload = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);

    let encoded = BASE64_STANDARD.encode(&payload);
    Ok(format!("{}{}", CIPHER_PREFIX, encoded))
}

/// Decrypts an encrypted string (`enc:v1:...`) using XChaCha20-Poly1305.
/// If string is not prefixed with `enc:v1:`, returns it as-is.
pub fn decrypt_value(key: &[u8; 32], encrypted: &str) -> Result<String, AgentError> {
    if !encrypted.starts_with(CIPHER_PREFIX) {
        return Ok(encrypted.to_string());
    }

    let b64_part = &encrypted[CIPHER_PREFIX.len()..];
    let payload = BASE64_STANDARD
        .decode(b64_part)
        .map_err(|e| AgentError::InvalidInput(format!("Payload base64 tidak valid: {}", e)))?;

    if payload.len() < NONCE_LEN + TAG_LEN {
        return Err(AgentError::InvalidInput(
            "Payload ciphertext terlalu pendek".to_string(),
        ));
    }

    let (nonce_bytes, ciphertext) = payload.split_at(NONCE_LEN);
    let nonce = XNonce::from_slice(nonce_bytes);

    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| AgentError::Internal(format!("Inisialisasi cipher gagal: {}", e)))?;

    let decrypted_bytes = cipher.decrypt(nonce, ciphertext).map_err(|e| {
        AgentError::SecurityViolation(format!(
            "Dekripsi gagal (tag mismatch atau ciphertext korup): {}",
            e
        ))
    })?;

    String::from_utf8(decrypted_bytes)
        .map_err(|e| AgentError::InvalidInput(format!("Plaintext bukan UTF-8 valid: {}", e)))
}

/// Encrypts all environment variables in a map.
pub fn encrypt_env_vars(
    key: &[u8; 32],
    vars: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AgentError> {
    let mut out = BTreeMap::new();
    for (k, v) in vars {
        validate_env_key(k)?;
        let encrypted = encrypt_value(key, v)?;
        out.insert(k.clone(), encrypted);
    }
    Ok(out)
}

/// Decrypts all environment variables in a map.
pub fn decrypt_env_vars(
    key: &[u8; 32],
    vars: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AgentError> {
    let mut out = BTreeMap::new();
    for (k, v) in vars {
        validate_env_key(k)?;
        let decrypted = decrypt_value(key, v)?;
        out.insert(k.clone(), decrypted);
    }
    Ok(out)
}

/// Validates that an environment variable key conforms to standard POSIX naming:
/// Must start with a letter or underscore, and contain only alphanumeric and underscores.
pub fn validate_env_key(key: &str) -> Result<(), AgentError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(AgentError::InvalidInput(
            "Nama environment variable tidak boleh kosong".to_string(),
        ));
    }
    let first = trimmed.chars().next().unwrap_or('_');
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(AgentError::InvalidInput(format!(
            "Nama variable '{}' harus diawali dengan huruf atau underscore",
            trimmed
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AgentError::InvalidInput(format!(
            "Nama variable '{}' mengandung karakter tidak valid",
            trimmed
        )));
    }
    Ok(())
}

/// Generates systemd / POSIX `.env` file content from decrypted variables.
pub fn generate_env_content(vars: &BTreeMap<String, String>) -> Result<String, AgentError> {
    let mut out = String::new();
    for (k, v) in vars {
        if k.trim().is_empty() {
            continue;
        }
        validate_env_key(k)?;
        // Handle quotes, spaces, newlines, tabs
        if v.contains(' ')
            || v.contains('\t')
            || v.contains('\n')
            || v.contains('"')
            || v.contains('\'')
        {
            let escaped = v
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            out.push_str(&format!("{}=\"{}\"\n", k, escaped));
        } else {
            out.push_str(&format!("{}={}\n", k, v));
        }
    }
    Ok(out)
}

/// Applies environment variables by decrypting them in-memory and writing the `.env` file
/// with mode `0600` (root-readable only, systemd EnvironmentFile loads it prior to privilege drop).
pub fn apply_site_env_file(
    runner: &dyn SystemRunner,
    site_id: &str,
    _username: &str,
    env_file_path: Option<&str>,
    encrypted_vars: &BTreeMap<String, String>,
    custom_key_path: Option<&str>,
) -> Result<String, AgentError> {
    if site_id.trim().is_empty() {
        return Err(AgentError::InvalidInput("Site ID kosong".to_string()));
    }

    let master_key = get_or_create_master_key(runner, custom_key_path)?;
    let decrypted_vars = decrypt_env_vars(&master_key, encrypted_vars)?;
    let content = generate_env_content(&decrypted_vars)?;

    let target_path_str = env_file_path
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}/site-{}.env", DEFAULT_ENV_DIR, site_id));
    let target_path = PathBuf::from(&target_path_str);

    // Write file with permissions 0600
    runner.write_file(&target_path, content.as_bytes(), 0o600)?;

    Ok(target_path_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;

    #[test]
    fn test_master_key_generation_and_persistence() {
        let runner = MockSystemRunner::new();
        let key_path = "/tmp/test-secret.key";

        // 1. First call generates key
        let key1 = get_or_create_master_key(&runner, Some(key_path)).expect("generate key");
        assert_eq!(key1.len(), 32);

        // 2. Second call loads exact same key
        let key2 = get_or_create_master_key(&runner, Some(key_path)).expect("load key");
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_encryption_and_decryption_roundtrip() {
        let runner = MockSystemRunner::new();
        let key = get_or_create_master_key(&runner, Some("/tmp/key.bin")).expect("key");

        let plaintext = "postgresql://user:p@ssw0rd!#%^&*()@127.0.0.1:5432/production_db";
        let encrypted = encrypt_value(&key, plaintext).expect("encrypt");

        assert!(encrypted.starts_with(CIPHER_PREFIX));
        assert_ne!(plaintext, encrypted);

        let decrypted = decrypt_value(&key, &encrypted).expect("decrypt");
        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_map_encryption_and_decryption() {
        let runner = MockSystemRunner::new();
        let key = get_or_create_master_key(&runner, Some("/tmp/key2.bin")).expect("key");

        let mut vars = BTreeMap::new();
        vars.insert(
            "DATABASE_URL".to_string(),
            "mysql://app:secret@db/prod".to_string(),
        );
        vars.insert("API_SECRET".to_string(), "super-secret-key-123".to_string());
        vars.insert("PORT".to_string(), "3000".to_string());

        let encrypted_vars = encrypt_env_vars(&key, &vars).expect("encrypt map");
        for (k, enc) in &encrypted_vars {
            assert!(enc.starts_with(CIPHER_PREFIX));
            assert_ne!(&vars[k], enc);
        }

        let decrypted_vars = decrypt_env_vars(&key, &encrypted_vars).expect("decrypt map");
        assert_eq!(vars, decrypted_vars);
    }

    #[test]
    fn test_apply_site_env_file() {
        let runner = MockSystemRunner::new();
        let key = get_or_create_master_key(&runner, Some("/tmp/key3.bin")).expect("key");

        let mut vars = BTreeMap::new();
        vars.insert("STRIPE_KEY".to_string(), "sk_live_123456789".to_string());
        let encrypted_vars = encrypt_env_vars(&key, &vars).expect("encrypt");

        let path = apply_site_env_file(
            &runner,
            "mysite",
            "u1001",
            Some("/tmp/env/mysite.env"),
            &encrypted_vars,
            Some("/tmp/key3.bin"),
        )
        .expect("apply env");

        assert_eq!(path, "/tmp/env/mysite.env");

        let content = runner
            .read_file(Path::new("/tmp/env/mysite.env"))
            .expect("read env file");
        let content_str = String::from_utf8(content).expect("utf8");
        assert!(content_str.contains("STRIPE_KEY=sk_live_123456789"));
    }

    #[test]
    fn test_corrupt_ciphertext_fails_gracefully() {
        let runner = MockSystemRunner::new();
        let key = get_or_create_master_key(&runner, Some("/tmp/key4.bin")).expect("key");

        let plaintext = "secret_data";
        let encrypted = encrypt_value(&key, plaintext).expect("encrypt");

        // Corrupt the base64 string
        let mut corrupted = encrypted.clone();
        corrupted.replace_range(15..20, "AAAAA");

        let res = decrypt_value(&key, &corrupted);
        assert!(res.is_err());
    }

    #[test]
    fn test_invalid_key_names() {
        assert!(validate_env_key("VALID_KEY_1").is_ok());
        assert!(validate_env_key("_VALID_2").is_ok());
        assert!(validate_env_key("1INVALID").is_err());
        assert!(validate_env_key("INVALID-DASH").is_err());
        assert!(validate_env_key("INVALID SPACE").is_err());
        assert!(validate_env_key("").is_err());
    }
}
