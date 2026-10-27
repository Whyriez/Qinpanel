//! Git deploy repository for configuration, releases, deployment history, and analytics markers.

use crate::error::DbError;
use crate::models::{
    CreateSiteDeployment, DeployMarker, SiteDeployment, SiteGitConfig, UpdateSiteDeployment,
    UpsertSiteGitConfig,
};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteGitDeployRepository {
    pool: DbPool,
}

impl SqliteGitDeployRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Gets the Git deploy configuration for a given site, if configured.
    pub async fn get_config(&self, site_id: i64) -> Result<Option<SiteGitConfig>, DbError> {
        let config = sqlx::query_as::<_, SiteGitConfig>(
            r#"
            SELECT
                id, site_id, repo_url, branch, deploy_key_pub, deploy_key_priv_path,
                build_command, post_deploy_hook, docroot_subpath, releases_to_keep,
                webhook_secret, auto_deploy_enabled, current_release_id, created_at, updated_at
            FROM site_git_configs
            WHERE site_id = ?
            "#,
        )
        .bind(site_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(config)
    }

    /// Upserts (inserts or updates) the Git deploy configuration for a site.
    pub async fn upsert_config(
        &self,
        input: &UpsertSiteGitConfig,
    ) -> Result<SiteGitConfig, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let docroot_subpath = input.docroot_subpath.as_deref().unwrap_or("");
        let releases_to_keep = input.releases_to_keep.unwrap_or(5);
        let auto_deploy_enabled = if input.auto_deploy_enabled.unwrap_or(false) {
            1
        } else {
            0
        };

        sqlx::query(
            r#"
            INSERT INTO site_git_configs (
                site_id, repo_url, branch, deploy_key_pub, deploy_key_priv_path,
                build_command, post_deploy_hook, docroot_subpath, releases_to_keep,
                webhook_secret, auto_deploy_enabled, current_release_id, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(site_id) DO UPDATE SET
                repo_url = excluded.repo_url,
                branch = excluded.branch,
                deploy_key_pub = COALESCE(excluded.deploy_key_pub, site_git_configs.deploy_key_pub),
                deploy_key_priv_path = COALESCE(excluded.deploy_key_priv_path, site_git_configs.deploy_key_priv_path),
                build_command = excluded.build_command,
                post_deploy_hook = excluded.post_deploy_hook,
                docroot_subpath = excluded.docroot_subpath,
                releases_to_keep = excluded.releases_to_keep,
                webhook_secret = COALESCE(excluded.webhook_secret, site_git_configs.webhook_secret),
                auto_deploy_enabled = excluded.auto_deploy_enabled,
                current_release_id = COALESCE(excluded.current_release_id, site_git_configs.current_release_id),
                updated_at = excluded.updated_at
            "#,
        )
        .bind(input.site_id)
        .bind(&input.repo_url)
        .bind(&input.branch)
        .bind(&input.deploy_key_pub)
        .bind(&input.deploy_key_priv_path)
        .bind(&input.build_command)
        .bind(&input.post_deploy_hook)
        .bind(docroot_subpath)
        .bind(releases_to_keep)
        .bind(&input.webhook_secret)
        .bind(auto_deploy_enabled)
        .bind(&input.current_release_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        self.get_config(input.site_id).await?.ok_or_else(|| {
            DbError::NotFound(format!("Git config for site {} not found", input.site_id))
        })
    }

    /// Updates the currently active release ID for a site.
    pub async fn update_current_release(
        &self,
        site_id: i64,
        release_id: &str,
    ) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            r#"
            UPDATE site_git_configs
            SET current_release_id = ?, updated_at = ?
            WHERE site_id = ?
            "#,
        )
        .bind(release_id)
        .bind(now)
        .bind(site_id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(())
    }

    /// Creates a new deployment record in pending or running status.
    pub async fn create_deployment(
        &self,
        input: &CreateSiteDeployment,
    ) -> Result<SiteDeployment, DbError> {
        let res = sqlx::query(
            r#"
            INSERT INTO site_deployments (
                site_id, release_id, commit_hash, commit_message, commit_author,
                branch, trigger_type, status, log_output, duration_ms, created_at, finished_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(input.site_id)
        .bind(&input.release_id)
        .bind(&input.commit_hash)
        .bind(&input.commit_message)
        .bind(&input.commit_author)
        .bind(&input.branch)
        .bind(&input.trigger_type)
        .bind(&input.status)
        .bind(&input.log_output)
        .bind(input.duration_ms)
        .bind(input.created_at)
        .bind(input.finished_at)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let id = res.last_insert_rowid();

        self.get_deployment(input.site_id, id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Deployment {} not found after insert", id)))
    }

    /// Updates the final status and execution details of a deployment.
    pub async fn update_deployment(
        &self,
        deployment_id: i64,
        update: &UpdateSiteDeployment,
    ) -> Result<(), DbError> {
        sqlx::query(
            r#"
            UPDATE site_deployments
            SET
                status = ?,
                commit_hash = COALESCE(?, commit_hash),
                commit_message = COALESCE(?, commit_message),
                commit_author = COALESCE(?, commit_author),
                log_output = COALESCE(?, log_output),
                duration_ms = COALESCE(?, duration_ms),
                finished_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&update.status)
        .bind(&update.commit_hash)
        .bind(&update.commit_message)
        .bind(&update.commit_author)
        .bind(&update.log_output)
        .bind(update.duration_ms)
        .bind(update.finished_at)
        .bind(deployment_id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(())
    }

    /// Retrieves a single deployment by site ID and deployment ID.
    pub async fn get_deployment(
        &self,
        site_id: i64,
        deployment_id: i64,
    ) -> Result<Option<SiteDeployment>, DbError> {
        let deployment = sqlx::query_as::<_, SiteDeployment>(
            r#"
            SELECT
                id, site_id, release_id, commit_hash, commit_message, commit_author,
                branch, trigger_type, status, log_output, duration_ms, created_at, finished_at
            FROM site_deployments
            WHERE site_id = ? AND id = ?
            "#,
        )
        .bind(site_id)
        .bind(deployment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(deployment)
    }

    /// Lists deployments for a site, ordered from newest to oldest.
    pub async fn list_deployments(
        &self,
        site_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SiteDeployment>, DbError> {
        let deployments = sqlx::query_as::<_, SiteDeployment>(
            r#"
            SELECT
                id, site_id, release_id, commit_hash, commit_message, commit_author,
                branch, trigger_type, status, log_output, duration_ms, created_at, finished_at
            FROM site_deployments
            WHERE site_id = ?
            ORDER BY created_at DESC, id DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(site_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(deployments)
    }

    /// Finds deployment markers within a timestamp range (Phase 9 analytics prasyarat).
    pub async fn list_deploy_markers(
        &self,
        site_id: i64,
        since: i64,
        until: i64,
    ) -> Result<Vec<DeployMarker>, DbError> {
        let markers = sqlx::query_as::<_, DeployMarker>(
            r#"
            SELECT
                id, site_id, release_id, commit_hash, branch,
                commit_message, commit_author, trigger_type, created_at
            FROM site_deployments
            WHERE site_id = ? AND status = 'success' AND created_at >= ? AND created_at <= ?
            ORDER BY created_at ASC
            "#,
        )
        .bind(site_id)
        .bind(since)
        .bind(until)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(markers)
    }
}
