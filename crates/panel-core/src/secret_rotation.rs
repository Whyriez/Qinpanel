//! Cryptographic secret rotation manager with dual-key graceful decryption fallback.
//!
//! Enables zero-downtime rotation of master encryption keys and session secrets.
//! Ciphertexts encrypted with the prior key can still be decrypted gracefully,
//! and automatically migrated/re-encrypted with the active primary key.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAGIC_HEADER: &[u8] = b"VP01";
pub const NONCE_LEN: usize = 12;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RotationError {
    #[error("Base64 decoding failed: {0}")]
    Base64Decode(String),
    #[error("Ciphertext payload is too short or malformed")]
    PayloadTooShort,
    #[error("Invalid magic header: expected VP01")]
    InvalidHeader,
    #[error("Decryption failed with both primary and secondary keys")]
    DecryptionFailed,
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
}

/// Indicates whether decryption succeeded via the current primary key or fallback secondary key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecryptionOutcome {
    PrimaryKey,
    FallbackSecondaryKey,
}

/// Dual-key encryption vault supporting zero-downtime secret rotation.
#[derive(Debug, Clone)]
pub struct DualKeyRotationVault {
    primary_key: [u8; 32],
    secondary_key: Option<[u8; 32]>,
    key_version: u32,
}

impl DualKeyRotationVault {
    /// Initializes a new vault with a single primary key (version 1).
    pub fn new(primary_key: [u8; 32]) -> Self {
        Self {
            primary_key,
            secondary_key: None,
            key_version: 1,
        }
    }

    /// Initializes a vault with both active primary and historical secondary fallback key.
    pub fn with_secondary(
        primary_key: [u8; 32],
        secondary_key: [u8; 32],
        key_version: u32,
    ) -> Self {
        Self {
            primary_key,
            secondary_key: Some(secondary_key),
            key_version,
        }
    }

    /// Returns the current active key version.
    pub fn key_version(&self) -> u32 {
        self.key_version
    }

    /// Checks if a secondary fallback key is currently active.
    pub fn has_secondary(&self) -> bool {
        self.secondary_key.is_some()
    }

    /// Rotates keys: the existing primary becomes secondary fallback, and new_primary becomes active.
    pub fn rotate(&mut self, new_primary_key: [u8; 32]) {
        self.secondary_key = Some(self.primary_key);
        self.primary_key = new_primary_key;
        self.key_version += 1;
    }

    /// Generates a new random 256-bit key using OS entropy.
    pub fn generate_random_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        key
    }

    /// Encrypts plaintext using the active primary key.
    ///
    /// Encrypted payload format:
    /// `[MAGIC_HEADER (4 bytes)] + [NONCE (12 bytes)] + [CIPHERTEXT + TAG]`
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<String, RotationError> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.primary_key));
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| RotationError::EncryptionFailed(e.to_string()))?;

        let mut payload = Vec::with_capacity(MAGIC_HEADER.len() + NONCE_LEN + ciphertext.len());
        payload.extend_from_slice(MAGIC_HEADER);
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);

        Ok(BASE64.encode(payload))
    }

    /// Decrypts a base64 encoded ciphertext, attempting primary key first,
    /// falling back to secondary key if primary key fails.
    pub fn decrypt(&self, encoded: &str) -> Result<(Vec<u8>, DecryptionOutcome), RotationError> {
        let raw = BASE64
            .decode(encoded.trim())
            .map_err(|e| RotationError::Base64Decode(e.to_string()))?;

        if raw.len() < MAGIC_HEADER.len() + NONCE_LEN {
            return Err(RotationError::PayloadTooShort);
        }

        if &raw[..MAGIC_HEADER.len()] != MAGIC_HEADER {
            return Err(RotationError::InvalidHeader);
        }

        let nonce_slice = &raw[MAGIC_HEADER.len()..MAGIC_HEADER.len() + NONCE_LEN];
        let ciphertext = &raw[MAGIC_HEADER.len() + NONCE_LEN..];
        let nonce = Nonce::from_slice(nonce_slice);

        // 1. Try primary key
        let primary_cipher = ChaCha20Poly1305::new(Key::from_slice(&self.primary_key));
        if let Ok(plaintext) = primary_cipher.decrypt(nonce, ciphertext) {
            return Ok((plaintext, DecryptionOutcome::PrimaryKey));
        }

        // 2. Try secondary fallback key if present
        if let Some(sec_key) = &self.secondary_key {
            let secondary_cipher = ChaCha20Poly1305::new(Key::from_slice(sec_key));
            if let Ok(plaintext) = secondary_cipher.decrypt(nonce, ciphertext) {
                return Ok((plaintext, DecryptionOutcome::FallbackSecondaryKey));
            }
        }

        Err(RotationError::DecryptionFailed)
    }

    /// If ciphertext was encrypted with an older secondary key, returns a newly encrypted
    /// ciphertext using the primary key to facilitate seamless online migration.
    pub fn re_encrypt_if_needed(&self, encoded: &str) -> Result<Option<String>, RotationError> {
        let (plaintext, outcome) = self.decrypt(encoded)?;
        match outcome {
            DecryptionOutcome::PrimaryKey => Ok(None),
            DecryptionOutcome::FallbackSecondaryKey => {
                let upgraded = self.encrypt(&plaintext)?;
                Ok(Some(upgraded))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip_primary() {
        let key1 = [0x42u8; 32];
        let vault = DualKeyRotationVault::new(key1);

        let data = b"DATABASE_URL=postgres://user:pass@127.0.0.1:5432/app";
        let encrypted = vault.encrypt(data).expect("encryption");
        let (decrypted, outcome) = vault.decrypt(&encrypted).expect("decryption");

        assert_eq!(decrypted, data);
        assert_eq!(outcome, DecryptionOutcome::PrimaryKey);
    }

    #[test]
    fn test_secret_rotation_graceful_fallback() {
        let key1 = [0x11u8; 32];
        let mut vault = DualKeyRotationVault::new(key1);

        let secret = b"STRIPE_API_KEY=sk_live_very_secret_token";
        let cipher1 = vault.encrypt(secret).expect("encrypt with key 1");

        // Rotate vault to key 2
        let key2 = [0x22u8; 32];
        vault.rotate(key2);
        assert_eq!(vault.key_version(), 2);
        assert!(vault.has_secondary());

        // Decrypt with vault containing key 2 (primary) and key 1 (secondary fallback)
        let (decrypted, outcome) = vault.decrypt(&cipher1).expect("fallback decrypt");
        assert_eq!(decrypted, secret);
        assert_eq!(outcome, DecryptionOutcome::FallbackSecondaryKey);

        // Re-encrypt online upgrade
        let re_encrypted = vault
            .re_encrypt_if_needed(&cipher1)
            .expect("re-encrypt")
            .expect("should produce updated ciphertext");

        // Now decrypt upgraded ciphertext -> should be DecryptionOutcome::PrimaryKey
        let (dec2, outcome2) = vault.decrypt(&re_encrypted).expect("decrypt upgraded");
        assert_eq!(dec2, secret);
        assert_eq!(outcome2, DecryptionOutcome::PrimaryKey);
    }

    #[test]
    fn test_two_rotations_evicts_oldest_key() {
        let key1 = [0x01u8; 32];
        let mut vault = DualKeyRotationVault::new(key1);
        let secret = b"MY_SECRET";
        let cipher1 = vault.encrypt(secret).expect("encrypt key1");

        let key2 = [0x02u8; 32];
        vault.rotate(key2);

        let key3 = [0x03u8; 32];
        vault.rotate(key3); // key2 becomes secondary, key1 is dropped

        // key1 cipher should now fail decryption
        let err = vault.decrypt(&cipher1);
        assert_eq!(err, Err(RotationError::DecryptionFailed));
    }
}
