//! Explicit owner-controlled anonymous read, with persistent reconciliation.
use super::*;
use crate::auth::{RequireAuth, RequireCsrf};
use axum::extract::{Path, State, rejection::JsonRejection};
use panel_proto::ManagedStorageRequest;
use serde::Deserialize;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Access {
    revision: i64,
    visibility: String,
    desired_visibility: String,
    state: String,
    request_id: Option<String>,
}
async fn load(state: &AppState, id: &str) -> Result<Access, Error> {
    sqlx::query("INSERT OR IGNORE INTO storage_bucket_access(connection_id) VALUES(?)")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(db_error)?;
    sqlx::query("UPDATE storage_bucket_access SET state='unknown' WHERE connection_id=? AND state='applying' AND updated_at<?")
        .bind(id).bind(chrono::Utc::now().timestamp()-300).execute(&state.pool).await.map_err(db_error)?;
    sqlx::query_as("SELECT revision,visibility,desired_visibility,state,request_id FROM storage_bucket_access WHERE connection_id=?")
        .bind(id).fetch_one(&state.pool).await.map_err(db_error)
}
fn view(row: &Access, managed: bool) -> Value {
    json!({"revision":row.revision,"visibility":if managed {row.visibility.as_str()} else {"provider"},"desired_visibility":row.desired_visibility,"state":row.state,"request_id":row.request_id,"can_change":managed})
}
#[utoipa::path(get,path="/api/v1/storage/connections/{id}/access",params(("id"=String,Path)),responses((status=200,body=Value)),tag="storage")]
pub async fn overview(
    State(state): State<AppState>,
    RequireAuth(actor): RequireAuth,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let connection = store::load(&state, &actor, &id).await?;
    Ok(ok(view(&load(&state, &id).await?, connection.managed)))
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessInput {
    #[schema(value_type=String, format="uuid")]
    request_id: Uuid,
    expected_revision: i64,
    visibility: String,
}
#[utoipa::path(post,path="/api/v1/storage/connections/{id}/access",request_body=AccessInput,params(("id"=String,Path)),responses((status=200,body=Value),(status=409)),tag="storage")]
pub async fn update(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    input: Result<Json<AccessInput>, JsonRejection>,
) -> Result<Response, Error> {
    let input = handlers::input(input)?;
    let connection = store::load_content(&state, &actor, &id).await?;
    if !connection.managed || connection.managed_state.as_deref() != Some("ready") {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Perubahan akses ini untuk bucket Qinpanel aktif. Bucket eksternal mengikuti pengaturan penyedianya.",
        ));
    }
    if input.request_id.is_nil()
        || input.expected_revision < 1
        || !matches!(input.visibility.as_str(), "private" | "public")
    {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Pilih akses Private atau Public dan revisi yang valid.",
        ));
    }
    let old = load(&state, &id).await?;
    let same = old.request_id.as_deref() == Some(input.request_id.to_string().as_str());
    if same {
        if old.desired_visibility != input.visibility {
            return Err(bad(
                StatusCode::CONFLICT,
                "Identitas perubahan sudah dipakai untuk akses berbeda.",
            ));
        }
        if old.state == "ready" {
            return Ok(ok(view(&old, true)));
        }
        if old.state == "applying" {
            return Err(bad(
                StatusCode::CONFLICT,
                "Perubahan akses masih berjalan. Muat ulang status.",
            ));
        }
        let changed = sqlx::query("UPDATE storage_bucket_access SET state='applying',updated_at=? WHERE connection_id=? AND revision=? AND state='unknown'")
            .bind(chrono::Utc::now().timestamp()).bind(&id).bind(old.revision).execute(&state.pool).await.map_err(db_error)?;
        if changed.rows_affected() != 1 {
            return Err(bad(
                StatusCode::CONFLICT,
                "Status akses berubah. Muat ulang.",
            ));
        }
    } else {
        let changed = sqlx::query("UPDATE storage_bucket_access SET revision=revision+1,desired_visibility=?,request_id=?,state='applying',updated_at=? WHERE connection_id=? AND revision=? AND state='ready' AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
            .bind(&input.visibility).bind(input.request_id.to_string()).bind(chrono::Utc::now().timestamp()).bind(&id).bind(input.expected_revision).bind(&id).bind(chrono::Utc::now().timestamp()-120)
            .execute(&state.pool).await.map_err(db_error)?;
        if changed.rows_affected() != 1 {
            return Err(bad(
                StatusCode::CONFLICT,
                "Akses berubah atau transfer masih berjalan. Sinkronkan perubahan yang tertunda sebelum mengubahnya lagi.",
            ));
        }
    }
    let row = load(&state, &id).await?;
    let revision = u64::try_from(row.revision).map_err(db_error)?;
    let response = services
        .runtime
        .managed(ManagedStorageRequest::SetBucketAccess {
            connection_id: Uuid::parse_str(&id).map_err(db_error)?,
            owner_id: connection.owner_id,
            bucket: connection.bucket,
            expected_revision: revision - 1,
            revision,
            public: row.desired_visibility == "public",
        })
        .await;
    let success = response.as_ref().is_ok_and(|value| {
        value["ready"] == true
            && value.get("error").is_none()
            && value["revision"] == revision
            && value["public"] == (row.desired_visibility == "public")
    });
    sqlx::query("UPDATE storage_bucket_access SET state=?,visibility=CASE WHEN ? THEN desired_visibility ELSE visibility END,updated_at=? WHERE connection_id=? AND revision=? AND state='applying'")
        .bind(if success {"ready"} else {"unknown"}).bind(success).bind(chrono::Utc::now().timestamp()).bind(&id).bind(row.revision)
        .execute(&state.pool).await.map_err(db_error)?;
    handlers::log(
        &state,
        &actor,
        "storage.access.update",
        &id,
        success,
        json!({"visibility":row.desired_visibility,"revision":row.revision}),
    );
    if !success {
        return Err(bad(
            StatusCode::SERVICE_UNAVAILABLE,
            "Hasil perubahan akses belum dapat dipastikan. Sinkronkan permintaan yang sama; bucket dan key tetap dipertahankan.",
        ));
    }
    Ok(ok(view(&load(&state, &id).await?, true)))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkInput {
    expected_revision: i64,
    key: String,
    expires_in: u32,
}
#[utoipa::path(post,path="/api/v1/storage/connections/{id}/link",request_body=LinkInput,params(("id"=String,Path)),responses((status=200,body=Value)),tag="storage")]
pub async fn link(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    input: Result<Json<LinkInput>, JsonRejection>,
) -> Result<Response, Error> {
    let input = handlers::input(input)?;
    let connection = store::load_content(&state, &actor, &id).await?;
    super::policy::authorize(&state.pool, &id, "read").await?;
    if connection.managed && connection.managed_state.as_deref() != Some("ready") {
        return Err(bad(
            StatusCode::CONFLICT,
            "Alokasi bucket belum siap untuk membuat URL.",
        ));
    }
    if input.expected_revision != connection.revision || !(60..=86400).contains(&input.expires_in) {
        return Err(bad(
            StatusCode::CONFLICT,
            "Revisi atau masa berlaku tautan tidak valid.",
        ));
    }
    super::domain::key(&input.key)
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Object key tidak valid."))?;
    let access = load(&state, &id).await?;
    if connection.managed && access.state != "ready" {
        return Err(bad(
            StatusCode::CONFLICT,
            "Sinkronkan perubahan akses bucket sebelum mengambil URL.",
        ));
    }
    if connection.managed && access.visibility == "public" {
        if connection.host().map_err(db_error)?
            == handlers::panel(&state, &headers).await?.host.as_str()
        {
            return Err(failure(Failure::Endpoint));
        }
        return Ok(ok(
            json!({"kind":"public","url":signing::public_url(&connection,&input.key).map_err(failure)?,"expires_at":null}),
        ));
    }
    let outcome = handlers::run(
        &state,
        &services,
        &headers,
        &connection,
        Operation::AccessLink {
            key: input.key,
            expires_in: input.expires_in,
        },
    )
    .await?;
    let Outcome::Download { url, expires_at } = outcome else {
        return Err(failure(Failure::Invalid));
    };
    Ok(ok(
        json!({"kind":"signed","url":url,"expires_at":expires_at}),
    ))
}
