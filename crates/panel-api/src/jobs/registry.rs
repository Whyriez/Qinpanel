//! Handler registry and execution context for background jobs.

use crate::jobs::events::JobProgressEvent;
use panel_db::error::DbError;
use panel_db::models::Job;
use panel_db::pool::DbPool;
use panel_db::repositories::SqliteJobRepository;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub type JobFuture<'a> = Pin<Box<dyn Future<Output = Result<Option<String>, String>> + Send + 'a>>;

struct CloudflareHandler(Arc<crate::agent_client::AgentClient>);
impl JobHandler for CloudflareHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let actor: Option<(String, String)> =
                sqlx::query_as("SELECT role,status FROM users WHERE id=?")
                    .bind(job.user_id)
                    .fetch_optional(&ctx.pool)
                    .await
                    .map_err(|_| "Gagal memeriksa administrator tugas.".to_owned())?;
            if actor != Some(("admin".into(), "active".into())) {
                return Err("Tugas Cloudflare memerlukan administrator aktif.".into());
            }
            ctx.report_progress(10, Some("Menyiapkan operasi Cloudflare."))
                .await
                .map_err(|e| e.to_string())?;
            let value = if job.name == "cloudflare.install" {
                match ctx
                    .install(
                        &self.0,
                        panel_proto::InstallOperation::CloudflareConnector,
                        40,
                    )
                    .await?
                {
                    panel_proto::ResponsePayload::Cloudflare(value) => value,
                    _ => return Err("Respons instalasi konektor tidak sesuai.".into()),
                }
            } else {
                let command: panel_proto::CloudflareRequest =
                    serde_json::from_str(&job.payload_json)
                        .map_err(|_| "Payload tugas Cloudflare tidak valid.".to_owned())?;
                if !matches!(
                    command,
                    panel_proto::CloudflareRequest::CreateTunnel { .. }
                        | panel_proto::CloudflareRequest::TunnelAction { .. }
                        | panel_proto::CloudflareRequest::Publish { .. }
                        | panel_proto::CloudflareRequest::Unpublish { .. }
                ) {
                    return Err("Operasi tidak diizinkan dalam antrean Cloudflare.".into());
                }
                let command = crate::network::validate_publication(&ctx.pool, command)
                    .await
                    .map_err(|e| {
                        e.1.0["error"]
                            .as_str()
                            .unwrap_or("Validasi project gagal.")
                            .to_owned()
                    })?;
                ctx.report_progress(
                    30,
                    Some("Menerapkan perubahan. Memeriksa DNS, rute, atau layanan tunnel."),
                )
                .await
                .map_err(|e| e.to_string())?;
                self.0
                    .cloudflare(command)
                    .await
                    .map_err(|e| e.to_string())?
            };
            ctx.report_progress(95,Some("Perubahan Cloudflare selesai. Muat ulang status koneksi untuk memeriksa kesehatan tunnel.")).await.map_err(|e|e.to_string())?;
            Ok(Some(value.to_string()))
        })
    }
}

async fn validate_job_identity(
    ctx: &JobContext,
    job: &Job,
    username: &str,
    uid: Option<u32>,
) -> Result<(), String> {
    let repo = panel_db::repositories::SqliteUserRepository::new(ctx.pool.clone());
    let tenant = repo
        .find_by_linux_username(username)
        .await
        .map_err(|_| "Gagal memuat identitas pekerjaan".to_owned())?
        .ok_or_else(|| "Identitas tenant pekerjaan tidak ditemukan".to_owned())?;
    let (_, actual_uid) =
        crate::tenant_context::execution_identity(&tenant).map_err(|(_, message)| message)?;
    if uid.is_some_and(|provided| provided != actual_uid) {
        return Err("UID pekerjaan tidak sesuai dengan tenant".to_owned());
    }
    if let Some(actor_id) = job.user_id {
        let actor = repo
            .find_by_id(actor_id)
            .await
            .map_err(|_| "Gagal memuat pemilik pekerjaan".to_owned())?
            .ok_or_else(|| "Pemilik pekerjaan tidak ditemukan".to_owned())?;
        if actor.status != "active" || (actor.role != "admin" && actor.id != tenant.id) {
            return Err("Akses tenant pekerjaan ditolak".to_owned());
        }
    }
    Ok(())
}

/// Execution context passed to each job handler.
pub struct JobContext {
    pub job_id: String,
    pub pool: DbPool,
    pub job_repo: SqliteJobRepository,
    pub event_tx: broadcast::Sender<JobProgressEvent>,
    pub cancel_token: CancellationToken,
}

impl JobContext {
    pub(crate) async fn install(
        &self,
        client: &crate::agent_client::AgentClient,
        operation: panel_proto::InstallOperation,
        progress: i64,
    ) -> Result<panel_proto::ResponsePayload, String> {
        client
            .install_with_progress(operation, |message| async move {
                self.report_progress(progress, Some(&message))
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())
    }
    /// Node/Python installation cancels on RPC disconnect in the agent. Do not apply
    /// this to APT or other operations without agent-side cancellation support.
    pub async fn run_cancellable_runtime_install<F: Future>(
        &self,
        operation: F,
    ) -> Result<F::Output, String> {
        tokio::select! {
            biased;
            _ = self.cancel_token.cancelled() => Err("Pemasangan runtime dibatalkan".into()),
            result = operation => Ok(result),
        }
    }

    /// Reports incremental progress (0 to 100) and broadcasts it to SSE subscribers.
    pub async fn report_progress(
        &self,
        progress: i64,
        message: Option<&str>,
    ) -> Result<(), DbError> {
        let bounded = progress.clamp(0, 100);
        self.job_repo.update_progress(&self.job_id, bounded).await?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        crate::jobs::events::publish(
            &self.job_repo,
            &self.event_tx,
            JobProgressEvent {
                sequence: None,
                job_id: self.job_id.clone(),
                status: "running".to_string(),
                progress: bounded,
                message: message.map(ToString::to_string),
                timestamp: now,
            },
        )
        .await?;

        Ok(())
    }

    /// Checks if a cancellation request has been signaled for this job.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }
}

/// Interface for background job handlers.
pub trait JobHandler: Send + Sync {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a>;
}

/// Test sleep handler that emits progress milestones.
pub struct SleepHandler;

impl JobHandler for SleepHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let duration_ms = payload
                .get("duration_ms")
                .and_then(Value::as_u64)
                .unwrap_or(100);
            let steps = payload
                .get("steps")
                .and_then(Value::as_u64)
                .unwrap_or(4)
                .max(1);

            let step_duration = Duration::from_millis(duration_ms / steps);

            for step in 1..=steps {
                if ctx.is_cancelled() {
                    return Err("Job was cancelled during execution".to_string());
                }
                tokio::time::sleep(step_duration).await;
                if ctx.is_cancelled() {
                    return Err("Job was cancelled during execution".to_string());
                }

                let pct = (step * 100 / steps) as i64;
                let msg = format!("Step {}/{} finished", step, steps);
                if let Err(e) = ctx.report_progress(pct, Some(&msg)).await {
                    warn!("Failed to report progress: {}", e);
                }
            }

            Ok(Some(
                serde_json::to_string(&serde_json::json!({
                    "slept_ms": duration_ms,
                    "steps": steps,
                }))
                .unwrap_or_default(),
            ))
        })
    }
}

/// Test failure handler to verify retry and backoff logic.
pub struct FailHandler;

impl JobHandler for FailHandler {
    fn run<'a>(&'a self, job: &'a Job, _ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let error_msg = payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Simulated job failure");

            Err(error_msg.to_string())
        })
    }
}

/// Echo handler returning the payload as result.
pub struct EchoHandler;

impl JobHandler for EchoHandler {
    fn run<'a>(&'a self, job: &'a Job, _ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move { Ok(Some(job.payload_json.clone())) })
    }
}

/// Quota report handler querying tenant quota usage via agent client.
pub struct QuotaReportHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl QuotaReportHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for QuotaReportHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let uid = payload
                .get("uid")
                .and_then(Value::as_u64)
                .filter(|uid| (10000..=59999).contains(uid))
                .ok_or_else(|| "UID tenant yang valid wajib disertakan".to_string())?
                as u32;
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Identitas Linux tenant wajib disertakan".to_string())?
                .to_string();
            validate_job_identity(ctx, job, &username, Some(uid)).await?;

            let _ = ctx
                .report_progress(10, Some("Querying quota metrics from agent..."))
                .await;

            let quota_res = self
                .agent_client
                .get_quota(panel_proto::QuotaGetRequest {
                    uid,
                    username: username.clone(),
                    mount_point: None,
                })
                .await
                .map_err(|e| format!("Agent quota query failed: {e}"))?;

            let _ = ctx
                .report_progress(70, Some("Aggregating quota usage data..."))
                .await;

            let result = serde_json::json!({
                "uid": quota_res.uid,
                "username": quota_res.username,
                "bytes_used": quota_res.bytes_used,
                "bytes_limit": quota_res.bytes_limit,
                "inodes_used": quota_res.inodes_used,
                "inodes_limit": quota_res.inodes_limit,
                "backend": quota_res.backend,
            });

            let _ = ctx
                .report_progress(100, Some("Quota report finished successfully"))
                .await;

            Ok(Some(result.to_string()))
        })
    }
}

/// Quota scan handler executing recursive directory scan via agent client.
pub struct QuotaScanHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl QuotaScanHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for QuotaScanHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let uid = payload
                .get("uid")
                .and_then(Value::as_u64)
                .filter(|uid| (10000..=59999).contains(uid))
                .ok_or_else(|| "UID tenant yang valid wajib disertakan".to_string())?
                as u32;
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Identitas Linux tenant wajib disertakan".to_string())?
                .to_string();
            validate_job_identity(ctx, job, &username, Some(uid)).await?;

            let _ = ctx
                .report_progress(15, Some("Scanning tenant filesystem tree..."))
                .await;

            let scan_res = self
                .agent_client
                .scan_quota(panel_proto::QuotaScanRequest {
                    uid,
                    username: username.clone(),
                    path: None,
                })
                .await
                .map_err(|e| format!("Agent quota scan failed: {e}"))?;

            let _ = ctx
                .report_progress(80, Some("Finalizing directory scan metrics..."))
                .await;

            let result = serde_json::json!({
                "uid": scan_res.uid,
                "username": scan_res.username,
                "bytes_used": scan_res.bytes_used,
                "inodes_used": scan_res.inodes_used,
                "scan_duration_ms": scan_res.scan_duration_ms,
            });

            let _ = ctx
                .report_progress(100, Some("Filesystem scan complete"))
                .await;

            Ok(Some(result.to_string()))
        })
    }
}

/// PHP Sury/PPA repository setup handler.
pub struct PhpRepoSetupHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl PhpRepoSetupHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for PhpRepoSetupHandler {
    fn run<'a>(&'a self, _job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let _ = ctx
                .report_progress(10, Some("Menyiapkan repositori PHP sesuai OS server..."))
                .await;

            if ctx.is_cancelled() {
                return Err("Job was cancelled before execution".to_string());
            }

            let res = match ctx
                .install(
                    &self.agent_client,
                    panel_proto::InstallOperation::PhpRepository,
                    10,
                )
                .await?
            {
                panel_proto::ResponsePayload::PhpRepoSetup(res) => res,
                _ => return Err("Respons konfigurasi repositori tidak sesuai".into()),
            };

            let _ = ctx
                .report_progress(100, Some("Repositori PHP berhasil dikonfigurasi"))
                .await;

            let result = serde_json::json!({
                "success": res.success,
                "message": res.message,
            });

            Ok(Some(result.to_string()))
        })
    }
}

/// Node.js installation uses the persistent worker rather than an HTTP request.
pub struct NodeInstallHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl NodeInstallHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for NodeInstallHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let req: panel_proto::NodeRuntimeInstallRequest =
                serde_json::from_str(&job.payload_json)
                    .map_err(|e| format!("Payload pemasangan Node.js tidak valid: {e}"))?;
            let owner_id = job
                .user_id
                .ok_or_else(|| "Pemilik job Node.js wajib ada".to_string())?;
            let owner = panel_db::repositories::SqliteUserRepository::new(ctx.pool.clone())
                .find_by_id(owner_id)
                .await
                .map_err(|e| format!("Gagal memeriksa pemilik job: {e}"))?
                .ok_or_else(|| "Pemilik job tidak ditemukan".to_string())?;
            if owner.role != "admin" || owner.status != "active" {
                return Err("Pemasangan runtime Node.js memerlukan administrator aktif".to_string());
            }
            if !crate::runtime_catalog::node_installable(&ctx.pool, &req.version).await? {
                return Err("Versi Node.js tidak termasuk LTS yang masih didukung; perbarui katalog runtime".into());
            }
            if ctx.is_cancelled() {
                return Err("Job dibatalkan sebelum pemasangan Node.js".to_string());
            }
            let version = req.version.clone();
            ctx.report_progress(15, Some(&format!("Mengunduh dan memverifikasi Node.js v{version}; kemudian mengekstrak runtime dan menyiapkan Corepack.")))
                .await.map_err(|e| e.to_string())?;
            let res = ctx
                .run_cancellable_runtime_install(self.agent_client.install_node_runtime(req))
                .await?
                .map_err(|e| format!("Instalasi Node.js v{version} gagal: {e}"))?;
            if !res.success {
                return Err(res.message);
            }
            ctx.report_progress(100, Some(&res.message))
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&res)
                .map(Some)
                .map_err(|e| e.to_string())
        })
    }
}

pub struct PythonInstallHandler(Arc<crate::agent_client::AgentClient>);
impl JobHandler for PythonInstallHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let actor: Option<(String, String)> =
                sqlx::query_as("SELECT role,status FROM users WHERE id=?")
                    .bind(job.user_id)
                    .fetch_optional(&ctx.pool)
                    .await
                    .map_err(|e| e.to_string())?;
            if actor != Some(("admin".into(), "active".into())) {
                return Err("Pemasangan Python memerlukan administrator aktif".into());
            }
            let request: panel_proto::PythonRuntimeInstallRequest =
                serde_json::from_str(&job.payload_json).map_err(|e| e.to_string())?;
            panel_runtime::python::validate_python_version(&request.version)
                .map_err(|e| e.to_string())?;
            if request.base_dir.is_some() {
                return Err("Direktori runtime dikelola agent".into());
            }
            if ctx.is_cancelled() {
                return Err("Tugas Python dibatalkan sebelum pemasangan".into());
            }
            ctx.report_progress(
                15,
                Some(&format!(
                    "Mengunduh dan memasang Python {} melalui uv",
                    request.version
                )),
            )
            .await
            .map_err(|e| e.to_string())?;
            let installed = ctx
                .run_cancellable_runtime_install(self.0.install_python_runtime(request))
                .await?
                .map_err(|e| e.to_string())?;
            if !installed.success {
                return Err(installed.message);
            }
            ctx.report_progress(100, Some(&installed.message))
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&installed)
                .map(Some)
                .map_err(|e| e.to_string())
        })
    }
}

/// PHP runtime installation handler.
pub struct PhpInstallHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl PhpInstallHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for PhpInstallHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let version = payload
                .get("version")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'version' wajib disertakan dalam payload".to_string())?;

            panel_runtime::php::PhpVersion::parse(version)
                .map_err(|e| format!("Versi PHP tidak valid: {e}"))?;

            let _ = ctx
                .report_progress(15, Some(&format!("Memulai instalasi PHP {}...", version)))
                .await;

            if ctx.is_cancelled() {
                return Err("Job was cancelled before installation".to_string());
            }

            let _ = ctx
                .report_progress(
                    35,
                    Some(&format!(
                        "Menyiapkan repositori dan indeks paket, memasang PHP {} serta melakukan masking service FPM...",
                        version
                    )),
                )
                .await;

            let res = match ctx
                .install(
                    &self.agent_client,
                    panel_proto::InstallOperation::PhpRuntime(
                        panel_proto::PhpRuntimeInstallRequest {
                            version: version.into(),
                        },
                    ),
                    35,
                )
                .await?
            {
                panel_proto::ResponsePayload::PhpRuntimeInstalled(res) if res.success => res,
                _ => {
                    return Err(
                        "Instalasi PHP tidak berhasil atau respons agent tidak sesuai".into(),
                    );
                }
            };

            let _ = ctx
                .report_progress(
                    100,
                    Some(&format!(
                        "PHP {} berhasil dipasang dan service FPM dimask",
                        version
                    )),
                )
                .await;

            let result = serde_json::json!({
                "version": res.version,
                "success": res.success,
                "masked": res.masked,
                "message": res.message,
            });

            Ok(Some(result.to_string()))
        })
    }
}

/// PHP runtime removal handler.
pub struct PhpRemoveHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl PhpRemoveHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for PhpRemoveHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let version = payload
                .get("version")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'version' wajib disertakan dalam payload".to_string())?;

            panel_runtime::php::PhpVersion::parse(version)
                .map_err(|e| format!("Versi PHP tidak valid: {e}"))?;

            let _ = ctx
                .report_progress(
                    10,
                    Some(&format!(
                        "Memeriksa penggunaan PHP {} pada situs aktif...",
                        version
                    )),
                )
                .await;

            // Check if any sites are using this PHP version
            let site_repo = panel_db::repositories::SqliteSiteRepository::new(ctx.pool.clone());
            let count = site_repo
                .count_by_runtime_and_version("php", version)
                .await
                .map_err(|e| format!("Gagal memeriksa penggunaan PHP di database: {e}"))?;

            if count > 0 {
                return Err(format!(
                    "Tidak dapat menghapus PHP {}: sedang digunakan oleh {} situs.",
                    version, count
                ));
            }

            if ctx.is_cancelled() {
                return Err("Job was cancelled before removal".to_string());
            }

            let _ = ctx
                .report_progress(40, Some(&format!("Menghapus paket PHP {}...", version)))
                .await;

            let res = self
                .agent_client
                .remove_php_runtime(version)
                .await
                .map_err(|e| format!("Penghapusan PHP {version} gagal: {e}"))?;

            let _ = ctx
                .report_progress(100, Some(&format!("PHP {} berhasil dihapus", version)))
                .await;

            let result = serde_json::json!({
                "version": res.version,
                "success": res.success,
                "masked": res.masked,
                "message": res.message,
            });

            Ok(Some(result.to_string()))
        })
    }
}

pub struct PhpExtensionInstallHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl PhpExtensionInstallHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for PhpExtensionInstallHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let version = payload
                .get("version")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'version' wajib disertakan dalam payload".to_string())?;
            let extension = payload
                .get("extension")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'extension' wajib disertakan dalam payload".to_string())?;

            panel_runtime::php::PhpVersion::parse(version)
                .map_err(|e| format!("Versi PHP tidak valid: {e}"))?;
            panel_runtime::php::find_curated_extension(extension)
                .map_err(|e| format!("Ekstensi tidak valid: {e}"))?;

            let _ = ctx
                .report_progress(
                    20,
                    Some(&format!(
                        "Memulai pemasangan ekstensi {} untuk PHP {}...",
                        extension, version
                    )),
                )
                .await;

            let res = self
                .agent_client
                .install_php_extension(version, extension)
                .await
                .map_err(|e| format!("Pemasangan ekstensi {extension} gagal: {e}"))?;

            let _ = ctx
                .report_progress(
                    100,
                    Some(&format!("Ekstensi {} berhasil dipasang", extension)),
                )
                .await;

            let result = serde_json::json!({
                "version": res.version,
                "extension": res.extension,
                "success": res.success,
                "message": res.message,
            });

            Ok(Some(result.to_string()))
        })
    }
}

pub struct PhpExtensionRemoveHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl PhpExtensionRemoveHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for PhpExtensionRemoveHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let version = payload
                .get("version")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'version' wajib disertakan dalam payload".to_string())?;
            let extension = payload
                .get("extension")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'extension' wajib disertakan dalam payload".to_string())?;

            panel_runtime::php::PhpVersion::parse(version)
                .map_err(|e| format!("Versi PHP tidak valid: {e}"))?;
            let def = panel_runtime::php::find_curated_extension(extension)
                .map_err(|e| format!("Ekstensi tidak valid: {e}"))?;

            if def.is_base {
                return Err(format!("Ekstensi dasar '{extension}' tidak dapat dihapus"));
            }

            let _ = ctx
                .report_progress(
                    20,
                    Some(&format!(
                        "Menghapus ekstensi {} dari PHP {}...",
                        extension, version
                    )),
                )
                .await;

            let res = self
                .agent_client
                .remove_php_extension(version, extension)
                .await
                .map_err(|e| format!("Penghapusan ekstensi {extension} gagal: {e}"))?;

            let _ = ctx
                .report_progress(
                    100,
                    Some(&format!("Ekstensi {} berhasil dihapus", extension)),
                )
                .await;

            let result = serde_json::json!({
                "version": res.version,
                "extension": res.extension,
                "success": res.success,
                "message": res.message,
            });

            Ok(Some(result.to_string()))
        })
    }
}

pub struct FileArchiveCompressHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl FileArchiveCompressHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for FileArchiveCompressHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'username' wajib disertakan dalam payload".to_string())?;
            validate_job_identity(ctx, job, username, None).await?;
            let destination = payload
                .get("destination")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'destination' wajib disertakan dalam payload".to_string())?;
            let source_paths = payload
                .get("source_paths")
                .and_then(Value::as_array)
                .ok_or_else(|| "Field 'source_paths' wajib berupa array".to_string())?
                .iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            let format_str = payload.get("format").and_then(Value::as_str);
            let format = match format_str {
                Some("targz") | Some("tar.gz") => panel_proto::ArchiveFormatDto::TarGz,
                _ => {
                    if destination.ends_with(".tar.gz") || destination.ends_with(".tgz") {
                        panel_proto::ArchiveFormatDto::TarGz
                    } else {
                        panel_proto::ArchiveFormatDto::Zip
                    }
                }
            };

            let _ = ctx
                .report_progress(10, Some("Memulai pembuatan arsip..."))
                .await;

            if ctx.is_cancelled() {
                return Err("Job was cancelled before compression".to_string());
            }

            let res = self
                .agent_client
                .fs_archive_compress(panel_proto::FsArchiveCompressRequest {
                    username: username.to_string(),
                    source_paths,
                    destination: destination.to_string(),
                    format,
                })
                .await
                .map_err(|e| format!("Kompresi arsip gagal: {e}"))?;

            let _ = ctx
                .report_progress(100, Some("Arsip berhasil dibuat"))
                .await;

            let result = serde_json::json!({
                "success": true,
                "result": res.result,
            });

            Ok(Some(result.to_string()))
        })
    }
}

pub struct FileArchiveExtractHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl FileArchiveExtractHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for FileArchiveExtractHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'username' wajib disertakan dalam payload".to_string())?;
            validate_job_identity(ctx, job, username, None).await?;
            let archive_path = payload
                .get("archive_path")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'archive_path' wajib disertakan dalam payload".to_string())?;
            let destination_dir = payload
                .get("destination_dir")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    "Field 'destination_dir' wajib disertakan dalam payload".to_string()
                })?;
            let format_str = payload.get("format").and_then(Value::as_str);
            let format = match format_str {
                Some("zip") => Some(panel_proto::ArchiveFormatDto::Zip),
                Some("targz") | Some("tar.gz") => Some(panel_proto::ArchiveFormatDto::TarGz),
                _ => None,
            };

            let _ = ctx
                .report_progress(10, Some("Memulai ekstraksi arsip..."))
                .await;

            if ctx.is_cancelled() {
                return Err("Job was cancelled before extraction".to_string());
            }

            let res = self
                .agent_client
                .fs_archive_extract(panel_proto::FsArchiveExtractRequest {
                    username: username.to_string(),
                    archive_path: archive_path.to_string(),
                    destination_dir: destination_dir.to_string(),
                    format,
                })
                .await
                .map_err(|e| format!("Ekstraksi arsip gagal: {e}"))?;

            let _ = ctx
                .report_progress(100, Some("Arsip berhasil diekstrak"))
                .await;

            let result = serde_json::json!({
                "success": true,
                "result": res.result,
            });

            Ok(Some(result.to_string()))
        })
    }
}

/// File search background job handler coordinating deep searches.
pub struct FileSearchHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl FileSearchHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for FileSearchHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'username' wajib disertakan dalam payload".to_string())?;
            validate_job_identity(ctx, job, username, None).await?;
            let path = payload
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let query = payload
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'query' wajib disertakan dalam payload".to_string())?
                .to_string();
            let search_content = payload
                .get("search_content")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let case_sensitive = payload
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let extension = payload
                .get("extension")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let max_results = payload
                .get("max_results")
                .and_then(Value::as_u64)
                .map(|v| v as usize);
            let timeout_secs = payload.get("timeout_secs").and_then(Value::as_u64);

            let _ = ctx
                .report_progress(10, Some("Memulai pemindaian berkas..."))
                .await;

            if ctx.is_cancelled() {
                return Err("Job was cancelled before search started".to_string());
            }

            let res = self
                .agent_client
                .fs_search(panel_proto::FsSearchRequest {
                    username: username.to_string(),
                    path,
                    query,
                    search_content,
                    case_sensitive,
                    extension,
                    max_results,
                    timeout_secs,
                })
                .await
                .map_err(|e| format!("Pencarian berkas gagal: {e}"))?;

            let _ = ctx
                .report_progress(100, Some("Pencarian berkas selesai"))
                .await;

            let result = serde_json::json!({
                "items": res.items,
                "total_scanned": res.total_scanned,
                "limit_reached": res.limit_reached,
                "timed_out": res.timed_out,
                "duration_ms": res.duration_ms,
            });

            Ok(Some(result.to_string()))
        })
    }
}

/// Central registry mapping job names to their respective handlers.
pub struct JobRegistry {
    handlers: HashMap<String, Arc<dyn JobHandler>>,
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            handlers: HashMap::new(),
        };
        reg.register("test.sleep", SleepHandler);
        reg.register("test.fail", FailHandler);
        reg.register("system.echo", EchoHandler);
        reg
    }

    pub fn new_with_agent(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        let mut reg = Self::new();
        reg.register(
            "ssl.dns01.issue",
            crate::dns01::Handler(Arc::clone(&agent_client)),
        );
        reg.register(
            "storage.tls.issue",
            crate::storage::tls::Handler(Arc::clone(&agent_client)),
        );
        reg.register(
            "panel.tls.issue",
            crate::panel_tls::PanelTlsHandler(Arc::clone(&agent_client)),
        );
        for name in [
            "email.install",
            "email.mailbox",
            "email.send",
            "email.deliver",
        ] {
            reg.register(name, crate::email::EmailHandler(Arc::clone(&agent_client)));
        }
        reg.register(
            "cloudflare.install",
            CloudflareHandler(Arc::clone(&agent_client)),
        );
        reg.register(
            "cloudflare.change",
            CloudflareHandler(Arc::clone(&agent_client)),
        );
        reg.register(
            "node.install",
            NodeInstallHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "python.install",
            PythonInstallHandler(Arc::clone(&agent_client)),
        );
        reg.register(
            "quota.report",
            QuotaReportHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "quota.scan",
            QuotaScanHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "php.repo.setup",
            PhpRepoSetupHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "php.install",
            PhpInstallHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "php.remove",
            PhpRemoveHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "php.extension.install",
            PhpExtensionInstallHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "php.extension.remove",
            PhpExtensionRemoveHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "files.archive.compress",
            FileArchiveCompressHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "files.archive.extract",
            FileArchiveExtractHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "files.search",
            FileSearchHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "database.server.install",
            DatabaseServerInstallHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "database.web.install",
            crate::database_web::WebToolsInstallHandler(Arc::clone(&agent_client)),
        );
        reg.register(
            "database.export",
            DatabaseExportHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "database.import",
            DatabaseImportHandler::new(Arc::clone(&agent_client)),
        );
        reg.register(
            "backup.create",
            BackupCreateHandler::new(Arc::clone(&agent_client)),
        );
        reg.register("backup.restore", BackupRestoreHandler::new(agent_client));
        reg
    }

    pub fn register<H: JobHandler + 'static>(&mut self, name: &str, handler: H) {
        self.handlers.insert(name.to_string(), Arc::new(handler));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn JobHandler>> {
        self.handlers.get(name).cloned()
    }
}

/// Database engine server installation job handler.
pub struct DatabaseServerInstallHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl DatabaseServerInstallHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for DatabaseServerInstallHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let engine = payload
                .get("engine")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'engine' wajib disertakan dalam payload".to_string())?
                .to_string();

            let _ = ctx
                .report_progress(
                    15,
                    Some(&format!("Memulai instalasi paket database {engine}...")),
                )
                .await;

            let res = match ctx
                .install(
                    &self.agent_client,
                    panel_proto::InstallOperation::DatabaseServer(
                        panel_proto::DatabaseServerInstallRequest {
                            engine: engine.clone(),
                        },
                    ),
                    15,
                )
                .await?
            {
                panel_proto::ResponsePayload::DatabaseServerInstalled(res) if res.success => res,
                _ => {
                    return Err(
                        "Instalasi database tidak berhasil atau respons agent tidak sesuai".into(),
                    );
                }
            };

            let _ = ctx
                .report_progress(80, Some("Memperbarui status server database..."))
                .await;

            let db_repo = panel_db::repositories::SqliteDatabaseRepository::new(ctx.pool.clone());
            let inventory = self
                .agent_client
                .database_server_status(panel_proto::DatabaseServerStatusRequest {
                    engines: vec![engine.clone()],
                })
                .await
                .map_err(|e| e.to_string())?;
            let server = inventory
                .servers
                .into_iter()
                .find(|server| server.engine == engine)
                .ok_or_else(|| "Database server status missing".to_string())?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            db_repo
                .upsert_server(&panel_db::models::DatabaseServer {
                    engine: engine.clone(),
                    is_installed: server.is_installed,
                    is_running: server.is_running,
                    version: server.version,
                    port: i64::from(server.port),
                    socket_path: server.socket_path,
                    updated_at: now,
                })
                .await
                .map_err(|e| e.to_string())?;

            if !server.is_installed || !server.is_running {
                return Err("Paket database selesai diproses tetapi service belum aktif; periksa log instalasi.".into());
            }

            let _ = ctx
                .report_progress(100, Some("Instalasi database selesai"))
                .await;

            let result = serde_json::json!({
                "engine": res.engine,
                "success": res.success,
                "message": res.message,
            });

            Ok(Some(result.to_string()))
        })
    }
}

/// Database export dump job handler.
pub struct DatabaseExportHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl DatabaseExportHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for DatabaseExportHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let database_id = payload
                .get("database_id")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let engine = payload
                .get("engine")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'engine' wajib disertakan dalam payload".to_string())?
                .to_string();
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'name' wajib disertakan dalam payload".to_string())?
                .to_string();
            let db_user = payload
                .get("db_user")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let export_file_path = payload
                .get("export_file_path")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'export_file_path' wajib disertakan".to_string())?
                .to_string();
            let tenant_username = payload
                .get("tenant_username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'tenant_username' wajib disertakan".to_string())?
                .to_string();
            let tenant_uid = payload
                .get("tenant_uid")
                .and_then(Value::as_u64)
                .filter(|uid| (10000..=59999).contains(uid))
                .ok_or_else(|| "UID tenant yang valid wajib disertakan".to_string())?
                as u32;
            validate_job_identity(ctx, job, &tenant_username, Some(tenant_uid)).await?;

            let _ = ctx
                .report_progress(20, Some(&format!("Mengekspor basis data {name}...")))
                .await;

            let res = self
                .agent_client
                .database_export(panel_proto::DatabaseExportRequest {
                    engine,
                    name,
                    db_user,
                    export_file_path,
                    tenant_username,
                    tenant_uid,
                })
                .await
                .map_err(|e| format!("Ekspor basis data gagal: {e}"))?;

            if database_id > 0 && res.size_bytes > 0 {
                let db_repo =
                    panel_db::repositories::SqliteDatabaseRepository::new(ctx.pool.clone());
                let _ = db_repo
                    .update_size(database_id, res.size_bytes as i64)
                    .await;
            }

            let _ = ctx
                .report_progress(100, Some("Ekspor basis data berhasil"))
                .await;

            let result = serde_json::json!({
                "engine": res.engine,
                "name": res.name,
                "file_path": res.file_path,
                "size_bytes": res.size_bytes,
                "success": res.success,
                "message": res.message,
            });

            Ok(Some(result.to_string()))
        })
    }
}

/// Database import dump job handler.
pub struct DatabaseImportHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl DatabaseImportHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for DatabaseImportHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let database_id = payload
                .get("database_id")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let engine = payload
                .get("engine")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'engine' wajib disertakan dalam payload".to_string())?
                .to_string();
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'name' wajib disertakan dalam payload".to_string())?
                .to_string();
            let db_user = payload
                .get("db_user")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let import_file_path = payload
                .get("import_file_path")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'import_file_path' wajib disertakan".to_string())?
                .to_string();
            let tenant_username = payload
                .get("tenant_username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'tenant_username' wajib disertakan".to_string())?
                .to_string();
            let tenant_uid = payload
                .get("tenant_uid")
                .and_then(Value::as_u64)
                .filter(|uid| (10000..=59999).contains(uid))
                .ok_or_else(|| "UID tenant yang valid wajib disertakan".to_string())?
                as u32;
            validate_job_identity(ctx, job, &tenant_username, Some(tenant_uid)).await?;

            let _ = ctx
                .report_progress(
                    20,
                    Some(&format!("Mengimpor berkas SQL ke database {name}...")),
                )
                .await;

            let res = self
                .agent_client
                .database_import(panel_proto::DatabaseImportRequest {
                    engine: engine.clone(),
                    name: name.clone(),
                    db_user,
                    import_file_path,
                    tenant_username,
                    tenant_uid,
                })
                .await
                .map_err(|e| format!("Impor database gagal: {e}"))?;

            let _ = ctx
                .report_progress(80, Some("Menghitung ukuran basis data terbaru..."))
                .await;

            if database_id > 0
                && let Ok(size_res) = self
                    .agent_client
                    .database_size_check(panel_proto::DatabaseSizeCheckRequest { engine, name })
                    .await
            {
                let db_repo =
                    panel_db::repositories::SqliteDatabaseRepository::new(ctx.pool.clone());
                let _ = db_repo
                    .update_size(database_id, size_res.size_bytes as i64)
                    .await;
            }

            let _ = ctx
                .report_progress(100, Some("Impor basis data berhasil"))
                .await;

            let result = serde_json::json!({
                "engine": res.engine,
                "name": res.name,
                "file_path": res.file_path,
                "success": res.success,
                "message": res.message,
            });

            Ok(Some(result.to_string()))
        })
    }
}

/// Site backup snapshot creation job handler.
pub struct BackupCreateHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl BackupCreateHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for BackupCreateHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let site_id = payload
                .get("site_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| "Field 'site_id' wajib disertakan dalam payload".to_string())?;
            let backup_id = payload
                .get("backup_id")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let tenant_username = payload
                .get("tenant_username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'tenant_username' wajib disertakan".to_string())?
                .to_string();
            let tenant_uid = payload
                .get("tenant_uid")
                .and_then(Value::as_u64)
                .filter(|uid| (10000..=59999).contains(uid))
                .ok_or_else(|| "UID tenant yang valid wajib disertakan".to_string())?
                as u32;
            validate_job_identity(ctx, job, &tenant_username, Some(tenant_uid)).await?;
            let site_dir = payload
                .get("site_dir")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'site_dir' wajib disertakan".to_string())?
                .to_string();
            let backup_dir = payload
                .get("backup_dir")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'backup_dir' wajib disertakan".to_string())?
                .to_string();
            let filename = payload
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'filename' wajib disertakan".to_string())?
                .to_string();
            let include_files = payload
                .get("include_files")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let include_databases = payload
                .get("include_databases")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let format = payload
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("tar.zst")
                .to_string();
            let retention_count = payload
                .get("retention_count")
                .and_then(Value::as_i64)
                .unwrap_or(0);

            let mut databases = Vec::new();
            if let Some(dbs_val) = payload.get("databases").and_then(Value::as_array) {
                for d in dbs_val {
                    if let (Some(eng), Some(nm), Some(usr)) = (
                        d.get("engine").and_then(Value::as_str),
                        d.get("name").and_then(Value::as_str),
                        d.get("username").and_then(Value::as_str),
                    ) {
                        databases.push(panel_proto::BackupDatabaseInfo {
                            engine: eng.to_string(),
                            name: nm.to_string(),
                            username: usr.to_string(),
                            password: d
                                .get("password")
                                .and_then(Value::as_str)
                                .map(ToString::to_string),
                        });
                    }
                }
            }

            let _ = ctx
                .report_progress(15, Some("Memulai pembuatan snapshot cadangan..."))
                .await;

            let res = self
                .agent_client
                .backup_create(panel_proto::BackupCreateRequest {
                    site_id,
                    tenant_username: tenant_username.clone(),
                    tenant_uid,
                    site_dir,
                    backup_dir,
                    filename: filename.clone(),
                    include_files,
                    include_databases,
                    databases,
                    format,
                })
                .await
                .map_err(|e| format!("Pembuatan cadangan gagal: {e}"))?;

            if !res.success {
                let err_msg = res
                    .error
                    .unwrap_or_else(|| "Gagal membuat arsip cadangan".to_string());
                if backup_id > 0 {
                    let backup_repo =
                        panel_db::repositories::SqliteBackupRepository::new(ctx.pool.clone());
                    let _ = backup_repo
                        .update_backup_status(backup_id, "failed", None, None, Some(&err_msg))
                        .await;
                }
                return Err(err_msg);
            }

            let _ = ctx
                .report_progress(80, Some("Memperbarui status cadangan di basis data..."))
                .await;

            let backup_repo = panel_db::repositories::SqliteBackupRepository::new(ctx.pool.clone());
            if backup_id > 0 {
                let _ = backup_repo
                    .update_backup_status(
                        backup_id,
                        "completed",
                        Some(res.size_bytes as i64),
                        Some(&res.checksum_sha256),
                        None,
                    )
                    .await;
            }

            // Prune old backups exceeding retention if configured
            if retention_count > 0
                && let Ok(pruned) = backup_repo
                    .prune_old_backups(site_id, retention_count)
                    .await
            {
                for p in pruned {
                    let _ = self
                        .agent_client
                        .backup_delete(panel_proto::BackupDeleteRequest {
                            filepath: p.filepath,
                            tenant_username: tenant_username.clone(),
                        })
                        .await;
                }
            }

            let _ = ctx
                .report_progress(100, Some("Pembuatan cadangan situs berhasil"))
                .await;

            let result = serde_json::json!({
                "site_id": site_id,
                "backup_id": backup_id,
                "filepath": res.filepath,
                "size_bytes": res.size_bytes,
                "checksum_sha256": res.checksum_sha256,
                "success": true,
            });

            Ok(Some(result.to_string()))
        })
    }
}

/// Site backup restoration job handler.
pub struct BackupRestoreHandler {
    agent_client: Arc<crate::agent_client::AgentClient>,
}

impl BackupRestoreHandler {
    pub fn new(agent_client: Arc<crate::agent_client::AgentClient>) -> Self {
        Self { agent_client }
    }
}

impl JobHandler for BackupRestoreHandler {
    fn run<'a>(&'a self, job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let payload: Value = serde_json::from_str(&job.payload_json).unwrap_or(Value::Null);
            let site_id = payload
                .get("site_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| "Field 'site_id' wajib disertakan dalam payload".to_string())?;
            let tenant_username = payload
                .get("tenant_username")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'tenant_username' wajib disertakan".to_string())?
                .to_string();
            let tenant_uid = payload
                .get("tenant_uid")
                .and_then(Value::as_u64)
                .filter(|uid| (10000..=59999).contains(uid))
                .ok_or_else(|| "UID tenant yang valid wajib disertakan".to_string())?
                as u32;
            validate_job_identity(ctx, job, &tenant_username, Some(tenant_uid)).await?;
            let backup_filepath = payload
                .get("backup_filepath")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'backup_filepath' wajib disertakan".to_string())?
                .to_string();
            let target_site_dir = payload
                .get("target_site_dir")
                .and_then(Value::as_str)
                .ok_or_else(|| "Field 'target_site_dir' wajib disertakan".to_string())?
                .to_string();
            let restore_files = payload
                .get("restore_files")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let restore_databases = payload
                .get("restore_databases")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            let mut databases = Vec::new();
            if let Some(dbs_val) = payload.get("databases").and_then(Value::as_array) {
                for d in dbs_val {
                    if let (Some(eng), Some(nm), Some(usr)) = (
                        d.get("engine").and_then(Value::as_str),
                        d.get("name").and_then(Value::as_str),
                        d.get("username").and_then(Value::as_str),
                    ) {
                        databases.push(panel_proto::BackupDatabaseInfo {
                            engine: eng.to_string(),
                            name: nm.to_string(),
                            username: usr.to_string(),
                            password: d
                                .get("password")
                                .and_then(Value::as_str)
                                .map(ToString::to_string),
                        });
                    }
                }
            }

            let _ = ctx
                .report_progress(20, Some("Mengekstrak dan memverifikasi arsip cadangan..."))
                .await;

            let res = self
                .agent_client
                .backup_restore(panel_proto::BackupRestoreRequest {
                    site_id,
                    tenant_username,
                    tenant_uid,
                    backup_filepath,
                    target_site_dir,
                    restore_files,
                    restore_databases,
                    databases,
                })
                .await
                .map_err(|e| format!("Pemulihan cadangan gagal: {e}"))?;

            if !res.success {
                let err_msg = res
                    .error
                    .unwrap_or_else(|| "Gagal memulihkan arsip cadangan".to_string());
                return Err(err_msg);
            }

            let _ = ctx
                .report_progress(100, Some("Pemulihan cadangan situs berhasil"))
                .await;

            let result = serde_json::json!({
                "site_id": site_id,
                "restored_files_count": res.restored_files_count,
                "restored_databases_count": res.restored_databases_count,
                "success": true,
            });

            Ok(Some(result.to_string()))
        })
    }
}
