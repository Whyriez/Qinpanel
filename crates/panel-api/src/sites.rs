//! Static website and domain alias/redirect management API handlers and services.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use panel_core::domain::DomainName;
use panel_db::models::{
    CreateDomain, CreateSite, Domain, RedirectMode, Site, UpdateDomain, UpdateSite, User,
};
use panel_db::repositories::{
    SqliteBackupRepository, SqliteCertificateRepository, SqliteDatabaseRepository,
    SqliteDomainRepository, SqliteGitDeployRepository, SqlitePlanRepository, SqliteSiteRepository,
    SqliteUserRepository,
};
use panel_proto::{SiteApplyRequest, SiteProvisionRequest, SiteRemoveRequest};
use panel_webserver::template::{
    DomainRedirectConfig, FastCgiCacheConfig, HeaderConfig, NginxTemplateRenderer,
    NodejsVhostContext, PhpPreset, PhpVhostContext, PythonVhostContext, ReverseProxyVhostContext,
    StaticVhostContext, render_placeholder_html,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::middleware::AuthenticatedUser;
use crate::routes::AppState;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, Default)]
pub struct CreateStaticSiteRequest {
    pub domain: String,
    pub user_id: Option<i64>,
    pub spa_fallback: Option<bool>,
    pub force_https: Option<bool>,
    pub docroot: Option<String>,
    pub kind: Option<String>,
    pub php_version: Option<String>,
    pub preset: Option<String>,
    pub custom_entry_file: Option<String>,
    pub upload_max_size: Option<String>,
    pub fastcgi_cache: Option<bool>,
    pub node_version: Option<String>,
    pub port: Option<u16>,
    pub app_dir: Option<String>,
    pub entrypoint_type: Option<String>,
    pub entrypoint_value: Option<String>,
    pub auto_restart: Option<bool>,
    pub env_vars: Option<std::collections::BTreeMap<String, String>>,
    pub python_version: Option<String>,
    pub framework: Option<String>,
    pub server_type: Option<String>,
    pub workers: Option<u32>,
    pub venv_dir: Option<String>,
    pub static_url: Option<String>,
    pub static_dir: Option<String>,
    pub media_url: Option<String>,
    pub media_dir: Option<String>,
    pub upstream_url: Option<String>,
    pub preserve_host: Option<bool>,
    pub websocket_support: Option<bool>,
    pub proxy_connect_timeout: Option<String>,
    pub proxy_send_timeout: Option<String>,
    pub proxy_read_timeout: Option<String>,
    pub proxy_buffering: Option<bool>,
    pub custom_request_headers: Option<Vec<crate::proxy_site::ProxyHeaderConfig>>,
    pub custom_response_headers: Option<Vec<crate::proxy_site::ProxyHeaderConfig>>,
}

pub type CreateSiteRequest = CreateStaticSiteRequest;

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListSitesQuery {
    pub user_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct AddSiteDomainRequest {
    pub domain: String,
    pub redirect_mode: Option<String>,
    pub redirect_target: Option<String>,
    pub is_primary: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct UpdateSiteDomainRequest {
    pub redirect_mode: Option<String>,
    pub redirect_target: Option<String>,
    pub is_primary: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SetForceHttpsRequest {
    pub force_https: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SiteDomainResponse {
    pub id: i64,
    pub site_id: i64,
    pub name: String,
    pub unicode_name: String,
    pub is_primary: bool,
    pub redirect_mode: String,
    pub redirect_target: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SiteResponse {
    pub id: i64,
    pub user_id: i64,
    pub kind: String,
    pub primary_domain: String,
    pub docroot: String,
    pub status: String,
    pub spa_fallback: bool,
    pub force_https: bool,
    pub domains: Vec<SiteDomainResponse>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub preset: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SiteMessageResponse {
    pub message: String,
    pub site_id: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteLiveStatus {
    pub webserver_status: String,
    pub runtime_status: String,
    pub healthy: bool,
    pub uptime_secs: Option<u64>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteResourceMetrics {
    pub cpu_usage_usec: u64,
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    pub memory_limit_bytes: u64,
    pub pids_current: u64,
    pub pids_limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteSslSummary {
    pub has_certificate: bool,
    pub issuer: Option<String>,
    pub valid_until: Option<i64>,
    pub days_remaining: Option<i64>,
    pub auto_renew: bool,
    pub status: String,
    pub force_https: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteRuntimeSummary {
    pub kind: String,
    pub version: String,
    pub preset: Option<String>,
    pub entrypoint_or_target: String,
    pub framework: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteStorageBreakdown {
    pub docroot_bytes: u64,
    pub databases_bytes: u64,
    pub backups_bytes: u64,
    pub total_site_bytes: u64,
    pub plan_disk_limit_bytes: u64,
    pub usage_percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteContextualWarning {
    pub id: String,
    pub level: String,
    pub title: String,
    pub message: String,
    pub action_tab: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LatestDeploymentSummary {
    pub deployment_id: i64,
    pub release_id: String,
    pub commit_hash: String,
    pub commit_message: String,
    pub commit_author: String,
    pub branch: String,
    pub trigger_type: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub can_rollback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LatestBackupSummary {
    pub backup_id: i64,
    pub filename: String,
    pub size_bytes: i64,
    pub status: String,
    pub includes_files: bool,
    pub includes_databases: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteOverviewResponse {
    pub site_id: i64,
    pub domain: String,
    pub status: SiteLiveStatus,
    pub quick_actions_available: Vec<String>,
    pub resources: SiteResourceMetrics,
    pub ssl: SiteSslSummary,
    pub runtime: SiteRuntimeSummary,
    pub storage: SiteStorageBreakdown,
    pub warnings: Vec<SiteContextualWarning>,
    pub latest_deployment: Option<LatestDeploymentSummary>,
    pub latest_backup: Option<LatestBackupSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SiteQuickActionRequest {
    pub action: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SiteQuickActionResponse {
    pub success: bool,
    pub action: String,
    pub message: String,
}

fn domain_to_response(d: &Domain) -> SiteDomainResponse {
    SiteDomainResponse {
        id: d.id,
        site_id: d.site_id,
        name: d.name.clone(),
        unicode_name: d.unicode_name.clone(),
        is_primary: d.is_primary,
        redirect_mode: d.redirect_mode.clone(),
        redirect_target: d.redirect_target.clone(),
        status: d.status.clone(),
        created_at: d.created_at,
        updated_at: d.updated_at,
    }
}

fn build_site_response(s: &Site, domains: &[Domain]) -> SiteResponse {
    let primary_name = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let settings: serde_json::Value =
        serde_json::from_str(&s.settings_json).unwrap_or(serde_json::json!({}));
    let spa_fallback = settings
        .get("spa_fallback")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let force_https = settings
        .get("force_https")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let preset = settings
        .get("preset")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let domain_dtos = domains.iter().map(domain_to_response).collect();

    SiteResponse {
        id: s.id,
        user_id: s.user_id,
        kind: s.kind.clone(),
        primary_domain: primary_name,
        docroot: s.docroot.clone(),
        status: s.status.clone(),
        spa_fallback,
        force_https,
        domains: domain_dtos,
        runtime: s.runtime.clone(),
        version: s.version.clone(),
        preset,
        created_at: s.created_at,
        updated_at: s.updated_at,
    }
}

pub async fn fetch_and_authorize_site(
    site_repo: &SqliteSiteRepository,
    auth_user: &AuthenticatedUser,
    site_id: i64,
) -> Result<Site, (StatusCode, &'static str)> {
    let site = site_repo
        .find_by_id(site_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::NOT_FOUND, "Site not found"))?;

    if !auth_user.can_access_user(site.user_id) {
        return Err((StatusCode::FORBIDDEN, "Access denied"));
    }

    Ok(site)
}

async fn resolve_target_user(
    state: &AppState,
    auth_user: &AuthenticatedUser,
    requested_user_id: Option<i64>,
) -> Result<User, (StatusCode, String)> {
    let target = requested_user_id.map(|id| id.to_string());
    crate::tenant_context::resolve_tenant_context(state, auth_user, target.as_deref(), None)
        .await
        .map(|c| c.user)
        .map_err(|(status, body)| {
            (
                status,
                body.0["error"]
                    .as_str()
                    .unwrap_or("Tenant context invalid")
                    .to_owned(),
            )
        })
}

fn build_domain_redirects_and_aliases(
    domains: &[Domain],
    primary_domain: &str,
    has_ssl: bool,
    force_https: bool,
) -> (Vec<String>, Vec<DomainRedirectConfig>) {
    let mut alias_domains = Vec::new();
    let mut redirects = Vec::new();

    for d in domains {
        if d.is_primary {
            continue;
        }
        match d.redirect_mode.as_str() {
            "none" => {
                alias_domains.push(d.name.clone());
            }
            "to_primary" => {
                let target = if has_ssl || force_https {
                    format!("https://{primary_domain}$request_uri")
                } else {
                    format!("$scheme://{primary_domain}$request_uri")
                };
                redirects.push(DomainRedirectConfig {
                    domain: d.name.clone(),
                    target,
                    status_code: 301,
                });
            }
            "to_www" => {
                let target_host = if d.name.starts_with("www.") {
                    d.name.clone()
                } else {
                    format!("www.{}", d.name)
                };
                let target = if has_ssl || force_https {
                    format!("https://{target_host}$request_uri")
                } else {
                    format!("$scheme://{target_host}$request_uri")
                };
                redirects.push(DomainRedirectConfig {
                    domain: d.name.clone(),
                    target,
                    status_code: 301,
                });
            }
            "to_non_www" => {
                let target_host = if let Some(non_www) = d.name.strip_prefix("www.") {
                    non_www.to_string()
                } else {
                    d.name.clone()
                };
                let target = if has_ssl || force_https {
                    format!("https://{target_host}$request_uri")
                } else {
                    format!("$scheme://{target_host}$request_uri")
                };
                redirects.push(DomainRedirectConfig {
                    domain: d.name.clone(),
                    target,
                    status_code: 301,
                });
            }
            "custom" => {
                if let Some(ref cust) = d.redirect_target {
                    redirects.push(DomainRedirectConfig {
                        domain: d.name.clone(),
                        target: cust.clone(),
                        status_code: 301,
                    });
                }
            }
            _ => {}
        }
    }

    (alias_domains, redirects)
}

struct PythonVhostRenderArgs<'a> {
    site: &'a Site,
    primary_domain: &'a str,
    linux_uname: &'a str,
    settings: &'a serde_json::Value,
    alias_domains: Vec<String>,
    redirects: Vec<DomainRedirectConfig>,
    has_ssl: bool,
    ssl_cert: Option<String>,
    ssl_key: Option<String>,
    force_https: bool,
    site_rules_block: Option<String>,
}

fn render_python_vhost_for_site(
    renderer: &NginxTemplateRenderer,
    args: PythonVhostRenderArgs<'_>,
) -> Result<String, (StatusCode, String)> {
    let socket_path = format!("/run/panel/python/site_{}/app.sock", args.site.id);

    let static_url = args
        .settings
        .get("static_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let static_dir = args
        .settings
        .get("static_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let media_url = args
        .settings
        .get("media_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let media_dir = args
        .settings
        .get("media_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    for directory in [static_dir.as_deref(), media_dir.as_deref()]
        .into_iter()
        .flatten()
    {
        require_nginx_tenant_directory(directory, args.linux_uname)?;
    }

    let python_ctx = PythonVhostContext {
        site_id: format!("site_{}", args.site.id),
        primary_domain: args.primary_domain.to_string(),
        alias_domains: args.alias_domains,
        redirects: args.redirects,
        unix_socket: socket_path,
        log_dir: format!(
            "/var/log/panel/{}/{}",
            args.linux_uname, args.primary_domain
        ),
        snippets_dir: "/etc/panel/nginx/snippets".to_string(),
        has_ssl: args.has_ssl,
        ssl_cert: args.ssl_cert,
        ssl_key: args.ssl_key,
        force_https: args.force_https,
        ocsp_stapling: args
            .settings
            .get("ocsp_stapling")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        custom_headers: vec![],
        static_url,
        static_dir,
        media_url,
        media_dir,
        upload_max_size: args
            .settings
            .get("upload_max_size")
            .or_else(|| args.settings.get("client_max_body_size"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        proxy_connect_timeout: args
            .settings
            .get("proxy_connect_timeout")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        proxy_send_timeout: None,
        proxy_read_timeout: args
            .settings
            .get("proxy_read_timeout")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        proxy_buffering: None,
        site_rules_block: args.site_rules_block,
    };

    renderer.render_python(&python_ctx).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Python vhost render failed: {e}"),
        )
    })
}

/// Shared Nginx workers must never read another tenant directory.
fn require_nginx_tenant_directory(path: &str, username: &str) -> Result<(), (StatusCode, String)> {
    if !path.starts_with(&format!("/home/{username}/"))
        || path.split('/').any(|part| matches!(part, "." | ".."))
        || path
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\\' | '$' | '%'))
    {
        return Err((StatusCode::FORBIDDEN, "Direktori Nginx harus berada di home tenant pemilik tanpa traversal atau ekspansi variabel".into()));
    }
    Ok(())
}

/// Render only: certificate switches must not restart application runtimes.
pub(crate) async fn render_site_vhost(
    pool: &panel_db::pool::DbPool,
    site: &Site,
    user: &User,
    domains: &[Domain],
) -> Result<String, (StatusCode, String)> {
    for domain in domains {
        crate::domain_policy::require_tenant_domain(pool, &domain.name, None).await?;
    }
    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.as_str())
        .unwrap_or("unknown");

    let settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
    let spa_fallback = settings
        .get("spa_fallback")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let force_https = settings
        .get("force_https")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_ssl = settings
        .get("has_ssl")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let ssl_cert = settings
        .get("ssl_cert")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let ssl_key = settings
        .get("ssl_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let (alias_domains, redirects) =
        build_domain_redirects_and_aliases(domains, primary_domain, has_ssl, force_https);

    let (resolved_linux_name, _resolved_linux_uid) =
        crate::tenant_context::execution_identity(user)?;
    let linux_uname = resolved_linux_name.as_str();
    require_nginx_tenant_directory(&site.docroot, linux_uname)?;

    let renderer = NginxTemplateRenderer::new();
    let is_php = site.kind == "php" || site.runtime.as_deref() == Some("php");
    let is_nodejs = site.kind == "nodejs" || site.runtime.as_deref() == Some("nodejs");
    let is_python = site.kind == "python" || site.runtime.as_deref() == Some("python");
    let is_proxy = site.kind == "proxy" || site.runtime.as_deref() == Some("proxy");

    let site_rules_block = if let Some(rules_val) = settings.get("site_rules") {
        if let Ok(rules) =
            serde_json::from_value::<panel_core::site_rules::SiteRulesConfig>(rules_val.clone())
        {
            let htpasswd_path = format!("/etc/panel/nginx/auth/site_{}.htpasswd", site.id);
            let block = panel_webserver::rules::generate_site_rules_nginx_block(
                &rules,
                &format!("site_{}", site.id),
                Some(&htpasswd_path),
            );
            if block.trim().is_empty() {
                None
            } else {
                Some(block)
            }
        } else {
            None
        }
    } else {
        None
    };

    let rendered = if is_php {
        let preset_str = settings
            .get("preset")
            .and_then(|v| v.as_str())
            .unwrap_or("generic");
        let preset = PhpPreset::parse(preset_str).unwrap_or(PhpPreset::Generic);
        let custom_entry_file = settings
            .get("custom_entry_file")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let upload_max_size = settings
            .get("upload_max_size")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let php_ver = site.version.clone().unwrap_or_else(|| "8.3".to_string());
        let fpm_socket = format!("unix:/run/panel/php/{linux_uname}-{php_ver}/fpm.sock");

        let fastcgi_cache = settings.get("fastcgi_cache").and_then(|v| {
            if let Some(b) = v.as_bool() {
                if b {
                    Some(FastCgiCacheConfig::enabled_default())
                } else {
                    None
                }
            } else if let Ok(cfg) = serde_json::from_value::<FastCgiCacheConfig>(v.clone()) {
                if cfg.enabled { Some(cfg) } else { None }
            } else {
                None
            }
        });

        let php_vhost_ctx = PhpVhostContext {
            site_id: format!("site_{}", site.id),
            primary_domain: primary_domain.to_string(),
            alias_domains,
            redirects,
            docroot: site.docroot.clone(),
            log_dir: format!("/var/log/panel/{linux_uname}/{primary_domain}"),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            index_files: Some("index.php index.html index.htm".to_string()),
            has_ssl,
            ssl_cert,
            ssl_key,
            force_https,
            ocsp_stapling: settings
                .get("ocsp_stapling")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            custom_headers: vec![],
            php_version: php_ver,
            fpm_socket,
            preset,
            custom_entry_file,
            upload_max_size,
            fastcgi_cache,
            site_rules_block: site_rules_block.clone(),
        };

        renderer.render_php(&php_vhost_ctx).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("PHP vhost render failed: {e}"),
            )
        })?
    } else if is_nodejs {
        let port = settings["port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Port runtime tidak valid".into()))?;
        crate::node_site::validate_runtime_port(pool, Some(site.id), port).await?;
        let port = settings
            .get("port")
            .and_then(|v| v.as_u64())
            .map(|p| p as u16)
            .unwrap_or(20000);
        let upstream_url = format!("http://127.0.0.1:{port}");

        let nodejs_ctx = NodejsVhostContext {
            site_id: format!("site_{}", site.id),
            primary_domain: primary_domain.to_string(),
            alias_domains,
            redirects,
            docroot: Some(site.docroot.clone()),
            log_dir: format!("/var/log/panel/{linux_uname}/{primary_domain}"),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            has_ssl,
            ssl_cert,
            ssl_key,
            force_https,
            ocsp_stapling: settings
                .get("ocsp_stapling")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            custom_headers: vec![],
            upstream_url,
            upload_max_size: settings
                .get("upload_max_size")
                .or_else(|| settings.get("client_max_body_size"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            static_assets_enabled: false,
            proxy_connect_timeout: settings
                .get("proxy_connect_timeout")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            proxy_send_timeout: None,
            proxy_read_timeout: settings
                .get("proxy_read_timeout")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            proxy_buffering: None,
            site_rules_block: site_rules_block.clone(),
        };

        renderer.render_nodejs(&nodejs_ctx).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Node.js vhost render failed: {e}"),
            )
        })?
    } else if is_python {
        render_python_vhost_for_site(
            &renderer,
            PythonVhostRenderArgs {
                site,
                primary_domain,
                linux_uname,
                settings: &settings,
                alias_domains,
                redirects,
                has_ssl,
                ssl_cert,
                ssl_key,
                force_https,
                site_rules_block: site_rules_block.clone(),
            },
        )?
    } else if is_proxy {
        let upstream_url = settings
            .get("upstream_url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://127.0.0.1:8080")
            .to_string();
        let admitted = crate::proxy_policy::admit(pool, site.user_id, &upstream_url).await?;
        let upstream_url = admitted.url;
        let preserve_host = settings
            .get("preserve_host")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let websocket_support = settings
            .get("websocket_support")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let proxy_connect_timeout = settings
            .get("proxy_connect_timeout")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let proxy_send_timeout = settings
            .get("proxy_send_timeout")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let proxy_read_timeout = settings
            .get("proxy_read_timeout")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let proxy_buffering = settings.get("proxy_buffering").and_then(|v| v.as_bool());
        let proxy_buffering_str = proxy_buffering.map(|b| {
            if b {
                "on".to_string()
            } else {
                "off".to_string()
            }
        });
        let custom_request_headers: Vec<HeaderConfig> = settings
            .get("custom_request_headers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let custom_headers: Vec<HeaderConfig> = settings
            .get("custom_response_headers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let proxy_ssl_server_name = upstream_url.starts_with("https://");

        let proxy_ctx = ReverseProxyVhostContext {
            site_id: format!("site_{}", site.id),
            primary_domain: primary_domain.to_string(),
            alias_domains,
            redirects,
            log_dir: format!("/var/log/panel/{linux_uname}/{primary_domain}"),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            has_ssl,
            ssl_cert,
            ssl_key,
            force_https,
            ocsp_stapling: settings
                .get("ocsp_stapling")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            upstream_url,
            preserve_host,
            custom_host_header: admitted.authority,
            proxy_ssl_name: admitted.tls_name,
            websocket_support,
            upload_max_size: None,
            proxy_connect_timeout,
            proxy_send_timeout,
            proxy_read_timeout,
            proxy_buffering: proxy_buffering_str,
            custom_headers,
            custom_request_headers,
            proxy_ssl_server_name,
            allow_external_upstream: true,
            site_rules_block: site_rules_block.clone(),
        };

        renderer.render_proxy(&proxy_ctx).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Proxy vhost render failed: {e}"),
            )
        })?
    } else {
        let vhost_ctx = StaticVhostContext {
            site_id: format!("site_{}", site.id),
            primary_domain: primary_domain.to_string(),
            alias_domains,
            redirects,
            docroot: site.docroot.clone(),
            log_dir: format!("/var/log/panel/{linux_uname}/{primary_domain}"),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            index_files: Some("index.html index.htm".to_string()),
            spa_fallback,
            has_ssl,
            ssl_cert,
            ssl_key,
            force_https,
            ocsp_stapling: settings
                .get("ocsp_stapling")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            custom_headers: vec![],
            site_rules_block: site_rules_block.clone(),
        };

        renderer.render_static(&vhost_ctx).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Vhost render failed: {e}"),
            )
        })?
    };

    Ok(rendered)
}

/// Recompiles the Nginx virtual host configuration for a site from its current DB state
/// and applies it atomically with automatic rollback via the privileged agent.
pub async fn recompile_and_apply_site_vhost(
    state: &AppState,
    site: &Site,
    user: &User,
    domains: &[Domain],
) -> Result<(), (StatusCode, String)> {
    let rendered = render_site_vhost(&state.pool, site, user, domains).await?;
    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.as_str())
        .unwrap_or("unknown");
    let settings: serde_json::Value = serde_json::from_str(&site.settings_json).unwrap_or_default();
    let (resolved_linux_name, resolved_linux_uid) =
        crate::tenant_context::execution_identity(user)?;
    let linux_uname = resolved_linux_name.as_str();
    let is_nodejs = site.kind == "nodejs" || site.runtime.as_deref() == Some("nodejs");
    let is_python = site.kind == "python" || site.runtime.as_deref() == Some("python");
    let enable = site.status == "active";
    // If Node.js site is active, also ensure systemd unit and env file are applied on host
    if is_nodejs && site.status == "active" {
        let node_ver = site
            .version
            .clone()
            .or_else(|| {
                settings
                    .get("node_version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "22.14.0".to_string());
        let port = settings
            .get("port")
            .and_then(|v| v.as_u64())
            .map(|p| p as u16)
            .unwrap_or(20000);
        let entrypoint = match settings.get("entrypoint_type").and_then(|v| v.as_str()) {
            Some("npm_start") => panel_proto::NodeSiteEntrypoint::NpmStart,
            Some("npm_custom") => {
                let subcmd = settings
                    .get("entrypoint_value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("start")
                    .to_string();
                panel_proto::NodeSiteEntrypoint::NpmCustom(subcmd)
            }
            _ => {
                let script = settings
                    .get("entrypoint_value")
                    .or_else(|| settings.get("entrypoint"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("server.js")
                    .to_string();
                panel_proto::NodeSiteEntrypoint::NodeScript(script)
            }
        };

        let mut env_vars = std::collections::BTreeMap::new();
        if let Some(env_obj) = settings.get("env_vars").and_then(|v| v.as_object()) {
            for (k, v) in env_obj {
                if let Some(val_str) = v.as_str() {
                    env_vars.insert(k.clone(), val_str.to_string());
                }
            }
        }

        let auto_restart = settings
            .get("auto_restart")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let app_dir = settings
            .get("app_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(&site.docroot)
            .to_string();

        let node_apply_req = panel_proto::NodeSiteApplyRequest {
            site_id: format!("site_{}", site.id),
            username: linux_uname.to_string(),
            uid: resolved_linux_uid,
            node_version: node_ver,
            app_dir,
            port,
            entrypoint,
            env_vars,
            auto_restart,
            env_file_path: None,
            unit_path: None,
            base_runtime_dir: None,
        };

        state
            .agent_client
            .apply_node_site(node_apply_req)
            .await
            .map_err(|error| {
                site_agent_error_details(
                    error,
                    "Runtime Node.js gagal diterapkan",
                    "perubahan situs dibatalkan",
                )
            })?;
        wait_runtime_ready(state, site, "nodejs", port, None).await?;
    }

    // If Python site is active, also ensure systemd unit and env file are applied on host
    if is_python && site.status == "active" {
        let py_ver = site
            .version
            .clone()
            .or_else(|| {
                settings
                    .get("python_version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "3.12.9".to_string());

        let app_dir = settings
            .get("app_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(&site.docroot)
            .to_string();

        let venv_dir = settings
            .get("venv_dir")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}/.venv", app_dir));

        let socket_path = settings
            .get("socket_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("/run/panel/python/site_{}/app.sock", site.id));

        let fw_str = settings
            .get("framework_preset")
            .or_else(|| settings.get("framework"))
            .and_then(|v| v.as_str())
            .unwrap_or("generic");
        let framework = match fw_str {
            "django" => panel_proto::PythonSiteFramework::Django,
            "flask" => panel_proto::PythonSiteFramework::Flask,
            "fastapi" => panel_proto::PythonSiteFramework::FastApi,
            _ => panel_proto::PythonSiteFramework::Generic,
        };

        let srv_str = settings
            .get("server_type")
            .and_then(|v| v.as_str())
            .unwrap_or("gunicorn_wsgi");
        let server_type = match srv_str {
            "uvicorn_asgi" => panel_proto::PythonSiteServer::UvicornAsgi,
            "gunicorn_uvicorn_worker" => panel_proto::PythonSiteServer::GunicornUvicornWorker,
            _ => panel_proto::PythonSiteServer::GunicornWsgi,
        };

        let ep_type = settings
            .get("entrypoint_type")
            .and_then(|v| v.as_str())
            .unwrap_or("wsgi");
        let ep_val = settings
            .get("entrypoint_value")
            .or_else(|| settings.get("entrypoint"))
            .and_then(|v| v.as_str())
            .unwrap_or("app:app")
            .to_string();

        let entrypoint = match ep_type {
            "asgi" => panel_proto::PythonSiteEntrypoint::Asgi(ep_val),
            "custom" => panel_proto::PythonSiteEntrypoint::CustomCommand(ep_val),
            _ => panel_proto::PythonSiteEntrypoint::Wsgi(ep_val),
        };

        let workers = settings
            .get("workers")
            .and_then(|v| v.as_u64())
            .map(|w| w as u32)
            .unwrap_or(3);

        let mut env_vars = std::collections::BTreeMap::new();
        if let Some(env_obj) = settings.get("env_vars").and_then(|v| v.as_object()) {
            for (k, v) in env_obj {
                if let Some(val_str) = v.as_str() {
                    env_vars.insert(k.clone(), val_str.to_string());
                }
            }
        }

        let auto_restart = settings
            .get("auto_restart")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let py_apply_req = panel_proto::PythonSiteApplyRequest {
            site_id: format!("site_{}", site.id),
            username: linux_uname.to_string(),
            uid: resolved_linux_uid,
            python_version: py_ver,
            app_dir,
            venv_dir,
            socket_path: socket_path.clone(),
            framework,
            server_type,
            entrypoint,
            workers,
            env_vars,
            auto_restart,
            env_file_path: None,
            unit_path: None,
            base_runtime_dir: None,
        };

        state
            .agent_client
            .apply_python_site(py_apply_req)
            .await
            .map_err(|error| {
                site_agent_error_details(
                    error,
                    "Runtime Python gagal diterapkan",
                    "perubahan situs dibatalkan",
                )
            })?;
        wait_runtime_ready(state, site, "python", 0, Some(socket_path)).await?;
    }

    let apply_req = SiteApplyRequest {
        site_id: format!("site_{}", site.id),
        config_content: rendered,
        enable,
        nginx_base_dir: None,
        log_context: Some(panel_proto::SiteLogContext {
            username: linux_uname.to_string(),
            domain: primary_domain.to_string(),
        }),
        test_config: true,
        reload: true,
    };

    state
        .agent_client
        .apply_site(apply_req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Host webserver apply failed: {e}"),
            )
        })?;

    Ok(())
}

async fn wait_runtime_ready(
    state: &AppState,
    site: &Site,
    runtime: &str,
    port: u16,
    socket: Option<String>,
) -> Result<(), (StatusCode, String)> {
    let runtime_name = if runtime == "nodejs" {
        "Node.js"
    } else {
        "Python"
    };
    let mut diagnostic = "Proses belum siap".to_string();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    for attempt in 0..50 {
        let result = if runtime == "nodejs" {
            state
                .agent_client
                .status_node_site(panel_proto::NodeSiteStatusRequest {
                    site_id: format!("site_{}", site.id),
                    port: Some(port),
                    probe_path: Some("/".into()),
                })
                .await
                .map(|status| (status.active && status.healthy, status.probe_message))
        } else {
            state
                .agent_client
                .status_python_site(panel_proto::PythonSiteStatusRequest {
                    site_id: format!("site_{}", site.id),
                    socket_path: socket.clone(),
                })
                .await
                .map(|status| {
                    (
                        status.active && status.healthy && status.socket_exists,
                        status.probe_message,
                    )
                })
        };
        match result {
            Ok((true, _)) => return Ok(()),
            Ok((false, message)) => {
                diagnostic = message.unwrap_or_else(|| "Service atau endpoint belum siap".into())
            }
            Err(error) => {
                return Err(site_agent_error_details(
                    error,
                    &format!("Health check {runtime_name} gagal"),
                    "runtime situs belum siap",
                ));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        if attempt < 49 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    let diagnostic = diagnostic.chars().take(2048).collect::<String>();
    Err((
        StatusCode::UNPROCESSABLE_ENTITY,
        format!(
            "Aplikasi {runtime_name} belum siap: {diagnostic}. Periksa entrypoint dan log proses."
        ),
    ))
}

/// Helper to recompile and apply a site configuration by fetching user and domains from DB.
pub async fn apply_site_config(state: &AppState, site: &Site) -> Result<(), (StatusCode, String)> {
    let domain_repo =
        panel_db::repositories::domain_repo::SqliteDomainRepository::new(state.pool.clone());
    let user_repo =
        panel_db::repositories::user_repo::SqliteUserRepository::new(state.pool.clone());

    let domains = domain_repo
        .find_by_site_id(site.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Pemilik situs tidak ditemukan".to_string(),
            )
        })?;

    recompile_and_apply_site_vhost(state, site, &user, &domains).await
}

fn log_site_cleanup_error(site_id: i64, operation: &str, error: impl std::fmt::Display) {
    tracing::error!(site_id, operation, error=%error, "Provisioning rollback failed");
}

async fn remove_site_process(
    state: &AppState,
    site: &Site,
) -> Result<(), crate::agent_client::AgentClientError> {
    if site.kind == "nodejs" || site.runtime.as_deref() == Some("nodejs") {
        state
            .agent_client
            .remove_node_site(panel_proto::NodeSiteRemoveRequest {
                site_id: format!("site_{}", site.id),
                unit_path: None,
                env_file_path: None,
            })
            .await?;
    } else if site.kind == "python" || site.runtime.as_deref() == Some("python") {
        let settings: serde_json::Value =
            serde_json::from_str(&site.settings_json).unwrap_or_default();
        let socket = settings
            .get("socket_path")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("/run/panel/python/site_{}/app.sock", site.id));
        state
            .agent_client
            .remove_python_site(panel_proto::PythonSiteRemoveRequest {
                site_id: format!("site_{}", site.id),
                socket_path: Some(socket),
                unit_path: None,
                env_file_path: None,
            })
            .await?;
    }
    Ok(())
}

async fn rollback_created_site(
    state: &AppState,
    site: &Site,
    username: &str,
    php_version: Option<&str>,
) {
    let mut cleanup_failed = false;
    if let Err(err) = state
        .agent_client
        .remove_site(SiteRemoveRequest {
            site_id: format!("site_{}", site.id),
            nginx_base_dir: None,
            reload: true,
        })
        .await
    {
        cleanup_failed = true;
        log_site_cleanup_error(site.id, "remove vhost", err);
    }
    if let Some(version) = php_version
        && let Err(err) = state
            .agent_client
            .remove_php_fpm_pool(username, version, format!("site_{}", site.id), true)
            .await
    {
        cleanup_failed = true;
        log_site_cleanup_error(site.id, "remove FPM pool", err);
    }
    if let Err(error) = remove_site_process(state, site).await {
        cleanup_failed = true;
        log_site_cleanup_error(site.id, "remove runtime", error);
    }
    if cleanup_failed {
        let _ = SqliteSiteRepository::new(state.pool.clone())
            .update(
                site.id,
                &UpdateSite {
                    kind: None,
                    docroot: None,
                    runtime: None,
                    version: None,
                    status: Some("suspended".into()),
                    settings_json: None,
                },
            )
            .await;
        return;
    }
    if let Err(err) = SqliteSiteRepository::new(state.pool.clone())
        .delete(site.id)
        .await
    {
        log_site_cleanup_error(site.id, "delete site", err);
    }
}

fn site_deletion_failure(
    error: crate::agent_client::AgentClientError,
    stage: &'static str,
) -> axum::response::Response {
    site_agent_failure(
        error,
        stage,
        "penghapusan situs belum selesai; domain tetap dicatat",
        "site_deletion_failed",
    )
}

fn site_provisioning_failure(
    error: crate::agent_client::AgentClientError,
    stage: &'static str,
) -> axum::response::Response {
    site_agent_failure(
        error,
        stage,
        "pembuatan situs dibatalkan",
        "site_provisioning_failed",
    )
}

fn site_agent_failure(
    error: crate::agent_client::AgentClientError,
    stage: &'static str,
    operation: &str,
    code: &str,
) -> axum::response::Response {
    let (status, message) = site_agent_error_details(error, stage, operation);
    (
        status,
        Json(serde_json::json!({"error":message,"code":code})),
    )
        .into_response()
}

fn site_agent_error_details(
    error: crate::agent_client::AgentClientError,
    stage: &str,
    operation: &str,
) -> (StatusCode, String) {
    // An agent-reported provisioning rejection is not a proxy failure. Returning
    // 502 makes it indistinguishable from an unavailable origin behind a gateway.
    match error {
        crate::agent_client::AgentClientError::AgentError(detail) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "{stage}; {operation}. {}",
                detail.chars().take(2048).collect::<String>()
            ),
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "{stage}; {operation}. Agent tidak dapat dihubungi atau responsnya tidak valid. Periksa layanan agent."
            ),
        ),
    }
}

/// Creates a site with runtime prerequisites, atomic Nginx apply, and failure cleanup.
#[allow(clippy::cognitive_complexity)]
pub async fn create_static_site_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    context: Option<axum::Extension<crate::public_url::PublicRequest>>,
    Json(payload): Json<CreateStaticSiteRequest>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    // 1. Resolve target tenant
    let target_user = match resolve_target_user(&state, &auth_user, payload.user_id).await {
        Ok(u) => u,
        Err((status, msg)) => {
            return (status, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    // 2. Validate domain
    let parsed_domain = match DomainName::parse(&payload.domain) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Invalid domain name: {e}") })),
            )
                .into_response();
        }
    };
    let ascii_domain = parsed_domain.as_ascii().to_string();
    let unicode_domain = parsed_domain.as_unicode().to_string();
    if let Err((status, message)) = crate::domain_policy::require_tenant_domain(
        &state.pool,
        &ascii_domain,
        context.as_ref().map(|value| &value.0),
    )
    .await
    {
        return (status, Json(serde_json::json!({"error":message}))).into_response();
    }

    // 3. Check domain global availability
    match domain_repo.is_domain_available(&ascii_domain).await {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": format!("Domain '{ascii_domain}' is already in use") })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    }

    // 4. Resolve docroot & settings
    let (linux_identity, tenant_uid) = match crate::tenant_context::execution_identity(&target_user)
    {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error":message}))).into_response();
        }
    };
    let linux_uname = linux_identity.as_str();
    let is_php = payload.kind.as_deref() == Some("php");
    let is_nodejs = payload.kind.as_deref() == Some("nodejs");
    let is_python = payload.kind.as_deref() == Some("python");
    let is_proxy = payload.kind.as_deref() == Some("proxy");
    let preset_str = payload.preset.as_deref().unwrap_or("generic");
    let preset = match PhpPreset::parse(preset_str) {
        Some(preset) => preset,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"Preset PHP tidak valid."})),
            )
                .into_response();
        }
    };

    let docroot = payload.docroot.unwrap_or_else(|| {
        if is_nodejs || is_python || (is_php && preset == PhpPreset::Laravel) {
            format!("/home/{linux_uname}/sites/{ascii_domain}/app")
        } else {
            format!("/home/{linux_uname}/sites/{ascii_domain}/public")
        }
    });
    let spa_fallback = payload.spa_fallback.unwrap_or(false);
    let force_https = payload.force_https.unwrap_or(false);
    let php_version = payload.php_version.unwrap_or_else(|| "8.3".to_string());
    if is_php {
        if panel_runtime::php::PhpVersion::parse(&php_version).is_err() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"Versi PHP tidak valid."})),
            )
                .into_response();
        }
        match state.agent_client.list_php_runtimes().await {
            Ok(runtimes) if runtimes.versions.iter().any(|v| v.version == php_version && v.installed && v.fpm_binary_exists) => {},
            Ok(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"Runtime PHP belum terpasang. Buka Runtime Manager terlebih dahulu."}))).into_response(),
            Err(err) => { tracing::error!(error=%err,"PHP runtime prerequisite lookup failed"); return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error":"Gagal memeriksa runtime PHP. Periksa layanan agent."}))).into_response(); }
        }
    }
    let upload_max_size = payload.upload_max_size.unwrap_or_else(|| "64M".to_string());
    let fastcgi_cache = payload.fastcgi_cache.unwrap_or(false);

    let mut settings_map = serde_json::Map::new();
    settings_map.insert("spa_fallback".to_string(), serde_json::json!(spa_fallback));
    settings_map.insert("force_https".to_string(), serde_json::json!(force_https));
    if is_php {
        settings_map.insert("preset".to_string(), serde_json::json!(preset.as_str()));
        settings_map.insert(
            "upload_max_size".to_string(),
            serde_json::json!(upload_max_size),
        );
        settings_map.insert(
            "fastcgi_cache".to_string(),
            serde_json::json!(fastcgi_cache),
        );
        if let Some(ref entry) = payload.custom_entry_file {
            settings_map.insert("custom_entry_file".to_string(), serde_json::json!(entry));
        }
    } else if is_nodejs {
        let port = match payload.port {
            Some(p) => p,
            None => match crate::node_site::allocate_node_site_port(&state.pool).await {
                Ok(p) => p,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": e })),
                    )
                        .into_response();
                }
            },
        };
        if let Err((status, message)) =
            crate::node_site::validate_runtime_port(&state.pool, None, port).await
        {
            return (status, Json(serde_json::json!({"error":message}))).into_response();
        }
        let node_ver = payload
            .node_version
            .clone()
            .unwrap_or_else(|| "22.14.0".to_string());
        let app_dir = payload.app_dir.clone().unwrap_or_else(|| docroot.clone());
        let entrypoint_type = payload
            .entrypoint_type
            .clone()
            .unwrap_or_else(|| "script".to_string());
        let entrypoint_value = payload
            .entrypoint_value
            .clone()
            .unwrap_or_else(|| "server.js".to_string());
        let auto_restart = payload.auto_restart.unwrap_or(true);
        let env_vars = payload.env_vars.clone().unwrap_or_default();

        settings_map.insert("port".to_string(), serde_json::json!(port));
        settings_map.insert(
            "upstream_url".to_string(),
            serde_json::json!(format!("http://127.0.0.1:{}", port)),
        );
        settings_map.insert("node_version".to_string(), serde_json::json!(node_ver));
        settings_map.insert("app_dir".to_string(), serde_json::json!(app_dir));
        settings_map.insert(
            "entrypoint_type".to_string(),
            serde_json::json!(entrypoint_type),
        );
        settings_map.insert(
            "entrypoint_value".to_string(),
            serde_json::json!(entrypoint_value),
        );
        settings_map.insert("auto_restart".to_string(), serde_json::json!(auto_restart));
        settings_map.insert("env_vars".to_string(), serde_json::json!(env_vars));
        if let Some(ref fw) = payload.framework {
            settings_map.insert("framework_preset".to_string(), serde_json::json!(fw));
        }
    } else if is_python {
        let py_ver = payload
            .python_version
            .clone()
            .unwrap_or_else(|| "3.12.9".to_string());
        let fw_preset = payload
            .framework
            .clone()
            .unwrap_or_else(|| "generic".to_string());
        let srv_type = payload.server_type.clone().unwrap_or_else(|| {
            if fw_preset == "fastapi" {
                "uvicorn_asgi".to_string()
            } else {
                "gunicorn_wsgi".to_string()
            }
        });
        let ep_type = payload.entrypoint_type.clone().unwrap_or_else(|| {
            if srv_type != "gunicorn_wsgi" {
                "asgi".to_string()
            } else {
                "wsgi".to_string()
            }
        });
        let ep_val = payload
            .entrypoint_value
            .clone()
            .unwrap_or_else(|| match fw_preset.as_str() {
                "django" => "wsgi:application".to_string(),
                "fastapi" => "main:app".to_string(),
                _ => "app:app".to_string(),
            });
        let app_dir = payload.app_dir.clone().unwrap_or_else(|| docroot.clone());
        let venv_dir = payload
            .venv_dir
            .clone()
            .unwrap_or_else(|| format!("{}/.venv", app_dir));
        let workers = payload.workers.unwrap_or(3);
        let auto_restart = payload.auto_restart.unwrap_or(true);
        let env_vars = payload.env_vars.clone().unwrap_or_default();

        settings_map.insert("python_version".to_string(), serde_json::json!(py_ver));
        settings_map.insert("framework_preset".to_string(), serde_json::json!(fw_preset));
        settings_map.insert("server_type".to_string(), serde_json::json!(srv_type));
        settings_map.insert("entrypoint_type".to_string(), serde_json::json!(ep_type));
        settings_map.insert("entrypoint_value".to_string(), serde_json::json!(ep_val));
        settings_map.insert("workers".to_string(), serde_json::json!(workers));
        settings_map.insert("app_dir".to_string(), serde_json::json!(app_dir));
        settings_map.insert("venv_dir".to_string(), serde_json::json!(venv_dir));
        settings_map.insert("auto_restart".to_string(), serde_json::json!(auto_restart));
        settings_map.insert("env_vars".to_string(), serde_json::json!(env_vars));

        if fw_preset == "django" {
            settings_map.insert("static_url".to_string(), serde_json::json!("/static/"));
            settings_map.insert(
                "static_dir".to_string(),
                serde_json::json!(format!("{}/static", app_dir)),
            );
            settings_map.insert("media_url".to_string(), serde_json::json!("/media/"));
            settings_map.insert(
                "media_dir".to_string(),
                serde_json::json!(format!("{}/media", app_dir)),
            );
        } else if fw_preset == "flask" {
            settings_map.insert("static_url".to_string(), serde_json::json!("/static/"));
            settings_map.insert(
                "static_dir".to_string(),
                serde_json::json!(format!("{}/static", app_dir)),
            );
        }
    } else if is_proxy {
        let upstream = payload
            .upstream_url
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());

        let allow_external = crate::proxy_site::check_user_allow_external_upstream(
            &state.pool,
            target_user.id,
            auth_user.is_admin(),
        )
        .await;

        if let Err(e) = panel_webserver::template::validate_reverse_proxy_upstream_url(
            &upstream,
            allow_external,
        ) {
            let err_str = e.to_string();
            let status = if err_str.contains("not permit") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            };
            return (status, Json(serde_json::json!({ "error": err_str }))).into_response();
        }

        if let Err((status, message)) =
            crate::proxy_policy::admit(&state.pool, target_user.id, &upstream).await
        {
            return (status, Json(serde_json::json!({"error":message}))).into_response();
        }
        let preserve_host = payload.preserve_host.unwrap_or(true);
        let websocket_support = payload.websocket_support.unwrap_or(true);

        settings_map.insert("upstream_url".to_string(), serde_json::json!(upstream));
        settings_map.insert(
            "preserve_host".to_string(),
            serde_json::json!(preserve_host),
        );
        settings_map.insert(
            "websocket_support".to_string(),
            serde_json::json!(websocket_support),
        );
        if let Some(t) = &payload.proxy_connect_timeout {
            settings_map.insert("proxy_connect_timeout".to_string(), serde_json::json!(t));
        }
        if let Some(t) = &payload.proxy_send_timeout {
            settings_map.insert("proxy_send_timeout".to_string(), serde_json::json!(t));
        }
        if let Some(t) = &payload.proxy_read_timeout {
            settings_map.insert("proxy_read_timeout".to_string(), serde_json::json!(t));
        }
        if let Some(b) = payload.proxy_buffering {
            settings_map.insert("proxy_buffering".to_string(), serde_json::json!(b));
        }
        if let Some(req_headers) = &payload.custom_request_headers {
            settings_map.insert(
                "custom_request_headers".to_string(),
                serde_json::json!(req_headers),
            );
        }
        if let Some(resp_headers) = &payload.custom_response_headers {
            settings_map.insert(
                "custom_response_headers".to_string(),
                serde_json::json!(resp_headers),
            );
        }
    }
    let settings = serde_json::Value::Object(settings_map);

    // 5. Create database record for Site
    let site_input = CreateSite {
        user_id: target_user.id,
        kind: if is_php {
            "php".to_string()
        } else if is_nodejs {
            "nodejs".to_string()
        } else if is_python {
            "python".to_string()
        } else if is_proxy {
            "proxy".to_string()
        } else {
            "static".to_string()
        },
        docroot: docroot.clone(),
        runtime: if is_php {
            Some("php".to_string())
        } else if is_nodejs {
            Some("nodejs".to_string())
        } else if is_python {
            Some("python".to_string())
        } else if is_proxy {
            Some("proxy".to_string())
        } else {
            Some("none".to_string())
        },
        version: if is_php {
            Some(php_version.clone())
        } else if is_nodejs {
            Some(
                payload
                    .node_version
                    .clone()
                    .unwrap_or_else(|| "22.14.0".to_string()),
            )
        } else if is_python {
            Some(
                payload
                    .python_version
                    .clone()
                    .unwrap_or_else(|| "3.12.9".to_string()),
            )
        } else {
            Some("none".to_string())
        },
        status: Some("active".to_string()),
        settings_json: Some(settings.to_string()),
    };

    let created_site = match site_repo.create(&site_input).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // 6. Create primary domain record
    let domain_input = CreateDomain {
        site_id: created_site.id,
        name: ascii_domain.clone(),
        unicode_name: unicode_domain.clone(),
        is_primary: true,
        redirect_mode: Some("none".to_string()),
        redirect_target: None,
        status: Some("active".to_string()),
    };

    let created_domain = match domain_repo.create(&domain_input).await {
        Ok(d) => d,
        Err(e) => {
            let _ = site_repo.delete(created_site.id).await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // 7. Provision the correct starter while retaining existing project files.
    let (starter, placeholder) = if is_php {
        (
            panel_proto::SiteStarter::Php,
            render_php_placeholder(&unicode_domain, &php_version, preset.as_str()),
        )
    } else if is_nodejs {
        let entry = settings
            .get("entrypoint_value")
            .and_then(|v| v.as_str())
            .unwrap_or("server.js");
        let npm = settings
            .get("entrypoint_type")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v.starts_with("npm_"));
        (
            if entry == "server.js" || npm {
                panel_proto::SiteStarter::Nodejs
            } else {
                panel_proto::SiteStarter::Empty
            },
            render_nodejs_placeholder(&unicode_domain),
        )
    } else if is_python {
        let entry = settings
            .get("entrypoint_value")
            .and_then(|v| v.as_str())
            .unwrap_or("app:app");
        let module = entry.split(':').next().unwrap_or("");
        let starter = match module {
            "app" => panel_proto::SiteStarter::PythonApp,
            "main" => panel_proto::SiteStarter::PythonMain,
            "wsgi" => panel_proto::SiteStarter::PythonWsgi,
            "myproject.wsgi" => panel_proto::SiteStarter::PythonDjango,
            _ => panel_proto::SiteStarter::Empty,
        };
        let asgi = settings.get("entrypoint_type").and_then(|v| v.as_str()) == Some("asgi");
        (starter, render_python_starter(&unicode_domain, asgi))
    } else {
        (
            panel_proto::SiteStarter::Html,
            render_placeholder_html(&unicode_domain),
        )
    };
    let provision_dir = settings
        .get("app_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(&docroot);
    let prov_req = SiteProvisionRequest {
        site_id: created_site.id,
        username: linux_uname.to_string(),
        uid: tenant_uid,
        home_dir: format!("/home/{linux_uname}"),
        docroot: if is_php && preset == PhpPreset::Laravel {
            format!("{docroot}/public")
        } else {
            provision_dir.to_string()
        },
        web_user: Some("www-data".to_string()),
        index_content: Some(placeholder),
        starter,
    };
    if let Err(e) = state.agent_client.provision_site(prov_req).await {
        tracing::error!(error=%e, "Site docroot provisioning failed");
        rollback_created_site(
            &state,
            &created_site,
            linux_uname,
            is_php.then_some(&php_version),
        )
        .await;
        return site_provisioning_failure(e, "Provisioning direktori situs gagal");
    }

    // 7b. If PHP site, ensure PHP-FPM master and pool exist
    if is_php {
        let uid = tenant_uid;
        if let Err(e) = state
            .agent_client
            .ensure_php_fpm_master(linux_uname, uid, &php_version)
            .await
        {
            tracing::error!(error=%e, "Ensure PHP-FPM master failed");
            rollback_created_site(&state, &created_site, linux_uname, Some(&php_version)).await;
            return site_provisioning_failure(e, "Master PHP-FPM gagal disiapkan");
        }

        let pool_req = panel_proto::PhpFpmPoolApplyRequest {
            username: linux_uname.to_string(),
            uid,
            version: php_version.clone(),
            pool_name: format!("site_{}", created_site.id),
            docroot: docroot.clone(),
            socket_path: None,
            max_children: None,
            process_idle_timeout_secs: None,
            max_requests: None,
            open_basedir: None,
            disable_functions: None,
            settings: None,
            admin_ini: None,
            reload: true,
        };
        if let Err(e) = state.agent_client.apply_php_fpm_pool(pool_req).await {
            tracing::error!(error=%e, "Apply PHP-FPM pool failed");
            rollback_created_site(&state, &created_site, linux_uname, Some(&php_version)).await;
            return site_provisioning_failure(e, "Pool PHP-FPM gagal diterapkan");
        }
    }

    // Check FPM readiness before publishing the site's Nginx vhost.
    if is_php {
        let failure = match state
            .agent_client
            .check_php_fpm_health(
                linux_uname,
                &php_version,
                format!("site_{}", created_site.id),
            )
            .await
        {
            Ok(health)
                if health.healthy
                    && health.service_active
                    && health.socket_exists
                    && health.pool_exists =>
            {
                None
            }
            Ok(health) => Some(crate::agent_client::AgentClientError::AgentError(
                health.message,
            )),
            Err(error) => Some(error),
        };
        if let Some(error) = failure {
            tracing::error!(error=%error, "PHP-FPM readiness check failed");
            rollback_created_site(&state, &created_site, linux_uname, Some(&php_version)).await;
            return site_provisioning_failure(error, "Health check PHP-FPM gagal");
        }
    }

    // 8. Recompile and apply vhost atomically
    let domains = vec![created_domain.clone()];
    if let Err((status, msg)) =
        recompile_and_apply_site_vhost(&state, &created_site, &target_user, &domains).await
    {
        rollback_created_site(
            &state,
            &created_site,
            linux_uname,
            is_php.then_some(&php_version),
        )
        .await;
        return (
            status,
            Json(serde_json::json!({ "error": msg, "code": "site_provisioning_failed" })),
        )
            .into_response();
    }

    // 9. Audit log
    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.create",
        format!("site:{}", created_site.id),
        "success",
        None,
        Some(format!(
            "domain:{ascii_domain},kind:{}",
            created_site.kind.as_str()
        )),
    );

    (
        StatusCode::CREATED,
        Json(build_site_response(&created_site, &domains)),
    )
        .into_response()
}

/// Lists websites accessible to the authenticated user.
pub async fn list_sites_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<ListSitesQuery>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());

    let sites_res = if auth_user.is_admin() {
        if let Some(uid) = query.user_id {
            site_repo.find_by_user_id(uid).await
        } else {
            site_repo.list_all().await
        }
    } else {
        site_repo.find_by_user_id(auth_user.id).await
    };

    let sites = match sites_res {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let mut response_list = Vec::with_capacity(sites.len());
    for s in sites {
        let domains = domain_repo.find_by_site_id(s.id).await.unwrap_or_default();
        response_list.push(build_site_response(&s, &domains));
    }

    (StatusCode::OK, Json(response_list)).into_response()
}

/// Retrieves details for a specific site.
pub async fn get_site_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());

    let site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let domains = domain_repo
        .find_by_site_id(site.id)
        .await
        .unwrap_or_default();
    (StatusCode::OK, Json(build_site_response(&site, &domains))).into_response()
}

/// Enables a suspended site.
pub async fn enable_site_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let updated = match site_repo
        .update(
            id,
            &UpdateSite {
                kind: None,
                docroot: None,
                runtime: None,
                version: None,
                status: Some("active".to_string()),
                settings_json: None,
            },
        )
        .await
    {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let apply_req = SiteApplyRequest {
        site_id: format!("site_{id}"),
        config_content: String::new(),
        enable: true,
        nginx_base_dir: None,
        log_context: None,
        test_config: false,
        reload: true,
    };
    let _ = state.agent_client.apply_site(apply_req).await;

    // If Node.js site, start service
    if updated.kind == "nodejs" || updated.runtime.as_deref() == Some("nodejs") {
        let _ = state
            .agent_client
            .action_node_site(panel_proto::NodeSiteActionRequest {
                site_id: format!("site_{id}"),
                action: panel_proto::NodeSiteActionType::Start,
            })
            .await;
    }

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.enable",
        format!("site:{id}"),
        "success",
        None,
        None,
    );

    (
        StatusCode::OK,
        Json(SiteMessageResponse {
            message: "Situs berhasil diaktifkan".to_string(),
            site_id: updated.id,
            status: updated.status,
        }),
    )
        .into_response()
}

/// Suspends/disables an active site.
pub async fn disable_site_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let updated = match site_repo
        .update(
            id,
            &UpdateSite {
                kind: None,
                docroot: None,
                runtime: None,
                version: None,
                status: Some("suspended".to_string()),
                settings_json: None,
            },
        )
        .await
    {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let apply_req = SiteApplyRequest {
        site_id: format!("site_{id}"),
        config_content: String::new(),
        enable: false,
        nginx_base_dir: None,
        log_context: None,
        test_config: false,
        reload: true,
    };
    let _ = state.agent_client.apply_site(apply_req).await;

    // If Node.js site, stop service
    if updated.kind == "nodejs" || updated.runtime.as_deref() == Some("nodejs") {
        let _ = state
            .agent_client
            .action_node_site(panel_proto::NodeSiteActionRequest {
                site_id: format!("site_{id}"),
                action: panel_proto::NodeSiteActionType::Stop,
            })
            .await;
    }

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.disable",
        format!("site:{id}"),
        "success",
        None,
        None,
    );

    (
        StatusCode::OK,
        Json(SiteMessageResponse {
            message: "Situs berhasil dinonaktifkan".to_string(),
            site_id: updated.id,
            status: updated.status,
        }),
    )
        .into_response()
}

/// Retrieves the comprehensive final overview details for a specific site (P8-T8).
#[allow(clippy::cognitive_complexity)]
pub async fn get_site_overview_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    let git_repo = SqliteGitDeployRepository::new(state.pool.clone());
    let backup_repo = SqliteBackupRepository::new(state.pool.clone());
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());

    let user = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Pemilik situs tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let domains = domain_repo
        .find_by_site_id(site.id)
        .await
        .unwrap_or_default();
    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| format!("site-{}.local", site.id));

    let plan = if let Some(pid) = user.plan_id {
        plan_repo.find_by_id(pid).await.ok().flatten()
    } else {
        None
    };

    let plan_disk_limit_bytes = plan
        .as_ref()
        .map(|p| p.disk_limit_bytes as u64)
        .unwrap_or(10 * 1024 * 1024 * 1024);
    let plan_memory_limit_bytes = plan
        .as_ref()
        .map(|p| p.memory_limit_bytes as u64)
        .unwrap_or(1024 * 1024 * 1024);
    let plan_pids_limit = plan.as_ref().map(|p| p.tasks_max as u64).unwrap_or(100);

    let (tenant_linux_name, tenant_linux_uid) =
        match crate::tenant_context::execution_identity(&user) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error": message}))).into_response();
            }
        };

    // 1. Live Runtime Status
    let (runtime_status, healthy, uptime_secs, pid) = match site.kind.as_str() {
        "nodejs" => {
            let settings: serde_json::Value =
                serde_json::from_str(&site.settings_json).unwrap_or_default();
            let port = settings
                .get("port")
                .and_then(|v| v.as_u64())
                .map(|p| p as u16)
                .unwrap_or(3000);
            let req = panel_proto::NodeSiteStatusRequest {
                site_id: format!("site_{}", site.id),
                port: Some(port),
                probe_path: Some("/".to_string()),
            };
            match state.agent_client.status_node_site(req).await {
                Ok(st) => (
                    if st.active {
                        "running".to_string()
                    } else {
                        "inactive".to_string()
                    },
                    st.active && st.healthy,
                    st.uptime_secs,
                    st.pid,
                ),
                Err(_) => ("unknown".to_string(), false, None, None),
            }
        }
        "python" => {
            let req = panel_proto::PythonSiteStatusRequest {
                site_id: format!("site_{}", site.id),
                socket_path: None,
            };
            match state.agent_client.status_python_site(req).await {
                Ok(st) => (
                    if st.active {
                        "running".to_string()
                    } else {
                        "inactive".to_string()
                    },
                    st.active && st.healthy,
                    st.uptime_secs,
                    st.pid,
                ),
                Err(_) => ("unknown".to_string(), false, None, None),
            }
        }
        "php" => match state
            .agent_client
            .get_php_fpm_pool_status(
                &tenant_linux_name,
                site.version.as_deref().unwrap_or("8.3"),
                format!("site_{}", site.id),
            )
            .await
        {
            Ok(status) => (
                if status.is_active {
                    "running"
                } else {
                    "inactive"
                }
                .to_string(),
                status.is_active && status.socket_exists && site.status == "active",
                None,
                None,
            ),
            Err(_) => ("unknown".to_string(), false, None, None),
        },
        _ => (
            "not_applicable".to_string(),
            site.status == "active",
            None,
            None,
        ),
    };

    let live_status = SiteLiveStatus {
        webserver_status: site.status.clone(),
        runtime_status,
        healthy,
        uptime_secs,
        pid,
    };

    // 2. Resource Metrics
    let (cpu_usage_usec, memory_bytes, pids_current) = if let Some(uid) = user.linux_uid {
        match state.agent_client.read_slice_resources(uid as u32).await {
            Ok(m) => (m.cpu_usage_usec, m.memory_current_bytes, m.pids_current),
            Err(_) => (0, 0, 0),
        }
    } else {
        (0, 0, 0)
    };

    let cpu_percent = if cpu_usage_usec > 0 {
        ((cpu_usage_usec as f64 / 1_000_000.0) % 100.0 * 10.0).round() / 10.0
    } else {
        0.0
    };

    let resources = SiteResourceMetrics {
        cpu_usage_usec,
        cpu_percent,
        memory_bytes,
        memory_limit_bytes: plan_memory_limit_bytes,
        pids_current,
        pids_limit: plan_pids_limit,
    };

    // 3. SSL Summary
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let cert_opt = cert_repo
        .find_active_by_site_id(site.id)
        .await
        .unwrap_or_default();
    let site_settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or_default();
    let is_force_https = site_settings
        .get("force_https")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let ssl = match cert_opt {
        Some(c) => {
            let days_remaining = (c.not_after - now) / 86400;
            let status = if days_remaining <= 0 {
                "expired".to_string()
            } else if days_remaining < 14 {
                "expiring_soon".to_string()
            } else {
                "valid".to_string()
            };
            SiteSslSummary {
                has_certificate: true,
                issuer: Some(c.issuer),
                valid_until: Some(c.not_after),
                days_remaining: Some(days_remaining),
                auto_renew: c.auto_renew != 0,
                status,
                force_https: is_force_https,
            }
        }
        None => SiteSslSummary {
            has_certificate: false,
            issuer: None,
            valid_until: None,
            days_remaining: None,
            auto_renew: false,
            status: "none".to_string(),
            force_https: is_force_https,
        },
    };

    // 4. Runtime Summary
    let settings: serde_json::Value = serde_json::from_str(&site.settings_json).unwrap_or_default();
    let preset = settings
        .get("preset")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let framework = settings
        .get("framework")
        .or_else(|| settings.get("framework_preset"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let entrypoint_or_target = match site.kind.as_str() {
        "nodejs" => {
            let port = settings
                .get("port")
                .and_then(|v| v.as_u64())
                .unwrap_or(3000);
            format!("127.0.0.1:{port}")
        }
        "python" => {
            format!("/run/panel/python/site_{}/app.sock", site.id)
        }
        "php" => {
            let ver = site.version.as_deref().unwrap_or("8.3");
            let uname = &tenant_linux_name;
            format!("/run/panel/php/{uname}-{ver}/fpm.sock")
        }
        "proxy" => settings
            .get("upstream_url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://127.0.0.1:8080")
            .to_string(),
        _ => site.docroot.clone(),
    };

    let runtime = SiteRuntimeSummary {
        kind: site.kind.clone(),
        version: site
            .version
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        preset,
        entrypoint_or_target,
        framework,
    };

    // 5. Storage Breakdown
    let dbs = db_repo.list_by_site(site.id).await.unwrap_or_default();
    let databases_bytes: u64 = dbs.iter().map(|d| d.size_bytes.max(0) as u64).sum();

    let backups = backup_repo
        .list_backups_by_site(site.id)
        .await
        .unwrap_or_default();
    let backups_bytes: u64 = backups.iter().map(|b| b.size_bytes.max(0) as u64).sum();

    let quota_used = if let Some(ref uname) = user.linux_username {
        state
            .agent_client
            .get_quota(panel_proto::QuotaGetRequest {
                uid: tenant_linux_uid,
                username: uname.clone(),
                mount_point: None,
            })
            .await
            .ok()
            .map(|q| q.bytes_used)
            .unwrap_or(0)
    } else {
        0
    };

    let docroot_bytes = if quota_used > (databases_bytes + backups_bytes) {
        quota_used - (databases_bytes + backups_bytes)
    } else {
        quota_used
    };

    let total_site_bytes = docroot_bytes + databases_bytes + backups_bytes;
    let usage_percentage = if plan_disk_limit_bytes > 0 {
        ((total_site_bytes as f64 / plan_disk_limit_bytes as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };

    let storage = SiteStorageBreakdown {
        docroot_bytes,
        databases_bytes,
        backups_bytes,
        total_site_bytes,
        plan_disk_limit_bytes,
        usage_percentage,
    };

    // 6. Latest Deployment
    let deployments = git_repo
        .list_deployments(site.id, 2, 0)
        .await
        .unwrap_or_default();
    let latest_deployment = deployments.first().map(|d| {
        let can_rollback = deployments.len() > 1 && deployments[1].status == "success";
        LatestDeploymentSummary {
            deployment_id: d.id,
            release_id: d.release_id.clone(),
            commit_hash: d.commit_hash.clone().unwrap_or_default(),
            commit_message: d.commit_message.clone().unwrap_or_default(),
            commit_author: d.commit_author.clone().unwrap_or_default(),
            branch: d.branch.clone(),
            trigger_type: d.trigger_type.clone(),
            status: d.status.clone(),
            duration_ms: d.duration_ms,
            created_at: d.created_at,
            finished_at: d.finished_at,
            can_rollback,
        }
    });

    // 7. Latest Backup
    let latest_backup = backups.first().map(|b| LatestBackupSummary {
        backup_id: b.id,
        filename: b.filename.clone(),
        size_bytes: b.size_bytes,
        status: b.status.clone(),
        includes_files: b.includes_files,
        includes_databases: b.includes_databases,
        created_at: b.created_at,
    });

    // 8. Contextual Warnings
    let mut warnings = Vec::new();

    if ssl.status == "expired" {
        warnings.push(SiteContextualWarning {
            id: "ssl_expired".to_string(),
            level: "danger".to_string(),
            title: "Sertifikat SSL Telah Kedaluwarsa".to_string(),
            message: "Sertifikat SSL untuk domain ini telah kedaluwarsa. Pengunjung akan menerima peringatan koneksi tidak aman.".to_string(),
            action_tab: Some("ssl".to_string()),
        });
    } else if ssl.status == "expiring_soon" {
        let days = ssl.days_remaining.unwrap_or(0);
        warnings.push(SiteContextualWarning {
            id: "ssl_expiring_soon".to_string(),
            level: "warning".to_string(),
            title: "Sertifikat SSL Hampir Kedaluwarsa".to_string(),
            message: format!("Sertifikat SSL akan berakhir dalam {days} hari. Pastikan auto-renew aktif atau perbarui sertifikat secara manual."),
            action_tab: Some("ssl".to_string()),
        });
    }

    if is_force_https && (ssl.status == "none" || ssl.status == "expired") {
        warnings.push(SiteContextualWarning {
            id: "force_https_no_ssl".to_string(),
            level: "danger".to_string(),
            title: "Force HTTPS Aktif Tanpa Sertifikat Valid".to_string(),
            message: "Pengalihan paksa HTTPS aktif namun tidak ada sertifikat valid terpasang. Situs tidak dapat diakses pengguna melalui HTTPS.".to_string(),
            action_tab: Some("ssl".to_string()),
        });
    }

    if usage_percentage >= 90.0 {
        warnings.push(SiteContextualWarning {
            id: "disk_critical".to_string(),
            level: "danger".to_string(),
            title: "Penggunaan Kuota Disk Kritis (> 90%)".to_string(),
            message: format!("Penyimpanan situs telah mencapai {:.1}% dari kuota paket hosting. Harap bersihkan berkas yang tidak terpakai.", usage_percentage),
            action_tab: Some("files".to_string()),
        });
    } else if usage_percentage >= 80.0 {
        warnings.push(SiteContextualWarning {
            id: "disk_warning".to_string(),
            level: "warning".to_string(),
            title: "Penyimpanan Mendekati Batas (> 80%)".to_string(),
            message: format!(
                "Penyimpanan situs mencapai {:.1}% dari batas paket.",
                usage_percentage
            ),
            action_tab: Some("files".to_string()),
        });
    }

    if site.status != "active" {
        warnings.push(SiteContextualWarning {
            id: "site_disabled".to_string(),
            level: "warning".to_string(),
            title: "Situs Saat Ini Dinonaktifkan".to_string(),
            message: "Konfigurasi webserver vhost dan runtime proses sedang ditangguhkan."
                .to_string(),
            action_tab: Some("overview".to_string()),
        });
    }

    if let Some(dep) = latest_deployment.as_ref().filter(|d| d.status == "failed") {
        warnings.push(SiteContextualWarning {
            id: "latest_deploy_failed".to_string(),
            level: "danger".to_string(),
            title: "Deployment Terakhir Gagal".to_string(),
            message: format!(
                "Rilis '{}' gagal dibangun. Periksa log rilis atau pulihkan ke rilis sebelumnya.",
                dep.release_id
            ),
            action_tab: Some("deploy".to_string()),
        });
    }

    if let Some(bkp) = latest_backup.as_ref().filter(|b| b.status == "failed") {
        warnings.push(SiteContextualWarning {
            id: "latest_backup_failed".to_string(),
            level: "warning".to_string(),
            title: "Pencadangan Terakhir Gagal".to_string(),
            message: format!("Pencadangan '{}' mengalami kegagalan proses.", bkp.filename),
            action_tab: Some("backups".to_string()),
        });
    }

    let mut quick_actions_available = vec![
        "start".to_string(),
        "stop".to_string(),
        "reload".to_string(),
    ];
    if matches!(site.kind.as_str(), "nodejs" | "python") {
        quick_actions_available.push("restart".to_string());
    }

    (
        StatusCode::OK,
        Json(SiteOverviewResponse {
            site_id: site.id,
            domain: primary_domain,
            status: live_status,
            quick_actions_available,
            resources,
            ssl,
            runtime,
            storage,
            warnings,
            latest_deployment,
            latest_backup,
        }),
    )
        .into_response()
}

// A control action must not acknowledge a failed agent response as success.
async fn control_site_process(
    state: &AppState,
    site: &Site,
    action: &str,
) -> Result<(), (StatusCode, String)> {
    let site_id = format!("site_{}", site.id);
    let result = if site.kind == "nodejs" {
        let action = match action {
            "start" => panel_proto::NodeSiteActionType::Start,
            "stop" => panel_proto::NodeSiteActionType::Stop,
            _ => panel_proto::NodeSiteActionType::Restart,
        };
        state
            .agent_client
            .action_node_site(panel_proto::NodeSiteActionRequest { site_id, action })
            .await
            .map(|r| r.success)
    } else if site.kind == "python" {
        let action = match action {
            "start" => panel_proto::PythonSiteActionType::Start,
            "stop" => panel_proto::PythonSiteActionType::Stop,
            _ => panel_proto::PythonSiteActionType::Restart,
        };
        state
            .agent_client
            .action_python_site(panel_proto::PythonSiteActionRequest { site_id, action })
            .await
            .map(|r| r.success)
    } else {
        return Ok(());
    };
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err((
            StatusCode::BAD_GATEWAY,
            "Agent belum berhasil menjalankan aksi aplikasi. Periksa status dan log situs.".into(),
        )),
        Err(error) => Err((
            StatusCode::BAD_GATEWAY,
            format!("Aksi aplikasi gagal: {error}. Periksa ulang status situs."),
        )),
    }
}

// Reload only the vhost. The general reconciliation helper also applies app
// services, which must not be a side effect of the Nginx-only control.
async fn control_site_vhost(
    state: &AppState,
    site: &Site,
    enable: bool,
    render: bool,
) -> Result<(), (StatusCode, String)> {
    let config_content = if render {
        let user = SqliteUserRepository::new(state.pool.clone())
            .find_by_id(site.user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    "Pemilik situs tidak ditemukan".into(),
                )
            })?;
        let domains = SqliteDomainRepository::new(state.pool.clone())
            .find_by_site_id(site.id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        render_site_vhost(&state.pool, site, &user, &domains).await?
    } else {
        String::new()
    };
    let result = state
        .agent_client
        .apply_site(SiteApplyRequest {
            site_id: format!("site_{}", site.id),
            config_content,
            enable,
            nginx_base_dir: None,
            log_context: None,
            test_config: true,
            reload: true,
        })
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Perubahan Nginx gagal: {e}. Periksa ulang status situs."),
            )
        })?;
    if result.enabled != enable || !result.reloaded {
        return Err((
            StatusCode::BAD_GATEWAY,
            "Nginx belum mengonfirmasi perubahan dan reload. Periksa log sebelum mencoba lagi."
                .into(),
        ));
    }
    Ok(())
}

/// Executes a typed site control and commits its configured state only after acknowledgement.
pub async fn site_quick_action_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<SiteQuickActionRequest>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(site) => site,
        Err((code, message)) => {
            return (code, Json(serde_json::json!({"error": message}))).into_response();
        }
    };
    let action = payload.action.trim().to_lowercase();
    let result = match action.as_str() {
        "start" | "stop" => {
            match control_site_process(&state, &site, &action).await {
                Err(error) => Err(error),
                Ok(()) => control_site_vhost(&state, &site, action == "start", false).await,
            }
        }
        "restart" if matches!(site.kind.as_str(), "nodejs" | "python") => control_site_process(&state, &site, "restart").await,
        // Retain the legacy endpoint for existing clients; the UI no longer
        // advertises an app restart for PHP/static/proxy sites.
        "restart" | "reload" => control_site_vhost(&state, &site, site.status == "active", true).await,
        "clear_cache" => Err((StatusCode::BAD_REQUEST, "Pembersihan cache aplikasi tidak tersedia dari kontrol layanan. Reload Nginx tidak menghapus cache.".into())),
        _ => Err((StatusCode::BAD_REQUEST, "Aksi situs tidak dikenal.".into())),
    };
    if let Err((code, message)) = result {
        state.audit_writer.log_async(
            Some(auth_user.id),
            &auth_user.username,
            "site.control_failed",
            format!("site:{}", site.id),
            "failed",
            None,
            None,
        );
        return (code, Json(serde_json::json!({"error": message}))).into_response();
    }
    if matches!(action.as_str(), "start" | "stop") {
        let update = UpdateSite {
            kind: None,
            docroot: None,
            runtime: None,
            version: None,
            status: Some(
                if action == "start" {
                    "active"
                } else {
                    "suspended"
                }
                .into(),
            ),
            settings_json: None,
        };
        if let Err(error) = site_repo.update(site.id, &update).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("Layanan sudah diubah, tetapi status tersimpan belum diperbarui: {error}. Segarkan dan periksa situs.")}))).into_response();
        }
    }
    let message = match action.as_str() {
        "start" => "Situs berhasil diaktifkan. Status aplikasi diperiksa kembali.",
        "stop" => "Akses situs berhasil dinonaktifkan. Status aplikasi diperiksa kembali.",
        "restart" if matches!(site.kind.as_str(), "nodejs" | "python") => {
            "Proses aplikasi berhasil dimulai ulang. Status diperiksa kembali."
        }
        _ => "Konfigurasi Nginx berhasil dimuat ulang. Proses aplikasi tidak dimulai ulang.",
    };
    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        format!("site.{action}"),
        format!("site:{}", site.id),
        "success",
        None,
        None,
    );
    (
        StatusCode::OK,
        Json(SiteQuickActionResponse {
            success: true,
            action,
            message: message.into(),
        }),
    )
        .into_response()
}

/// Deletes a website and its associated virtual host configurations.
pub async fn delete_site_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = match fetch_and_authorize_site(&site_repo, &auth_user, id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    // 1. Remove Nginx configuration on host via agent
    let rm_req = SiteRemoveRequest {
        site_id: format!("site_{id}"),
        nginx_base_dir: None,
        reload: true,
    };
    if let Err(error) = state.agent_client.remove_site(rm_req).await {
        return site_deletion_failure(error, "Penghapusan vhost gagal");
    }

    // 1b. If PHP site, remove PHP-FPM pool via agent
    if site.kind == "php" || site.runtime.as_deref() == Some("php") {
        let user_repo = SqliteUserRepository::new(state.pool.clone());
        if let Ok(Some(owner)) = user_repo.find_by_id(site.user_id).await {
            let (uname, _) = match crate::tenant_context::execution_identity(&owner) {
                Ok(identity) => identity,
                Err((status, message)) => {
                    return (status, Json(serde_json::json!({"error":message}))).into_response();
                }
            };
            let ver = site.version.as_deref().unwrap_or("8.3");
            if let Err(error) = state
                .agent_client
                .remove_php_fpm_pool(&uname, ver, format!("site_{id}"), true)
                .await
            {
                return site_deletion_failure(error, "Penghapusan pool PHP-FPM gagal");
            }
        }
    }

    if let Err(error) = remove_site_process(&state, &site).await {
        return site_deletion_failure(error, "Penghapusan proses aplikasi gagal");
    }

    // 2. Delete database record (cascading deletes domains)
    if let Err(e) = site_repo.delete(id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    // 3. Audit log
    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.delete",
        format!("site:{id}"),
        "success",
        None,
        Some(format!("docroot:{}", site.docroot)),
    );

    (
        StatusCode::OK,
        Json(SiteMessageResponse {
            message: "Situs berhasil dihapus".to_string(),
            site_id: id,
            status: "deleted".to_string(),
        }),
    )
        .into_response()
}

/// Lists all domains associated with a website.
pub async fn list_site_domains_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());

    if let Err((code, msg)) = fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        return (code, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    let domains = match domain_repo.find_by_site_id(site_id).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let dtos: Vec<SiteDomainResponse> = domains.iter().map(domain_to_response).collect();
    (StatusCode::OK, Json(dtos)).into_response()
}

/// Adds a new domain (alias, subdomain, or redirect) to a website.
pub async fn add_site_domain_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    context: Option<axum::Extension<crate::public_url::PublicRequest>>,
    Path(site_id): Path<i64>,
    Json(payload): Json<AddSiteDomainRequest>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());

    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let user = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Site owner user not found" })),
            )
                .into_response();
        }
    };

    // Validate domain format
    let parsed = match DomainName::parse(&payload.domain) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Invalid domain name: {e}") })),
            )
                .into_response();
        }
    };
    let ascii_domain = parsed.as_ascii().to_string();
    let unicode_domain = parsed.as_unicode().to_string();
    if let Err((status, message)) = crate::domain_policy::require_tenant_domain(
        &state.pool,
        &ascii_domain,
        context.as_ref().map(|value| &value.0),
    )
    .await
    {
        return (status, Json(serde_json::json!({"error":message}))).into_response();
    }

    // Check domain global availability
    match domain_repo.is_domain_available(&ascii_domain).await {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": format!("Domain '{ascii_domain}' is already in use") })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    }

    // Validate redirect mode
    let redirect_mode_str = payload.redirect_mode.as_deref().unwrap_or("none");
    if RedirectMode::parse(redirect_mode_str).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Invalid redirect_mode '{redirect_mode_str}'") })),
        )
            .into_response();
    }

    let is_primary = payload.is_primary.unwrap_or(false);

    let create_input = CreateDomain {
        site_id,
        name: ascii_domain.clone(),
        unicode_name: unicode_domain,
        is_primary,
        redirect_mode: Some(redirect_mode_str.to_string()),
        redirect_target: payload.redirect_target,
        status: Some("active".to_string()),
    };

    let created_domain = match domain_repo.create(&create_input).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Recompile virtual host with new domain
    let all_domains = match domain_repo.find_by_site_id(site_id).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if let Err((status, msg)) =
        recompile_and_apply_site_vhost(&state, &site, &user, &all_domains).await
    {
        // Rollback created domain in DB if apply fails
        let _ = domain_repo.delete(created_domain.id).await;
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "domain.create",
        format!("domain:{}", created_domain.id),
        "success",
        None,
        Some(format!("site:{site_id},domain:{ascii_domain}")),
    );

    (
        StatusCode::CREATED,
        Json(domain_to_response(&created_domain)),
    )
        .into_response()
}

/// Updates domain configuration (redirect mode, redirect target, or primary promotion).
pub async fn update_site_domain_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, domain_id)): Path<(i64, i64)>,
    Json(payload): Json<UpdateSiteDomainRequest>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());

    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let user = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Site owner user not found" })),
            )
                .into_response();
        }
    };

    let existing_domain = match domain_repo.find_by_id(domain_id).await {
        Ok(Some(d)) if d.site_id == site_id => d,
        Ok(Some(_)) | Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Domain not found for this site" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if payload
        .redirect_mode
        .as_deref()
        .is_some_and(|mode| RedirectMode::parse(mode).is_none())
    {
        let mode = payload.redirect_mode.as_deref().unwrap_or_default();
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Invalid redirect_mode '{mode}'") })),
        )
            .into_response();
    }

    let updated_domain = match domain_repo
        .update(
            domain_id,
            &UpdateDomain {
                name: None,
                unicode_name: None,
                is_primary: payload.is_primary,
                redirect_mode: payload.redirect_mode,
                redirect_target: payload.redirect_target,
                status: None,
            },
        )
        .await
    {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let all_domains = match domain_repo.find_by_site_id(site_id).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if let Err((status, msg)) =
        recompile_and_apply_site_vhost(&state, &site, &user, &all_domains).await
    {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "domain.update",
        format!("domain:{domain_id}"),
        "success",
        None,
        Some(format!("site:{site_id},domain:{}", existing_domain.name)),
    );

    (StatusCode::OK, Json(domain_to_response(&updated_domain))).into_response()
}

/// Deletes an auxiliary domain from a website.
pub async fn delete_site_domain_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path((site_id, domain_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());

    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let user = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Site owner user not found" })),
            )
                .into_response();
        }
    };

    let existing_domain = match domain_repo.find_by_id(domain_id).await {
        Ok(Some(d)) if d.site_id == site_id => d,
        Ok(Some(_)) | Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Domain not found for this site" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if existing_domain.is_primary {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Domain utama tidak dapat dihapus. Silakan tetapkan domain lain sebagai domain utama terlebih dahulu."
            })),
        )
            .into_response();
    }

    if let Err(e) = domain_repo.delete(domain_id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let all_domains = match domain_repo.find_by_site_id(site_id).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if let Err((status, msg)) =
        recompile_and_apply_site_vhost(&state, &site, &user, &all_domains).await
    {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "domain.delete",
        format!("domain:{domain_id}"),
        "success",
        None,
        Some(format!("site:{site_id},domain:{}", existing_domain.name)),
    );

    (
        StatusCode::OK,
        Json(SiteMessageResponse {
            message: "Domain berhasil dihapus".to_string(),
            site_id,
            status: "deleted".to_string(),
        }),
    )
        .into_response()
}

/// Configures force-HTTPS redirection for a website.
pub async fn set_force_https_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(payload): Json<SetForceHttpsRequest>,
) -> impl IntoResponse {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());

    let site = match fetch_and_authorize_site(&site_repo, &auth_user, site_id).await {
        Ok(s) => s,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({ "error": msg }))).into_response();
        }
    };

    let user = match user_repo.find_by_id(site.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Site owner user not found" })),
            )
                .into_response();
        }
    };

    let mut settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
    settings["force_https"] = serde_json::Value::Bool(payload.force_https);

    let updated_site = match site_repo
        .update(
            site_id,
            &UpdateSite {
                kind: None,
                docroot: None,
                runtime: None,
                version: None,
                status: None,
                settings_json: Some(settings.to_string()),
            },
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let all_domains = match domain_repo.find_by_site_id(site_id).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if let Err((status, msg)) =
        recompile_and_apply_site_vhost(&state, &updated_site, &user, &all_domains).await
    {
        return (status, Json(serde_json::json!({ "error": msg }))).into_response();
    }

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.force_https",
        format!("site:{site_id}"),
        "success",
        None,
        Some(format!("force_https:{}", payload.force_https)),
    );

    (
        StatusCode::OK,
        Json(build_site_response(&updated_site, &all_domains)),
    )
        .into_response()
}

fn render_php_placeholder(domain: &str, php_version: &str, preset: &str) -> String {
    // Provisioning writes index.html. Keep this welcome page valid when served as HTML.
    panel_webserver::placeholder::render_welcome_html(
        domain,
        &format!("PHP {php_version} · {preset}"),
        "Unggah aplikasi PHP ke direktori website. Ganti halaman bawaan ini dengan halaman aplikasi Anda.",
    )
}

fn render_nodejs_placeholder(domain: &str) -> String {
    let html = panel_webserver::placeholder::render_welcome_html(
        domain,
        "Node.js",
        "Unggah aplikasi Node.js, periksa pengaturan runtime, lalu jalankan deployment dari panel.",
    );
    let html_json = serde_json::json!(html);
    format!(
        r#"const http = require('http');

const port = process.env.PORT || 3000;
const server = http.createServer((req, res) => {{
    res.writeHead(200, {{ 'Content-Type': 'text/html; charset=utf-8' }});
    res.end({html_json});
}});

server.listen(port, '127.0.0.1', () => {{
    console.log(`Server listening on port ${{port}}`);
}});
"#
    )
}

fn render_python_placeholder(domain: &str) -> String {
    let html = panel_webserver::placeholder::render_welcome_html(
        domain,
        "Python",
        "Unggah aplikasi Python, periksa pengaturan runtime, lalu jalankan deployment dari panel.",
    );
    // A JSON string literal is also a Python string literal; encode after parsing
    // so Indonesian copy and international domain names produce valid UTF-8 bytes.
    let html_json = serde_json::json!(html);
    format!(
        r#"def app(environ, start_response):
    start_response('200 OK', [('Content-Type', 'text/html; charset=utf-8')])
    html = {html_json}
    return [html.encode('utf-8')]

application = app
"#
    )
}

fn render_python_starter(domain: &str, asgi: bool) -> String {
    if !asgi {
        return render_python_placeholder(domain);
    }
    let html = panel_webserver::placeholder::render_welcome_html(
        domain,
        "Python",
        "Unggah aplikasi Python Anda untuk memulai deployment.",
    );
    let html = serde_json::json!(html);
    format!(
        r#"async def app(scope, receive, send):
    if scope['type'] == 'lifespan':
        while True:
            message = await receive()
            if message['type'] == 'lifespan.startup':
                await send({{'type': 'lifespan.startup.complete'}})
            elif message['type'] == 'lifespan.shutdown':
                await send({{'type': 'lifespan.shutdown.complete'}})
                return
    elif scope['type'] == 'http':
        html = {html}
        await send({{'type': 'http.response.start', 'status': 200, 'headers': [(b'content-type', b'text/html; charset=utf-8')]}})
        await send({{'type': 'http.response.body', 'body': html.encode('utf-8')}})

application = app
"#
    )
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod placeholder_tests {
    use super::*;

    #[test]
    fn generated_python_wsgi_and_asgi_starters_execute_with_utf8_responses() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let script = r#"import sys, asyncio
ns = {}
exec(sys.stdin.read(), ns)
if sys.argv[1] == 'asgi':
    messages = []
    async def receive(): return {'type': 'http.request', 'body': b''}
    async def send(message): messages.append(message)
    asyncio.run(ns['app']({'type': 'http'}, receive, send))
    assert messages[0]['status'] == 200
    body = messages[1]['body']
else:
    statuses = []
    body = b''.join(ns['application']({}, lambda status, headers: statuses.append(status)))
    assert statuses == ['200 OK']
assert 'toko-\u00e9.example.test' in body.decode('utf-8')
"#;
        for asgi in [false, true] {
            let mut child = Command::new(if cfg!(windows) { "python" } else { "python3" })
                .args([
                    "-X",
                    "utf8",
                    "-c",
                    script,
                    if asgi { "asgi" } else { "wsgi" },
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(render_python_starter("toko-\u{00e9}.example.test", asgi).as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn generated_node_starter_answers_http_on_a_temporary_loopback_port() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let program = format!(
            "process.env.PORT = '0';\n{}\n{}",
            render_nodejs_placeholder("runtime.example.test"),
            r#"
server.on('listening', () => {
  http.get({host: '127.0.0.1', port: server.address().port, path: '/'}, response => {
    let body = '';
    response.setEncoding('utf8');
    response.on('data', chunk => body += chunk);
    response.on('end', () => {
      server.close();
      if (response.statusCode !== 200 || !body.includes('runtime.example.test')) process.exitCode = 1;
    });
  }).on('error', () => { server.close(); process.exitCode = 2; });
});
setTimeout(() => process.exit(3), 3000).unref();
"#
        );
        let mut child = Command::new("node")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(program.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn provisioning_rejection_keeps_bounded_diagnostics_in_json() {
        let response = site_provisioning_failure(
            crate::agent_client::AgentClientError::AgentError(format!(
                "No pool defined: {}",
                "é".repeat(4000)
            )),
            "Pool PHP-FPM gagal diterapkan",
        );
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], "application/json");
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON error");
        let message = body["error"].as_str().expect("message");
        assert!(message.contains("No pool defined"));
        assert!(message.chars().count() < 2200, "bounded UTF-8 diagnostic");
        assert_eq!(body["code"], "site_provisioning_failed");
    }

    #[tokio::test]
    async fn unavailable_agent_is_a_service_error_without_internal_socket_path() {
        let response = site_provisioning_failure(
            crate::agent_client::AgentClientError::Connect {
                path: std::path::PathBuf::from("/private/operator/agent.sock"),
                source: std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "offline"),
            },
            "Master PHP-FPM gagal disiapkan",
        );
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("body");
        let body = std::str::from_utf8(&bytes).expect("UTF-8");
        assert!(body.contains("Agent tidak dapat dihubungi"));
        assert!(!body.contains("/private/operator"));
    }

    #[test]
    fn runtime_welcome_pages_share_design_and_preserve_output_formats() {
        let domain = "toko.contoh.id";
        let php = render_php_placeholder(domain, "8.4", "generic");
        assert!(php.starts_with("<!doctype html>"));
        assert!(php.contains("PHP 8.4 · generic"));
        assert!(!php.contains("<?php"));
        let node = render_nodejs_placeholder(domain);
        let python = render_python_placeholder(domain);
        assert!(node.contains("require('http')"));
        assert!(node.contains("server.listen(port, '127.0.0.1'"));
        assert!(python.contains("html.encode('utf-8')"));
        assert!(!python.contains("b\"\"\""));
        for source in [&php, &node, &python] {
            assert!(source.contains(domain));
            assert!(source.contains("Publikasikan website Anda"));
            assert!(!source.contains("backdrop-filter"));
            assert!(!source.contains("gradient("));
        }
    }

    #[test]
    fn application_sources_keep_unicode_and_quotes_in_string_literals() {
        let domain = "toko-é.id`'\"${not_code}<script>";
        let node = render_nodejs_placeholder(domain);
        let python = render_python_placeholder(domain);
        for (source, prefix, suffix) in
            [(&node, "    res.end(", ");"), (&python, "    html = ", "")]
        {
            let literal = source
                .lines()
                .find_map(|line| line.strip_prefix(prefix))
                .and_then(|line| line.strip_suffix(suffix))
                .expect("serialized HTML literal");
            let html: String = serde_json::from_str(literal).expect("valid quoted string");
            assert!(html.contains("toko-é.id`&#39;&quot;${not_code}&lt;script&gt;"));
            assert!(!html.contains("<script>"));
            assert!(html.contains("<main class=\"wrap\">"));
        }
    }
}
