//! Managed S3 allocation. The agent owns lifecycle; the API owns tenant authority.
use super::*;
use crate::auth::{AuthenticatedUser, RequireAdminCsrf, RequireAuth, RequireCsrf};
use axum::{
    extract::{Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use panel_core::connectivity::EndpointHost;
use panel_proto::{ManagedStorageRequest, StorageSecretAction};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedStorageConfiguration {
    hostname: String,
    fullchain_pem: String,
    #[schema(write_only)]
    private_key_pem: String,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedStorageAllocation {
    #[schema(value_type = String)]
    request_id: Uuid,
    owner_id: i64,
    site_id: Option<i64>,
    label: String,
    bucket: String,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedStorageKeyInput {
    expected_revision: i64,
    expected_generation: i64,
    confirmation: String,
    #[serde(default)]
    retry: bool,
}

async fn control(services: &Services, request: ManagedStorageRequest) -> Result<Value, Error> {
    let value = services.runtime.managed(request).await.map_err(failure)?;
    if let Some(message) = value.get("error").and_then(Value::as_str) {
        return Err(bad(StatusCode::SERVICE_UNAVAILABLE, message));
    }
    Ok(value)
}
fn admin(actor: &AuthenticatedUser) -> Result<(), Error> {
    if !actor.is_admin() {
        return Err(bad(
            StatusCode::FORBIDDEN,
            "Pengelolaan layanan storage hanya untuk administrator.",
        ));
    }
    Ok(())
}
fn new_keys() -> S3Credentials {
    S3Credentials {
        access_key: format!("qp{}", Uuid::new_v4().simple()),
        secret_key: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
        session_token: None,
    }
}
fn secret(keys: &S3Credentials) -> Result<StorageSecret, Error> {
    serde_json::to_string(keys)
        .map(StorageSecret)
        .map_err(db_error)
}
fn require_managed(connection: &Connection) -> Result<(), Error> {
    if !connection.managed {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Operasi ini hanya berlaku untuk bucket Qinpanel.",
        ));
    }
    Ok(())
}

#[utoipa::path(get, path="/api/v1/storage/managed", responses((status=200,body=Value)), tag="storage")]
pub async fn status(
    Extension(services): Extension<Services>,
    RequireAuth(actor): RequireAuth,
) -> Result<Response, Error> {
    admin(&actor)?;
    Ok(ok(
        control(&services, ManagedStorageRequest::Status {}).await?
    ))
}
#[utoipa::path(post, path="/api/v1/storage/managed", request_body=ManagedStorageConfiguration, responses((status=200,body=Value)), tag="storage")]
pub async fn configure(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    headers: HeaderMap,
    value: Result<Json<ManagedStorageConfiguration>, JsonRejection>,
) -> Result<Response, Error> {
    let value = handlers::input(value)?;
    let host = EndpointHost::parse(&value.hostname).map_err(|_| {
        bad(
            StatusCode::BAD_REQUEST,
            "Gunakan domain storage yang valid.",
        )
    })?;
    if host.as_str() != value.hostname || host == handlers::panel(&state, &headers).await?.host {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Gunakan hostname HTTPS tersendiri, terpisah dari domain panel.",
        ));
    }
    let hostname = value.hostname.clone();
    let revision = super::tls::reserve_manual(&state.pool).await?;
    let result = control(
        &services,
        ManagedStorageRequest::Configure {
            hostname: value.hostname,
            fullchain_pem: StorageSecret(value.fullchain_pem),
            private_key_pem: StorageSecret(value.private_key_pem),
        },
    )
    .await;
    super::tls::finish_manual(&state.pool, revision, &hostname, result.is_ok()).await?;
    handlers::log(
        &state,
        &actor,
        "storage.managed.configure",
        "storage",
        result.is_ok(),
        json!({"engine":"seaweedfs","version":"4.50"}),
    );
    Ok(ok(result?))
}
async fn endpoint(services: &Services) -> Result<String, Error> {
    let value = control(services, ManagedStorageRequest::Status {}).await?;
    if value["configured"] != true || value["running"] != true {
        return Err(bad(
            StatusCode::CONFLICT,
            "Aktifkan layanan Storage Qinpanel dan periksa statusnya sebelum membuat bucket.",
        ));
    }
    value["endpoint"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failure(Failure::Invalid))
}
#[utoipa::path(post, path="/api/v1/storage/managed/buckets", request_body=ManagedStorageAllocation, responses((status=200,body=Value)), tag="storage")]
pub async fn create(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    value: Result<Json<ManagedStorageAllocation>, JsonRejection>,
) -> Result<Response, Error> {
    let value = handlers::input(value)?;
    if value.request_id.is_nil() || !valid_bucket(&value.bucket) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Gunakan nama bucket 3–63 huruf kecil, angka atau tanda hubung.",
        ));
    }
    store::tenant(&state, &actor, value.owner_id, value.site_id).await?;
    let id = value.request_id.to_string();
    let existing: Option<Connection> =
        sqlx::query_as("SELECT * FROM storage_connections WHERE id=?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .map_err(db_error)?;
    let connection = if let Some(connection) = existing {
        require_managed(&connection)?;
        if connection.owner_id != value.owner_id
            || connection.bucket != value.bucket
            || connection.site_id != value.site_id
        {
            return Err(bad(
                StatusCode::CONFLICT,
                "Identitas permintaan sudah dipakai untuk alokasi berbeda.",
            ));
        }
        connection
    } else {
        reserve_allocation(&state, &services, value).await?
    };
    reconcile(&state, &services, &actor, connection).await
}
fn valid_bucket(value: &str) -> bool {
    (3..=63).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}
async fn reserve_allocation(
    state: &AppState,
    services: &Services,
    value: ManagedStorageAllocation,
) -> Result<Connection, Error> {
    let id = value.request_id.to_string();
    let cipher = handlers::seal(services, &id, value.owner_id, new_keys()).await?;
    let now = chrono::Utc::now().timestamp();
    let connection = Connection {
        id,
        owner_id: value.owner_id,
        site_id: value.site_id,
        label: value.label.trim().to_owned(),
        provider: "compatible".into(),
        endpoint: endpoint(services).await?,
        region: "us-east-1".into(),
        addressing: "path".into(),
        bucket: value.bucket,
        credentials_enc: cipher,
        revision: 1,
        verified_revision: None,
        verified_at: None,
        created_at: now,
        updated_at: now,
        last_operation: None,
        managed: true,
        managed_state: Some("pending".into()),
        application_key_generation: 0,
        application_key_state: None,
    };
    connection.validate().map_err(|_| {
        bad(
            StatusCode::BAD_REQUEST,
            "Nama atau konfigurasi bucket tidak valid.",
        )
    })?;
    sqlx::query("INSERT INTO storage_connections(id,owner_id,site_id,label,provider,endpoint,region,addressing,bucket,credentials_enc,created_at,updated_at,managed,managed_state) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,1,'pending')")
        .bind(&connection.id).bind(connection.owner_id).bind(connection.site_id).bind(&connection.label).bind(&connection.provider).bind(&connection.endpoint).bind(&connection.region).bind(&connection.addressing).bind(&connection.bucket).bind(&connection.credentials_enc).bind(now).bind(now).execute(&state.pool).await
        .map_err(|_|bad(StatusCode::CONFLICT,"Nama bucket atau identitas alokasi sudah digunakan. Muat ulang daftar storage."))?;
    Ok(connection)
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}/provision", request_body=handlers::StorageConfirmation, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn provision(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    value: Result<Json<handlers::StorageConfirmation>, JsonRejection>,
) -> Result<Response, Error> {
    let value = handlers::input(value)?;
    let connection = store::load(&state, &actor, &id).await?;
    handlers::ensure_revision(&connection, value.expected_revision)?;
    require_managed(&connection)?;
    reconcile(&state, &services, &actor, connection).await
}
async fn reconcile(
    state: &AppState,
    services: &Services,
    actor: &AuthenticatedUser,
    mut connection: Connection,
) -> Result<Response, Error> {
    if matches!(
        connection.managed_state.as_deref(),
        Some("deleting" | "deleted")
    ) {
        return Err(bad(
            StatusCode::CONFLICT,
            "Penghapusan bucket belum selesai. Lanjutkan penghapusan yang sama.",
        ));
    }
    // A reservation is durable before touching the engine. Repeating it supplies
    // the same owner, bucket and panel credential, even after a lost HTTP reply.
    let keys = handlers::credentials(services, &connection).await?;
    let result = control(
        services,
        ManagedStorageRequest::Provision {
            connection_id: Uuid::parse_str(&connection.id).map_err(db_error)?,
            owner_id: connection.owner_id,
            bucket: connection.bucket.clone(),
            credentials: secret(&keys)?,
        },
    )
    .await;
    let ready = result.as_ref().is_ok_and(|v| v["ready"] == true);
    connection.managed_state = Some(if ready { "ready" } else { "unknown" }.into());
    let changed = sqlx::query(
        "UPDATE storage_connections SET managed_state=?,updated_at=? WHERE id=? AND managed=1 AND managed_state IN ('pending','ready','unknown')",
    )
    .bind(&connection.managed_state)
    .bind(chrono::Utc::now().timestamp())
    .bind(&connection.id)
    .execute(&state.pool)
    .await
    .map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Status bucket berubah. Muat ulang.",
        ));
    }
    handlers::log(
        state,
        actor,
        "storage.managed.provision",
        &connection.id,
        ready,
        json!({"owner_id":connection.owner_id}),
    );
    result?;
    if !ready {
        return Err(failure(Failure::Invalid));
    }
    Ok(ok(store::view(&connection)))
}

async fn key_credentials(
    services: &Services,
    connection: &Connection,
    cipher: String,
) -> Result<S3Credentials, Error> {
    let raw = services
        .runtime
        .secret(StorageSecretRequest {
            connection_id: Uuid::parse_str(&connection.id).map_err(db_error)?,
            owner_id: connection.owner_id,
            action: StorageSecretAction::Open,
            value: StorageSecret(cipher),
        })
        .await
        .map_err(failure)?;
    let keys: S3Credentials = serde_json::from_str(&raw.0).map_err(db_error)?;
    if !keys.valid() {
        return Err(failure(Failure::Invalid));
    }
    Ok(keys)
}
async fn reserve_key(
    state: &AppState,
    services: &Services,
    connection: &Connection,
    input: &ManagedStorageKeyInput,
) -> Result<(S3Credentials, i64), Error> {
    if input.retry {
        let cipher: Option<String> = sqlx::query_scalar("SELECT application_credentials_enc FROM storage_connections WHERE id=? AND application_key_generation=? AND application_key_state IN ('pending','unknown')")
            .bind(&connection.id).bind(input.expected_generation).fetch_optional(&state.pool).await.map_err(db_error)?.flatten();
        return Ok((
            key_credentials(
                services,
                connection,
                cipher.ok_or_else(|| {
                    bad(
                        StatusCode::CONFLICT,
                        "Tidak ada operasi key tertunda. Muat ulang.",
                    )
                })?,
            )
            .await?,
            input.expected_generation,
        ));
    }
    let keys = new_keys();
    let cipher =
        handlers::seal(services, &connection.id, connection.owner_id, keys.clone()).await?;
    let generation = input
        .expected_generation
        .checked_add(1)
        .filter(|g| *g <= i64::from(i32::MAX))
        .ok_or_else(|| bad(StatusCode::CONFLICT, "Generasi key tidak valid."))?;
    let changed = sqlx::query("UPDATE storage_connections SET application_credentials_enc=?,application_key_generation=?,application_key_state='pending' WHERE id=? AND revision=? AND application_key_generation=? AND managed=1 AND managed_state='ready' AND (application_key_state IS NULL OR application_key_state='ready')")
        .bind(cipher).bind(generation).bind(&connection.id).bind(connection.revision).bind(input.expected_generation).execute(&state.pool).await.map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Key berubah atau operasi belum selesai. Muat ulang dan sinkronkan operasi tertunda.",
        ));
    }
    Ok((keys, generation))
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}/application-key", request_body=ManagedStorageKeyInput, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn application_key(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    value: Result<Json<ManagedStorageKeyInput>, JsonRejection>,
) -> Result<Response, Error> {
    let input = handlers::input(value)?;
    let connection = store::load_content(&state, &actor, &id).await?;
    require_managed(&connection)?;
    handlers::ensure_revision(&connection, input.expected_revision)?;
    if input.confirmation != connection.bucket
        || input.expected_generation != connection.application_key_generation
        || connection.managed_state.as_deref() != Some("ready")
    {
        return Err(bad(
            StatusCode::CONFLICT,
            "Konfirmasi bucket dan generasi key harus sesuai. Sinkronkan bucket lebih dahulu.",
        ));
    }
    let (keys, generation) = reserve_key(&state, &services, &connection, &input).await?;
    let result = control(
        &services,
        ManagedStorageRequest::ApplicationKey {
            connection_id: Uuid::parse_str(&id).map_err(db_error)?,
            owner_id: connection.owner_id,
            bucket: connection.bucket.clone(),
            generation: u32::try_from(generation).map_err(db_error)?,
            credentials: secret(&keys)?,
        },
    )
    .await;
    let ready = result
        .as_ref()
        .is_ok_and(|v| v["ready"] == true && v["generation"] == generation);
    let changed = sqlx::query("UPDATE storage_connections SET application_key_state=? WHERE id=? AND application_key_generation=? AND application_key_state IN ('pending','unknown')")
        .bind(if ready {"ready"} else {"unknown"}).bind(&id).bind(generation).execute(&state.pool).await.map_err(db_error)?;
    handlers::log(
        &state,
        &actor,
        "storage.managed.key",
        &id,
        ready,
        json!({"generation":generation}),
    );
    result?;
    if !ready {
        return Err(failure(Failure::Invalid));
    }
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Key sudah diterbitkan melalui permintaan lain. Muat ulang; rotasi lagi bila salinannya hilang.",
        ));
    }
    // No GET endpoint can return this value. A lost final reply requires explicit
    // rotation; pending/unknown operations resume with the same encrypted key.
    Ok(ok(json!({"credentials":keys,"generation":generation})))
}

#[utoipa::path(post, path="/api/v1/storage/connections/{id}/decommission", request_body=handlers::StorageConfirmation, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn decommission(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    value: Result<Json<handlers::StorageConfirmation>, JsonRejection>,
) -> Result<Response, Error> {
    let input = handlers::input(value)?;
    let connection = store::load(&state, &actor, &id).await?;
    require_managed(&connection)?;
    handlers::ensure_revision(&connection, input.expected_revision)?;
    if input.confirmation.as_deref() != Some(connection.bucket.as_str()) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Ketik nama bucket untuk menghapus bucket kosong.",
        ));
    }
    let changed = sqlx::query("UPDATE storage_connections SET managed_state='deleting' WHERE id=? AND revision=? AND (application_key_state IS NULL OR application_key_state='ready') AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
        .bind(&id).bind(connection.revision).bind(&id).bind(chrono::Utc::now().timestamp()-120).execute(&state.pool).await.map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Transfer atau operasi key belum selesai. Sinkronkan sebelum menghapus bucket.",
        ));
    }
    let result = control(
        &services,
        ManagedStorageRequest::Decommission {
            connection_id: Uuid::parse_str(&id).map_err(db_error)?,
            owner_id: connection.owner_id,
            bucket: connection.bucket,
        },
    )
    .await;
    handlers::log(
        &state,
        &actor,
        "storage.managed.remove",
        &id,
        result.as_ref().is_ok_and(|v| v["removed"] == true),
        json!({"empty_only":true}),
    );
    let result = result?; // Unknown outcomes retain 'deleting' and require the same explicit operation.
    if result["not_empty"] == true {
        sqlx::query("UPDATE storage_connections SET managed_state=? WHERE id=? AND managed_state='deleting'")
            .bind(connection.managed_state).bind(&id).execute(&state.pool).await.map_err(db_error)?;
        return Err(bad(
            StatusCode::CONFLICT,
            "Bucket masih berisi object atau versi object. Kosongkan dengan SDK/panel sebelum menghapus bucket.",
        ));
    }
    if result["removed"] != true {
        return Err(failure(Failure::Invalid));
    }
    let mut transaction = state.pool.begin().await.map_err(db_error)?;
    sqlx::query("UPDATE storage_connections SET managed_state='deleted' WHERE id=? AND managed_state='deleting'")
        .bind(&id).execute(&mut *transaction).await.map_err(db_error)?;
    sqlx::query("DELETE FROM storage_connections WHERE id=? AND managed_state='deleted'")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
    transaction.commit().await.map_err(db_error)?;
    Ok(ok(json!({"removed":true})))
}

#[cfg(test)]
mod tests {
    use super::ManagedStorageAllocation;

    #[test]
    fn allocation_requires_valid_uuid_despite_string_schema() {
        let request_id = "63a83d1c-710a-49a1-97d0-a7b6c1d2ed70";
        let mut allocation = serde_json::json!({
            "request_id": request_id,
            "owner_id": 1,
            "site_id": null,
            "label": "Project files",
            "bucket": "project-files"
        });
        let parsed = serde_json::from_value::<ManagedStorageAllocation>(allocation.clone())
            .expect("valid allocation UUID should deserialize");
        assert_eq!(parsed.request_id.to_string(), request_id);

        allocation["request_id"] = serde_json::json!("invalid-uuid");
        assert!(serde_json::from_value::<ManagedStorageAllocation>(allocation).is_err());
    }
}
