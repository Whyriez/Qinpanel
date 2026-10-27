//! Audit log repository for recording operational and security actions.

use crate::error::DbError;
use crate::models::{AuditLog, CreateAuditLog};
use crate::pool::DbPool;
use panel_core::audit::AuditCategory;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Filter criteria for querying audit logs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLogFilter {
    pub category: Option<AuditCategory>,
    pub q: Option<String>,
    pub user_id: Option<i64>,
    pub actor_username: Option<String>,
    pub action: Option<String>,
    pub target_resource: Option<String>,
    pub status: Option<String>,
    pub from_time: Option<i64>,
    pub to_time: Option<i64>,
}

#[derive(Clone)]
pub struct SqliteAuditLogRepository {
    pool: DbPool,
}

impl SqliteAuditLogRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: &CreateAuditLog) -> Result<AuditLog, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query(
            r#"
            INSERT INTO audit_logs (
                user_id, actor_username, action, target_resource, details_json, ip_address, status, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(input.user_id)
        .bind(&input.actor_username)
        .bind(&input.action)
        .bind(&input.target_resource)
        .bind(&input.details_json)
        .bind(&input.ip_address)
        .bind(&input.status)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let id = res.last_insert_rowid();

        Ok(AuditLog {
            id,
            user_id: input.user_id,
            actor_username: input.actor_username.clone(),
            action: input.action.clone(),
            target_resource: input.target_resource.clone(),
            details_json: input.details_json.clone(),
            ip_address: input.ip_address.clone(),
            status: input.status.clone(),
            created_at: now,
        })
    }

    pub async fn find_by_id(&self, id: i64) -> Result<Option<AuditLog>, DbError> {
        let log = sqlx::query_as::<_, AuditLog>(
            r#"
            SELECT id, user_id, actor_username, action, target_resource, details_json, ip_address, status, created_at
            FROM audit_logs
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(log)
    }

    pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<AuditLog>, DbError> {
        self.list_filtered(&AuditLogFilter::default(), limit, offset)
            .await
    }

    pub async fn list_by_user(
        &self,
        user_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AuditLog>, DbError> {
        let filter = AuditLogFilter {
            user_id: Some(user_id),
            ..Default::default()
        };
        self.list_filtered(&filter, limit, offset).await
    }

    pub async fn list_filtered(
        &self,
        filter: &AuditLogFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AuditLog>, DbError> {
        let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT id, user_id, actor_username, action, target_resource, details_json, ip_address, status, created_at FROM audit_logs WHERE 1=1",
        );

        Self::apply_filter(&mut builder, filter);

        builder.push(" ORDER BY created_at DESC, id DESC LIMIT ");
        builder.push_bind(limit);
        builder.push(" OFFSET ");
        builder.push_bind(offset);

        let query = builder.build_query_as::<AuditLog>();
        let logs = query.fetch_all(&self.pool).await?;

        Ok(logs)
    }

    fn apply_filter<'a>(
        builder: &mut sqlx::QueryBuilder<'a, sqlx::Sqlite>,
        filter: &'a AuditLogFilter,
    ) {
        if let Some(category) = filter.category {
            if category == AuditCategory::Other {
                builder.push(" AND NOT (");
            } else {
                builder.push(" AND (");
            }
            let prefixes: Vec<_> = if category == AuditCategory::Other {
                AuditCategory::ALL
                    .into_iter()
                    .flat_map(AuditCategory::prefixes)
                    .copied()
                    .collect()
            } else {
                category.prefixes().to_vec()
            };
            for (index, prefix) in prefixes.into_iter().enumerate() {
                if index > 0 {
                    builder.push(" OR ");
                }
                builder
                    .push("instr(action, ")
                    .push_bind(prefix)
                    .push(") = 1");
            }
            builder.push(")");
        }
        if let Some(q) = &filter.q {
            builder.push(" AND (");
            for (index, column) in ["action", "actor_username", "target_resource", "ip_address"]
                .into_iter()
                .enumerate()
            {
                if index > 0 {
                    builder.push(" OR ");
                }
                builder
                    .push("instr(lower(")
                    .push(column)
                    .push("), lower(")
                    .push_bind(q)
                    .push(")) > 0");
            }
            builder.push(")");
        }
        if let Some(user_id) = filter.user_id {
            builder.push(" AND user_id = ");
            builder.push_bind(user_id);
        }
        if let Some(actor) = &filter.actor_username {
            builder.push(" AND actor_username = ");
            builder.push_bind(actor);
        }
        if let Some(action) = &filter.action {
            builder.push(" AND action = ");
            builder.push_bind(action);
        }
        if let Some(target) = &filter.target_resource {
            builder.push(" AND target_resource = ");
            builder.push_bind(target);
        }
        if let Some(status) = &filter.status {
            builder.push(" AND status = ");
            builder.push_bind(status);
        }
        if let Some(from) = filter.from_time {
            builder.push(" AND created_at >= ");
            builder.push_bind(from);
        }
        if let Some(to) = filter.to_time {
            builder.push(" AND created_at <= ");
            builder.push_bind(to);
        }
    }

    pub async fn count_filtered(&self, filter: &AuditLogFilter) -> Result<i64, DbError> {
        let mut builder =
            sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT COUNT(*) FROM audit_logs WHERE 1=1");

        Self::apply_filter(&mut builder, filter);

        let query = builder.build_query_scalar::<i64>();
        let count = query.fetch_one(&self.pool).await?;

        Ok(count)
    }
}
