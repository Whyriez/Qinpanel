//! `panel-core`: Domain types, newtype validators, and common error definitions.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod audit;
pub mod auth;
pub mod backup;
pub mod capacity;
pub mod connectivity;
pub mod cron;
pub mod database;
pub mod doctor;
pub mod domain;
pub mod git_deploy;
pub mod hardening;
pub mod memory_profile;
pub mod notification;
pub mod object_storage;
pub mod panel_tls;
pub mod php;
pub mod platform;
pub mod production;
pub mod rbac;
pub mod reconcile;
pub mod secret_rotation;
pub mod site_rules;
pub mod ssh;
pub mod system_backup;
pub mod tenant;
pub mod totp;
pub mod update;
pub use auth::{hash_password, hash_token, verify_password};
pub use backup::{
    BackupDatabaseManifestEntry, BackupFormat, BackupManifest, BackupScheduleFrequency,
    BackupStatus, BackupStorageType, generate_backup_filename, validate_backup_filename,
    validate_backup_label, validate_backup_schedule, validate_restore_destination,
    validate_retention_count, validate_s3_config,
};
pub use cron::{
    ParsedCron, cron_to_human_en, cron_to_human_id, cron_to_systemd_calendar,
    validate_cron_expression,
};
pub use database::{
    DatabaseEngine, generate_secure_db_password, tenant_db_prefix, validate_allowed_hosts,
    validate_database_name, validate_database_password, validate_database_user, validate_dump_path,
};
pub use doctor::{
    DoctorCheckCategory, DoctorCheckItem, DoctorCheckStatus, DoctorRemediationResult, DoctorReport,
    format_doctor_report_cli,
};
pub use domain::{DomainName, DomainValidationError};
pub use git_deploy::{
    ParsedWebhookPayload, bytes_to_hex, compute_hmac_sha256, constant_time_compare,
    generate_release_id, parse_github_push_payload, parse_gitlab_push_payload,
    validate_docroot_subpath, validate_git_branch, validate_git_repo_url, verify_github_signature,
    verify_gitlab_token,
};
pub use hardening::{
    ASVS_TARGET_LEVEL, ASVS_VERSION, AsvsControlStatus, AsvsRequirement, AsvsVerificationLevel,
    LOOPBACK_PORT_MAX, LOOPBACK_PORT_MIN, OwaspAsvsAuditReport, SystemdDirective,
    SystemdServiceSecurityScore, evaluate_systemd_service_security, generate_asvs_l2_audit,
    generate_asvs_markdown_report, get_default_service_security_evaluations,
    validate_loopback_isolation_binding,
};
pub use memory_profile::{
    MemoryTargetEvaluation, PanelMemoryProfileReport, ProcessMemorySnapshot, ReleaseProfileInfo,
    TARGET_API_READ_LATENCY_P95_MS, TARGET_COLD_START_MS, TARGET_RAM_AGENT_IDLE_MB,
    TARGET_RAM_API_IDLE_MB, TARGET_RAM_TOTAL_MEDIUM_LOAD_MB, TARGET_WEB_BUNDLE_GZIP_KB,
    build_memory_profile_report, evaluate_prd_memory_targets, generate_markdown_report,
    parse_proc_statm, read_current_process_memory,
};
pub use notification::{
    NotificationCategory, NotificationSeverity, SmtpEncryption, format_alert_email_body,
    format_alert_email_subject, validate_email_address, validate_notification_message,
    validate_notification_title, validate_quota_warning_percent, validate_smtp_host,
    validate_smtp_port, validate_ssl_warning_days,
};
pub use rbac::{Action, Role, authorize, check_permission};
pub use reconcile::{
    DriftItem, DriftType, ReconcileExecutionResult, ReconcileReport, format_reconcile_report_cli,
};
pub use secret_rotation::{DecryptionOutcome, DualKeyRotationVault, RotationError};
pub use ssh::{
    ParsedSshKey, SshConnectionParams, SshKeyError, escape_shell_single_quote,
    generate_sftp_command, generate_sftp_url, generate_ssh_command, generate_ssh_config_snippet,
    parse_ssh_public_key,
};
pub use system_backup::{
    SYSTEM_BACKUP_MANIFEST_VERSION, SYSTEM_BACKUP_TYPE, SystemBackupFileEntry,
    SystemBackupManifest, SystemBackupSummary, SystemBackupVerificationResult,
    compute_bytes_sha256, generate_system_backup_filename, validate_system_backup_filename,
};
pub use tenant::{MAX_TENANT_UID, MIN_TENANT_UID, ShellPolicy, TenantUid, is_reserved_username};
pub use totp::{
    generate_recovery_codes, generate_totp_qr_base64, generate_totp_secret, generate_totp_uri,
    hash_recovery_code, normalize_recovery_code, verify_totp_code,
};

/// Common error types for domain operations.
#[derive(Debug, Error, Serialize, Deserialize)]
pub enum CoreError {
    #[error("Invalid username: {0}")]
    InvalidUsername(String),
    #[error("Invalid domain: {0}")]
    InvalidDomain(String),
    #[error("Invalid relative path: {0}")]
    InvalidPath(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
}

/// Linux tenant username (e.g. `p_alice`).
/// Must match `^p_[a-z0-9_]{1,28}$` and not collide with system users.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Username(String);

impl Username {
    pub fn parse(input: impl Into<String>) -> Result<Self, CoreError> {
        let s = input.into();
        if !s.starts_with("p_") {
            return Err(CoreError::InvalidUsername(
                "Username must start with 'p_' prefix".to_string(),
            ));
        }
        if s.len() < 3 || s.len() > 30 {
            return Err(CoreError::InvalidUsername(
                "Username length must be between 3 and 30 characters".to_string(),
            ));
        }
        let rest = &s[2..];
        if !rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(CoreError::InvalidUsername(
                "Username may only contain lowercase letters, numbers, and underscores".to_string(),
            ));
        }
        if is_reserved_username(&s) {
            return Err(CoreError::InvalidUsername(format!(
                "Username '{}' collides with reserved system user",
                s
            )));
        }
        Ok(Self(s))
    }

    /// Generates a valid Linux tenant username from a user slug/name.
    pub fn from_slug(slug: &str) -> Result<Self, CoreError> {
        let cleaned: String = slug
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        let stripped = cleaned.trim_matches('_');
        let raw = if stripped.starts_with("p_") {
            stripped.to_string()
        } else {
            format!("p_{stripped}")
        };
        Self::parse(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Username {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_username() {
        let u = Username::parse("p_alice").expect("valid username");
        assert_eq!(u.as_str(), "p_alice");
    }

    #[test]
    fn test_invalid_username_no_prefix() {
        assert!(Username::parse("alice").is_err());
    }

    #[test]
    fn test_invalid_username_characters() {
        assert!(Username::parse("p_Alice!").is_err());
    }

    #[test]
    fn test_reserved_username_rejected() {
        assert!(Username::parse("p_root").is_err());
        assert!(Username::parse("p_nginx").is_err());
        assert!(Username::parse("p_admin").is_err());
    }

    #[test]
    fn test_username_from_slug() {
        let u1 = Username::from_slug("My Client").expect("from slug");
        assert_eq!(u1.as_str(), "p_my_client");

        let u2 = Username::from_slug("p_bob").expect("from slug with prefix");
        assert_eq!(u2.as_str(), "p_bob");

        assert!(Username::from_slug("root").is_err());
    }
}

pub mod email;
pub mod file_revision;
