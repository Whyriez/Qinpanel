//! Tenant-scoped object storage. No provider key leaves the authenticated backend.
pub mod domain;
pub(crate) mod handlers;
pub(crate) mod managed;
pub(crate) mod policy;
mod signing;
mod store;
pub(crate) mod tls;
mod transport;

use crate::{agent_client::AgentClient, routes::AppState};
use axum::{
    Extension, Json, Router,
    extract::DefaultBodyLimit,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use domain::*;
use futures_util::future::BoxFuture;
use panel_core::object_storage::identity::StorageEndpoint;
use panel_proto::{S3Credentials, StorageSecret, StorageSecretRequest};
use serde_json::{Value, json};
use std::sync::Arc;

pub type Error = (StatusCode, Json<Value>);
pub fn bad(status: StatusCode, message: &str) -> Error {
    (status, Json(json!({"error":message})))
}
pub fn db_error(_: impl Sized) -> Error {
    bad(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata storage tidak dapat disimpan/dimuat.",
    )
}
pub fn failure(error: Failure) -> Error {
    match error {
        Failure::ConditionalUnsupported => bad(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Provider belum terverifikasi mendukung pencegahan overwrite. Pilih object key unik dan setujui PUT yang dapat menimpa, atau gunakan AWS S3/R2.",
        ),
        Failure::Endpoint => bad(
            StatusCode::BAD_REQUEST,
            "Gunakan endpoint HTTPS publik yang terpisah dari panel. Alamat privat/metadata atau DNS tidak aman ditolak.",
        ),
        Failure::Provider(404) => bad(
            StatusCode::NOT_FOUND,
            "Bucket atau object tidak ditemukan di provider.",
        ),
        Failure::Provider(409 | 412) => bad(
            StatusCode::CONFLICT,
            "Object/bucket sudah ada atau kondisi penulisan tidak terpenuhi. Muat ulang sebelum mencoba kembali.",
        ),
        Failure::Provider(401 | 403) => bad(
            StatusCode::BAD_GATEWAY,
            "Provider menolak credential atau izin bucket. Periksa key dan policy S3.",
        ),
        Failure::Provider(429) => bad(
            StatusCode::TOO_MANY_REQUESTS,
            "Transfer atau provider sedang sibuk. Coba lagi nanti.",
        ),
        Failure::Timeout => bad(
            StatusCode::GATEWAY_TIMEOUT,
            "Operasi terputus. Keadaan provider belum dipastikan; muat ulang daftar sebelum mencoba kembali.",
        ),
        _ => bad(
            StatusCode::BAD_GATEWAY,
            "Provider tidak tersedia atau jawabannya tidak valid. Periksa endpoint, region, izin dan kompatibilitas S3.",
        ),
    }
}
pub fn ok(value: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}

pub trait StorageRuntime: Send + Sync {
    fn dns_status(&self) -> BoxFuture<'_, Result<Value, Failure>> {
        Box::pin(async { Err(Failure::Unavailable) })
    }
    fn certificate(
        &self,
        _pool: panel_db::DbPool,
        _hostname: String,
        _email: String,
        _revision: String,
    ) -> BoxFuture<'_, Result<StorageCertificate, String>> {
        Box::pin(async { Err("Layanan penerbitan sertifikat belum tersedia.".into()) })
    }
    fn managed(
        &self,
        _request: panel_proto::ManagedStorageRequest,
    ) -> BoxFuture<'_, Result<Value, Failure>> {
        Box::pin(async { Err(Failure::Unavailable) })
    }
    fn secret(
        &self,
        request: StorageSecretRequest,
    ) -> BoxFuture<'_, Result<StorageSecret, Failure>>;
    fn execute(
        &self,
        connection: Connection,
        credentials: S3Credentials,
        operation: Operation,
        panel: StorageEndpoint,
    ) -> BoxFuture<'_, Result<Outcome, Failure>>;
}
struct LiveRuntime {
    agent: Arc<AgentClient>,
    network: tokio::sync::Semaphore,
}
impl StorageRuntime for LiveRuntime {
    fn dns_status(&self) -> BoxFuture<'_, Result<Value, Failure>> {
        Box::pin(async move {
            self.agent
                .dns01(panel_proto::Dns01Request::Status)
                .await
                .map_err(|_| Failure::Unavailable)
        })
    }
    fn certificate(
        &self,
        pool: panel_db::DbPool,
        hostname: String,
        email: String,
        revision: String,
    ) -> BoxFuture<'_, Result<StorageCertificate, String>> {
        Box::pin(async move {
            let issued = crate::dns01::issue_storage_certificate(
                &pool,
                &self.agent,
                &hostname,
                &email,
                &revision,
            )
            .await?;
            Ok(StorageCertificate {
                chain: issued.chain,
                key: StorageSecret(issued.key),
            })
        })
    }
    fn managed(
        &self,
        request: panel_proto::ManagedStorageRequest,
    ) -> BoxFuture<'_, Result<Value, Failure>> {
        Box::pin(async move {
            self.agent
                .managed_storage(request)
                .await
                .map_err(|_| Failure::Unavailable)
        })
    }
    fn secret(
        &self,
        request: StorageSecretRequest,
    ) -> BoxFuture<'_, Result<StorageSecret, Failure>> {
        Box::pin(async move {
            self.agent
                .storage_secret(request)
                .await
                .map_err(|_| Failure::Unavailable)
        })
    }
    fn execute(
        &self,
        connection: Connection,
        credentials: S3Credentials,
        operation: Operation,
        panel: StorageEndpoint,
    ) -> BoxFuture<'_, Result<Outcome, Failure>> {
        Box::pin(async move {
            let _permit = self
                .network
                .try_acquire()
                .map_err(|_| Failure::Provider(429))?;
            transport::execute(connection, credentials, operation, panel).await
        })
    }
}
#[derive(Clone)]
pub struct Services {
    runtime: Arc<dyn StorageRuntime>,
    transfers: Arc<tokio::sync::Semaphore>,
}
pub fn router(agent: Arc<AgentClient>) -> Router<AppState> {
    router_with_runtime(Arc::new(LiveRuntime {
        agent,
        network: tokio::sync::Semaphore::new(8),
    }))
}
pub fn router_with_runtime(runtime: Arc<dyn StorageRuntime>) -> Router<AppState> {
    Router::new()
        .route("/api/v1/storage/context", get(handlers::context))
        .route(
            "/api/v1/storage/connections/{id}/policy",
            get(policy::overview).post(policy::rules),
        )
        .route(
            "/api/v1/storage/connections/{id}/allocation",
            post(policy::allocation),
        )
        .route(
            "/api/v1/storage/connections/{id}/usage",
            post(policy::usage),
        )
        .route(
            "/api/v1/storage/managed",
            get(managed::status).post(managed::configure),
        )
        .route("/api/v1/storage/managed/buckets", post(managed::create))
        .route(
            "/api/v1/storage/managed/tls",
            get(tls::overview).post(tls::generate),
        )
        .route(
            "/api/v1/storage/connections/{id}/provision",
            post(managed::provision),
        )
        .route(
            "/api/v1/storage/connections/{id}/decommission",
            post(managed::decommission),
        )
        .route(
            "/api/v1/storage/connections/{id}/application-key",
            post(managed::application_key),
        )
        .route(
            "/api/v1/storage/connections",
            get(handlers::list).post(handlers::create),
        )
        .route(
            "/api/v1/storage/connections/{id}",
            post(handlers::update).delete(handlers::disconnect),
        )
        .route(
            "/api/v1/storage/connections/{id}/test",
            post(handlers::test),
        )
        .route(
            "/api/v1/storage/connections/{id}/binding",
            post(handlers::bind),
        )
        .route(
            "/api/v1/storage/connections/{id}/bucket",
            post(handlers::create_bucket),
        )
        .route(
            "/api/v1/storage/connections/{id}/objects",
            get(handlers::objects),
        )
        .route(
            "/api/v1/storage/connections/{id}/upload",
            post(handlers::upload),
        )
        .route(
            "/api/v1/storage/connections/{id}/delete",
            post(handlers::delete_object),
        )
        .route(
            "/api/v1/storage/connections/{id}/download",
            get(handlers::download_proxy).post(handlers::download),
        )
        .layer(DefaultBodyLimit::max(196608))
        .layer(Extension(Services {
            runtime,
            transfers: Arc::new(tokio::sync::Semaphore::new(4)),
        }))
}

#[cfg(test)]
mod tests;
