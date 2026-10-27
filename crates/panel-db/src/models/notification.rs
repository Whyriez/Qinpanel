use serde::{Deserialize, Serialize};

/// Entity representing an in-app system notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationModel {
    pub id: String,
    pub user_id: Option<i64>,
    pub site_id: Option<i64>,
    pub category: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub metadata_json: Option<String>,
    pub action_url: Option<String>,
    pub is_read: bool,
    pub read_at: Option<i64>,
    pub created_at: i64,
}

/// DTO for inserting a new notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateNotificationInput {
    pub id: Option<String>,
    pub user_id: Option<i64>,
    pub site_id: Option<i64>,
    pub category: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub metadata_json: Option<String>,
    pub action_url: Option<String>,
}

/// Filter options when querying notifications.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationFilter {
    pub unread_only: bool,
    pub category: Option<String>,
    pub severity: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

/// Entity representing global notification & SMTP settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettingsModel {
    pub id: i64,
    pub smtp_enabled: bool,
    pub smtp_host: Option<String>,
    pub smtp_port: i64,
    pub smtp_username: Option<String>,
    pub smtp_password_enc: Option<String>,
    pub smtp_encryption: String,
    pub from_address: String,
    pub from_name: String,
    pub admin_email: Option<String>,
    pub ssl_warning_days: i64,
    pub quota_warning_percent: i64,
    pub notify_ssl_expiring: bool,
    pub notify_quota_warning: bool,
    pub notify_job_failed: bool,
    pub notify_deploy_failed: bool,
    pub updated_at: i64,
}

/// DTO for updating notification & SMTP settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateNotificationSettingsInput {
    pub smtp_enabled: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_username: Option<String>,
    pub smtp_password_enc: Option<String>,
    pub smtp_encryption: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub admin_email: Option<String>,
    pub ssl_warning_days: Option<i64>,
    pub quota_warning_percent: Option<i64>,
    pub notify_ssl_expiring: Option<bool>,
    pub notify_quota_warning: Option<bool>,
    pub notify_job_failed: Option<bool>,
    pub notify_deploy_failed: Option<bool>,
}
