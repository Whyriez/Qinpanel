//! SQLite repository for notifications and alert settings.

use crate::error::DbError;
use crate::models::{
    CreateNotificationInput, NotificationFilter, NotificationModel, NotificationSettingsModel,
    UpdateNotificationSettingsInput,
};
use crate::pool::DbPool;
use sqlx::Row;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone)]
pub struct SqliteNotificationRepository {
    pool: DbPool,
}

impl SqliteNotificationRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Inserts a new notification record.
    pub async fn create(
        &self,
        input: &CreateNotificationInput,
    ) -> Result<NotificationModel, DbError> {
        let id = input
            .id
            .clone()
            .unwrap_or_else(|| format!("notif_{}", Uuid::new_v4().simple()));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            r#"
            INSERT INTO notifications (
                id, user_id, site_id, category, severity,
                title, message, metadata_json, action_url,
                is_read, read_at, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, NULL, ?)
            "#,
        )
        .bind(&id)
        .bind(input.user_id)
        .bind(input.site_id)
        .bind(&input.category)
        .bind(&input.severity)
        .bind(&input.title)
        .bind(&input.message)
        .bind(&input.metadata_json)
        .bind(&input.action_url)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(NotificationModel {
            id,
            user_id: input.user_id,
            site_id: input.site_id,
            category: input.category.clone(),
            severity: input.severity.clone(),
            title: input.title.clone(),
            message: input.message.clone(),
            metadata_json: input.metadata_json.clone(),
            action_url: input.action_url.clone(),
            is_read: false,
            read_at: None,
            created_at: now,
        })
    }

    /// Finds a single notification by ID.
    pub async fn find_by_id(&self, id: &str) -> Result<Option<NotificationModel>, DbError> {
        let row = sqlx::query(
            r#"
            SELECT
                id, user_id, site_id, category, severity,
                title, message, metadata_json, action_url,
                is_read, read_at, created_at
            FROM notifications
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(row.map(Self::map_notification_row))
    }

    /// Lists notifications with filtering and pagination.
    /// If `user_id` is Some, filters by `(user_id = ? OR user_id IS NULL)`.
    pub async fn list(
        &self,
        user_id: Option<i64>,
        filter: &NotificationFilter,
    ) -> Result<(Vec<NotificationModel>, i64), DbError> {
        let mut count_query = String::from("SELECT COUNT(*) FROM notifications WHERE 1=1");
        let mut list_query = String::from(
            r#"
            SELECT
                id, user_id, site_id, category, severity,
                title, message, metadata_json, action_url,
                is_read, read_at, created_at
            FROM notifications
            WHERE 1=1
            "#,
        );

        let mut conditions = String::new();
        if user_id.is_some() {
            conditions.push_str(" AND (user_id = ? OR user_id IS NULL)");
        }
        if filter.unread_only {
            conditions.push_str(" AND is_read = 0");
        }
        if filter.category.is_some() {
            conditions.push_str(" AND category = ?");
        }
        if filter.severity.is_some() {
            conditions.push_str(" AND severity = ?");
        }

        count_query.push_str(&conditions);
        list_query.push_str(&conditions);
        list_query.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");

        // Execute count
        let mut count_cmd = sqlx::query_scalar::<_, i64>(&count_query);
        if let Some(uid) = user_id {
            count_cmd = count_cmd.bind(uid);
        }
        if let Some(ref cat) = filter.category {
            count_cmd = count_cmd.bind(cat);
        }
        if let Some(ref sev) = filter.severity {
            count_cmd = count_cmd.bind(sev);
        }
        let total = count_cmd
            .fetch_one(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        // Execute list
        let mut list_cmd = sqlx::query(&list_query);
        if let Some(uid) = user_id {
            list_cmd = list_cmd.bind(uid);
        }
        if let Some(ref cat) = filter.category {
            list_cmd = list_cmd.bind(cat);
        }
        if let Some(ref sev) = filter.severity {
            list_cmd = list_cmd.bind(sev);
        }
        let limit = if filter.limit > 0 { filter.limit } else { 50 };
        list_cmd = list_cmd.bind(limit).bind(filter.offset.max(0));

        let rows = list_cmd
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;
        let items = rows.into_iter().map(Self::map_notification_row).collect();

        Ok((items, total))
    }

    /// Fast count of unread notifications for a user/global.
    pub async fn unread_count(&self, user_id: Option<i64>) -> Result<i64, DbError> {
        let count = if let Some(uid) = user_id {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM notifications WHERE is_read = 0 AND (user_id = ? OR user_id IS NULL)",
            )
            .bind(uid)
            .fetch_one(&self.pool)
            .await
            .map_err(DbError::Sqlx)?
        } else {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM notifications WHERE is_read = 0")
                .fetch_one(&self.pool)
                .await
                .map_err(DbError::Sqlx)?
        };

        Ok(count)
    }

    /// Marks a single notification as read.
    pub async fn mark_as_read(&self, id: &str, user_id: Option<i64>) -> Result<bool, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let rows_affected = if let Some(uid) = user_id {
            sqlx::query(
                "UPDATE notifications SET is_read = 1, read_at = ? WHERE id = ? AND (user_id = ? OR user_id IS NULL)",
            )
            .bind(now)
            .bind(id)
            .bind(uid)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?
            .rows_affected()
        } else {
            sqlx::query("UPDATE notifications SET is_read = 1, read_at = ? WHERE id = ?")
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(DbError::Sqlx)?
                .rows_affected()
        };

        Ok(rows_affected > 0)
    }

    /// Marks all matching notifications as read.
    pub async fn mark_all_read(&self, user_id: Option<i64>) -> Result<i64, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let rows_affected = if let Some(uid) = user_id {
            sqlx::query(
                "UPDATE notifications SET is_read = 1, read_at = ? WHERE is_read = 0 AND (user_id = ? OR user_id IS NULL)",
            )
            .bind(now)
            .bind(uid)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?
            .rows_affected()
        } else {
            sqlx::query("UPDATE notifications SET is_read = 1, read_at = ? WHERE is_read = 0")
                .bind(now)
                .execute(&self.pool)
                .await
                .map_err(DbError::Sqlx)?
                .rows_affected()
        };

        Ok(rows_affected as i64)
    }

    /// Deletes a single notification by ID.
    pub async fn delete(&self, id: &str, user_id: Option<i64>) -> Result<bool, DbError> {
        let rows_affected = if let Some(uid) = user_id {
            sqlx::query(
                "DELETE FROM notifications WHERE id = ? AND (user_id = ? OR user_id IS NULL)",
            )
            .bind(id)
            .bind(uid)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?
            .rows_affected()
        } else {
            sqlx::query("DELETE FROM notifications WHERE id = ?")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(DbError::Sqlx)?
                .rows_affected()
        };

        Ok(rows_affected > 0)
    }

    /// Clears notifications, optionally only read ones.
    pub async fn clear_all(&self, user_id: Option<i64>, read_only: bool) -> Result<i64, DbError> {
        let mut query = String::from("DELETE FROM notifications WHERE 1=1");
        if read_only {
            query.push_str(" AND is_read = 1");
        }
        if user_id.is_some() {
            query.push_str(" AND (user_id = ? OR user_id IS NULL)");
        }

        let mut cmd = sqlx::query(&query);
        if let Some(uid) = user_id {
            cmd = cmd.bind(uid);
        }

        let rows = cmd
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?
            .rows_affected();
        Ok(rows as i64)
    }

    /// Retrieves global notification & SMTP settings.
    pub async fn get_settings(&self) -> Result<NotificationSettingsModel, DbError> {
        let row = sqlx::query(
            r#"
            SELECT
                id, smtp_enabled, smtp_host, smtp_port, smtp_username,
                smtp_password_enc, smtp_encryption, from_address, from_name,
                admin_email, ssl_warning_days, quota_warning_percent,
                notify_ssl_expiring, notify_quota_warning, notify_job_failed,
                notify_deploy_failed, updated_at
            FROM notification_settings
            WHERE id = 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        match row {
            Some(r) => Ok(Self::map_settings_row(r)),
            None => {
                // Return default fallback if row missing
                Ok(NotificationSettingsModel {
                    id: 1,
                    smtp_enabled: false,
                    smtp_host: None,
                    smtp_port: 587,
                    smtp_username: None,
                    smtp_password_enc: None,
                    smtp_encryption: "starttls".to_string(),
                    from_address: "noreply@panel.local".to_string(),
                    from_name: "Qinpanel".to_string(),
                    admin_email: None,
                    ssl_warning_days: 30,
                    quota_warning_percent: 90,
                    notify_ssl_expiring: true,
                    notify_quota_warning: true,
                    notify_job_failed: true,
                    notify_deploy_failed: true,
                    updated_at: 0,
                })
            }
        }
    }

    /// Updates global notification & SMTP settings.
    pub async fn update_settings(
        &self,
        input: &UpdateNotificationSettingsInput,
    ) -> Result<NotificationSettingsModel, DbError> {
        let existing = self.get_settings().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let smtp_enabled = input.smtp_enabled.unwrap_or(existing.smtp_enabled);
        let smtp_host = input.smtp_host.clone().or(existing.smtp_host);
        let smtp_port = input.smtp_port.unwrap_or(existing.smtp_port);
        let smtp_username = input.smtp_username.clone().or(existing.smtp_username);
        let smtp_password_enc = input
            .smtp_password_enc
            .clone()
            .or(existing.smtp_password_enc);
        let smtp_encryption = input
            .smtp_encryption
            .clone()
            .unwrap_or(existing.smtp_encryption);
        let from_address = input.from_address.clone().unwrap_or(existing.from_address);
        let from_name = input.from_name.clone().unwrap_or(existing.from_name);
        let admin_email = input.admin_email.clone().or(existing.admin_email);
        let ssl_warning_days = input.ssl_warning_days.unwrap_or(existing.ssl_warning_days);
        let quota_warning_percent = input
            .quota_warning_percent
            .unwrap_or(existing.quota_warning_percent);
        let notify_ssl_expiring = input
            .notify_ssl_expiring
            .unwrap_or(existing.notify_ssl_expiring);
        let notify_quota_warning = input
            .notify_quota_warning
            .unwrap_or(existing.notify_quota_warning);
        let notify_job_failed = input
            .notify_job_failed
            .unwrap_or(existing.notify_job_failed);
        let notify_deploy_failed = input
            .notify_deploy_failed
            .unwrap_or(existing.notify_deploy_failed);

        sqlx::query(
            r#"
            INSERT INTO notification_settings (
                id, smtp_enabled, smtp_host, smtp_port, smtp_username,
                smtp_password_enc, smtp_encryption, from_address, from_name,
                admin_email, ssl_warning_days, quota_warning_percent,
                notify_ssl_expiring, notify_quota_warning, notify_job_failed,
                notify_deploy_failed, updated_at
            )
            VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                smtp_enabled = excluded.smtp_enabled,
                smtp_host = excluded.smtp_host,
                smtp_port = excluded.smtp_port,
                smtp_username = excluded.smtp_username,
                smtp_password_enc = excluded.smtp_password_enc,
                smtp_encryption = excluded.smtp_encryption,
                from_address = excluded.from_address,
                from_name = excluded.from_name,
                admin_email = excluded.admin_email,
                ssl_warning_days = excluded.ssl_warning_days,
                quota_warning_percent = excluded.quota_warning_percent,
                notify_ssl_expiring = excluded.notify_ssl_expiring,
                notify_quota_warning = excluded.notify_quota_warning,
                notify_job_failed = excluded.notify_job_failed,
                notify_deploy_failed = excluded.notify_deploy_failed,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(if smtp_enabled { 1 } else { 0 })
        .bind(&smtp_host)
        .bind(smtp_port)
        .bind(&smtp_username)
        .bind(&smtp_password_enc)
        .bind(&smtp_encryption)
        .bind(&from_address)
        .bind(&from_name)
        .bind(&admin_email)
        .bind(ssl_warning_days)
        .bind(quota_warning_percent)
        .bind(if notify_ssl_expiring { 1 } else { 0 })
        .bind(if notify_quota_warning { 1 } else { 0 })
        .bind(if notify_job_failed { 1 } else { 0 })
        .bind(if notify_deploy_failed { 1 } else { 0 })
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(NotificationSettingsModel {
            id: 1,
            smtp_enabled,
            smtp_host,
            smtp_port,
            smtp_username,
            smtp_password_enc,
            smtp_encryption,
            from_address,
            from_name,
            admin_email,
            ssl_warning_days,
            quota_warning_percent,
            notify_ssl_expiring,
            notify_quota_warning,
            notify_job_failed,
            notify_deploy_failed,
            updated_at: now,
        })
    }

    fn map_notification_row(r: sqlx::sqlite::SqliteRow) -> NotificationModel {
        NotificationModel {
            id: r.get("id"),
            user_id: r.get("user_id"),
            site_id: r.get("site_id"),
            category: r.get("category"),
            severity: r.get("severity"),
            title: r.get("title"),
            message: r.get("message"),
            metadata_json: r.get("metadata_json"),
            action_url: r.get("action_url"),
            is_read: r.get::<i64, _>("is_read") != 0,
            read_at: r.get("read_at"),
            created_at: r.get("created_at"),
        }
    }

    fn map_settings_row(r: sqlx::sqlite::SqliteRow) -> NotificationSettingsModel {
        NotificationSettingsModel {
            id: r.get("id"),
            smtp_enabled: r.get::<i64, _>("smtp_enabled") != 0,
            smtp_host: r.get("smtp_host"),
            smtp_port: r.get("smtp_port"),
            smtp_username: r.get("smtp_username"),
            smtp_password_enc: r.get("smtp_password_enc"),
            smtp_encryption: r.get("smtp_encryption"),
            from_address: r.get("from_address"),
            from_name: r.get("from_name"),
            admin_email: r.get("admin_email"),
            ssl_warning_days: r.get("ssl_warning_days"),
            quota_warning_percent: r.get("quota_warning_percent"),
            notify_ssl_expiring: r.get::<i64, _>("notify_ssl_expiring") != 0,
            notify_quota_warning: r.get::<i64, _>("notify_quota_warning") != 0,
            notify_job_failed: r.get::<i64, _>("notify_job_failed") != 0,
            notify_deploy_failed: r.get::<i64, _>("notify_deploy_failed") != 0,
            updated_at: r.get("updated_at"),
        }
    }
}
