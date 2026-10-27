//! Domain logic, validations, and manifest models for site backups and restores.

use serde::{Deserialize, Serialize};

/// Supported backup archive formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupFormat {
    #[serde(rename = "tar.zst")]
    TarZst,
    #[serde(rename = "tar.gz")]
    TarGz,
}

impl BackupFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::TarZst => "tar.zst",
            Self::TarGz => "tar.gz",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "tar.gz" | "gz" | "gzip" => Self::TarGz,
            _ => Self::TarZst,
        }
    }
}

/// Backup execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Restoring,
}

impl BackupStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Restoring => "restoring",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "restoring" => Some(Self::Restoring),
            _ => None,
        }
    }
}

/// Frequency schedule for automatic backups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupScheduleFrequency {
    Daily,
    Weekly,
    Monthly,
}

impl BackupScheduleFrequency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            _ => None,
        }
    }
}

/// Storage target for backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupStorageType {
    Local,
    S3,
}

impl BackupStorageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::S3 => "s3",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "local" => Some(Self::Local),
            "s3" => Some(Self::S3),
            _ => None,
        }
    }
}

/// Metadata entry for a database included inside a backup snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupDatabaseManifestEntry {
    pub engine: String,
    pub name: String,
    pub username: String,
    pub dump_file: String,
    pub size_bytes: u64,
}

/// Manifest saved inside the backup archive as `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupManifest {
    pub version: u32,
    pub site_id: i64,
    pub primary_domain: String,
    pub created_at: i64,
    pub includes_files: bool,
    pub includes_databases: bool,
    pub databases: Vec<BackupDatabaseManifestEntry>,
    pub files_count: u64,
    pub uncompressed_bytes: u64,
    pub archive_format: String,
    pub panel_version: String,
}

impl Default for BackupManifest {
    fn default() -> Self {
        Self {
            version: 1,
            site_id: 0,
            primary_domain: String::new(),
            created_at: 0,
            includes_files: true,
            includes_databases: false,
            databases: Vec::new(),
            files_count: 0,
            uncompressed_bytes: 0,
            archive_format: "tar.zst".to_string(),
            panel_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// Validates user-provided label for a backup.
pub fn validate_backup_label(label: &str) -> Result<(), &'static str> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.len() > 100 {
        return Err("Label cadangan maksimal 100 karakter");
    }
    if trimmed.contains('\n') || trimmed.contains('\r') || trimmed.contains('\0') {
        return Err("Label cadangan mengandung karakter tidak valid");
    }
    Ok(())
}

/// Validates backup filename format (must not contain path traversal).
pub fn validate_backup_filename(filename: &str) -> Result<(), &'static str> {
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        return Err("Nama berkas cadangan tidak boleh kosong");
    }
    if trimmed.len() > 128 {
        return Err("Nama berkas cadangan terlalu panjang (maksimal 128 karakter)");
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("Nama berkas cadangan tidak boleh mengandung jalur direktori atau traversal");
    }
    if !trimmed.ends_with(".tar.zst") && !trimmed.ends_with(".tar.gz") {
        return Err("Ekstensi berkas cadangan harus berupa .tar.zst atau .tar.gz");
    }
    // Only allow alphanumeric, dash, underscore, and dot
    for ch in trimmed.chars() {
        if !ch.is_alphanumeric() && ch != '-' && ch != '_' && ch != '.' {
            return Err(
                "Nama berkas cadangan hanya boleh berupa alfanumerik, titik, tanda hubung, dan garis bawah",
            );
        }
    }
    Ok(())
}

/// Generates a standardized backup filename.
pub fn generate_backup_filename(
    primary_domain: &str,
    timestamp: i64,
    format: BackupFormat,
) -> String {
    let sanitized_domain: String = primary_domain
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!(
        "backup_{}_{}.{}",
        sanitized_domain,
        timestamp,
        format.extension()
    )
}

/// Validates restore destination path against directory traversal.
/// Returns Ok(()) if destination is strictly confined inside `tenant_home`.
pub fn validate_restore_destination(
    dest_path: &str,
    tenant_home: &str,
) -> Result<(), &'static str> {
    let dest = dest_path.trim();
    if dest.is_empty() {
        return Err("Target direktori pemulihan tidak boleh kosong");
    }
    if dest.contains('\0') {
        return Err("Jalur tujuan mengandung karakter null byte");
    }
    if dest.contains("..") {
        return Err("Jalur tujuan tidak boleh mengandung traversal ..");
    }

    let dest_norm = dest.replace('\\', "/");
    let home_norm = tenant_home.trim().replace('\\', "/");

    // Must be within tenant_home
    if !dest_norm.starts_with(&home_norm) {
        return Err("Target pemulihan harus berada di dalam direktori kerja tenant");
    }

    Ok(())
}

/// Validates retention count (number of backups to retain).
pub fn validate_retention_count(retention: u32) -> Result<(), &'static str> {
    if retention == 0 {
        return Err("Jumlah retensi cadangan minimal 1");
    }
    if retention > 100 {
        return Err("Jumlah retensi cadangan maksimal 100");
    }
    Ok(())
}

/// Validates backup schedule parameters.
pub fn validate_backup_schedule(
    frequency: &str,
    hour: u32,
    minute: u32,
) -> Result<(), &'static str> {
    if BackupScheduleFrequency::parse(frequency).is_none() {
        return Err("Frekuensi jadwal tidak valid (gunakan daily, weekly, atau monthly)");
    }
    if hour > 23 {
        return Err("Jam eksekusi jadwal harus antara 0 dan 23");
    }
    if minute > 59 {
        return Err("Menit eksekusi jadwal harus antara 0 dan 59");
    }
    Ok(())
}

/// Validates S3-compatible remote storage configuration.
pub fn validate_s3_config(
    endpoint: &str,
    bucket: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
) -> Result<(), &'static str> {
    let ep = endpoint.trim();
    if ep.is_empty() {
        return Err("S3 endpoint tidak boleh kosong");
    }
    if !ep.starts_with("http://") && !ep.starts_with("https://") {
        return Err("S3 endpoint harus diawali dengan http:// atau https://");
    }

    let b = bucket.trim();
    if b.is_empty() {
        return Err("Nama bucket S3 tidak boleh kosong");
    }
    if b.len() < 3 || b.len() > 63 {
        return Err("Nama bucket S3 harus antara 3 dan 63 karakter");
    }
    for ch in b.chars() {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '-' && ch != '.' {
            return Err(
                "Nama bucket S3 hanya boleh berisi huruf kecil, angka, tanda hubung, dan titik",
            );
        }
    }

    if region.trim().is_empty() {
        return Err("Region S3 tidak boleh kosong");
    }
    if access_key.trim().is_empty() {
        return Err("Access Key S3 tidak boleh kosong");
    }
    if secret_key.trim().is_empty() {
        return Err("Secret Key S3 tidak boleh kosong");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_format_and_extensions() {
        assert_eq!(BackupFormat::TarZst.extension(), "tar.zst");
        assert_eq!(BackupFormat::TarGz.extension(), "tar.gz");
        assert_eq!(BackupFormat::from_str_loose("tar.gz"), BackupFormat::TarGz);
        assert_eq!(BackupFormat::from_str_loose("gzip"), BackupFormat::TarGz);
        assert_eq!(
            BackupFormat::from_str_loose("tar.zst"),
            BackupFormat::TarZst
        );
        assert_eq!(
            BackupFormat::from_str_loose("unknown"),
            BackupFormat::TarZst
        );
    }

    #[test]
    fn test_validate_backup_label() {
        assert!(validate_backup_label("").is_ok());
        assert!(validate_backup_label("Sebelum update plugin").is_ok());
        assert!(validate_backup_label(&"x".repeat(101)).is_err());
        assert!(validate_backup_label("Label\nDengan\nNewline").is_err());
    }

    #[test]
    fn test_validate_backup_filename() {
        assert!(validate_backup_filename("backup_example_com_1700000000.tar.zst").is_ok());
        assert!(validate_backup_filename("backup_site_1.tar.gz").is_ok());
        assert!(validate_backup_filename("").is_err());
        assert!(validate_backup_filename("../escape.tar.zst").is_err());
        assert!(validate_backup_filename("/etc/passwd.tar.zst").is_err());
        assert!(validate_backup_filename("backup.zip").is_err());
        assert!(validate_backup_filename("backup space.tar.zst").is_err());
    }

    #[test]
    fn test_generate_backup_filename() {
        let name = generate_backup_filename("my-domain.com", 1700000000, BackupFormat::TarZst);
        assert_eq!(name, "backup_my_domain_com_1700000000.tar.zst");
    }

    #[test]
    fn test_validate_restore_destination() {
        assert!(validate_restore_destination("/home/alice/sites/mysite", "/home/alice").is_ok());
        assert!(validate_restore_destination("/home/alice/restored_site", "/home/alice").is_ok());
        assert!(validate_restore_destination("/home/alice/../bob/site", "/home/alice").is_err());
        assert!(validate_restore_destination("/var/www/site", "/home/alice").is_err());
    }

    #[test]
    fn test_validate_retention_and_schedule() {
        assert!(validate_retention_count(1).is_ok());
        assert!(validate_retention_count(30).is_ok());
        assert!(validate_retention_count(0).is_err());
        assert!(validate_retention_count(101).is_err());

        assert!(validate_backup_schedule("daily", 3, 30).is_ok());
        assert!(validate_backup_schedule("weekly", 0, 0).is_ok());
        assert!(validate_backup_schedule("invalid", 1, 0).is_err());
        assert!(validate_backup_schedule("daily", 24, 0).is_err());
        assert!(validate_backup_schedule("daily", 12, 60).is_err());
    }

    #[test]
    fn test_validate_s3_config() {
        assert!(
            validate_s3_config(
                "https://s3.ap-southeast-1.amazonaws.com",
                "my-vpanel-backups",
                "ap-southeast-1",
                "AKIAIOSFODNN7EXAMPLE",
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
            )
            .is_ok()
        );

        assert!(validate_s3_config("ftp://invalid.com", "bucket", "region", "ak", "sk").is_err());

        assert!(
            validate_s3_config(
                "https://s3.us-east-1.amazonaws.com",
                "INVALID_UPPERCASE",
                "us-east-1",
                "ak",
                "sk"
            )
            .is_err()
        );
    }

    #[test]
    fn test_manifest_serialization() {
        let mut manifest = BackupManifest {
            site_id: 42,
            primary_domain: "blog.example.com".to_string(),
            created_at: 1700000000,
            ..Default::default()
        };
        manifest.databases.push(BackupDatabaseManifestEntry {
            engine: "mariadb".to_string(),
            name: "p_alice_blog".to_string(),
            username: "p_alice_user".to_string(),
            dump_file: "databases/p_alice_blog.sql".to_string(),
            size_bytes: 1048576,
        });

        let json = serde_json::to_string(&manifest).expect("manifest should serialize");
        let parsed: BackupManifest =
            serde_json::from_str(&json).expect("manifest should deserialize");
        assert_eq!(parsed.site_id, 42);
        assert_eq!(parsed.primary_domain, "blog.example.com");
        assert_eq!(parsed.databases.len(), 1);
        assert_eq!(parsed.databases[0].name, "p_alice_blog");
    }
}
