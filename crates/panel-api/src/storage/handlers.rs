use super::*;
use crate::auth::{AuthenticatedUser, RequireAdminCsrf, RequireAuth, RequireCsrf};
use axum::extract::{Path, Query, Request, State, rejection::JsonRejection};
use axum::http::HeaderMap;
use panel_core::connectivity::EndpointHost;
use panel_proto::{StorageSecretAction, StorageSecretRequest};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageConfiguration {
    pub expected_revision: i64,
    pub owner_id: i64,
    pub site_id: Option<i64>,
    pub label: String,
    pub provider: String,
    pub endpoint: String,
    pub region: String,
    pub addressing: String,
    pub bucket: String,
    pub credentials: Option<S3Credentials>,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageConfirmation {
    pub expected_revision: i64,
    pub confirmation: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub site_id: Option<i64>,
    #[serde(default)]
    pub prefix: String,
    pub token: Option<String>,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageObjectInput {
    pub expected_revision: i64,
    pub key: String,
    #[serde(default)]
    pub overwrite: bool,
    pub confirmation: Option<String>,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageBinding {
    pub expected_revision: i64,
    pub site_id: Option<i64>,
}
pub(super) fn input<T>(value: Result<Json<T>, JsonRejection>) -> Result<T, Error> {
    value
        .map(|v| v.0)
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Isian storage tidak valid."))
}
pub(super) fn ensure_revision(connection: &Connection, revision: i64) -> Result<(), Error> {
    if connection.revision != revision {
        return Err(bad(
            StatusCode::CONFLICT,
            "Koneksi berubah. Muat ulang sebelum melanjutkan.",
        ));
    }
    Ok(())
}
pub(super) fn log(
    state: &AppState,
    actor: &AuthenticatedUser,
    action: &str,
    target: &str,
    success: bool,
    detail: Value,
) {
    state.audit_writer.log_async(
        Some(actor.id),
        actor.username.clone(),
        action,
        target,
        if success { "success" } else { "failed" },
        None,
        Some(detail.to_string()),
    );
}
#[utoipa::path(get, path="/api/v1/storage/context", responses((status=200,body=Value)), tag="storage")]
pub async fn context(
    State(state): State<AppState>,
    RequireAuth(actor): RequireAuth,
) -> Result<Response, Error> {
    let owners:Vec<(i64,String)>=sqlx::query_as("SELECT id,username FROM users WHERE role='user' AND status='active' AND linux_uid BETWEEN 10000 AND 59999 AND (?=1 OR id=?) ORDER BY username LIMIT 1000")
        .bind(actor.is_admin()).bind(actor.id).fetch_all(&state.pool).await.map_err(db_error)?;
    let sites:Vec<(i64,i64,String)>=sqlx::query_as("SELECT s.id,s.user_id,d.name FROM sites s JOIN users u ON u.id=s.user_id JOIN domains d ON d.site_id=s.id AND d.is_primary=1 WHERE u.role='user' AND u.status='active' AND (?=1 OR s.user_id=?) ORDER BY d.name LIMIT 5000")
        .bind(actor.is_admin()).bind(actor.id).fetch_all(&state.pool).await.map_err(db_error)?;
    Ok(ok(
        json!({"owners":owners.into_iter().map(|(id,username)|json!({"id":id,"username":username})).collect::<Vec<_>>(),"sites":sites.into_iter().map(|(id,owner_id,domain)|json!({"id":id,"owner_id":owner_id,"domain":domain})).collect::<Vec<_>>(),"upload_limit_bytes":UPLOAD_LIMIT}),
    ))
}
#[utoipa::path(get, path="/api/v1/storage/connections", responses((status=200,body=Value)), tag="storage")]
pub async fn list(
    State(state): State<AppState>,
    RequireAuth(actor): RequireAuth,
    Query(query): Query<ListQuery>,
) -> Result<Response, Error> {
    if let Some(site) = query.site_id {
        crate::tenant_context::resolve_tenant_context(&state, &actor, None, Some(site)).await?;
    }
    let values:Vec<Connection>=sqlx::query_as("SELECT c.*,(SELECT CASE WHEN state='running' AND created_at<=? THEN 'unknown' ELSE state END FROM storage_operations WHERE connection_id=c.id ORDER BY created_at DESC,rowid DESC LIMIT 1) AS last_operation FROM storage_connections c WHERE (?=1 OR owner_id=?) AND (? IS NULL OR site_id=?) ORDER BY label LIMIT 500")
        .bind(chrono::Utc::now().timestamp()-120).bind(actor.is_admin()).bind(actor.id).bind(query.site_id).bind(query.site_id).fetch_all(&state.pool).await.map_err(db_error)?;
    Ok(ok(
        json!({"connections":values.iter().map(store::view).collect::<Vec<_>>(),"upload_limit_bytes":UPLOAD_LIMIT}),
    ))
}
fn from_input(
    value: StorageConfiguration,
    id: String,
    revision: i64,
    cipher: String,
) -> Result<Connection, Error> {
    let mut connection = Connection {
        id,
        owner_id: value.owner_id,
        site_id: value.site_id,
        label: value.label.trim().into(),
        provider: value.provider,
        endpoint: value.endpoint,
        region: value.region,
        addressing: value.addressing,
        bucket: value.bucket,
        credentials_enc: cipher,
        revision,
        verified_revision: None,
        verified_at: None,
        created_at: chrono::Utc::now().timestamp(),
        updated_at: chrono::Utc::now().timestamp(),
        last_operation: None,
        managed: false,
        managed_state: None,
        application_key_generation: 0,
        application_key_state: None,
    };
    connection.validate().map_err(|_|bad(StatusCode::BAD_REQUEST,"Periksa nama, tenant, bucket, region dan endpoint HTTPS publik. R2 memakai region auto/path-style; Supabase memakai endpoint /storage/v1/s3."))?;
    let host = connection
        .core()
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Host tidak valid."))?
        .config
        .endpoint
        .host;
    connection.endpoint = format!("https://{}{}", host.as_str(), connection.base_path());
    Ok(connection)
}
pub(super) async fn seal(
    services: &Services,
    id: &str,
    owner: i64,
    credentials: S3Credentials,
) -> Result<String, Error> {
    if !credentials.valid() {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Gunakan access key ID, secret access key dan session token S3 yang valid.",
        ));
    }
    let request = StorageSecretRequest {
        connection_id: Uuid::parse_str(id).map_err(db_error)?,
        owner_id: owner,
        action: StorageSecretAction::Seal,
        value: StorageSecret(serde_json::to_string(&credentials).map_err(db_error)?),
    };
    let cipher=services.runtime.secret(request).await.map_err(|_|bad(StatusCode::SERVICE_UNAVAILABLE,"Credential belum dapat dienkripsi. Perbarui paket API/agent lengkap dan periksa master key."))?.0;
    if !cipher.starts_with("enc:v1:") {
        return Err(bad(
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent mengembalikan credential tidak valid.",
        ));
    }
    Ok(cipher)
}
pub(super) async fn credentials(
    services: &Services,
    connection: &Connection,
) -> Result<S3Credentials, Error> {
    let request = StorageSecretRequest {
        connection_id: Uuid::parse_str(&connection.id).map_err(db_error)?,
        owner_id: connection.owner_id,
        action: StorageSecretAction::Open,
        value: StorageSecret(connection.credentials_enc.clone()),
    };
    let secret = services.runtime.secret(request).await.map_err(|_| {
        bad(
            StatusCode::SERVICE_UNAVAILABLE,
            "Credential tidak dapat dibuka oleh agent.",
        )
    })?;
    let value: S3Credentials = serde_json::from_str(&secret.0).map_err(db_error)?;
    if !value.valid() {
        return Err(bad(
            StatusCode::SERVICE_UNAVAILABLE,
            "Credential S3 tidak valid.",
        ));
    }
    Ok(value)
}
fn write_error(error: sqlx::Error) -> Error {
    if error
        .as_database_error()
        .is_some_and(|v| v.is_unique_violation())
    {
        bad(
            StatusCode::CONFLICT,
            "Endpoint dan bucket sudah terhubung. Muat ulang daftar koneksi.",
        )
    } else {
        db_error(error)
    }
}
#[utoipa::path(post, path="/api/v1/storage/connections", request_body=StorageConfiguration, responses((status=200,body=Value)), tag="storage")]
pub async fn create(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    value: Result<Json<StorageConfiguration>, JsonRejection>,
) -> Result<Response, Error> {
    let mut value = input(value)?;
    if value.expected_revision != 0 {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Revisi koneksi baru harus nol.",
        ));
    }
    store::tenant(&state, &actor, value.owner_id, value.site_id).await?;
    let id = Uuid::new_v4().to_string();
    let key = value
        .credentials
        .take()
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST, "Isi credential S3 khusus bucket."))?;
    let mut connection = from_input(value, id, 1, String::new())?;
    connection.credentials_enc = seal(&services, &connection.id, connection.owner_id, key).await?;
    sqlx::query("INSERT INTO storage_connections(id,owner_id,site_id,label,provider,endpoint,region,addressing,bucket,credentials_enc,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&connection.id).bind(connection.owner_id).bind(connection.site_id).bind(&connection.label).bind(&connection.provider).bind(&connection.endpoint).bind(&connection.region).bind(&connection.addressing).bind(&connection.bucket).bind(&connection.credentials_enc).bind(connection.created_at).bind(connection.updated_at).execute(&state.pool).await.map_err(write_error)?;
    log(
        &state,
        &actor,
        "storage.configure",
        &connection.id,
        true,
        json!({"owner_id":connection.owner_id,"revision":1}),
    );
    Ok(ok(store::view(&connection)))
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}", request_body=StorageConfiguration, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn update(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    value: Result<Json<StorageConfiguration>, JsonRejection>,
) -> Result<Response, Error> {
    let mut value = input(value)?;
    let previous = store::load(&state, &actor, &id).await?;
    if previous.managed {
        return Err(bad(
            StatusCode::CONFLICT,
            "Gunakan pengelolaan bucket Qinpanel untuk storage ini.",
        ));
    }
    ensure_revision(&previous, value.expected_revision)?;
    if previous.owner_id != value.owner_id {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Pemilik koneksi tidak dapat dipindah ke tenant lain.",
        ));
    }
    store::tenant(&state, &actor, value.owner_id, value.site_id).await?;
    store::available(&state.pool, &id, previous.revision).await?;
    let keys = value.credentials.take();
    let mut connection = from_input(
        value,
        id,
        previous.revision + 1,
        previous.credentials_enc.clone(),
    )?;
    if keys.is_none()
        && (connection.endpoint != previous.endpoint
            || connection.provider != previous.provider
            || connection.bucket != previous.bucket)
    {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Isi ulang credential khusus bucket saat mengganti provider, endpoint atau bucket.",
        ));
    }
    if let Some(keys) = keys {
        connection.credentials_enc =
            seal(&services, &connection.id, connection.owner_id, keys).await?;
    }
    let changed=sqlx::query("UPDATE storage_connections SET site_id=?,label=?,provider=?,endpoint=?,region=?,addressing=?,bucket=?,credentials_enc=?,revision=revision+1,verified_revision=NULL,verified_at=NULL,updated_at=? WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
        .bind(connection.site_id).bind(&connection.label).bind(&connection.provider).bind(&connection.endpoint).bind(&connection.region).bind(&connection.addressing).bind(&connection.bucket).bind(&connection.credentials_enc).bind(connection.updated_at).bind(&connection.id).bind(previous.revision).bind(&connection.id).bind(chrono::Utc::now().timestamp()-120).execute(&state.pool).await.map_err(write_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Koneksi berubah atau sedang dipakai. Muat ulang.",
        ));
    }
    connection.created_at = previous.created_at;
    log(
        &state,
        &actor,
        "storage.configure",
        &connection.id,
        true,
        json!({"revision":connection.revision}),
    );
    Ok(ok(store::view(&connection)))
}
pub(super) async fn panel(state: &AppState, headers: &HeaderMap) -> Result<StorageEndpoint, Error> {
    let origin = match crate::public_url::canonical(&state.pool)
        .await
        .map_err(db_error)?
    {
        Some(value) => value,
        None => crate::public_url::PanelOrigin::from_authority(
            headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| bad(StatusCode::BAD_REQUEST, "Host panel tidak tersedia."))?,
            true,
        )
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Host panel tidak valid."))?,
    };
    Ok(StorageEndpoint {
        host: EndpointHost::parse(&origin.host).map_err(db_error)?,
        port: origin.port,
    })
}
pub(super) async fn run(
    state: &AppState,
    services: &Services,
    headers: &HeaderMap,
    connection: &Connection,
    operation: Operation,
) -> Result<Outcome, Error> {
    let keys = credentials(services, connection).await?;
    services
        .runtime
        .execute(
            connection.clone(),
            keys,
            operation,
            panel(state, headers).await?,
        )
        .await
        .map_err(failure)
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}/test", request_body=StorageConfirmation, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn test(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    headers: HeaderMap,
    value: Result<Json<StorageConfirmation>, JsonRejection>,
) -> Result<Response, Error> {
    let value = input(value)?;
    let connection = store::load_content(&state, &actor, &id).await?;
    super::policy::authorize(&state.pool, &id, "read").await?;
    ensure_revision(&connection, value.expected_revision)?;
    run(
        &state,
        &services,
        &headers,
        &connection,
        Operation::List {
            prefix: String::new(),
            token: None,
        },
    )
    .await?;
    store::verify(&state.pool, &connection).await?;
    log(
        &state,
        &actor,
        "storage.test",
        &id,
        true,
        json!({"revision":connection.revision,"access":"list"}),
    );
    Ok(ok(
        json!({"success":true,"revision":connection.revision,"verified_at":chrono::Utc::now().timestamp()}),
    ))
}
#[utoipa::path(get, path="/api/v1/storage/connections/{id}/objects", params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn objects(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAuth(actor): RequireAuth,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Response, Error> {
    if !list_input(&query.prefix, query.token.as_deref()) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Prefix atau token tidak valid.",
        ));
    }
    let connection = store::load_content(&state, &actor, &id).await?;
    super::policy::authorize(&state.pool, &id, "read").await?;
    let outcome = run(
        &state,
        &services,
        &headers,
        &connection,
        Operation::List {
            prefix: query.prefix,
            token: query.token,
        },
    )
    .await?;
    store::verify(&state.pool, &connection).await?;
    let Outcome::Listing(listing) = outcome else {
        return Err(failure(Failure::Invalid));
    };
    Ok(ok(
        json!({"listing":listing,"revision":connection.revision,"verified_at":chrono::Utc::now().timestamp()}),
    ))
}
async fn mutate(
    state: &AppState,
    services: &Services,
    actor: &AuthenticatedUser,
    headers: &HeaderMap,
    connection: &Connection,
    operation: Operation,
    action: &str,
) -> Result<Outcome, Error> {
    let keys = credentials(services, connection).await?;
    let origin = panel(state, headers).await?;
    let id = store::reserve(&state.pool, connection, actor.id, action).await?;
    if let Operation::Upload { key, body, .. } = &operation
        && let Err(error) =
            super::policy::admit_upload(state, services, headers, connection, key, body.len()).await
    {
        store::finish(&state.pool, &id, &Err(Failure::Provider(413))).await?;
        return Err(error);
    }

    let result = services
        .runtime
        .execute(connection.clone(), keys, operation, origin)
        .await;
    store::finish(&state.pool, &id, &result).await?;
    log(
        state,
        actor,
        &format!("storage.{action}"),
        &connection.id,
        result.is_ok(),
        json!({"operation_id":id,"revision":connection.revision,"outcome":store::outcome_state(&result)}),
    );
    result.map_err(failure)
}
fn require_verified(connection: &Connection) -> Result<(), Error> {
    if connection.verified_revision != Some(connection.revision)
        || !connection
            .verified_at
            .is_some_and(|t| (0..600).contains(&(chrono::Utc::now().timestamp() - t)))
    {
        return Err(bad(
            StatusCode::CONFLICT,
            "Uji koneksi atau muat ulang daftar object sebelum menulis.",
        ));
    }
    Ok(())
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}/upload", params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn upload(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    Query(query): Query<StorageObjectInput>,
    request: Request,
) -> Result<Response, Error> {
    let _permit = services.transfers.try_acquire().map_err(|_| {
        bad(
            StatusCode::TOO_MANY_REQUESTS,
            "Transfer lain masih berjalan. Coba lagi nanti.",
        )
    })?;
    let connection = store::load_content(&state, &actor, &id).await?;
    super::policy::authorize(&state.pool, &id, "write").await?;
    ensure_revision(&connection, query.expected_revision)?;
    require_verified(&connection)?;
    if !query.overwrite && !connection.conditional_put() {
        return Err(failure(Failure::ConditionalUnsupported));
    }
    super::domain::key(&query.key)
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Object key tidak valid."))?;
    let headers = request.headers().clone();
    let body=tokio::time::timeout(std::time::Duration::from_secs(60),axum::body::to_bytes(request.into_body(),UPLOAD_LIMIT)).await
        .map_err(|_|bad(StatusCode::REQUEST_TIMEOUT,"Upload ke panel melewati batas waktu."))?
        .map_err(|_|bad(StatusCode::PAYLOAD_TOO_LARGE,"Upload melalui panel dibatasi 32 MiB per file. Gunakan SDK S3 multipart untuk file lebih besar."))?;
    mutate(
        &state,
        &services,
        &actor,
        &headers,
        &connection,
        Operation::Upload {
            key: query.key,
            body,
            overwrite: query.overwrite,
        },
        "upload",
    )
    .await?;
    Ok(ok(json!({"success":true})))
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}/delete", request_body=StorageObjectInput, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn delete_object(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    headers: HeaderMap,
    value: Result<Json<StorageObjectInput>, JsonRejection>,
) -> Result<Response, Error> {
    let value = input(value)?;
    let connection = store::load_content(&state, &actor, &id).await?;
    super::policy::authorize(&state.pool, &id, "delete").await?;
    ensure_revision(&connection, value.expected_revision)?;
    require_verified(&connection)?;
    if value.confirmation.as_deref() != Some(&value.key) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Konfirmasi object yang akan dihapus.",
        ));
    }
    super::domain::key(&value.key)
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Object key tidak valid."))?;
    mutate(
        &state,
        &services,
        &actor,
        &headers,
        &connection,
        Operation::Delete { key: value.key },
        "delete",
    )
    .await?;
    Ok(ok(json!({"success":true})))
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}/download", request_body=StorageObjectInput, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn download(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    headers: HeaderMap,
    value: Result<Json<StorageObjectInput>, JsonRejection>,
) -> Result<Response, Error> {
    let value = input(value)?;
    let connection = store::load_content(&state, &actor, &id).await?;
    super::policy::authorize(&state.pool, &id, "read").await?;
    ensure_revision(&connection, value.expected_revision)?;
    require_verified(&connection)?;
    super::domain::key(&value.key)
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Object key tidak valid."))?;
    let keys = credentials(&services, &connection).await?;
    if connection.managed || keys.session_token.is_some() {
        let expires_at = chrono::Utc::now().timestamp() + 60;
        let query = super::signing::query(&[
            ("key".into(), value.key),
            ("expected_revision".into(), connection.revision.to_string()),
            ("expires_at".into(), expires_at.to_string()),
        ]);
        return Ok(ok(
            json!({"url":format!("/api/v1/storage/connections/{}/download?{query}",connection.id),"expires_at":expires_at}),
        ));
    }
    let outcome = services
        .runtime
        .execute(
            connection.clone(),
            keys,
            Operation::Download { key: value.key },
            panel(&state, &headers).await?,
        )
        .await
        .map_err(failure)?;
    let Outcome::Download { url, expires_at } = outcome else {
        return Err(failure(Failure::Invalid));
    };
    Ok(ok(json!({"url":url,"expires_at":expires_at})))
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}/bucket", request_body=StorageConfirmation, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn create_bucket(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    headers: HeaderMap,
    value: Result<Json<StorageConfirmation>, JsonRejection>,
) -> Result<Response, Error> {
    let value = input(value)?;
    let connection = store::load(&state, &actor, &id).await?;
    if connection.managed {
        return Err(bad(
            StatusCode::CONFLICT,
            "Bucket Qinpanel tetap dipertahankan; operasi provider eksternal tidak berlaku.",
        ));
    }
    ensure_revision(&connection, value.expected_revision)?;
    if value.confirmation.as_deref() != Some(&connection.bucket) {
        return Err(bad(StatusCode::BAD_REQUEST, "Konfirmasi nama bucket baru."));
    }
    let probe = run(
        &state,
        &services,
        &headers,
        &connection,
        Operation::List {
            prefix: String::new(),
            token: None,
        },
    )
    .await;
    match probe {
        Err((StatusCode::NOT_FOUND, _)) => {}
        Ok(_) => {
            return Err(bad(
                StatusCode::CONFLICT,
                "Bucket sudah ada. Hubungkan bucket existing tanpa membuat ulang.",
            ));
        }
        Err(error) => return Err(error),
    }
    mutate(
        &state,
        &services,
        &actor,
        &headers,
        &connection,
        Operation::CreateBucket,
        "create_bucket",
    )
    .await?;
    Ok(ok(json!({"success":true})))
}
#[utoipa::path(post, path="/api/v1/storage/connections/{id}/binding", request_body=StorageBinding, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn bind(
    State(state): State<AppState>,
    RequireCsrf(actor): RequireCsrf,
    Path(id): Path<String>,
    value: Result<Json<StorageBinding>, JsonRejection>,
) -> Result<Response, Error> {
    let value = input(value)?;
    let connection = store::load(&state, &actor, &id).await?;
    ensure_revision(&connection, value.expected_revision)?;
    store::tenant(&state, &actor, connection.owner_id, value.site_id).await?;
    let changed=sqlx::query("UPDATE storage_connections SET site_id=?,revision=revision+1,verified_at=NULL,verified_revision=NULL,updated_at=? WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
        .bind(value.site_id).bind(chrono::Utc::now().timestamp()).bind(&id).bind(connection.revision).bind(&id).bind(chrono::Utc::now().timestamp()-120).execute(&state.pool).await.map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Koneksi berubah atau sedang dipakai.",
        ));
    }
    log(
        &state,
        &actor,
        "storage.bind",
        &id,
        true,
        json!({"site_id":value.site_id}),
    );
    Ok(ok(json!({"success":true})))
}
#[utoipa::path(delete, path="/api/v1/storage/connections/{id}", request_body=StorageConfirmation, params(("id"=String,Path)), responses((status=200,body=Value)), tag="storage")]
pub async fn disconnect(
    State(state): State<AppState>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    Path(id): Path<String>,
    value: Result<Json<StorageConfirmation>, JsonRejection>,
) -> Result<Response, Error> {
    let value = input(value)?;
    let connection = store::load(&state, &actor, &id).await?;
    if connection.managed {
        return Err(bad(
            StatusCode::CONFLICT,
            "Bucket Qinpanel tetap dipertahankan; operasi provider eksternal tidak berlaku.",
        ));
    }
    ensure_revision(&connection, value.expected_revision)?;
    if value.confirmation.as_deref() != Some(&connection.bucket) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Konfirmasi nama bucket untuk melepas koneksi.",
        ));
    }
    let changed=sqlx::query("DELETE FROM storage_connections WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM storage_operations WHERE connection_id=? AND state='running' AND created_at>?)")
        .bind(&id).bind(connection.revision).bind(&id).bind(chrono::Utc::now().timestamp()-120).execute(&state.pool).await.map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Koneksi berubah atau masih dipakai.",
        ));
    }
    log(
        &state,
        &actor,
        "storage.disconnect",
        &id,
        true,
        json!({"provider_data_deleted":false}),
    );
    Ok(ok(json!({"success":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadQuery {
    #[serde(default)]
    pub preview: bool,
    pub expected_revision: i64,
    pub key: String,
    pub expires_at: i64,
}
struct DownloadStream {
    bytes: axum::body::Bytes,
    offset: usize,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl futures_util::Stream for DownloadStream {
    type Item = Result<axum::body::Bytes, std::convert::Infallible>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let stream = self.get_mut();
        if stream.offset == stream.bytes.len() {
            return std::task::Poll::Ready(None);
        }
        let end = (stream.offset + 65536).min(stream.bytes.len());
        let data = axum::body::Bytes::copy_from_slice(&stream.bytes[stream.offset..end]);
        stream.offset = end;
        std::task::Poll::Ready(Some(Ok(data)))
    }
}
#[utoipa::path(get,path="/api/v1/storage/connections/{id}/download",params(("id"=String,Path)),responses((status=200,body=String)),tag="storage")]
pub async fn download_proxy(
    State(state): State<AppState>,
    Extension(services): Extension<Services>,
    RequireAuth(actor): RequireAuth,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<DownloadQuery>,
) -> Result<Response, Error> {
    if !query
        .expires_at
        .checked_sub(chrono::Utc::now().timestamp())
        .is_some_and(|delta| (0..=65).contains(&delta))
    {
        return Err(bad(
            StatusCode::GONE,
            "Tautan download kedaluwarsa. Minta tautan baru.",
        ));
    }
    let connection = store::load_content(&state, &actor, &id).await?;
    super::policy::authorize(&state.pool, &id, "read").await?;
    ensure_revision(&connection, query.expected_revision)?;
    super::domain::key(&query.key)
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Object key tidak valid."))?;
    let permit = Arc::clone(&services.transfers)
        .try_acquire_owned()
        .map_err(|_| {
            bad(
                StatusCode::TOO_MANY_REQUESTS,
                "Transfer lain masih berjalan.",
            )
        })?;
    let outcome = run(
        &state,
        &services,
        &headers,
        &connection,
        Operation::ReadObject {
            key: query.key.clone(),
        },
    )
    .await?;
    let Outcome::Content(bytes) = outcome else {
        return Err(failure(Failure::Invalid));
    };
    let name = query
        .key
        .rsplit('/')
        .next()
        .filter(|v| !v.is_empty())
        .unwrap_or("download");
    let content_type = if query.preview {
        preview_type(&bytes).ok_or_else(|| {
            bad(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Preview hanya mendukung PNG, JPEG, GIF dan WebP hingga 8 MiB.",
            )
        })?
    } else {
        "application/octet-stream"
    };
    let disposition_type = if query.preview {
        "inline"
    } else {
        "attachment"
    };
    let disposition = format!(
        "{disposition_type}; filename*=UTF-8''{}",
        super::signing::encode(name, false)
    );
    let length = bytes.len();
    let body = axum::body::Body::from_stream(DownloadStream {
        bytes,
        offset: 0,
        _permit: permit,
    });
    let mut response = Response::new(body);
    for (name, value) in [
        ("content-type", content_type.to_owned()),
        ("content-disposition", disposition),
        ("content-length", length.to_string()),
        ("cache-control", "no-store".to_owned()),
        ("x-content-type-options", "nosniff".to_owned()),
        (
            "content-security-policy",
            "sandbox; default-src 'none'".to_owned(),
        ),
    ] {
        response.headers_mut().insert(
            hyper::header::HeaderName::from_bytes(name.as_bytes()).map_err(db_error)?,
            hyper::header::HeaderValue::from_str(&value).map_err(db_error)?,
        );
    }
    Ok(response)
}

fn preview_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Some("image/webp");
    }
    None
}
