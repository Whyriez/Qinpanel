//! Notification domain types, categories, severities, and validation rules.

use serde::{Deserialize, Serialize};

/// Severity levels for notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

impl NotificationSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "info" => Some(Self::Info),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

/// Notification categories identifying the origin and purpose of the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationCategory {
    SslExpiring,
    QuotaWarning,
    JobFailed,
    DeployFailed,
    BackupFailed,
    Security,
    System,
    AnalyticsInsight,
}

impl NotificationCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SslExpiring => "ssl_expiring",
            Self::QuotaWarning => "quota_warning",
            Self::JobFailed => "job_failed",
            Self::DeployFailed => "deploy_failed",
            Self::BackupFailed => "backup_failed",
            Self::Security => "security",
            Self::System => "system",
            Self::AnalyticsInsight => "analytics_insight",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "ssl_expiring" => Some(Self::SslExpiring),
            "quota_warning" => Some(Self::QuotaWarning),
            "job_failed" => Some(Self::JobFailed),
            "deploy_failed" => Some(Self::DeployFailed),
            "backup_failed" => Some(Self::BackupFailed),
            "security" => Some(Self::Security),
            "system" => Some(Self::System),
            "analytics_insight" => Some(Self::AnalyticsInsight),
            _ => None,
        }
    }
}

/// Supported SMTP encryption modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpEncryption {
    None,
    StartTls,
    Tls,
}

impl SmtpEncryption {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::StartTls => "starttls",
            Self::Tls => "tls",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "starttls" => Some(Self::StartTls),
            "tls" | "ssl" => Some(Self::Tls),
            _ => None,
        }
    }
}

/// Validates notification title length and content.
pub fn validate_notification_title(title: &str) -> Result<(), &'static str> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("Judul notifikasi tidak boleh kosong");
    }
    if trimmed.len() > 200 {
        return Err("Judul notifikasi maksimal 200 karakter");
    }
    if trimmed.chars().any(|c| c.is_control() && c != ' ') {
        return Err("Judul notifikasi tidak boleh mengandung karakter kontrol");
    }
    Ok(())
}

/// Validates notification message length.
pub fn validate_notification_message(message: &str) -> Result<(), &'static str> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Err("Pesan notifikasi tidak boleh kosong");
    }
    if trimmed.len() > 4000 {
        return Err("Pesan notifikasi maksimal 4000 karakter");
    }
    Ok(())
}

/// Validates an SMTP server hostname or IP address.
pub fn validate_smtp_host(host: &str) -> Result<(), &'static str> {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return Err("Host SMTP tidak boleh kosong");
    }
    if trimmed.len() > 255 {
        return Err("Host SMTP maksimal 255 karakter");
    }
    if trimmed
        .chars()
        .any(|c| c.is_whitespace() || c == '/' || c == '\\' || c == ':')
    {
        return Err("Host SMTP tidak valid");
    }
    Ok(())
}

/// Validates an SMTP port.
pub fn validate_smtp_port(port: u16) -> Result<(), &'static str> {
    if port == 0 {
        return Err("Port SMTP tidak valid (harus 1-65535)");
    }
    Ok(())
}

/// Validates an email address format.
pub fn validate_email_address(email: &str) -> Result<(), &'static str> {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return Err("Alamat email tidak boleh kosong");
    }
    if trimmed.len() > 254 {
        return Err("Alamat email maksimal 254 karakter");
    }
    let parts: Vec<&str> = trimmed.split('@').collect();
    if parts.len() != 2 {
        return Err("Format alamat email tidak valid (wajib mengandung tepat satu '@')");
    }
    let local = parts[0];
    let domain = parts[1];
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err("Format alamat email tidak valid");
    }
    if domain.starts_with('.') || domain.ends_with('.') {
        return Err("Domain alamat email tidak valid");
    }
    Ok(())
}

/// Validates the SSL expiration warning threshold in days (1 to 90 days).
pub fn validate_ssl_warning_days(days: u32) -> Result<(), &'static str> {
    if !(1..=90).contains(&days) {
        return Err("Ambang batas kedaluwarsa SSL harus antara 1 dan 90 hari");
    }
    Ok(())
}

/// Validates disk quota warning percentage (50% to 99%).
pub fn validate_quota_warning_percent(percent: u8) -> Result<(), &'static str> {
    if !(50..=99).contains(&percent) {
        return Err("Persentase peringatan kuota harus antara 50% dan 99%");
    }
    Ok(())
}

/// Formats a standardized email subject for alerts.
pub fn format_alert_email_subject(category: NotificationCategory, title: &str) -> String {
    let prefix = match category {
        NotificationCategory::SslExpiring => "[SSL Alert]",
        NotificationCategory::QuotaWarning => "[Quota Alert]",
        NotificationCategory::JobFailed => "[Job Failed]",
        NotificationCategory::DeployFailed => "[Deploy Failed]",
        NotificationCategory::BackupFailed => "[Backup Failed]",
        NotificationCategory::Security => "[Security Alert]",
        NotificationCategory::System => "[System Notice]",
        NotificationCategory::AnalyticsInsight => "[Analytics Alert]",
    };
    format!("{prefix} {title}")
}

/// Formats a standardized plain-text email body for alerts.
pub fn format_alert_email_body(
    category: NotificationCategory,
    title: &str,
    message: &str,
    action_url: Option<&str>,
) -> String {
    let mut body = format!(
        "Pemberitahuan Sistem Qinpanel\n\
        ================================\n\n\
        Kategori : {}\n\
        Judul    : {}\n\n\
        Pesan:\n{}\n\n",
        category.as_str(),
        title,
        message
    );

    if let Some(url) = action_url {
        body.push_str(&format!(
            "Buka tautan berikut untuk memeriksa detail:\n{}\n\n",
            url
        ));
    }

    body.push_str("--\nDikirim secara otomatis oleh Qinpanel.\n");
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_enums() {
        assert_eq!(NotificationSeverity::Warning.as_str(), "warning");
        assert_eq!(
            NotificationSeverity::from_str_opt("critical"),
            Some(NotificationSeverity::Critical)
        );
        assert_eq!(NotificationSeverity::from_str_opt("unknown"), None);

        assert_eq!(NotificationCategory::SslExpiring.as_str(), "ssl_expiring");
        assert_eq!(
            NotificationCategory::from_str_opt("job_failed"),
            Some(NotificationCategory::JobFailed)
        );
        assert_eq!(NotificationCategory::from_str_opt("unknown"), None);

        assert_eq!(SmtpEncryption::StartTls.as_str(), "starttls");
        assert_eq!(
            SmtpEncryption::from_str_opt("tls"),
            Some(SmtpEncryption::Tls)
        );
    }

    #[test]
    fn test_validations() {
        assert!(validate_notification_title("Sertifikat SSL Hampir Kedaluwarsa").is_ok());
        assert!(validate_notification_title("").is_err());
        assert!(validate_notification_title(&"a".repeat(201)).is_err());

        assert!(validate_notification_message("Detail pesan peringatan kuota disk").is_ok());
        assert!(validate_notification_message("").is_err());

        assert!(validate_smtp_host("smtp.mailgun.org").is_ok());
        assert!(validate_smtp_host("127.0.0.1").is_ok());
        assert!(validate_smtp_host("").is_err());
        assert!(validate_smtp_host("smtp:server").is_err());

        assert!(validate_smtp_port(587).is_ok());
        assert!(validate_smtp_port(0).is_err());

        assert!(validate_email_address("admin@example.com").is_ok());
        assert!(validate_email_address("invalid").is_err());
        assert!(validate_email_address("user@").is_err());
        assert!(validate_email_address("@domain.com").is_err());

        assert!(validate_ssl_warning_days(30).is_ok());
        assert!(validate_ssl_warning_days(0).is_err());
        assert!(validate_ssl_warning_days(91).is_err());

        assert!(validate_quota_warning_percent(90).is_ok());
        assert!(validate_quota_warning_percent(40).is_err());
        assert!(validate_quota_warning_percent(100).is_err());
    }

    #[test]
    fn test_email_formatters() {
        let subject = format_alert_email_subject(
            NotificationCategory::SslExpiring,
            "Domain example.com tersisa 7 hari",
        );
        assert!(subject.starts_with("[SSL Alert]"));

        let body = format_alert_email_body(
            NotificationCategory::QuotaWarning,
            "Kuota disk > 90%",
            "Tenant p_alice telah menggunakan 9.2 GB dari 10 GB.",
            Some("https://panel.local/sites/1"),
        );
        assert!(body.contains("quota_warning"));
        assert!(body.contains("https://panel.local/sites/1"));
    }
}
