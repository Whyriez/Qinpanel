//! System Backup: Snapshot, verification, and restoration of panel SQLite database and system configurations.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SYSTEM_BACKUP_MANIFEST_VERSION: &str = "1.0";
pub const SYSTEM_BACKUP_TYPE: &str = "system_db_config";

/// An entry of an individual file stored in the system backup archive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemBackupFileEntry {
    pub relative_path: String,
    pub file_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Manifest containing metadata and checksums of the system backup snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemBackupManifest {
    pub version: String,
    pub created_at: u64,
    pub backup_type: String,
    pub panel_version: String,
    pub files: Vec<SystemBackupFileEntry>,
    pub total_uncompressed_bytes: u64,
    pub archive_sha256: Option<String>,
}

impl SystemBackupManifest {
    pub fn new(
        created_at: u64,
        panel_version: impl Into<String>,
        files: Vec<SystemBackupFileEntry>,
    ) -> Self {
        let total_uncompressed_bytes = files.iter().map(|f| f.size_bytes).sum();
        Self {
            version: SYSTEM_BACKUP_MANIFEST_VERSION.to_string(),
            created_at,
            backup_type: SYSTEM_BACKUP_TYPE.to_string(),
            panel_version: panel_version.into(),
            files,
            total_uncompressed_bytes,
            archive_sha256: None,
        }
    }

    /// Verifies that all expected entries in manifest have valid non-empty paths and hashes.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != SYSTEM_BACKUP_MANIFEST_VERSION {
            return Err(format!(
                "Versi manifest tidak didukung: diharapkan {}, didapat {}",
                SYSTEM_BACKUP_MANIFEST_VERSION, self.version
            ));
        }
        if self.files.is_empty() {
            return Err("Manifest cadangan tidak memuat berkas apapun".to_string());
        }
        for file in &self.files {
            if file.relative_path.is_empty() {
                return Err("Terdapat entri berkas dengan jalur relatif kosong".to_string());
            }
            if file.sha256.len() != 64 {
                return Err(format!(
                    "Checksum SHA-256 tidak valid untuk {}: {}",
                    file.relative_path, file.sha256
                ));
            }
        }
        Ok(())
    }
}

/// Metadata summary of an existing system backup archive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemBackupSummary {
    pub filename: String,
    pub filepath: String,
    pub created_at: u64,
    pub size_bytes: u64,
    pub checksum_sha256: String,
    pub files_count: usize,
    pub verified: bool,
}

/// Result of verifying a system backup archive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemBackupVerificationResult {
    pub is_valid: bool,
    pub manifest: Option<SystemBackupManifest>,
    pub errors: Vec<String>,
}

/// Generates standard filename for a system panel backup.
pub fn generate_system_backup_filename(timestamp: u64) -> String {
    format!("panel-backup-{}.tar.gz", timestamp)
}

/// Validates whether a filename matches the standard system backup naming convention.
pub fn validate_system_backup_filename(filename: &str) -> bool {
    if !filename.starts_with("panel-backup-") {
        return false;
    }
    if !(filename.ends_with(".tar.gz") || filename.ends_with(".tar.zst")) {
        return false;
    }
    // Reject directory traversal
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return false;
    }
    true
}

/// Computes SHA256 hex string for a byte slice.
pub fn compute_bytes_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_backup_manifest_validation() {
        let files = vec![
            SystemBackupFileEntry {
                relative_path: "database/panel.db".to_string(),
                file_type: "sqlite_db".to_string(),
                size_bytes: 1048576,
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
            },
            SystemBackupFileEntry {
                relative_path: "config/panel.toml".to_string(),
                file_type: "config_toml".to_string(),
                size_bytes: 2048,
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
            },
        ];

        let manifest = SystemBackupManifest::new(1720000000, "0.1.0", files);
        assert_eq!(manifest.total_uncompressed_bytes, 1048576 + 2048);
        assert!(manifest.validate().is_ok());

        let fname = generate_system_backup_filename(1720000000);
        assert!(validate_system_backup_filename(&fname));
        assert!(!validate_system_backup_filename("../evil.tar.gz"));
        assert!(!validate_system_backup_filename("other.zip"));
    }
}
