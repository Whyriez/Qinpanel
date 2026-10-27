//! Audit log writer service for recording actions within the API.

use panel_db::error::DbError;
use panel_db::models::{AuditLog, CreateAuditLog};
use panel_db::pool::DbPool;
use panel_db::repositories::SqliteAuditLogRepository;
use tracing::warn;

#[derive(Clone)]
pub struct AuditWriter {
    repo: SqliteAuditLogRepository,
}

impl AuditWriter {
    pub fn new(pool: DbPool) -> Self {
        Self {
            repo: SqliteAuditLogRepository::new(pool),
        }
    }

    /// Records an audit log entry synchronously.
    pub async fn record(&self, entry: CreateAuditLog) -> Result<AuditLog, DbError> {
        self.repo.create(&entry).await
    }

    /// Helper to record an audit log with standard fields synchronously.
    #[allow(clippy::too_many_arguments)]
    pub async fn log(
        &self,
        user_id: Option<i64>,
        actor_username: impl Into<String>,
        action: impl Into<String>,
        target_resource: impl Into<String>,
        status: impl Into<String>,
        ip_address: Option<String>,
        details_json: Option<String>,
    ) -> Result<AuditLog, DbError> {
        let entry = CreateAuditLog {
            user_id,
            actor_username: actor_username.into(),
            action: action.into(),
            target_resource: target_resource.into(),
            details_json,
            ip_address,
            status: status.into(),
        };
        self.record(entry).await
    }

    /// Records an audit log entry asynchronously in the background.
    /// Failures to record are logged with a warning rather than bubbling up to the caller.
    pub fn record_async(&self, entry: CreateAuditLog) {
        let repo = self.repo.clone();
        tokio::spawn(async move {
            if let Err(e) = repo.create(&entry).await {
                warn!("Failed to record audit log: {}", e);
            }
        });
    }

    /// Helper to record an audit log with standard fields asynchronously.
    #[allow(clippy::too_many_arguments)]
    pub fn log_async(
        &self,
        user_id: Option<i64>,
        actor_username: impl Into<String>,
        action: impl Into<String>,
        target_resource: impl Into<String>,
        status: impl Into<String>,
        ip_address: Option<String>,
        details_json: Option<String>,
    ) {
        let entry = CreateAuditLog {
            user_id,
            actor_username: actor_username.into(),
            action: action.into(),
            target_resource: target_resource.into(),
            details_json,
            ip_address,
            status: status.into(),
        };
        self.record_async(entry);
    }
}
