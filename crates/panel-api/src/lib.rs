pub mod admin;
pub mod agent_client;
pub mod ai;
pub mod analytics;
pub mod audit;
pub mod auth;
pub mod backups;
pub mod config;
pub mod connectivity;
mod connectivity_diagnosis;
pub mod cron;
pub mod database_web;
pub mod databases;
pub mod dns01;
pub mod email;
pub mod env_vars;
pub mod files;
pub mod git_deploy;
pub mod jobs;
pub mod logs;
pub mod mail;
pub mod maintenance;
pub mod memory_profile;
pub mod network;
pub mod node;
pub mod node_site;
pub mod notifications;
pub mod optimizer;
pub mod panel_tls;
pub mod panel_update;
pub mod panel_url_settings;
pub mod php;
pub mod plans;
pub mod processes;
pub mod proxy_site;
pub mod public_url;
pub mod python_site;
pub mod retention;
pub mod routes;
pub mod runtime_catalog;
pub mod security_analytics;
pub mod security_hardening;
pub mod security_headers;
pub mod server_backup;
pub mod setup;
pub mod site_rules;
pub mod sites;
pub mod ssh;
pub mod ssl;
pub mod static_files;
pub mod storage;
pub mod system_reliability;
pub mod telemetry;
pub mod tenant_context;
pub mod tenants;
pub mod terminal;

use axum::{
    Router,
    routing::{get, post, put},
};
use routes::{ApiDoc, AppState, healthz_handler, system_info_handler};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

pub fn create_router(state: AppState) -> Router {
    let api_doc = ApiDoc::openapi();

    let api_routes = Router::new()
        .merge(storage::router(std::sync::Arc::clone(&state.agent_client)))
        .merge(ai::router(std::sync::Arc::clone(&state.agent_client)))
        .merge(panel_update::router())
        .merge(server_backup::router())
        .merge(database_web::router())
        .merge(connectivity::router())
        .merge(panel_url_settings::router())
        .merge(panel_tls::router())
        .merge(dns01::router())
        .route(
            "/api/v1/runtimes/catalog",
            get(runtime_catalog::get_status).put(runtime_catalog::update_schedule),
        )
        .route(
            "/api/v1/runtimes/catalog/refresh",
            post(runtime_catalog::refresh_handler),
        )
        .route("/api/v1/email", get(email::overview))
        .route("/api/v1/email/install", post(email::install))
        .route(
            "/api/v1/email/templates",
            get(email::templates).put(email::save_template),
        )
        .route("/api/v1/email/mailboxes", post(email::create_mailbox))
        .route("/api/v1/email/mailboxes/{id}", post(email::change_mailbox))
        .route(
            "/api/v1/email/mailboxes/{id}/messages",
            get(email::messages).post(email::message),
        )
        .route("/api/v1/email/mailboxes/{id}/send", post(email::send))
        .route("/api/v1/email/mailboxes/{id}/dns", get(email::dns))
        .route(
            "/api/v1/network",
            get(network::overview).put(network::save_addresses),
        )
        .route("/api/v1/network/dns-check", post(network::dns_check))
        .route(
            "/api/v1/network/cloudflare",
            get(network::cloudflare_status).post(network::cloudflare_command),
        )
        .route("/api/v1/network/cloudflare/install", post(network::install))
        .route("/healthz", get(healthz_handler))
        .route("/api/v1/system/info", get(system_info_handler))
        .route("/api/v1/system/metrics", get(telemetry::current_metrics))
        .route(
            "/api/v1/system/memory-profile",
            get(memory_profile::get_memory_profile_handler),
        )
        .route("/api/v1/auth/login", post(auth::login_handler))
        .route("/api/v1/auth/setup/validate", post(setup::validate_token))
        .route("/api/v1/auth/setup", post(setup::create_administrator))
        .route("/api/v1/auth/logout", post(auth::logout_handler))
        .route("/api/v1/auth/me", get(auth::me_handler))
        .route("/api/v1/auth/csrf", get(auth::csrf_handler))
        .route("/api/v1/auth/password", post(auth::change_password_handler))
        .route(
            "/api/v1/auth/2fa/setup",
            post(auth::two_factor_setup_handler),
        )
        .route(
            "/api/v1/auth/2fa/enable",
            post(auth::two_factor_enable_handler),
        )
        .route(
            "/api/v1/auth/2fa/disable",
            post(auth::two_factor_disable_handler),
        )
        .route(
            "/api/v1/auth/2fa/status",
            get(auth::two_factor_status_handler),
        )
        .route("/api/v1/admin/users", get(admin::admin_list_users_handler))
        .route(
            "/api/v1/admin/users/{id}",
            get(admin::admin_get_user_handler),
        )
        .route(
            "/api/v1/admin/users/{id}/status",
            post(admin::admin_update_user_status_handler),
        )
        .route(
            "/api/v1/admin/settings",
            get(admin::admin_get_setting_handler),
        )
        .route(
            "/api/v1/admin/settings",
            post(admin::admin_update_setting_handler),
        )
        .route(
            "/api/v1/user/{id}/profile",
            get(admin::user_get_profile_handler),
        )
        .route("/api/v1/audit/logs", get(audit::list_audit_logs_handler))
        .route("/api/v1/audit/logs/{id}", get(audit::get_audit_log_handler))
        .route(
            "/api/v1/jobs",
            post(jobs::create_job_handler).get(jobs::list_jobs_handler),
        )
        .route("/api/v1/jobs/{id}", get(jobs::get_job_handler))
        .route("/api/v1/jobs/{id}/cancel", post(jobs::cancel_job_handler))
        .route("/api/v1/jobs/{id}/events", get(jobs::job_events_handler))
        .route("/api/v1/jobs/{id}/logs", get(jobs::job_logs_handler))
        // Plans endpoints
        .route(
            "/api/v1/plans",
            get(plans::list_plans_handler).post(plans::create_plan_handler),
        )
        .route(
            "/api/v1/plans/{id}",
            get(plans::get_plan_handler)
                .put(plans::update_plan_handler)
                .delete(plans::delete_plan_handler),
        )
        // Tenants endpoints
        .route(
            "/api/v1/tenants",
            get(tenants::list_tenants_handler).post(tenants::create_tenant_handler),
        )
        .route(
            "/api/v1/tenants/{id}",
            get(tenants::get_tenant_handler).delete(tenants::delete_tenant_handler),
        )
        .route(
            "/api/v1/tenants/{id}/resources",
            get(tenants::get_tenant_resources_handler),
        )
        .route(
            "/api/v1/tenants/{id}/suspend",
            post(tenants::suspend_tenant_handler),
        )
        .route(
            "/api/v1/tenants/{id}/unsuspend",
            post(tenants::unsuspend_tenant_handler),
        )
        .route(
            "/api/v1/tenants/{id}/impersonate",
            post(tenants::impersonate_tenant_handler),
        )
        .route(
            "/api/v1/auth/stop-impersonating",
            post(tenants::stop_impersonating_handler),
        )
        .route(
            "/api/v1/tenants/{id}/shell-policy",
            get(tenants::get_tenant_shell_policy_handler)
                .put(tenants::update_tenant_shell_policy_handler),
        )
        // Hardening & Prerequisites endpoints
        .route(
            "/api/v1/system/hardening",
            get(tenants::get_hardening_status_handler),
        )
        .route(
            "/api/v1/system/hardening/apply",
            post(tenants::apply_hardening_handler),
        )
        // Sites endpoints
        .route(
            "/api/v1/sites",
            post(sites::create_static_site_handler).get(sites::list_sites_handler),
        )
        .route(
            "/api/v1/sites/{id}",
            get(sites::get_site_handler).delete(sites::delete_site_handler),
        )
        .route(
            "/api/v1/sites/{id}/overview",
            get(sites::get_site_overview_handler),
        )
        .route(
            "/api/v1/sites/{id}/action",
            post(sites::site_quick_action_handler),
        )
        .route(
            "/api/v1/sites/{id}/enable",
            post(sites::enable_site_handler),
        )
        .route(
            "/api/v1/sites/{id}/disable",
            post(sites::disable_site_handler),
        )
        .route(
            "/api/v1/sites/{id}/force-https",
            post(sites::set_force_https_handler),
        )
        .route(
            "/api/v1/sites/{id}/domains",
            get(sites::list_site_domains_handler).post(sites::add_site_domain_handler),
        )
        .route(
            "/api/v1/sites/{id}/domains/{domain_id}",
            put(sites::update_site_domain_handler).delete(sites::delete_site_domain_handler),
        )
        // SSL & ACME endpoints
        .route(
            "/api/v1/sites/{id}/ssl",
            get(ssl::get_site_ssl_handler).delete(ssl::delete_site_ssl_handler),
        )
        .route(
            "/api/v1/sites/{id}/ssl/issue",
            post(ssl::issue_site_ssl_handler),
        )
        .route(
            "/api/v1/sites/{id}/ssl/self-signed",
            post(ssl::self_signed_site_ssl_handler),
        )
        .route(
            "/api/v1/sites/{id}/ssl/custom",
            post(ssl::custom_site_ssl_handler),
        )
        .route(
            "/api/v1/sites/{id}/ssl/renew",
            post(ssl::renew_site_ssl_handler),
        )
        // Site Logs & WebSocket Live Tail
        .route("/api/v1/sites/{id}/logs", get(logs::get_site_logs_handler))
        .route("/ws/logs/{id}", get(logs::site_logs_ws_handler))
        // PHP Runtime management endpoints
        .route("/api/v1/php/versions", get(php::list_php_versions_handler))
        .route("/api/v1/php/repo/setup", post(php::setup_php_repo_handler))
        .route(
            "/api/v1/php/versions/{version}/install",
            post(php::install_php_version_handler),
        )
        .route(
            "/api/v1/php/versions/{version}/remove",
            post(php::remove_php_version_handler),
        )
        .route(
            "/api/v1/php/versions/{version}/extensions",
            get(php::list_php_extensions_handler),
        )
        .route(
            "/api/v1/php/versions/{version}/extensions/{extension}/install",
            post(php::install_php_extension_handler),
        )
        .route(
            "/api/v1/php/versions/{version}/extensions/{extension}/toggle",
            post(php::toggle_php_extension_handler),
        )
        .route(
            "/api/v1/php/versions/{version}/extensions/{extension}/remove",
            post(php::remove_php_extension_handler),
        )
        .route(
            "/api/v1/sites/{id}/php/extensions",
            get(php::get_site_php_extensions_handler).put(php::update_site_php_extensions_handler),
        )
        .route(
            "/api/v1/sites/{id}/php/settings",
            get(php::get_site_php_settings_handler).put(php::update_site_php_settings_handler),
        )
        .route(
            "/api/v1/sites/{id}/php/version",
            post(php::switch_site_php_version_handler).put(php::switch_site_php_version_handler),
        )
        .route(
            "/api/v1/sites/{id}/php/status",
            get(php::get_site_php_status_handler),
        )
        .route(
            "/api/v1/sites/{id}/php/logs",
            get(php::get_site_php_logs_handler),
        )
        // Environment variables management endpoints (Phase 8 - P8-T1)
        .route(
            "/api/v1/sites/{id}/env",
            get(env_vars::get_site_env_handler).put(env_vars::update_site_env_handler),
        )
        .route(
            "/api/v1/sites/{id}/env/import",
            post(env_vars::import_site_env_handler),
        )
        .route(
            "/api/v1/sites/{id}/env/restart",
            post(env_vars::restart_site_env_handler),
        )
        // Cron jobs management endpoints (Phase 8 - P8-T2)
        .route(
            "/api/v1/sites/{id}/cron",
            get(cron::list_site_cron_handler).post(cron::create_site_cron_handler),
        )
        .route(
            "/api/v1/sites/{id}/cron/validate",
            post(cron::validate_cron_handler),
        )
        .route(
            "/api/v1/sites/{id}/cron/{job_id}",
            get(cron::get_site_cron_handler)
                .put(cron::update_site_cron_handler)
                .delete(cron::delete_site_cron_handler),
        )
        .route(
            "/api/v1/sites/{id}/cron/{job_id}/run",
            post(cron::run_site_cron_handler),
        )
        .route(
            "/api/v1/sites/{id}/cron/{job_id}/runs",
            get(cron::get_site_cron_runs_handler),
        )
        .route(
            "/api/v1/sites/{id}/cron/{job_id}/logs",
            get(cron::get_site_cron_logs_handler),
        )
        // Site rules management endpoints (Phase 8 - P8-T3)
        .route(
            "/api/v1/sites/{id}/rules",
            get(site_rules::get_site_rules_handler).put(site_rules::update_site_rules_handler),
        )
        .route(
            "/api/v1/sites/{id}/rules/hash-password",
            post(site_rules::hash_basic_auth_password_handler),
        )
        .route(
            "/api/v1/sites/{id}/rules/validate-snippet",
            post(site_rules::validate_snippet_handler),
        )
        // Git Deploy management endpoints (Phase 8 - P8-T4)
        .route(
            "/api/v1/sites/{id}/git",
            get(git_deploy::get_git_config_handler).put(git_deploy::upsert_git_config_handler),
        )
        .route(
            "/api/v1/sites/{id}/git/key/generate",
            post(git_deploy::generate_deploy_key_handler),
        )
        .route(
            "/api/v1/sites/{id}/git/deploy",
            post(git_deploy::trigger_deploy_handler),
        )
        .route(
            "/api/v1/sites/{id}/git/rollback",
            post(git_deploy::rollback_deploy_handler),
        )
        .route(
            "/api/v1/sites/{id}/git/deployments",
            get(git_deploy::list_deployments_handler),
        )
        .route(
            "/api/v1/sites/{id}/git/deployments/{deploy_id}",
            get(git_deploy::get_deployment_detail_handler),
        )
        // Public webhook endpoint
        .route(
            "/api/v1/webhooks/deploy/{site_id}",
            post(git_deploy::webhook_deploy_handler),
        )
        // Node.js site management endpoints
        .route(
            "/api/v1/sites/{id}/node/status",
            get(node_site::get_node_site_status_handler),
        )
        .route(
            "/api/v1/sites/{id}/node/action",
            post(node_site::action_node_site_handler),
        )
        .route(
            "/api/v1/sites/{id}/node/job",
            post(node_site::run_node_site_job_handler),
        )
        .route(
            "/api/v1/sites/{id}/node/settings",
            get(node_site::get_node_site_settings_handler)
                .put(node_site::update_node_site_settings_handler),
        )
        // PHP-FPM Master management endpoints
        .route(
            "/api/v1/php/masters",
            get(php::list_php_fpm_masters_handler),
        )
        .route(
            "/api/v1/php/masters/{username}/{version}/status",
            get(php::get_php_fpm_master_status_handler),
        )
        .route(
            "/api/v1/php/masters/{username}/{version}/reload",
            post(php::reload_php_fpm_master_handler),
        )
        .route(
            "/api/v1/php/masters/{username}/{version}/restart",
            post(php::restart_php_fpm_master_handler),
        )
        // File manager endpoints
        .route("/api/v1/files/list", get(files::list_files_handler))
        .route("/api/v1/files/stat", get(files::stat_file_handler))
        .route("/api/v1/files/mkdir", post(files::mkdir_handler))
        .route("/api/v1/files/rename", post(files::rename_handler))
        .route("/api/v1/files/move", post(files::rename_handler))
        .route("/api/v1/files/copy", post(files::copy_handler))
        .route(
            "/api/v1/files/trash",
            post(files::trash_handler).get(files::list_trash_handler),
        )
        .route(
            "/api/v1/files/trash/restore",
            post(files::restore_trash_handler),
        )
        .route(
            "/api/v1/files/trash/empty",
            axum::routing::delete(files::empty_trash_handler),
        )
        .route(
            "/api/v1/files/permanent",
            axum::routing::delete(files::delete_permanent_handler),
        )
        .route(
            "/api/v1/files/upload/init",
            post(files::upload_init_handler),
        )
        .route(
            "/api/v1/files/upload/{upload_id}",
            get(files::upload_status_handler),
        )
        .route(
            "/api/v1/files/upload/{upload_id}/chunk",
            post(files::upload_chunk_handler),
        )
        .route("/api/v1/files/download", get(files::download_file_handler))
        .route(
            "/api/v1/files/download-zip",
            get(files::download_zip_handler),
        )
        .route(
            "/api/v1/files/archive/compress",
            post(files::archive_compress_handler),
        )
        .route(
            "/api/v1/files/archive/extract",
            post(files::archive_extract_handler),
        )
        .route("/api/v1/files/chmod", post(files::chmod_handler))
        .route(
            "/api/v1/files/chmod-recursive",
            post(files::chmod_recursive_handler),
        )
        .route("/api/v1/files/chown", post(files::chown_handler))
        .route(
            "/api/v1/files/content",
            get(files::get_file_content_handler).put(files::save_file_content_handler),
        )
        .route("/api/v1/files/search", post(files::search_files_handler))
        // Web Terminal & PTY Streaming (Phase 6)
        .route(
            "/api/v1/terminal/sessions",
            post(terminal::create_session_handler).get(terminal::list_sessions_handler),
        )
        .route(
            "/api/v1/terminal/sessions/{id}",
            get(terminal::get_session_handler).delete(terminal::close_session_handler),
        )
        .route(
            "/api/v1/terminal/sessions/{id}/resize",
            post(terminal::resize_session_handler),
        )
        .route(
            "/api/v1/terminal/audit",
            get(terminal::get_terminal_audit_handler),
        )
        .route("/ws/terminal/{id}", get(terminal::terminal_ws_handler))
        // SSH Key Management (Phase 6 - P6-T5)
        .route(
            "/api/v1/ssh-keys",
            get(ssh::list_ssh_keys_handler).post(ssh::create_ssh_key_handler),
        )
        .route(
            "/api/v1/ssh-keys/{id}",
            axum::routing::delete(ssh::delete_ssh_key_handler),
        )
        .route(
            "/api/v1/ssh-keys/validate",
            post(ssh::validate_ssh_key_handler),
        )
        .route(
            "/api/v1/ssh/dropin/apply",
            post(ssh::apply_ssh_dropin_handler),
        )
        // SSH & SFTP Connection Command Info (Phase 6 - P6-T7)
        .route(
            "/api/v1/ssh/connection-info",
            get(ssh::get_ssh_connection_info_handler),
        )
        // Node.js Runtime management (Phase 7 - P7-T1)
        .route(
            "/api/v1/runtimes/node/versions",
            get(node::get_node_versions_handler),
        )
        .route(
            "/api/v1/runtimes/node/install",
            post(node::install_node_version_handler),
        )
        .route(
            "/api/v1/runtimes/node/{version}",
            axum::routing::delete(node::remove_node_version_handler),
        )
        .route(
            "/api/v1/runtimes/node/{version}/corepack",
            post(node::enable_node_corepack_handler),
        )
        // Python runtime and site management (Phase 7 - P7-T3)
        .route(
            "/api/v1/runtimes/python",
            get(python_site::list_python_versions_handler)
                .post(python_site::install_python_version_handler),
        )
        .route(
            "/api/v1/runtimes/python/versions",
            get(python_site::list_python_versions_handler),
        )
        .route(
            "/api/v1/runtimes/python/install",
            post(python_site::install_python_version_handler),
        )
        .route(
            "/api/v1/runtimes/python/{version}",
            axum::routing::delete(python_site::remove_python_version_handler),
        )
        .route(
            "/api/v1/sites/{id}/python/status",
            get(python_site::get_python_site_status_handler),
        )
        .route(
            "/api/v1/sites/{id}/python/action",
            post(python_site::action_python_site_handler),
        )
        .route(
            "/api/v1/sites/{id}/python/job",
            post(python_site::run_python_site_job_handler),
        )
        .route(
            "/api/v1/sites/{id}/python/settings",
            get(python_site::get_python_site_settings_handler)
                .put(python_site::update_python_site_settings_handler),
        )
        // Reverse proxy site management (Phase 7 - P7-T4)
        .route(
            "/api/v1/sites/{id}/proxy/settings",
            get(proxy_site::get_proxy_site_settings_handler)
                .put(proxy_site::update_proxy_site_settings_handler),
        )
        .route(
            "/api/v1/sites/{id}/proxy/probe",
            get(proxy_site::probe_proxy_upstream_handler),
        )
        // Unified application process management (Phase 7 - P7-T5)
        .route("/api/v1/processes", get(processes::list_processes_handler))
        // Database management (Phase 8 - P8-T5)
        .nest("/api/v1/databases", databases::router())
        // Site backups & snapshot restore (Phase 8 - P8-T6)
        .merge(backups::router())
        // System notifications (Phase 8 - P8-T7)
        .merge(notifications::router())
        // Analytics & Metrics (Phase 9 - P9-T5)
        .route(
            "/api/v1/sites/{id}/analytics/overview",
            get(analytics::get_site_analytics_overview_handler),
        )
        .route(
            "/api/v1/sites/{id}/analytics/timeseries",
            get(analytics::get_site_analytics_timeseries_handler),
        )
        .route(
            "/api/v1/sites/{id}/analytics/breakdowns",
            get(analytics::get_site_analytics_breakdowns_handler),
        )
        .route(
            "/api/v1/sites/{id}/analytics/compare",
            get(analytics::get_site_analytics_compare_handler),
        )
        .route(
            "/api/v1/sites/{id}/analytics/export",
            get(analytics::export_site_analytics_csv_handler),
        )
        .route(
            "/api/v1/sites/{id}/analytics/app",
            get(analytics::get_site_app_metrics_handler),
        )
        .route(
            "/api/v1/sites/{id}/analytics/slowlog",
            get(analytics::get_site_slowlog_handler),
        )
        .route(
            "/api/v1/sites/{id}/analytics/insights",
            get(analytics::get_site_insights_handler),
        )
        .route(
            "/api/v1/sites/{id}/analytics/insights/evaluate",
            post(analytics::evaluate_site_insights_handler),
        )
        .route(
            "/api/v1/system/analytics/timeseries",
            get(analytics::get_system_analytics_timeseries_handler),
        )
        .route(
            "/api/v1/system/analytics/insights",
            get(analytics::get_system_insights_handler),
        )
        // Security Analytics & IP Banning (Phase 9 - P9-T8)
        .route(
            "/api/v1/sites/{id}/analytics/security/overview",
            get(security_analytics::get_site_security_overview_handler),
        )
        .route(
            "/api/v1/sites/{id}/security/banned-ips",
            get(security_analytics::get_site_banned_ips_handler),
        )
        .route(
            "/api/v1/sites/{id}/security/ban-ip",
            post(security_analytics::ban_ip_handler),
        )
        .route(
            "/api/v1/sites/{id}/security/ban-ip/{ip}",
            axum::routing::delete(security_analytics::unban_ip_handler),
        )
        // Analytics Data Lifecycle & Retention (Phase 9 - P9-T9)
        .route(
            "/api/v1/system/analytics/retention",
            get(retention::get_analytics_retention_handler)
                .put(retention::update_analytics_retention_handler),
        )
        .route(
            "/api/v1/system/analytics/maintenance",
            post(retention::trigger_analytics_maintenance_handler),
        )
        .route(
            "/api/v1/system/analytics/integrity-check",
            post(retention::check_analytics_integrity_handler),
        )
        // Server Optimizer endpoints (Phase 10 - P10-T2)
        .route(
            "/api/v1/system/optimizer",
            get(optimizer::get_optimizer_status_handler),
        )
        .route(
            "/api/v1/system/optimizer/apply",
            post(optimizer::apply_optimizer_handler),
        )
        .route(
            "/api/v1/system/optimizer/rollback",
            post(optimizer::rollback_optimizer_handler),
        )
        // Hardening OWASP ASVS L2 & nftables firewall (Phase 10 - P10-T4)
        .route(
            "/api/v1/system/security-hardening",
            get(security_hardening::get_security_hardening_overview_handler),
        )
        .route(
            "/api/v1/system/security-hardening/nftables/sync",
            post(security_hardening::sync_nftables_loopback_handler),
        )
        // System Reliability & Diagnostics (Phase 10 - P10-T5)
        .route(
            "/api/v1/system/doctor",
            get(system_reliability::get_system_doctor_handler),
        )
        .route(
            "/api/v1/system/doctor/fix",
            post(system_reliability::fix_system_doctor_handler),
        )
        .route(
            "/api/v1/system/reconcile",
            get(system_reliability::get_system_reconcile_handler)
                .post(system_reliability::apply_system_reconcile_handler),
        )
        .route(
            "/api/v1/system/backups",
            get(system_reliability::list_system_backups_handler)
                .post(system_reliability::create_system_backup_handler),
        )
        .route(
            "/api/v1/system/backups/restore",
            post(system_reliability::restore_system_backup_handler),
        )
        .merge(SwaggerUi::new("/swagger-ui").url("/api/v1/openapi.json", api_doc))
        .fallback(static_files::static_handler)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            public_url::middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(maintenance::middleware))
        .layer(axum::middleware::from_fn(
            security_headers::security_headers_middleware,
        ))
        .with_state(state);

    // Fallback to static files from web/dist for all non-API paths
    api_routes
}
