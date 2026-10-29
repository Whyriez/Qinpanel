//! API route definitions and OpenAPI schemas.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use panel_proto::PROTOCOL_VERSION;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{OpenApi, ToSchema};

use crate::agent_client::AgentClient;
use crate::auth::{
    ChangePasswordRequest, CsrfResponse, LockoutManager, LoginRequest, LoginResponse,
    MessageResponse, TwoFactorDisableRequest, TwoFactorEnableRequest, TwoFactorSetupResponse,
    TwoFactorStatusResponse, UserInfo,
};
use crate::config::AuthConfig;
use panel_db::pool::DbPool;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::storage::managed::status,
        crate::storage::managed::configure,
        crate::storage::tls::overview,
        crate::storage::tls::generate,
        crate::storage::managed::create,
        crate::storage::managed::provision,
        crate::storage::managed::application_key,
        crate::storage::managed::decommission,
        crate::storage::policy::overview,
        crate::storage::policy::rules,
        crate::storage::policy::allocation,
        crate::storage::policy::usage,
        crate::storage::handlers::context,
        crate::storage::handlers::list,
        crate::storage::handlers::create,
        crate::storage::handlers::update,
        crate::storage::handlers::test,
        crate::storage::handlers::objects,
        crate::storage::handlers::upload,
        crate::storage::handlers::delete_object,
        crate::storage::handlers::download,
        crate::storage::handlers::download_proxy,
        crate::storage::handlers::create_bucket,
        crate::storage::handlers::bind,
        crate::storage::handlers::disconnect,
        crate::ai::overview,
        crate::ai::configure,
        crate::ai::test_connection,
        crate::ai::recommend,
        crate::server_backup::status,
        crate::server_backup::start,
        crate::server_backup::upload,
        crate::server_backup::download,
        crate::panel_update::overview,
        crate::panel_update::check,
        crate::panel_update::logs,
        crate::panel_update::apply,
        crate::connectivity::overview,
        crate::connectivity::find,
        crate::connectivity::create,
        crate::connectivity::replace,
        crate::connectivity::remove,
        crate::connectivity::diagnose,
        crate::panel_url_settings::overview,
        crate::dns01::overview,
        crate::dns01::configure,
        crate::dns01::revoke,
        crate::dns01::issue,
        crate::dns01::retry,
        crate::dns01::cleanup,
        crate::dns01::discard,
        crate::panel_tls::overview,
        crate::panel_tls::issue,
        crate::panel_tls::renew,
        crate::panel_tls::retry,
        crate::panel_tls::discard,
        crate::panel_url_settings::activate,
        crate::panel_url_settings::revoke,
        crate::database_web::tool_status,
        crate::database_web::install_tools,
        crate::database_web::launch,
        crate::email::templates,
        crate::email::save_template,
        crate::email::overview,
        crate::email::install,
        crate::email::create_mailbox,
        crate::email::change_mailbox,
        crate::email::messages,
        crate::email::message,
        crate::email::dns,
        crate::email::send,
        crate::network::overview,
        crate::network::save_addresses,
        crate::network::dns_check,
        crate::network::cloudflare_status,
        crate::network::cloudflare_command,
        crate::network::install,
        crate::setup::validate_token,
        crate::setup::create_administrator,
        crate::telemetry::current_metrics,
        healthz_handler,
        system_info_handler,
        crate::auth::handlers::login_handler,
        crate::auth::handlers::logout_handler,
        crate::auth::handlers::me_handler,
        crate::auth::handlers::csrf_handler,
        crate::auth::handlers::change_password_handler,
        crate::auth::handlers::two_factor_setup_handler,
        crate::auth::handlers::two_factor_enable_handler,
        crate::auth::handlers::two_factor_disable_handler,
        crate::auth::handlers::two_factor_status_handler,
        crate::admin::handlers::admin_list_users_handler,
        crate::admin::handlers::admin_get_user_handler,
        crate::admin::handlers::admin_update_user_status_handler,
        crate::admin::handlers::admin_get_setting_handler,
        crate::admin::handlers::admin_update_setting_handler,
        crate::admin::handlers::user_get_profile_handler,
        crate::audit::handlers::list_audit_logs_handler,
        crate::audit::handlers::get_audit_log_handler,
        crate::jobs::handlers::create_job_handler,
        crate::jobs::handlers::list_jobs_handler,
        crate::jobs::handlers::get_job_handler,
        crate::jobs::handlers::cancel_job_handler,
        crate::jobs::handlers::job_events_handler,
        crate::jobs::handlers::job_logs_handler,
        crate::ssl::get_site_ssl_handler,
        crate::ssl::issue_site_ssl_handler,
        crate::ssl::self_signed_site_ssl_handler,
        crate::ssl::custom_site_ssl_handler,
        crate::ssl::renew_site_ssl_handler,
        crate::ssl::delete_site_ssl_handler,
        crate::logs::get_site_logs_handler,
        crate::php::list_php_versions_handler,
        crate::php::setup_php_repo_handler,
        crate::php::install_php_version_handler,
        crate::php::remove_php_version_handler,
        crate::php::list_php_extensions_handler,
        crate::php::install_php_extension_handler,
        crate::php::toggle_php_extension_handler,
        crate::php::remove_php_extension_handler,
        crate::php::get_site_php_extensions_handler,
        crate::php::update_site_php_extensions_handler,
        crate::php::list_php_fpm_masters_handler,
        crate::php::get_php_fpm_master_status_handler,
        crate::php::reload_php_fpm_master_handler,
        crate::php::restart_php_fpm_master_handler,
        crate::php::get_site_php_status_handler,
        crate::php::get_site_php_logs_handler,
        crate::files::list_files_handler,
        crate::files::stat_file_handler,
        crate::files::mkdir_handler,
        crate::files::rename_handler,
        crate::files::copy_handler,
        crate::files::trash_handler,
        crate::files::list_trash_handler,
        crate::files::restore_trash_handler,
        crate::files::empty_trash_handler,
        crate::files::delete_permanent_handler,
        crate::files::upload_init_handler,
        crate::files::upload_status_handler,
        crate::files::upload_chunk_handler,
        crate::files::download_file_handler,
        crate::files::download_zip_handler,
        crate::files::archive_compress_handler,
        crate::files::archive_extract_handler,
        crate::files::chmod_handler,
        crate::files::chmod_recursive_handler,
        crate::files::chown_handler,
        crate::files::get_file_content_handler,
        crate::files::save_file_content_handler,
        crate::files::search_files_handler,
        crate::terminal::create_session_handler,
        crate::terminal::list_sessions_handler,
        crate::terminal::get_session_handler,
        crate::terminal::close_session_handler,
        crate::terminal::resize_session_handler,
        crate::terminal::get_terminal_audit_handler,
        crate::ssh::handlers::list_ssh_keys_handler,
        crate::ssh::handlers::create_ssh_key_handler,
        crate::ssh::handlers::delete_ssh_key_handler,
        crate::ssh::handlers::validate_ssh_key_handler,
        crate::ssh::handlers::apply_ssh_dropin_handler,
        crate::ssh::handlers::get_ssh_connection_info_handler,
        crate::tenants::handlers::get_tenant_shell_policy_handler,
        crate::tenants::handlers::update_tenant_shell_policy_handler,
        crate::node::get_node_versions_handler,
        crate::node::install_node_version_handler,
        crate::node::remove_node_version_handler,
        crate::node::enable_node_corepack_handler,
        crate::analytics::get_site_analytics_overview_handler,
        crate::analytics::get_site_analytics_timeseries_handler,
        crate::analytics::get_site_analytics_breakdowns_handler,
        crate::analytics::get_site_analytics_compare_handler,
        crate::analytics::get_site_app_metrics_handler,
        crate::analytics::get_site_slowlog_handler,
        crate::analytics::export_site_analytics_csv_handler,
        crate::analytics::get_site_insights_handler,
        crate::analytics::evaluate_site_insights_handler,
        crate::analytics::get_system_analytics_timeseries_handler,
        crate::analytics::get_system_insights_handler,
        crate::security_analytics::get_site_security_overview_handler,
        crate::security_analytics::get_site_banned_ips_handler,
        crate::security_analytics::ban_ip_handler,
        crate::security_analytics::unban_ip_handler,
        crate::retention::get_analytics_retention_handler,
        crate::retention::update_analytics_retention_handler,
        crate::retention::trigger_analytics_maintenance_handler,
        crate::retention::check_analytics_integrity_handler,
        crate::optimizer::get_optimizer_status_handler,
        crate::optimizer::apply_optimizer_handler,
        crate::optimizer::rollback_optimizer_handler,
        crate::memory_profile::get_memory_profile_handler,
        crate::security_hardening::get_security_hardening_overview_handler,
        crate::security_hardening::sync_nftables_loopback_handler,
        crate::system_reliability::get_system_doctor_handler,
        crate::system_reliability::fix_system_doctor_handler,
        crate::system_reliability::get_system_reconcile_handler,
        crate::system_reliability::apply_system_reconcile_handler,
        crate::system_reliability::list_system_backups_handler,
        crate::system_reliability::create_system_backup_handler,
        crate::system_reliability::restore_system_backup_handler,
    ),
    components(
        schemas(
            crate::storage::managed::ManagedStorageConfiguration,
            crate::storage::tls::Generate,
            crate::storage::managed::ManagedStorageAllocation,
            crate::storage::managed::ManagedStorageKeyInput,
            crate::storage::handlers::StorageConfiguration,
            crate::storage::handlers::StorageConfirmation,
            crate::storage::handlers::StorageObjectInput,
            crate::storage::handlers::StorageBinding,
            crate::storage::policy::Rules,
            crate::storage::policy::RulesInput,
            crate::storage::policy::Allocation,
            panel_proto::S3Credentials,
            crate::ai::AiConfiguration,
            crate::ai::AiConfirmation,
            crate::ai::AiRecommendationRequest,
            panel_proto::AiProvider,
            crate::panel_update::ApplyUpdate,
            panel_proto::PanelUpdateStatus,
            panel_proto::PanelUpdateRun,
            panel_proto::PanelRelease,
            panel_proto::UpdatePhase,
            crate::setup::SetupRequest,
            crate::setup::SetupResponse,
            crate::setup::SetupTokenRequest,
            HealthResponse,
            SystemInfoResponse,
            LoginRequest,
            LoginResponse,
            UserInfo,
            ChangePasswordRequest,
            MessageResponse,
            CsrfResponse,
            TwoFactorSetupResponse,
            TwoFactorEnableRequest,
            TwoFactorDisableRequest,
            TwoFactorStatusResponse,
            crate::admin::AdminUserItem,
            crate::admin::UpdateUserStatusRequest,
            crate::admin::UpdateSettingRequest,
            crate::admin::SettingResponse,
            crate::audit::AuditLogItem,
            crate::audit::AuditLogListResponse,
            crate::jobs::JobItem,
            crate::jobs::CreateJobRequest,
            crate::jobs::JobListResponse,
            crate::jobs::JobProgressEvent,
            crate::sites::CreateStaticSiteRequest,
            crate::sites::AddSiteDomainRequest,
            crate::sites::UpdateSiteDomainRequest,
            crate::sites::SetForceHttpsRequest,
            crate::sites::SiteResponse,
            crate::sites::SiteDomainResponse,
            crate::sites::SiteMessageResponse,
            crate::ssl::IssueSslRequest,
            crate::ssl::CustomCertRequest,
            crate::ssl::SslStatusResponse,
            crate::ssl::CertificateDetailResponse,
            crate::ssl::SslActionResponse,
            crate::logs::SiteLogsResponse,
            crate::php::PhpVersionStatusItem,
            crate::php::PhpRuntimeListApiResponse,
            crate::php::PhpExtensionItem,
            crate::php::PhpExtensionListApiResponse,
            crate::php::ToggleExtensionRequest,
            crate::php::PhpExtensionActionApiResponse,
            crate::php::SitePhpExtensionsResponse,
            crate::php::UpdateSitePhpExtensionsInput,
            crate::php::PhpFpmMasterOverviewDto,
            crate::php::PhpFpmMasterStatusDto,
            crate::php::PhpFpmActionResponseDto,
            crate::php::SwitchPhpVersionRequest,
            crate::php::SwitchPhpVersionResponse,
            crate::php::PhpFpmStatusMetricsDto,
            crate::php::SitePhpStatusResponse,
            crate::php::SitePhpLogsResponse,
            crate::files::MkdirPayload,
            crate::files::RenamePayload,
            crate::files::CopyPayload,
            crate::files::TrashPayload,
            crate::files::TrashRestorePayload,
            crate::files::DeletePayload,
            crate::files::FileActionResponse,
            crate::files::TrashEmptyApiResponse,
            crate::files::FsEntryItem,
            crate::files::FsListApiResponse,
            crate::files::TrashedItemApiResponse,
            crate::files::UploadMetadataApiResponse,
            crate::files::UploadInitPayload,
            crate::files::ArchiveCompressPayload,
            crate::files::ArchiveExtractPayload,
            crate::files::ArchiveResultApiResponse,
            crate::files::ArchiveActionResponse,
            crate::files::ChmodPayload,
            crate::files::ChmodApiResponse,
            crate::files::ChmodRecursivePayload,
            crate::files::ChmodRecursiveApiResponse,
            crate::files::ChownPayload,
            crate::files::ChownApiResponse,
            crate::files::FileContentApiResponse,
            crate::files::FileSavePayload,
            crate::files::FileSaveApiResponse,
            crate::files::FileSearchPayload,
            crate::files::ContentMatchApiDto,
            crate::files::SearchResultItemApi,
            crate::files::FileSearchApiResponse,
            crate::terminal::CreateTerminalSessionRequest,
            crate::terminal::TerminalSessionResponse,
            crate::terminal::TerminalSessionListResponse,
            crate::terminal::TerminalCloseApiResponse,
            crate::terminal::TerminalResizeRequest,
            crate::terminal::TerminalResizeResponse,
            crate::terminal::TerminalAuditItem,
            crate::terminal::TerminalAuditListResponse,
            crate::ssh::SshKeyResponse,
            crate::ssh::CreateSshKeyRequest,
            crate::ssh::ValidateSshKeyRequest,
            crate::ssh::ValidateSshKeyResponse,
            crate::ssh::SshDropinApplyRequestDto,
            crate::ssh::SshDropinApplyResponseDto,
            crate::ssh::SshConnectionInfoResponse,
            crate::tenants::TenantShellPolicyResponse,
            crate::tenants::UpdateTenantShellPolicyRequest,
            crate::node::NodeVersionItemDto,
            crate::node::NodeRuntimeListApiResponse,
            crate::node::NodeRuntimeInstallApiRequest,
            crate::node::NodeRuntimeActionApiResponse,
            crate::node_site::NodeSiteActionApiRequest,
            crate::node_site::NodeSiteJobApiRequest,
            crate::node_site::NodeSiteSettingsDto,
            crate::node_site::UpdateNodeSiteSettingsRequest,
            crate::node_site::NodeSiteStatusApiResponse,
            crate::analytics::SiteAnalyticsSummaryDto,
            crate::analytics::PeriodChangeDto,
            crate::analytics::SiteAnalyticsOverviewResponse,
            crate::analytics::SiteTimeseriesPointDto,
            crate::analytics::SiteTimeseriesResponse,
            crate::analytics::BreakdownItemDto,
            crate::analytics::SiteBreakdownsResponse,
            crate::analytics::PhpFpmPoolOverviewDto,
            crate::analytics::AppProcessOverviewDto,
            crate::analytics::SiteAppMetricsResponse,
            crate::analytics::PhpSlowlogFrameDto,
            crate::analytics::PhpSlowlogEntryDto,
            crate::analytics::SiteSlowlogResponse,
            crate::analytics::ServerTimeseriesPointDto,
            crate::analytics::SystemTimeseriesResponse,
            crate::analytics::InsightActionDto,
            crate::analytics::AnalyticsInsightDto,
            crate::analytics::SiteInsightsResponse,
            crate::analytics::SystemInsightsResponse,
            crate::security_analytics::BanIpRequest,
            crate::security_analytics::BannedIpDto,
            crate::security_analytics::TopSecurityIpDto,
            crate::security_analytics::ProbingPathStatDto,
            crate::security_analytics::BruteForceIncidentDto,
            crate::security_analytics::SecurityOverviewResponse,
            crate::security_analytics::UnbanIpResponse,
            crate::retention::AnalyticsRetentionResponse,
            crate::retention::RetentionConfigDto,
            crate::retention::DatabaseStorageStatsDto,
            crate::retention::DatabaseTableStatDto,
            crate::retention::RetentionMaintenanceReportDto,
            crate::retention::IntegrityCheckReportDto,
            crate::retention::UpdateRetentionRequest,
            crate::retention::TriggerMaintenanceRequest,
            crate::optimizer::ServerHardwareSpecsDto,
            crate::optimizer::NginxTuningConfigDto,
            crate::optimizer::SysctlTuningConfigDto,
            crate::optimizer::PhpFpmTuningConfigDto,
            crate::optimizer::AppRuntimeHintsDto,
            crate::optimizer::OptimizerProfileDto,
            crate::optimizer::OptimizerStatusResponseDto,
            crate::optimizer::ApplyOptimizerRequestDto,
            crate::optimizer::OptimizerActionResponseDto,
            panel_core::memory_profile::ProcessMemorySnapshot,
            panel_core::memory_profile::ReleaseProfileInfo,
            panel_core::memory_profile::MemoryTargetEvaluation,
            panel_core::memory_profile::PanelMemoryProfileReport,
            panel_core::hardening::AsvsVerificationLevel,
            panel_core::hardening::AsvsControlStatus,
            panel_core::hardening::AsvsRequirement,
            panel_core::hardening::OwaspAsvsAuditReport,
            panel_core::hardening::SystemdDirective,
            panel_core::hardening::SystemdServiceSecurityScore,
            crate::security_hardening::SecurityHardeningOverviewResponse,
            crate::security_hardening::TenantPortBindingDto,
            crate::security_hardening::NftablesStatusResponseDto,
            crate::security_hardening::NftablesSyncPayload,
            crate::security_hardening::NftablesSyncResponseDto,
            panel_core::doctor::DoctorCheckCategory,
            panel_core::doctor::DoctorCheckStatus,
            panel_core::doctor::DoctorCheckItem,
            panel_core::doctor::DoctorReport,
            panel_core::reconcile::DriftType,
            panel_core::reconcile::DriftItem,
            panel_core::reconcile::ReconcileReport,
            panel_core::system_backup::SystemBackupSummary,
            panel_core::system_backup::SystemBackupFileEntry,
            panel_core::system_backup::SystemBackupManifest,
            panel_core::system_backup::SystemBackupVerificationResult,
            crate::system_reliability::CreateSystemBackupPayload,
            crate::system_reliability::RestoreSystemBackupPayload,
        )
    ),
    tags(
        (name = "storage", description = "Tenant-scoped S3 connections, project bindings and object operations"),
        (name = "system", description = "System and health check endpoints"),
        (name = "auth", description = "Authentication and session management"),
        (name = "admin", description = "Administrator-only management endpoints"),
        (name = "user", description = "Tenant user resource endpoints"),
        (name = "audit", description = "Audit log query and tracking endpoints"),
        (name = "jobs", description = "Background jobs and real-time progress tracking endpoints"),
        (name = "ssl", description = "SSL/TLS and ACME certificate management"),
        (name = "runtime", description = "Runtime and PHP version management"),
        (name = "files", description = "File manager operations and trash management"),
        (name = "terminal", description = "Web terminal and PTY session endpoints"),
        (name = "ssh", description = "SSH public key management and validation"),
        (name = "tenants", description = "Tenant lifecycle and resource management"),
        (name = "analytics", description = "Deep traffic analytics, runtime metrics, and timeseries endpoints")
    ),
    info(
        title = "Qinpanel API",
        version = "0.1.0",
        description = "Lightweight, secure, tenant-isolated web hosting control panel API"
    )
)]
pub struct ApiDoc;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    #[schema(example = "ok")]
    pub status: String,
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SystemInfoResponse {
    #[schema(example = "0.1.0")]
    pub api_version: String,
    pub agent_version: Option<String>,
    pub proto_version: u32,
    pub os: String,
    pub arch: String,
    pub agent_connected: bool,
    pub environment: String,
}

#[derive(Clone)]
pub struct AppState {
    pub agent_client: Arc<AgentClient>,
    pub pool: DbPool,
    pub operations_pool: DbPool,
    pub operations_read_pool: DbPool,
    pub environment: String,
    pub auth_config: AuthConfig,
    pub lockout_manager: Arc<LockoutManager>,
    pub audit_writer: Arc<crate::audit::AuditWriter>,
    pub job_runner: Arc<crate::jobs::JobRunner>,
    pub terminal_hub: Arc<crate::terminal::TerminalHub>,
    pub analytics_storage: Arc<tokio::sync::RwLock<Option<Arc<panel_analytics::AnalyticsStorage>>>>,
    pub analytics_db_path: std::path::PathBuf,
    pub collector_status: Arc<tokio::sync::RwLock<crate::telemetry::CollectorStatus>>,
}

impl AppState {
    /// Single-database compatibility for isolated fixtures. The daemon initializes
    /// operational pools before constructing state with `with_operations`.
    pub fn new(
        agent_client: Arc<AgentClient>,
        pool: DbPool,
        environment: String,
        auth_config: AuthConfig,
        lockout_manager: Arc<LockoutManager>,
    ) -> Self {
        Self::with_operations(
            agent_client,
            pool.clone(),
            (pool.clone(), pool),
            environment,
            auth_config,
            lockout_manager,
        )
    }

    pub fn with_operations(
        agent_client: Arc<AgentClient>,
        pool: DbPool,
        operations: (DbPool, DbPool),
        environment: String,
        auth_config: AuthConfig,
        lockout_manager: Arc<LockoutManager>,
    ) -> Self {
        let (operations_pool, operations_read_pool) = operations;
        let audit_writer = Arc::new(crate::audit::AuditWriter::new(operations_pool.clone()));
        let job_registry = crate::jobs::JobRegistry::new_with_agent(Arc::clone(&agent_client));
        let job_runner = Arc::new(crate::jobs::JobRunner::with_pools(
            pool.clone(),
            operations_pool.clone(),
            job_registry,
        ));
        let terminal_hub = Arc::new(
            crate::terminal::TerminalHub::new(Arc::clone(&agent_client))
                .with_audit_writer(Arc::clone(&audit_writer)),
        );
        let analytics_storage = Arc::new(tokio::sync::RwLock::new(None));
        Self {
            agent_client,
            pool,
            operations_pool,
            operations_read_pool,
            environment,
            auth_config,
            lockout_manager,
            audit_writer,
            job_runner,
            terminal_hub,
            analytics_storage,
            analytics_db_path: std::env::var_os("PANEL_ANALYTICS_DB_PATH")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "/var/lib/panel/analytics.db".into()),
            collector_status: Arc::new(tokio::sync::RwLock::new(
                crate::telemetry::CollectorStatus::default(),
            )),
        }
    }

    /// Sets an explicit analytics storage instance (useful for hermetic tests).
    pub fn with_analytics(self, storage: Arc<panel_analytics::AnalyticsStorage>) -> Self {
        if let Ok(mut lock) = self.analytics_storage.try_write() {
            *lock = Some(storage);
        }
        self
    }

    /// Retrieves or lazily connects to the analytics storage.
    pub async fn get_analytics(
        &self,
    ) -> Result<Arc<panel_analytics::AnalyticsStorage>, (StatusCode, String)> {
        {
            let lock = self.analytics_storage.read().await;
            if let Some(ref s) = *lock {
                return Ok(Arc::clone(s));
            }
        }
        let mut lock = self.analytics_storage.write().await;
        if let Some(ref s) = *lock {
            return Ok(Arc::clone(s));
        }

        let storage = panel_analytics::AnalyticsStorage::connect(&self.analytics_db_path)
            .await
            .map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, {
                    tracing::error!(error=%e, "Analytics database initialization failed");
                    "Gagal menghubungkan database analitik.".to_owned()
                })
            })?;
        let arc_s = Arc::new(storage);
        *lock = Some(Arc::clone(&arc_s));
        Ok(arc_s)
    }
}

#[utoipa::path(
    get,
    path = "/healthz",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse)
    ),
    tag = "system"
)]
pub async fn healthz_handler() -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    (
        StatusCode::OK,
        [(
            "x-qonpanel-active-writes",
            crate::maintenance::active().to_string(),
        )],
        Json(HealthResponse {
            status: "ok".to_string(),
            timestamp: now,
        }),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/system/info",
    responses(
        (status = 200, description = "System information and agent status", body = SystemInfoResponse)
    ),
    tag = "system"
)]
pub async fn system_info_handler(State(state): State<AppState>) -> impl IntoResponse {
    let agent_ver = match state.agent_client.version().await {
        Ok(v) => (Some(v.version), true),
        Err(_) => (None, false),
    };

    (
        StatusCode::OK,
        Json(SystemInfoResponse {
            api_version: env!("CARGO_PKG_VERSION").to_string(),
            agent_version: agent_ver.0,
            proto_version: PROTOCOL_VERSION,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            agent_connected: agent_ver.1,
            environment: state.environment.clone(),
        }),
    )
}
