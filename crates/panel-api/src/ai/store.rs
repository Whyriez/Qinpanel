use super::{Error, bad, db_error};
use axum::http::StatusCode;
use panel_db::DbPool;
use panel_proto::AiProvider;
use sqlx::FromRow;

#[derive(Clone, FromRow)]
pub struct Settings {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub daily_request_limit: i64,
    pub revision: i64,
}
impl Settings {
    pub fn provider(&self) -> Result<AiProvider, Error> {
        match self.provider.as_str() {
            "gemini" => Ok(AiProvider::Gemini),
            "openai" => Ok(AiProvider::Openai),
            _ => Err(db_error(())),
        }
    }
}
pub async fn settings(pool: &DbPool) -> Result<Settings, Error> {
    sqlx::query_as(
        "SELECT enabled,provider,model,daily_request_limit,revision FROM ai_settings WHERE id=1",
    )
    .fetch_one(pool)
    .await
    .map_err(db_error)
}
pub async fn credential(pool: &DbPool, provider: AiProvider) -> Result<Option<String>, Error> {
    sqlx::query_scalar("SELECT key_enc FROM ai_credentials WHERE provider=?")
        .bind(provider.as_str())
        .fetch_optional(pool)
        .await
        .map_err(db_error)
}
pub async fn requests_today(pool: &DbPool, now: i64) -> Result<i64, Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM ai_requests WHERE created_at>=?")
        .bind(now / 86400 * 86400)
        .fetch_one(pool)
        .await
        .map_err(db_error)
}

/// One atomic SQL statement provides durable global concurrency/rate/day limits.
/// Failed or interrupted attempts count; rotating keys/providers cannot reset it.
pub async fn reserve(
    pool: &DbPool,
    config: &Settings,
    actor: i64,
    now: i64,
) -> Result<String, Error> {
    // Only usage metadata is retained, for at most 30 days. No prompt/output/key.
    sqlx::query("DELETE FROM ai_requests WHERE created_at<?")
        .bind(now - 30 * 86400)
        .execute(pool)
        .await
        .map_err(db_error)?;
    let id = uuid::Uuid::new_v4().to_string();
    let result = sqlx::query("INSERT INTO ai_requests(id,actor_id,provider,model,settings_revision,created_at,state) SELECT ?,?,?,?,?,?,'running' WHERE EXISTS(SELECT 1 FROM ai_settings WHERE id=1 AND revision=?) AND (SELECT COUNT(*) FROM ai_requests WHERE created_at>=?)<? AND NOT EXISTS(SELECT 1 FROM ai_requests WHERE state='running' AND created_at>?) AND NOT EXISTS(SELECT 1 FROM ai_requests WHERE created_at>?)")
        .bind(&id).bind(actor).bind(&config.provider).bind(&config.model).bind(config.revision).bind(now)
        .bind(config.revision).bind(now/86400*86400).bind(config.daily_request_limit).bind(now-90).bind(now-5)
        .execute(pool).await.map_err(db_error)?;
    if result.rows_affected() != 1 {
        return Err(bad(
            StatusCode::TOO_MANY_REQUESTS,
            "Batas harian tercapai, permintaan lain sedang berjalan, atau konfigurasi berubah. Tunggu lalu muat ulang.",
        ));
    }
    Ok(id)
}
pub async fn finish(
    pool: &DbPool,
    id: &str,
    success: bool,
    tokens: Option<u64>,
) -> Result<(), Error> {
    sqlx::query("UPDATE ai_requests SET state=?,completed_at=?,total_tokens=? WHERE id=?")
        .bind(if success { "completed" } else { "failed" })
        .bind(chrono::Utc::now().timestamp())
        .bind(tokens.and_then(|v| i64::try_from(v).ok()))
        .bind(id)
        .execute(pool)
        .await
        .map_err(db_error)?;
    Ok(())
}
