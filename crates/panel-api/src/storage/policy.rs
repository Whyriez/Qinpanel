//! Owner-only content policies and aggregate allocation monitoring.
//! Limits apply to panel transfers. Native S3 credentials remain provider-controlled.
use super::*;
use crate::auth::{RequireAdminCsrf, RequireAuth, RequireCsrf};
use axum::extract::{Path, State, rejection::JsonRejection};
use panel_db::DbPool;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub max_file_bytes: usize,
    pub extensions: Vec<String>,
}
impl Default for Rules {
    fn default() -> Self {
        Self {
            read: true,
            write: true,
            delete: true,
            max_file_bytes: UPLOAD_LIMIT,
            extensions: Vec::new(),
        }
    }
}
#[derive(sqlx::FromRow)]
struct Policy {
    revision: i64,
    quota_bytes: Option<i64>,
    used_bytes: Option<i64>,
    object_count: Option<i64>,
    measured_at: Option<i64>,
    rules_json: String,
}
async fn load(pool: &DbPool, id: &str) -> Result<Policy, Error> {
    let row = sqlx::query_as("SELECT revision,quota_bytes,used_bytes,object_count,measured_at,rules_json FROM storage_policies WHERE connection_id=?")
        .bind(id).fetch_optional(pool).await.map_err(db_error)?;
    Ok(row.unwrap_or(Policy {
        revision: 1,
        quota_bytes: None,
        used_bytes: None,
        object_count: None,
        measured_at: None,
        rules_json: serde_json::to_string(&Rules::default()).map_err(db_error)?,
    }))
}
async fn ensure(pool: &DbPool, id: &str) -> Result<(), Error> {
    sqlx::query("INSERT OR IGNORE INTO storage_policies(connection_id) VALUES(?)")
        .bind(id)
        .execute(pool)
        .await
        .map_err(db_error)?;
    Ok(())
}
pub async fn view(pool: &DbPool, id: &str) -> Result<Value, Error> {
    let row = load(pool, id).await?;
    let rules: Rules = serde_json::from_str(&row.rules_json).map_err(db_error)?;
    Ok(
        json!({"revision":row.revision,"quota_bytes":row.quota_bytes,"used_bytes":row.used_bytes,"object_count":row.object_count,"measured_at":row.measured_at,"rules":rules,"enforcement":"panel","visibility":"owner"}),
    )
}
pub async fn authorize(pool: &DbPool, id: &str, action: &str) -> Result<(), Error> {
    let row = load(pool, id).await?;
    let rules: Rules = serde_json::from_str(&row.rules_json).map_err(db_error)?;
    let allowed = match action {
        "read" => rules.read,
        "write" => rules.write,
        "delete" => rules.delete,
        _ => false,
    };
    if !allowed {
        return Err(bad(
            StatusCode::FORBIDDEN,
            "Operasi ditolak oleh rules bucket milik Anda.",
        ));
    }
    Ok(())
}
#[utoipa::path(get,path="/api/v1/storage/connections/{id}/policy",params(("id"=String,Path)),responses((status=200,body=Value),(status=403)),tag="storage")]
pub async fn overview(
    State(state): State<AppState>,
    RequireAuth(actor): RequireAuth,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    store::load(&state, &actor, &id).await?;
    Ok(ok(view(&state.pool, &id).await?))
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesInput {
    expected_revision: i64,
    rules: Rules,
}
#[utoipa::path(post,path="/api/v1/storage/connections/{id}/policy",request_body=RulesInput,params(("id"=String,Path)),responses((status=200,body=Value),(status=403),(status=409)),tag="storage")]
pub async fn rules(
    State(state): State<AppState>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    value: Result<Json<RulesInput>, JsonRejection>,
) -> Result<Response, Error> {
    let input = handlers::input(value)?;
    store::load_content(&state, &actor, &id).await?;
    validate_rules(&input.rules)?;
    ensure(&state.pool, &id).await?;
    let changed = sqlx::query("UPDATE storage_policies SET revision=revision+1,rules_json=? WHERE connection_id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
        .bind(serde_json::to_string(&input.rules).map_err(db_error)?).bind(&id).bind(input.expected_revision).bind(&id).bind(chrono::Utc::now().timestamp()-120).execute(&state.pool).await.map_err(db_error)?;
    changed_revision(changed.rows_affected())?;
    handlers::log(
        &state,
        &actor,
        "storage.rules.update",
        &id,
        true,
        json!({"scope":"panel"}),
    );
    Ok(ok(view(&state.pool, &id).await?))
}
fn validate_rules(rules: &Rules) -> Result<(), Error> {
    if rules.max_file_bytes == 0
        || rules.max_file_bytes > UPLOAD_LIMIT
        || rules.extensions.len() > 32
    {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Batas file harus 1 byte hingga 32 MiB; maksimal 32 ekstensi.",
        ));
    }
    if rules.extensions.iter().any(|extension| {
        extension.is_empty()
            || extension.len() > 16
            || !extension
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    }) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Ekstensi memakai huruf kecil/angka tanpa titik, misalnya png atau pdf.",
        ));
    }
    Ok(())
}
fn changed_revision(affected: u64) -> Result<(), Error> {
    if affected != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Alokasi/rules berubah atau transfer masih berjalan. Muat ulang sebelum menyimpan.",
        ));
    }
    Ok(())
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Allocation {
    expected_revision: i64,
    quota_bytes: Option<i64>,
}
#[utoipa::path(post,path="/api/v1/storage/connections/{id}/allocation",request_body=Allocation,params(("id"=String,Path)),responses((status=200,body=Value),(status=403),(status=409)),tag="storage")]
pub async fn allocation(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    value: Result<Json<Allocation>, JsonRejection>,
) -> Result<Response, Error> {
    let input = handlers::input(value)?;
    store::load(&state, &actor, &id).await?;
    if input
        .quota_bytes
        .is_some_and(|v| !(1..=9_007_199_254_740_991).contains(&v))
    {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Alokasi harus positif atau kosong untuk tanpa batas panel.",
        ));
    }
    ensure(&state.pool, &id).await?;
    let changed = sqlx::query("UPDATE storage_policies SET revision=revision+1,quota_bytes=? WHERE connection_id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
        .bind(input.quota_bytes).bind(&id).bind(input.expected_revision).bind(&id).bind(chrono::Utc::now().timestamp()-120).execute(&state.pool).await.map_err(db_error)?;
    changed_revision(changed.rows_affected())?;
    handlers::log(
        &state,
        &actor,
        "storage.allocation.update",
        &id,
        true,
        json!({"quota_bytes":input.quota_bytes}),
    );
    Ok(ok(view(&state.pool, &id).await?))
}

// Traverse prefixes and pagination internally. Only totals leave this module.
async fn scan(
    state: &AppState,
    services: &Services,
    headers: &axum::http::HeaderMap,
    connection: &Connection,
) -> Result<(i64, i64), Error> {
    tokio::time::timeout(
        std::time::Duration::from_secs(90),
        collect(state, services, headers, connection),
    )
    .await
    .map_err(|_| {
        bad(
            StatusCode::GATEWAY_TIMEOUT,
            "Pengukuran pemakaian melewati batas waktu; angka lama dipertahankan.",
        )
    })?
}
async fn collect(
    state: &AppState,
    services: &Services,
    headers: &axum::http::HeaderMap,
    connection: &Connection,
) -> Result<(i64, i64), Error> {
    let mut queue = vec![(String::new(), None)];
    let mut visited = HashSet::new();
    let mut seen_prefixes = HashSet::from([String::new()]);
    let (mut size, mut count) = (0i64, 0i64);
    while let Some((prefix, token)) = queue.pop() {
        if visited.len() >= 500 || !visited.insert((prefix.clone(), token.clone())) {
            return Err(bad(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Listing pemakaian terlalu besar atau berulang; angka lama dipertahankan.",
            ));
        }
        let result = handlers::run(
            state,
            services,
            headers,
            connection,
            Operation::List {
                prefix: prefix.clone(),
                token,
            },
        )
        .await?;
        let Outcome::Listing(listing) = result else {
            return Err(failure(Failure::Invalid));
        };
        for object in listing.objects {
            size = size
                .checked_add(i64::try_from(object.size).map_err(db_error)?)
                .ok_or_else(|| db_error(()))?;
            count = count.checked_add(1).ok_or_else(|| db_error(()))?;
        }
        for child in listing.prefixes {
            if !child.starts_with(&prefix)
                || child == prefix
                || !seen_prefixes.insert(child.clone())
            {
                continue;
            }
            queue.push((child, None));
        }
        if let Some(next) = listing.next_token {
            queue.push((prefix, Some(next)));
        }
    }
    Ok((size, count))
}
#[utoipa::path(post,path="/api/v1/storage/connections/{id}/usage",params(("id"=String,Path)),responses((status=200,body=Value),(status=403),(status=504)),tag="storage")]
pub async fn usage(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, Error> {
    let connection = store::load(&state, &actor, &id).await?;
    let _permit = services.transfers.try_acquire().map_err(|_| {
        bad(
            StatusCode::TOO_MANY_REQUESTS,
            "Pengukuran lain masih berjalan.",
        )
    })?;
    let (bytes, count) = scan(&state, &services, &headers, &connection).await?;
    ensure(&state.pool, &id).await?;
    sqlx::query("UPDATE storage_policies SET used_bytes=?,object_count=?,measured_at=? WHERE connection_id=?")
        .bind(bytes).bind(count).bind(chrono::Utc::now().timestamp()).bind(&id).execute(&state.pool).await.map_err(db_error)?;
    Ok(ok(view(&state.pool, &id).await?))
}
pub async fn admit_upload(
    state: &AppState,
    services: &Services,
    headers: &axum::http::HeaderMap,
    connection: &Connection,
    key: &str,
    bytes: usize,
) -> Result<(), Error> {
    let row = load(&state.pool, &connection.id).await?;
    let rules: Rules = serde_json::from_str(&row.rules_json).map_err(db_error)?;
    let extension = key
        .rsplit('.')
        .next()
        .filter(|_| key.contains('.'))
        .unwrap_or("")
        .to_ascii_lowercase();
    if !rules.write
        || bytes > rules.max_file_bytes
        || !rules.extensions.is_empty() && !rules.extensions.contains(&extension)
    {
        return Err(bad(
            StatusCode::FORBIDDEN,
            "Upload ditolak oleh batas ukuran atau ekstensi pada rules bucket.",
        ));
    }
    if let Some(limit) = row.quota_bytes {
        // Keep admission + the transport's 90s deadline inside the 120s operation lease.
        let (used, _) = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            scan(state, services, headers, connection),
        )
        .await
        .map_err(|_| {
            bad(
                StatusCode::GATEWAY_TIMEOUT,
                "Pemakaian belum dapat dipastikan; upload tidak dikirim. Coba lagi nanti.",
            )
        })??;
        // Conservative for overwrite: never assume that old bytes will be released.
        if used
            .checked_add(i64::try_from(bytes).map_err(db_error)?)
            .is_none_or(|total| total > limit)
        {
            return Err(bad(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Alokasi storage melalui panel tidak cukup. Minta administrator memperbesar alokasi.",
            ));
        }
    }
    Ok(())
}
