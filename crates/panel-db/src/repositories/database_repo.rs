//! Database repository for tenant database provisioning and engine server metadata.

use crate::error::DbError;
use crate::models::{CreateDatabaseInput, DatabaseModel, DatabaseServer, UpdateDatabaseInput};
use crate::pool::DbPool;
use sqlx::Row;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteDatabaseRepository {
    pool: DbPool,
}

impl SqliteDatabaseRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Lists all databases belonging to a specific tenant user.
    pub async fn list_by_user(&self, user_id: i64) -> Result<Vec<DatabaseModel>, DbError> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, site_id, engine, name, db_user, remote_access,
                   allowed_hosts, collation, character_set, size_bytes, created_at, updated_at
            FROM databases
            WHERE user_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(Self::row_to_database(&row)?);
        }
        Ok(out)
    }

    /// Lists all databases associated with a specific site.
    pub async fn list_by_site(&self, site_id: i64) -> Result<Vec<DatabaseModel>, DbError> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, site_id, engine, name, db_user, remote_access,
                   allowed_hosts, collation, character_set, size_bytes, created_at, updated_at
            FROM databases
            WHERE site_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(site_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(Self::row_to_database(&row)?);
        }
        Ok(out)
    }

    /// Lists all databases (administrator scope).
    pub async fn list_all(&self) -> Result<Vec<DatabaseModel>, DbError> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, site_id, engine, name, db_user, remote_access,
                   allowed_hosts, collation, character_set, size_bytes, created_at, updated_at
            FROM databases
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(Self::row_to_database(&row)?);
        }
        Ok(out)
    }

    /// Finds a database by its primary ID.
    pub async fn find_by_id(&self, id: i64) -> Result<Option<DatabaseModel>, DbError> {
        let row_opt = sqlx::query(
            r#"
            SELECT id, user_id, site_id, engine, name, db_user, remote_access,
                   allowed_hosts, collation, character_set, size_bytes, created_at, updated_at
            FROM databases
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        match row_opt {
            Some(row) => Ok(Some(Self::row_to_database(&row)?)),
            None => Ok(None),
        }
    }

    /// Finds a database by its unique name.
    pub async fn find_by_name(&self, name: &str) -> Result<Option<DatabaseModel>, DbError> {
        let row_opt = sqlx::query(
            r#"
            SELECT id, user_id, site_id, engine, name, db_user, remote_access,
                   allowed_hosts, collation, character_set, size_bytes, created_at, updated_at
            FROM databases
            WHERE name = ?
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        match row_opt {
            Some(row) => Ok(Some(Self::row_to_database(&row)?)),
            None => Ok(None),
        }
    }

    /// Counts how many databases a tenant currently owns.
    pub async fn count_by_user(&self, user_id: i64) -> Result<i64, DbError> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM databases WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        let cnt: i64 = row.try_get("cnt").map_err(DbError::Sqlx)?;
        Ok(cnt)
    }

    /// Creates a new database record.
    pub async fn create(&self, input: &CreateDatabaseInput) -> Result<DatabaseModel, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let remote_int = if input.remote_access { 1 } else { 0 };

        let res = sqlx::query(
            r#"
            INSERT INTO databases (
                user_id, site_id, engine, name, db_user, remote_access,
                allowed_hosts, collation, character_set, size_bytes,
                created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
            "#,
        )
        .bind(input.user_id)
        .bind(input.site_id)
        .bind(&input.engine)
        .bind(&input.name)
        .bind(&input.db_user)
        .bind(remote_int)
        .bind(&input.allowed_hosts)
        .bind(&input.collation)
        .bind(&input.character_set)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let id = res.last_insert_rowid();

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Database {} not found after insert", id)))
    }

    /// Updates mutable fields of a database record.
    pub async fn update(
        &self,
        id: i64,
        input: &UpdateDatabaseInput,
    ) -> Result<DatabaseModel, DbError> {
        let current = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Database {id} tidak ditemukan")))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let new_site_id = match input.site_id {
            Some(opt) => opt,
            None => current.site_id,
        };
        let new_remote = input.remote_access.unwrap_or(current.remote_access);
        let new_allowed = input.allowed_hosts.clone().unwrap_or(current.allowed_hosts);
        let new_collation = input.collation.clone().unwrap_or(current.collation);
        let new_charset = input.character_set.clone().unwrap_or(current.character_set);

        let remote_int = if new_remote { 1 } else { 0 };

        sqlx::query(
            r#"
            UPDATE databases
            SET site_id = ?, remote_access = ?, allowed_hosts = ?,
                collation = ?, character_set = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(new_site_id)
        .bind(remote_int)
        .bind(&new_allowed)
        .bind(&new_collation)
        .bind(&new_charset)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        self.find_by_id(id).await?.ok_or_else(|| {
            DbError::NotFound(format!("Database {id} tidak ditemukan setelah update"))
        })
    }

    /// Updates calculated disk size in bytes for a database.
    pub async fn update_size(&self, id: i64, size_bytes: i64) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query("UPDATE databases SET size_bytes = ?, updated_at = ? WHERE id = ?")
            .bind(size_bytes)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        Ok(())
    }

    /// Deletes a database record by ID.
    pub async fn delete(&self, id: i64) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM databases WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        Ok(res.rows_affected() > 0)
    }

    /// Lists database engine servers status.
    pub async fn list_servers(&self) -> Result<Vec<DatabaseServer>, DbError> {
        let rows = sqlx::query(
            r#"
            SELECT engine, is_installed, is_running, version, port, socket_path, updated_at
            FROM database_servers
            ORDER BY engine ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(Self::row_to_server(&row)?);
        }
        Ok(out)
    }

    /// Gets a specific engine server status.
    pub async fn get_server(&self, engine: &str) -> Result<Option<DatabaseServer>, DbError> {
        let row_opt = sqlx::query(
            r#"
            SELECT engine, is_installed, is_running, version, port, socket_path, updated_at
            FROM database_servers
            WHERE engine = ?
            "#,
        )
        .bind(engine)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        match row_opt {
            Some(row) => Ok(Some(Self::row_to_server(&row)?)),
            None => Ok(None),
        }
    }

    /// Upserts a database engine server status record.
    pub async fn upsert_server(&self, server: &DatabaseServer) -> Result<(), DbError> {
        let installed_int = if server.is_installed { 1 } else { 0 };
        let running_int = if server.is_running { 1 } else { 0 };

        sqlx::query(
            r#"
            INSERT INTO database_servers (
                engine, is_installed, is_running, version, port, socket_path, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(engine) DO UPDATE SET
                is_installed = excluded.is_installed,
                is_running = excluded.is_running,
                version = excluded.version,
                port = excluded.port,
                socket_path = excluded.socket_path,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&server.engine)
        .bind(installed_int)
        .bind(running_int)
        .bind(&server.version)
        .bind(server.port)
        .bind(&server.socket_path)
        .bind(server.updated_at)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(())
    }

    fn row_to_database(row: &sqlx::sqlite::SqliteRow) -> Result<DatabaseModel, DbError> {
        let remote_int: i64 = row.try_get("remote_access").map_err(DbError::Sqlx)?;
        Ok(DatabaseModel {
            id: row.try_get("id").map_err(DbError::Sqlx)?,
            user_id: row.try_get("user_id").map_err(DbError::Sqlx)?,
            site_id: row.try_get("site_id").map_err(DbError::Sqlx)?,
            engine: row.try_get("engine").map_err(DbError::Sqlx)?,
            name: row.try_get("name").map_err(DbError::Sqlx)?,
            db_user: row.try_get("db_user").map_err(DbError::Sqlx)?,
            remote_access: remote_int == 1,
            allowed_hosts: row.try_get("allowed_hosts").map_err(DbError::Sqlx)?,
            collation: row.try_get("collation").map_err(DbError::Sqlx)?,
            character_set: row.try_get("character_set").map_err(DbError::Sqlx)?,
            size_bytes: row.try_get("size_bytes").map_err(DbError::Sqlx)?,
            created_at: row.try_get("created_at").map_err(DbError::Sqlx)?,
            updated_at: row.try_get("updated_at").map_err(DbError::Sqlx)?,
        })
    }

    fn row_to_server(row: &sqlx::sqlite::SqliteRow) -> Result<DatabaseServer, DbError> {
        let inst_int: i64 = row.try_get("is_installed").map_err(DbError::Sqlx)?;
        let run_int: i64 = row.try_get("is_running").map_err(DbError::Sqlx)?;
        Ok(DatabaseServer {
            engine: row.try_get("engine").map_err(DbError::Sqlx)?,
            is_installed: inst_int == 1,
            is_running: run_int == 1,
            version: row.try_get("version").map_err(DbError::Sqlx)?,
            port: row.try_get("port").map_err(DbError::Sqlx)?,
            socket_path: row.try_get("socket_path").map_err(DbError::Sqlx)?,
            updated_at: row.try_get("updated_at").map_err(DbError::Sqlx)?,
        })
    }
}
