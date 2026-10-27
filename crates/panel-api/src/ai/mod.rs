//! Admin-only AI configuration and bounded, read-only hosting package drafts.
mod domain;
mod store;
pub mod transport;

use crate::{
    agent_client::AgentClient,
    auth::{AuthenticatedUser, RequireAdmin, RequireAdminCsrf},
    routes::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use domain::{Analysis, ProviderResult, models, parse_reply, profile_preset, request_body};
use futures_util::future::BoxFuture;
use panel_core::capacity::{CapacitySnapshot, PackageDemand, recommend_package};
use panel_proto::{AiProvider, AiSecret, AiSecretAction, AiSecretRequest};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use transport::ProviderError;
use utoipa::ToSchema;

type Error = (StatusCode, Json<Value>);
fn bad(status: StatusCode, message: &str) -> Error {
    (status, Json(json!({"error":message})))
}
fn db_error(_: impl Sized) -> Error {
    bad(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Konfigurasi atau pemakaian AI gagal disimpan/dimuat.",
    )
}
fn json_error(_: JsonRejection) -> Error {
    bad(StatusCode::BAD_REQUEST, "Isian AI tidak valid.")
}
fn ok(value: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}

/// Injectable boundary for hermetic tests. Production uses typed IPC and HTTPS.
#[derive(Debug, thiserror::Error)]
#[error("Layanan AI agent belum tersedia.")]
pub struct RuntimeUnavailable;

pub trait AiRuntime: Send + Sync {
    fn secret(
        &self,
        request: AiSecretRequest,
    ) -> BoxFuture<'_, Result<AiSecret, RuntimeUnavailable>>;
    fn capacity(&self) -> BoxFuture<'_, Result<CapacitySnapshot, RuntimeUnavailable>>;
    fn generate(
        &self,
        provider: AiProvider,
        model: String,
        key: AiSecret,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, ProviderError>>;
}
struct LiveRuntime(Arc<AgentClient>);
impl AiRuntime for LiveRuntime {
    fn secret(
        &self,
        request: AiSecretRequest,
    ) -> BoxFuture<'_, Result<AiSecret, RuntimeUnavailable>> {
        Box::pin(async move {
            self.0
                .ai_secret(request)
                .await
                .map_err(|_| RuntimeUnavailable)
        })
    }
    fn capacity(&self) -> BoxFuture<'_, Result<CapacitySnapshot, RuntimeUnavailable>> {
        Box::pin(async move {
            self.0
                .optimizer_status()
                .await
                .map_err(|_| RuntimeUnavailable)?
                .capacity
                .ok_or(RuntimeUnavailable)
        })
    }
    fn generate(
        &self,
        provider: AiProvider,
        model: String,
        key: AiSecret,
        body: Value,
    ) -> BoxFuture<'_, Result<Value, ProviderError>> {
        Box::pin(async move { transport::post(provider, &model, &key, body).await })
    }
}
pub fn router(agent: Arc<AgentClient>) -> Router<AppState> {
    router_with_runtime(Arc::new(LiveRuntime(agent)))
}
pub fn router_with_runtime(runtime: Arc<dyn AiRuntime>) -> Router<AppState> {
    Router::new()
        .route("/api/v1/system/ai", get(overview).put(configure))
        .route("/api/v1/system/ai/test", post(test_connection))
        .route("/api/v1/plans/ai/recommend", post(recommend))
        .layer(DefaultBodyLimit::max(16_384))
        .layer(Extension(runtime))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AiConfiguration {
    pub expected_revision: i64,
    pub enabled: bool,
    pub provider: AiProvider,
    pub model: String,
    pub daily_request_limit: i64,
    #[schema(write_only)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub remove_api_key: bool,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AiConfirmation {
    pub expected_revision: i64,
    pub consent: bool,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AiRecommendationRequest {
    pub expected_revision: i64,
    pub consent: bool,
    pub description: String,
    pub demand: PackageDemand,
    #[serde(default)]
    pub english: bool,
}

#[utoipa::path(get, path="/api/v1/system/ai", responses((status=200, body=Value)), tag="ai")]
pub async fn overview(
    State(state): State<AppState>,
    _auth: RequireAdmin,
) -> Result<Response, Error> {
    let config = store::settings(&state.pool).await?;
    let mut providers = Vec::new();
    for provider in [AiProvider::Gemini, AiProvider::Openai] {
        providers.push(json!({"id":provider,"models":models(provider),"has_key":store::credential(&state.pool,provider).await?.is_some()}));
    }
    Ok(ok(
        json!({"enabled":config.enabled,"provider":config.provider,"model":config.model,
        "daily_request_limit":config.daily_request_limit,"revision":config.revision,"providers":providers,
        "requests_today":store::requests_today(&state.pool,chrono::Utc::now().timestamp()).await?,
        "max_output_tokens":domain::MAX_OUTPUT_TOKENS,"reset_timezone":"UTC"}),
    ))
}

#[utoipa::path(put, path="/api/v1/system/ai", request_body=AiConfiguration, responses((status=200, body=Value)), tag="ai")]
pub async fn configure(
    State(state): State<AppState>,
    Extension(runtime): Extension<Arc<dyn AiRuntime>>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    input: Result<Json<AiConfiguration>, JsonRejection>,
) -> Result<Response, Error> {
    let mut input = input.map_err(json_error)?.0;
    if let Some(key) = &mut input.api_key {
        *key = key.trim().to_owned();
    }
    validate_configuration(&input)?;
    if store::settings(&state.pool).await?.revision != input.expected_revision {
        return Err(bad(
            StatusCode::CONFLICT,
            "Pengaturan AI berubah. Muat ulang sebelum menyimpan.",
        ));
    }
    let key_changed = input.api_key.is_some() || input.remove_api_key;
    let cipher = match input.api_key {
        Some(key) => Some(runtime.secret(AiSecretRequest {provider:input.provider,action:AiSecretAction::Seal,value:AiSecret(key)})
            .await.map_err(|_| bad(StatusCode::SERVICE_UNAVAILABLE,"Key belum dapat dienkripsi. Periksa agent dan gunakan paket API/agent yang sesuai."))?.0),
        None => None,
    };
    let mut transaction = state.pool.begin().await.map_err(db_error)?;
    let result = sqlx::query("UPDATE ai_settings SET enabled=?,provider=?,model=?,daily_request_limit=?,revision=revision+1 WHERE id=1 AND revision=?")
        .bind(input.enabled).bind(input.provider.as_str()).bind(&input.model).bind(input.daily_request_limit).bind(input.expected_revision)
        .execute(&mut *transaction).await.map_err(db_error)?;
    if result.rows_affected() != 1 {
        return Err(bad(
            StatusCode::CONFLICT,
            "Pengaturan AI berubah. Muat ulang sebelum menyimpan.",
        ));
    }
    if let Some(cipher) = cipher {
        if !cipher.starts_with("enc:v1:") {
            return Err(bad(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent tidak mengembalikan ciphertext AI yang valid.",
            ));
        }
        sqlx::query("INSERT INTO ai_credentials(provider,key_enc,revision) VALUES(?,?,1) ON CONFLICT(provider) DO UPDATE SET key_enc=excluded.key_enc,revision=ai_credentials.revision+1")
            .bind(input.provider.as_str()).bind(cipher).execute(&mut *transaction).await.map_err(db_error)?;
    } else if input.remove_api_key {
        sqlx::query("DELETE FROM ai_credentials WHERE provider=?")
            .bind(input.provider.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(db_error)?;
    }
    let has_key: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ai_credentials WHERE provider=?")
        .bind(input.provider.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(db_error)?;
    if input.enabled && has_key != 1 {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Simpan API key provider sebelum mengaktifkan AI.",
        ));
    }
    transaction.commit().await.map_err(db_error)?;
    state.audit_writer.log_async(Some(actor.id),actor.username,"system.ai.configure","ai","success",None,
        Some(json!({"provider":input.provider,"model":input.model,"enabled":input.enabled,"key_changed":key_changed}).to_string()));
    Ok(ok(
        json!({"success":true,"revision":input.expected_revision+1}),
    ))
}

fn validate_configuration(input: &AiConfiguration) -> Result<(), Error> {
    let invalid = if input.expected_revision < 1 {
        Some((
            "expected_revision",
            "Revisi pengaturan AI tidak valid. Muat ulang halaman.",
        ))
    } else if !(1..=1000).contains(&input.daily_request_limit) {
        Some((
            "daily_request_limit",
            "Batas permintaan harian harus bilangan bulat antara 1 dan 1000.",
        ))
    } else if !models(input.provider).contains(&input.model.as_str()) {
        Some((
            "model",
            "Model tidak tersedia untuk provider terpilih. Pilih ulang model dari daftar.",
        ))
    } else if input
        .api_key
        .as_ref()
        .is_some_and(|key| !panel_proto::valid_ai_key(key))
    {
        Some((
            "api_key",
            "Format API key tidak valid. Tempel key saja tanpa tanda kutip atau awalan Bearer; spasi di dalam key tidak diperbolehkan.",
        ))
    } else if input.remove_api_key && (input.enabled || input.api_key.is_some()) {
        Some((
            "remove_api_key",
            "Penghapusan key tidak dapat digabung dengan key baru atau pengaktifan AI.",
        ))
    } else {
        None
    };
    if let Some((field, message)) = invalid {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":message,"field":field})),
        ));
    }
    Ok(())
}

async fn configuration(
    state: &AppState,
    expected: i64,
    consent: bool,
    analysis: bool,
) -> Result<store::Settings, Error> {
    if !consent {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Setujui pengiriman data ke provider terlebih dahulu.",
        ));
    }
    let config = store::settings(&state.pool).await?;
    if config.revision != expected {
        return Err(bad(
            StatusCode::CONFLICT,
            "Provider atau pengaturan AI berubah. Muat ulang dan tinjau kembali persetujuan.",
        ));
    }
    if analysis && !config.enabled {
        return Err(bad(StatusCode::CONFLICT, "Integrasi AI belum diaktifkan."));
    }
    Ok(config)
}
async fn measured(
    runtime: &dyn AiRuntime,
    demand: PackageDemand,
) -> Result<panel_core::capacity::PackagePreset, Error> {
    let snapshot = runtime.capacity().await.map_err(|_| {
        bad(
            StatusCode::SERVICE_UNAVAILABLE,
            "Data kapasitas terverifikasi belum tersedia dari agent.",
        )
    })?;
    let now = u64::try_from(chrono::Utc::now().timestamp())
        .map_err(|_| bad(StatusCode::SERVICE_UNAVAILABLE, "Waktu server tidak valid."))?;
    recommend_package(&snapshot,demand,now)
        .map_err(|_| bad(StatusCode::SERVICE_UNAVAILABLE,"Kapasitas belum terverifikasi, usang, atau tidak mencukupi. Periksa pengoptimal server."))
}

async fn generate(
    state: &AppState,
    runtime: &dyn AiRuntime,
    actor: &AuthenticatedUser,
    config: &store::Settings,
    context: Value,
    english: bool,
) -> Result<ProviderResult, Error> {
    let provider = config.provider()?;
    let cipher = store::credential(&state.pool, provider)
        .await?
        .ok_or_else(|| {
            bad(
                StatusCode::CONFLICT,
                "API key provider belum dikonfigurasi.",
            )
        })?;
    let key = runtime
        .secret(AiSecretRequest {
            provider,
            action: AiSecretAction::Open,
            value: AiSecret(cipher),
        })
        .await
        .map_err(|_| {
            bad(
                StatusCode::SERVICE_UNAVAILABLE,
                "Key AI tidak dapat dibuka oleh agent.",
            )
        })?;
    if !panel_proto::valid_ai_key(&key.0) {
        return Err(bad(StatusCode::SERVICE_UNAVAILABLE, "Key AI tidak valid."));
    }
    // Even an accidentally pasted active key must not become prompt context.
    let context = serde_json::from_str(&context.to_string().replace(&key.0, "[redacted]"))
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "Konteks AI tidak valid."))?;
    let body = request_body(provider, &config.model, &context, english);
    let id = store::reserve(
        &state.pool,
        config,
        actor.id,
        chrono::Utc::now().timestamp(),
    )
    .await?;
    let response = runtime
        .generate(provider, config.model.clone(), key.clone(), body)
        .await;
    let parsed =
        response.and_then(|value| parse_reply(provider, value).map_err(|_| ProviderError::Invalid));
    let parsed = parsed.and_then(|value| {
        if serde_json::to_string(&value.analysis)
            .map_err(|_| ProviderError::Invalid)?
            .contains(&key.0)
        {
            return Err(ProviderError::Invalid);
        }
        Ok(value)
    });
    store::finish(
        &state.pool,
        &id,
        parsed.is_ok(),
        parsed.as_ref().ok().and_then(|r| r.total_tokens),
    )
    .await?;
    state.audit_writer.log_async(Some(actor.id), actor.username.clone(), "system.ai.request", "ai", if parsed.is_ok() { "success" } else { "failed" }, None,
        Some(json!({"request_id":id,"provider":provider,"model":config.model,"settings_revision":config.revision}).to_string()));
    let result = parsed.map_err(provider_error)?;
    if store::settings(&state.pool).await?.revision != config.revision {
        return Err(bad(
            StatusCode::CONFLICT,
            "Pengaturan AI berubah selama analisis. Hasil dibatalkan; buat preview baru.",
        ));
    }
    Ok(result)
}
fn provider_error(error: ProviderError) -> Error {
    match error {
        ProviderError::Credentials => bad(
            StatusCode::BAD_GATEWAY,
            "Provider menolak API key atau akses model. Periksa kredensial dan izin akun.",
        ),
        ProviderError::Limit => bad(
            StatusCode::TOO_MANY_REQUESTS,
            "Batas provider tercapai. Coba lagi nanti; batas pengelolaan manual tidak berubah.",
        ),
        ProviderError::Invalid => bad(
            StatusCode::BAD_GATEWAY,
            "Jawaban AI tidak sesuai format atau ditolak. Tidak ada draft yang diterapkan.",
        ),
        ProviderError::Timeout | ProviderError::Unavailable => bad(
            StatusCode::BAD_GATEWAY,
            "Provider AI belum dapat dihubungi atau melewati batas waktu. Coba lagi nanti.",
        ),
    }
}

#[utoipa::path(post, path="/api/v1/system/ai/test", request_body=AiConfirmation, responses((status=200, body=Value)), tag="ai")]
pub async fn test_connection(
    State(state): State<AppState>,
    Extension(runtime): Extension<Arc<dyn AiRuntime>>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    input: Result<Json<AiConfirmation>, JsonRejection>,
) -> Result<Response, Error> {
    let input = input.map_err(json_error)?.0;
    let config = configuration(&state, input.expected_revision, input.consent, false).await?;
    let result = generate(&state,runtime.as_ref(),&actor,&config,json!({"connection_test":true,"request":"Choose balanced and briefly confirm connectivity. No server data is sent."}),false).await?;
    state.audit_writer.log_async(
        Some(actor.id),
        actor.username,
        "system.ai.test",
        "ai",
        "success",
        None,
        None,
    );
    Ok(ok(
        json!({"success":true,"provider":config.provider,"model":config.model,"total_tokens":result.total_tokens}),
    ))
}

#[utoipa::path(post, path="/api/v1/plans/ai/recommend", request_body=AiRecommendationRequest, responses((status=200, body=Value)), tag="ai")]
pub async fn recommend(
    State(state): State<AppState>,
    Extension(runtime): Extension<Arc<dyn AiRuntime>>,
    RequireAdminCsrf(actor): RequireAdminCsrf,
    input: Result<Json<AiRecommendationRequest>, JsonRejection>,
) -> Result<Response, Error> {
    let input = input.map_err(json_error)?.0;
    if input.description.trim().is_empty()
        || input.description.len() > 6000
        || input
            .description
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        || input.demand.validate().is_err()
    {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "Jelaskan kebutuhan hingga 6000 byte dan pilih jumlah project/tenant yang valid.",
        ));
    }
    let config = configuration(&state, input.expected_revision, input.consent, true).await?;
    let ceiling = measured(runtime.as_ref(), input.demand.clone()).await?;
    let context = json!({"workload_description":input.description,"demand":input.demand,
        "ceiling":{"cpu_quota_percent":ceiling.cpu_quota_percent,"memory_limit_bytes":ceiling.memory_limit_bytes,"disk_limit_bytes":ceiling.disk_limit_bytes},
        "scope":"new_plan_draft_only","limits_are_not_reservations":true});
    let result = generate(
        &state,
        runtime.as_ref(),
        &actor,
        &config,
        context,
        input.english,
    )
    .await?;
    // Re-measure after the network call; never extend an old snapshot's validity.
    let fresh = measured(runtime.as_ref(), input.demand).await?;
    if store::settings(&state.pool).await?.revision != config.revision {
        return Err(bad(
            StatusCode::CONFLICT,
            "Pengaturan AI berubah. Buat preview baru.",
        ));
    }
    let preset = profile_preset(fresh, result.analysis.profile);
    use sha2::{Digest, Sha256};
    let fingerprint = panel_core::bytes_to_hex(&Sha256::digest(
        serde_json::to_vec(&preset).map_err(db_error)?,
    ));
    state.audit_writer.log_async(Some(actor.id),actor.username,"plan.ai.preview","new_plan","success",None,
        Some(json!({"provider":config.provider,"model":config.model,"settings_revision":config.revision,"snapshot_fingerprint":fingerprint}).to_string()));
    let Analysis {
        profile,
        summary,
        considerations,
    } = result.analysis;
    Ok(ok(
        json!({"id":uuid::Uuid::new_v4().to_string(),"provider":config.provider,"model":config.model,
        "settings_revision":config.revision,"snapshot_fingerprint":fingerprint,"profile":profile,"summary":summary,"considerations":considerations,
        "preset":preset,"total_tokens":result.total_tokens,"draft_only":true}),
    ))
}

#[cfg(test)]
mod tests;
