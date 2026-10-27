use super::*;
use crate::{auth::AuthenticatedUser, tenant_context::resolve_tenant_context};
use panel_db::DbPool;

pub async fn load(
    state: &AppState,
    actor: &AuthenticatedUser,
    id: &str,
) -> Result<Connection, Error> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| bad(StatusCode::NOT_FOUND, "Koneksi tidak ditemukan."))?;
    let value: Connection =
        sqlx::query_as("SELECT * FROM storage_connections WHERE id=? AND (?=1 OR owner_id=?)")
            .bind(id)
            .bind(actor.is_admin())
            .bind(actor.id)
            .fetch_optional(&state.pool)
            .await
            .map_err(db_error)?
            .ok_or_else(|| bad(StatusCode::NOT_FOUND, "Koneksi tidak ditemukan."))?;
    tenant(state, actor, value.owner_id, value.site_id).await?;
    Ok(value)
}
pub async fn tenant(
    state: &AppState,
    actor: &AuthenticatedUser,
    owner: i64,
    site: Option<i64>,
) -> Result<(), Error> {
    let owner = owner.to_string();
    resolve_tenant_context(state, actor, Some(&owner), site)
        .await
        .map(|_| ())
}
pub async fn available(pool: &DbPool, id: &str, revision: i64) -> Result<(), Error> {
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM storage_connections WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
        .bind(id).bind(revision).bind(id).bind(chrono::Utc::now().timestamp()-120).fetch_one(pool).await.map_err(db_error)?;
    if count != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Koneksi berubah atau operasi masih berjalan. Muat ulang.",
        ));
    }
    Ok(())
}
pub fn view(value: &Connection) -> Value {
    json!({"id":value.id,"owner_id":value.owner_id,"site_id":value.site_id,"label":value.label,"provider":value.provider,"endpoint":value.endpoint,"region":value.region,"addressing":value.addressing,"bucket":value.bucket,"revision":value.revision,"verified_at":if value.verified_revision==Some(value.revision){value.verified_at}else{None},"has_credentials":true,"created_at":value.created_at,"updated_at":value.updated_at,"last_operation":value.last_operation,"conditional_put":value.conditional_put(),"managed":value.managed,"managed_state":value.managed_state,"application_key_generation":value.application_key_generation,"application_key_state":value.application_key_state})
}
pub async fn verify(pool: &DbPool, value: &Connection) -> Result<(), Error> {
    sqlx::query("UPDATE storage_operations SET state='unknown',completed_at=? WHERE connection_id=? AND state='running' AND created_at<=?")
        .bind(chrono::Utc::now().timestamp()).bind(&value.id).bind(chrono::Utc::now().timestamp()-120).execute(pool).await.map_err(db_error)?;
    let changed=sqlx::query("UPDATE storage_connections SET verified_revision=revision,verified_at=? WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running')")
        .bind(chrono::Utc::now().timestamp()).bind(&value.id).bind(value.revision).bind(&value.id).execute(pool).await.map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Koneksi berubah selama permintaan. Hasil dibatalkan.",
        ));
    }
    Ok(())
}
pub async fn reserve(
    pool: &DbPool,
    value: &Connection,
    actor: i64,
    action: &str,
) -> Result<String, Error> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE storage_connections SET verified_at=NULL,verified_revision=NULL WHERE EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=storage_connections.id AND revision=storage_connections.revision AND state='running' AND created_at<=?)").bind(now-120).execute(pool).await.map_err(db_error)?;
    sqlx::query("UPDATE storage_operations SET state='unknown',completed_at=? WHERE state='running' AND created_at<=?").bind(now).bind(now-120).execute(pool).await.map_err(db_error)?;
    sqlx::query("DELETE FROM storage_operations WHERE created_at<?")
        .bind(now - 30 * 86400)
        .execute(pool)
        .await
        .map_err(db_error)?;
    let id = uuid::Uuid::new_v4().to_string();
    let changed=sqlx::query("INSERT INTO storage_operations(id,connection_id,actor_id,revision,action,state,created_at) SELECT ?,?,?,?,?,'running',? WHERE EXISTS(SELECT 1 FROM storage_connections c JOIN users u ON u.id=c.owner_id WHERE c.id=? AND c.revision=? AND u.role='user' AND u.status='active' AND (?='create_bucket' OR (c.verified_revision=c.revision AND c.verified_at>?))) AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
        .bind(&id).bind(&value.id).bind(actor).bind(value.revision).bind(action).bind(now).bind(&value.id).bind(value.revision).bind(action).bind(now-600).bind(&value.id).bind(now-120).execute(pool).await.map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Koneksi berubah atau operasi lain masih berjalan. Muat ulang.",
        ));
    }
    Ok(id)
}
pub async fn finish(
    pool: &DbPool,
    id: &str,
    result: &Result<Outcome, Failure>,
) -> Result<(), Error> {
    let status = outcome_state(result);
    let mut transaction = pool.begin().await.map_err(db_error)?;
    sqlx::query("UPDATE storage_operations SET state=?,completed_at=? WHERE id=?")
        .bind(status)
        .bind(chrono::Utc::now().timestamp())
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
    if status == "unknown" {
        sqlx::query("UPDATE storage_connections SET verified_at=NULL,verified_revision=NULL WHERE (id,revision)=(SELECT connection_id,revision FROM storage_operations WHERE id=?)")
            .bind(id).execute(&mut *transaction).await.map_err(db_error)?;
    }
    transaction.commit().await.map_err(db_error)?;
    Ok(())
}

pub fn outcome_state(result: &Result<Outcome, Failure>) -> &'static str {
    match result {
        Ok(_) => "succeeded",
        Err(Failure::Timeout | Failure::Unavailable | Failure::Invalid) => "unknown",
        Err(Failure::Provider(code)) if *code >= 500 => "unknown",
        Err(_) => "failed",
    }
}
