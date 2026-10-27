//! Notification dispatch and system alert evaluation service.

use crate::agent_client::AgentClient;

use panel_core::notification::{NotificationCategory, NotificationSeverity};
use panel_db::error::DbError;
use panel_db::models::{CreateNotificationInput, NotificationFilter, NotificationModel};
use panel_db::pool::DbPool;
use panel_db::repositories::{
    SqliteCertificateRepository, SqliteNotificationRepository, SqlitePlanRepository,
    SqliteUserRepository,
};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

#[derive(Debug, Clone)]
pub struct DispatchNotificationEvent {
    pub user_id: Option<i64>,
    pub site_id: Option<i64>,
    pub category: NotificationCategory,
    pub severity: NotificationSeverity,
    pub title: String,
    pub message: String,
    pub metadata_json: Option<String>,
    pub action_url: Option<String>,
}

#[derive(Clone)]
pub struct NotificationService {
    pool: DbPool,
    job_runner: Option<Arc<crate::jobs::runner::JobRunner>>,
    notif_repo: SqliteNotificationRepository,
    user_repo: SqliteUserRepository,
    cert_repo: SqliteCertificateRepository,
    plan_repo: SqlitePlanRepository,
    agent_client: Option<Arc<AgentClient>>,
}

impl NotificationService {
    pub fn new(pool: DbPool, agent_client: Option<Arc<AgentClient>>) -> Self {
        Self {
            notif_repo: SqliteNotificationRepository::new(pool.clone()),
            user_repo: SqliteUserRepository::new(pool.clone()),
            cert_repo: SqliteCertificateRepository::new(pool.clone()),
            plan_repo: SqlitePlanRepository::new(pool.clone()),
            pool,
            job_runner: None,
            agent_client,
        }
    }

    pub fn with_job_runner(mut self, runner: Arc<crate::jobs::runner::JobRunner>) -> Self {
        self.job_runner = Some(runner);
        self
    }
    pub fn notif_repo(&self) -> &SqliteNotificationRepository {
        &self.notif_repo
    }

    /// Dispatches a notification to in-app database and optionally via SMTP email.
    pub async fn dispatch(
        &self,
        event: DispatchNotificationEvent,
    ) -> Result<NotificationModel, DbError> {
        // 1. Create in-app record
        let model = self
            .notif_repo
            .create(&CreateNotificationInput {
                id: None,
                user_id: event.user_id,
                site_id: event.site_id,
                category: event.category.as_str().to_string(),
                severity: event.severity.as_str().to_string(),
                title: event.title.clone(),
                message: event.message.clone(),
                metadata_json: event.metadata_json.clone(),
                action_url: event.action_url.clone(),
            })
            .await?;

        // 2. Fetch notification settings for SMTP dispatch
        let settings = self.notif_repo.get_settings().await?;
        if !settings.smtp_enabled {
            return Ok(model);
        }

        // Check if event category has email notification enabled
        let is_category_enabled = match event.category {
            NotificationCategory::SslExpiring => settings.notify_ssl_expiring,
            NotificationCategory::QuotaWarning => settings.notify_quota_warning,
            NotificationCategory::JobFailed => settings.notify_job_failed,
            NotificationCategory::DeployFailed => settings.notify_deploy_failed,
            _ => true,
        };

        if !is_category_enabled {
            return Ok(model);
        }

        let smtp_host = settings.smtp_host.unwrap_or_default();
        if smtp_host.is_empty() {
            return Ok(model);
        }

        // Determine recipient email(s)
        let mut recipients = Vec::new();
        if let Some(uid) = event.user_id
            && let Ok(Some(u)) = self.user_repo.find_by_id(uid).await
            && let Some(email) = u.email
            && !email.trim().is_empty()
        {
            recipients.push(email.trim().to_string());
        }

        if let Some(ref admin_email) = settings.admin_email {
            let trimmed = admin_email.trim();
            if !trimmed.is_empty() && !recipients.contains(&trimmed.to_string()) {
                recipients.push(trimmed.to_string());
            }
        }

        if recipients.is_empty() {
            return Ok(model);
        }

        self.enqueue_email(&event, recipients).await;
        Ok(model)
    }

    async fn email_content(
        &self,
        event: &DispatchNotificationEvent,
    ) -> Result<(String, String), String> {
        let origin = crate::public_url::canonical(&self.pool)
            .await
            .map_err(|_| "URL panel belum dapat dimuat")?;
        let action = match origin {
            Some(origin) => origin
                .action_url(event.action_url.as_deref().unwrap_or("/dashboard"))?
                .to_string(),
            None => {
                "URL panel belum diaktifkan; buka alamat panel yang biasa Anda gunakan.".to_string()
            }
        };
        crate::email::load_template(&self.pool).await?.render(
            &event.title,
            &event.message,
            event.category.as_str(),
            &action,
        )
    }

    async fn queue_delivery(
        &self,
        user_id: Option<i64>,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<(), DbError> {
        let Some(runner) = &self.job_runner else {
            return Ok(());
        };
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO email_deliveries(id,user_id,to_address,subject,body,created_at) VALUES(?,?,?,?,?,?)")
            .bind(&id).bind(user_id).bind(to).bind(subject).bind(body).bind(chrono::Utc::now().timestamp()).execute(&self.pool).await.map_err(DbError::Sqlx)?;
        runner
            .enqueue(
                user_id,
                Some(id.clone()),
                "email.deliver".into(),
                serde_json::json!({"id":id}).to_string(),
                Some(0),
            )
            .await?;
        Ok(())
    }

    async fn enqueue_email(&self, event: &DispatchNotificationEvent, recipients: Vec<String>) {
        if self.job_runner.is_none() {
            return;
        }
        let Ok((subject, body)) = self.email_content(event).await else {
            warn!("Template email tidak dapat dirender; notifikasi tetap tersedia di panel");
            return;
        };
        for to in recipients {
            if self
                .queue_delivery(event.user_id, &to, &subject, &body)
                .await
                .is_err()
            {
                warn!("Notifikasi email gagal masuk antrean");
            }
        }
    }

    /// Evaluates system health and alerts: SSL expiration and Quota threshold.
    pub async fn evaluate_system_alerts(&self) -> Result<Vec<NotificationModel>, DbError> {
        let mut triggered = Vec::new();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let settings = self.notif_repo.get_settings().await?;

        // 1. SSL Certificate Expiration Check
        let ssl_threshold_secs = settings.ssl_warning_days * 86400;
        let expiring_certs = self
            .cert_repo
            .find_expiring_certificates(now, ssl_threshold_secs)
            .await?;

        for cert in expiring_certs {
            let days_left = ((cert.not_after - now) / 86400).max(0);

            // Deduplication: check if alert was already dispatched within last 24h for this site
            let filter = NotificationFilter {
                category: Some("ssl_expiring".to_string()),
                limit: 5,
                ..Default::default()
            };
            let (existing, _) = self.notif_repo.list(None, &filter).await?;
            let recently_sent = existing
                .iter()
                .any(|n| n.site_id == Some(cert.site_id) && (now - n.created_at) < 86400);

            if !recently_sent {
                let severity = if days_left <= 7 {
                    NotificationSeverity::Critical
                } else {
                    NotificationSeverity::Warning
                };
                let title = format!(
                    "Sertifikat SSL untuk {} hampir kedaluwarsa ({} hari lagi)",
                    cert.primary_domain, days_left
                );
                let message = format!(
                    "Sertifikat SSL untuk situs domain {} akan kedaluwarsa dalam {} hari. Silakan periksa status perpanjangan otomatis di tab SSL.",
                    cert.primary_domain, days_left
                );

                let notif = self
                    .dispatch(DispatchNotificationEvent {
                        user_id: None, // Can be seen by admin & site owner
                        site_id: Some(cert.site_id),
                        category: NotificationCategory::SslExpiring,
                        severity,
                        title,
                        message,
                        metadata_json: Some(
                            serde_json::json!({
                                "site_id": cert.site_id,
                                "domain": cert.primary_domain,
                                "days_left": days_left,
                                "not_after": cert.not_after
                            })
                            .to_string(),
                        ),
                        action_url: Some(format!("/sites/{}?tab=ssl", cert.site_id)),
                    })
                    .await?;

                triggered.push(notif);
            }
        }

        // 2. Disk Quota Usage Check
        let users = self.user_repo.list_all().await?;
        for user in users {
            if let Some(plan_id) = user.plan_id
                && let Ok(Some(plan)) = self.plan_repo.find_by_id(plan_id).await
            {
                let disk_limit = plan.disk_limit_bytes as u64;
                let used_bytes = if let Some(ref client) = self.agent_client {
                    if let (Some(linux_user), Some(uid)) = (&user.linux_username, user.linux_uid) {
                        client
                            .get_quota(panel_proto::QuotaGetRequest {
                                uid: uid as u32,
                                username: linux_user.clone(),
                                mount_point: None,
                            })
                            .await
                            .map(|u| u.bytes_used)
                            .unwrap_or(0)
                    } else {
                        0
                    }
                } else {
                    0
                };

                if let Some(pct_u64) = (used_bytes * 100).checked_div(disk_limit) {
                    let pct = pct_u64 as i64;
                    if pct >= settings.quota_warning_percent {
                        // Deduplication: check if quota alert for this user sent in last 24h
                        let filter = NotificationFilter {
                            category: Some("quota_warning".to_string()),
                            limit: 5,
                            ..Default::default()
                        };
                        let (existing, _) = self.notif_repo.list(Some(user.id), &filter).await?;
                        let recently_sent = existing
                            .iter()
                            .any(|n| n.user_id == Some(user.id) && (now - n.created_at) < 86400);

                        if !recently_sent {
                            let severity = if pct >= 95 {
                                NotificationSeverity::Critical
                            } else {
                                NotificationSeverity::Error
                            };
                            let title = format!(
                                "Penggunaan ruang disk tenant '{}' telah mencapai {}%",
                                user.username, pct
                            );
                            let message = format!(
                                "Tenant {} menggunakan {} MB dari kuota disk {} MB ({}%). Harap bersihkan file atau naikkan paket hosting.",
                                user.username,
                                used_bytes / (1024 * 1024),
                                disk_limit / (1024 * 1024),
                                pct
                            );

                            let notif = self
                                .dispatch(DispatchNotificationEvent {
                                    user_id: Some(user.id),
                                    site_id: None,
                                    category: NotificationCategory::QuotaWarning,
                                    severity,
                                    title,
                                    message,
                                    metadata_json: Some(
                                        serde_json::json!({
                                            "user_id": user.id,
                                            "username": user.username,
                                            "used_bytes": used_bytes,
                                            "limit_bytes": disk_limit,
                                            "used_percent": pct
                                        })
                                        .to_string(),
                                    ),
                                    action_url: Some("/settings".to_string()),
                                })
                                .await?;

                            triggered.push(notif);
                        }
                    }
                }
            }
        }

        Ok(triggered)
    }
}

#[cfg(test)]
mod panel_url_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[tokio::test]
    async fn email_actions_use_only_the_activated_url_and_reject_external_paths() {
        let pool = panel_db::init_pool("sqlite::memory:").await.unwrap();
        panel_db::run_migrations(&pool).await.unwrap();
        let service = NotificationService::new(pool.clone(), None);
        let mut event = DispatchNotificationEvent {
            user_id: None,
            site_id: None,
            category: NotificationCategory::JobFailed,
            severity: NotificationSeverity::Error,
            title: "Fixture".into(),
            message: "Fixture".into(),
            metadata_json: None,
            action_url: Some("/jobs?status=failed".into()),
        };
        let (_, body) = service.email_content(&event).await.unwrap();
        assert!(body.contains("URL panel belum diaktifkan"));
        sqlx::query("INSERT INTO panel_url_policy(id,revision,canonical_url,endpoint_id,endpoint_revision,updated_at) VALUES(1,1,'https://[2001:db8::7]:19443','fixture',1,0)").execute(&pool).await.unwrap();
        let (_, body) = service.email_content(&event).await.unwrap();
        assert!(body.contains("https://[2001:db8::7]:19443/jobs?status=failed"));
        for path in [
            "https://evil.test",
            "//evil.test",
            "/%2f%2fevil.test",
            "/../evil",
        ] {
            event.action_url = Some(path.into());
            assert!(service.email_content(&event).await.is_err());
        }
    }
}
