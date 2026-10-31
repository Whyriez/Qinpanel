//! Nginx webserver manager for host configuration and lifecycle operations.

use crate::AgentError;
use crate::system::SystemRunner;
use panel_proto::{
    AcmeChallengeCleanedResponse, AcmeChallengeCleanupRequest, AcmeChallengeDeployRequest,
    AcmeChallengeDeployedResponse, NginxBaselineAppliedResponse, NginxBaselineApplyRequest,
    SiteAppliedResponse, SiteApplyRequest, SiteHtpasswdDeployRequest, SiteHtpasswdDeployedResponse,
    SiteHtpasswdRemoveRequest, SiteHtpasswdRemovedResponse, SiteLogsReadRequest,
    SiteLogsReadResponse, SiteProvisionRequest, SiteProvisionResponse, SiteRemoveRequest,
    SiteRemovedResponse, SslCertDeployRequest, SslCertDeployedResponse,
};
use panel_webserver::baseline::NginxBaselineConfig;
use panel_webserver::pipeline::{NginxApplyPipeline, ReloadDebouncer};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

/// Resolves the base directory for Nginx configuration.
/// Priority:
/// 1. Explicit override passed in request or manager
/// 2. `PANEL_NGINX_BASE_DIR` environment variable
/// 3. In test runner environments without root permissions, fall back to temporary directory
/// 4. Default: `/etc/panel/nginx`
pub fn get_nginx_base_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(d) = override_dir {
        return d.to_path_buf();
    }
    if let Some(env_d) = std::env::var("PANEL_NGINX_BASE_DIR")
        .ok()
        .filter(|d| !d.trim().is_empty())
    {
        return PathBuf::from(env_d);
    }
    let is_test_runner = std::env::var("PANEL_TEST_MODE").is_ok()
        || std::env::var("RUST_TEST_THREADS").is_ok()
        || std::env::current_exe()
            .map(|p| {
                let s = p.to_string_lossy();
                s.contains("/deps/")
                    || s.contains("\\deps\\")
                    || p.file_name()
                        .map(|n| n.to_string_lossy().contains("test"))
                        .unwrap_or(false)
            })
            .unwrap_or(false);

    if is_test_runner {
        return std::env::temp_dir().join("panel_nginx_test");
    }

    PathBuf::from("/etc/panel/nginx")
}

/// The production installer retains the distribution's main configuration.
/// Alternate roots are standalone fixtures/development installations.
pub(crate) fn nginx_main_config_path(base_dir: &Path) -> PathBuf {
    if base_dir == Path::new("/etc/panel/nginx") {
        PathBuf::from("/etc/nginx/nginx.conf")
    } else {
        base_dir.join("nginx.conf")
    }
}

pub struct NginxManager {
    runner: Arc<dyn SystemRunner>,
    debouncer: ReloadDebouncer,
    base_dir: Option<PathBuf>,
    operation_lock: Mutex<()>,
}

impl NginxManager {
    pub fn panel_tls(
        &self,
        request: panel_proto::PanelTlsRequest,
    ) -> Result<panel_proto::PanelTlsStatus, AgentError> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| AgentError::Internal("Kunci Nginx gagal.".into()))?;
        crate::panel_tls::execute(self.runner.as_ref(), request)
    }
    pub fn new(runner: Arc<dyn SystemRunner>) -> Self {
        Self {
            runner,
            debouncer: ReloadDebouncer::default(),
            base_dir: None,
            operation_lock: Mutex::new(()),
        }
    }

    pub fn with_base_dir(runner: Arc<dyn SystemRunner>, base_dir: impl Into<PathBuf>) -> Self {
        Self {
            runner,
            debouncer: ReloadDebouncer::default(),
            base_dir: Some(base_dir.into()),
            operation_lock: Mutex::new(()),
        }
    }

    pub fn runner(&self) -> &Arc<dyn SystemRunner> {
        &self.runner
    }

    fn check_config(&self, config_path: &Path) -> Result<(), AgentError> {
        let path = config_path.to_string_lossy();
        let output = self.runner.run_command("nginx", &["-t", "-c", &path])?;
        if !output.status.success() {
            return Err(AgentError::InvalidInput(format!(
                "Validasi Nginx gagal: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(())
    }

    fn is_packaged_default(&self, path: &Path) -> Result<bool, AgentError> {
        use md5::{Digest, Md5};
        if self.runner.read_link(path).is_ok() {
            return Ok(false);
        }
        let bytes = self.runner.read_file(path)?;
        let digest = format!("{:x}", Md5::digest(bytes));
        for package in ["nginx-common", "nginx"] {
            let Ok(metadata) = self
                .runner
                .run_command("dpkg-query", &["-W", "-f=${Conffiles}\n", package])
            else {
                continue;
            };
            if String::from_utf8_lossy(&metadata.stdout)
                .lines()
                .any(|line| {
                    let fields: Vec<_> = line.split_whitespace().collect();
                    fields.len() == 2 && fields[0] == path.to_string_lossy() && fields[1] == digest
                })
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn packaged_default_paths(&self) -> Result<Vec<PathBuf>, AgentError> {
        let mut paths = Vec::new();
        let enabled = Path::new("/etc/nginx/sites-enabled/default");
        if self.runner.path_exists(enabled) {
            let stock = match self.runner.read_link(enabled) {
                Ok(target)
                    if [
                        Path::new("/etc/nginx/sites-available/default"),
                        Path::new("../sites-available/default"),
                    ]
                    .contains(&target.as_path()) =>
                {
                    self.is_packaged_default(Path::new("/etc/nginx/sites-available/default"))?
                }
                _ => false,
            };
            if !stock {
                return Err(AgentError::InvalidInput("Vhost default Nginx telah diubah atau checksum paket tidak tersedia. Berkas dipertahankan; nonaktifkan default tersebut secara eksplisit sebelum memasang penolak hostname.".into()));
            }
            paths.push(enabled.to_path_buf());
        }
        let default = Path::new("/etc/nginx/conf.d/default.conf");
        if self.runner.path_exists(default) {
            if !self.is_packaged_default(default)? {
                return Err(AgentError::InvalidInput("conf.d/default.conf bukan default paket terverifikasi. Berkas custom dipertahankan; periksa konfigurasi default Nginx.".into()));
            }
            paths.push(default.to_path_buf());
        }
        Ok(paths)
    }

    /// Install missing dependencies and the unknown-host guard, then publish the include last.
    /// All paths originate in the agent, never from an API-supplied host path.
    fn ensure_host_baseline(
        &self,
        config: &NginxBaselineConfig,
        main_conf: &Path,
        bridge: &Path,
    ) -> Result<(), AgentError> {
        let main = self.runner.read_file(main_conf).map_err(|error| {
            AgentError::InvalidInput(format!(
                "Konfigurasi utama Nginx {} tidak dapat dibaca: {error}",
                main_conf.display()
            ))
        })?;
        let main = String::from_utf8(main)
            .map_err(|_| AgentError::InvalidInput("Konfigurasi Nginx bukan UTF-8".into()))?;
        let directives = main
            .lines()
            .map(|line| line.split('#').next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(" ");
        let tokens: Vec<_> = directives.split_whitespace().collect();
        if !tokens
            .windows(2)
            .any(|pair| pair == ["include", "/etc/nginx/conf.d/*.conf;"])
        {
            return Err(AgentError::InvalidInput(
                "Nginx utama tidak memuat include /etc/nginx/conf.d/*.conf; di blok http. Konfigurasi custom dipertahankan; aktifkan include tersebut sebelum membuat situs.".into(),
            ));
        }

        let http_conf = config.base_dir.join("http.conf");
        let bridge_content = format!(
            "# Managed by QonPanel: tenant websites.\ninclude {};\n",
            http_conf.to_string_lossy().replace('\\', "/")
        );
        let legacy_bridge =
            bridge_content.replacen("Managed by QonPanel", "Managed by Nusa Panel", 1);
        if self.runner.path_exists(bridge) && {
            let existing = self.runner.read_file(bridge)?;
            existing != bridge_content.as_bytes()
                && existing != legacy_bridge.as_bytes()
                && existing
                    != bridge_content
                        .replacen("Managed by QonPanel", "Managed by Qpanel", 1)
                        .as_bytes()
        } {
            return Err(AgentError::InvalidInput(format!(
                "{} sudah ada dengan isi berbeda; periksa integrasi Nginx panel. Berkas tidak ditimpa.",
                bridge.display()
            )));
        }

        let reject_path = Path::new("/etc/nginx/conf.d/00-qpanel-reject-unknown.conf");
        let reject_content = config.render_default_catchall();
        if self.runner.read_link(reject_path).is_ok() {
            return Err(AgentError::InvalidInput("Guard hostname berupa symlink custom; berkas dipertahankan. Gunakan konfigurasi root-owned reguler.".into()));
        }
        if self.runner.path_exists(reject_path)
            && !managed_brand_equivalent(&self.runner.read_file(reject_path)?, &reject_content)
        {
            return Err(AgentError::InvalidInput("Konfigurasi penolak hostname Qinpanel berisi perubahan operator; periksa berkas 00-qpanel-reject-unknown.conf. Berkas tidak ditimpa.".into()));
        }
        let defaults = self.packaged_default_paths()?;

        for dir in [
            config.base_dir.clone(),
            config.conf_d_dir(),
            config.snippets_dir(),
            config.sites_available_dir(),
            config.sites_enabled_dir(),
            config.log_dir.clone(),
            config.acme_dir.clone(),
            config.cache_dir.clone(),
        ] {
            if !self.runner.path_exists(&dir) {
                self.runner.create_dir_all(&dir, 0o755, 0, 0)?;
            }
        }

        let files = [
            (reject_path.to_path_buf(), reject_content),
            (
                config.snippets_dir().join("acme.conf"),
                config.render_acme_snippet(),
            ),
            (
                config.snippets_dir().join("security-headers.conf"),
                config.render_security_headers_snippet(),
            ),
            (
                config.snippets_dir().join("deny-sensitive.conf"),
                config.render_deny_sensitive_snippet(),
            ),
            (
                config.snippets_dir().join("fastcgi-params.conf"),
                config.render_fastcgi_params_snippet(),
            ),
            (http_conf, config.render_host_http_conf()),
            (bridge.to_path_buf(), bridge_content),
        ];
        let mut created = Vec::new();
        let mut disabled = Vec::new();
        let result = (|| {
            for path in defaults {
                let quarantine = if path == Path::new("/etc/nginx/sites-enabled/default") {
                    PathBuf::from("/etc/nginx/sites-available")
                        .join(format!("default.qpanel-disabled-{}", uuid::Uuid::new_v4()))
                } else {
                    path.with_extension(format!("conf.qpanel-disabled-{}", uuid::Uuid::new_v4()))
                };
                self.runner.move_path(&path, &quarantine)?;
                disabled.push((path, quarantine));
            }
            for (path, content) in files {
                if self.runner.path_exists(&path) {
                    continue;
                }
                let staging = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
                let publish = (|| {
                    self.runner
                        .write_file(&staging, content.as_bytes(), 0o644)?;
                    self.runner.set_ownership(&staging, 0, 0)?;
                    self.runner.move_path(&staging, &path)
                })();
                if publish.is_err() {
                    let _ = self.runner.remove_file(&staging);
                }
                publish?;
                created.push(path);
            }
            if !created.is_empty() || !disabled.is_empty() {
                self.check_config(main_conf)?;
            }
            Ok::<(), AgentError>(())
        })();
        if result.is_err() {
            for path in created.iter().rev() {
                if let Err(error) = self.runner.remove_file(path) {
                    warn!(%error, "Nginx guard rollback cleanup failed");
                }
            }
            for (path, quarantine) in disabled.iter().rev() {
                self.runner.move_path(quarantine, path)?;
            }
        }
        result
    }

    fn prepare_site_config(&self, base_dir: &Path) -> Result<PathBuf, AgentError> {
        if base_dir == Path::new("/etc/panel/nginx") {
            self.ensure_host_baseline(
                &NginxBaselineConfig::default(),
                Path::new("/etc/nginx/nginx.conf"),
                Path::new("/etc/nginx/conf.d/00-nusa-panel-sites.conf"),
            )?;
        }
        Ok(nginx_main_config_path(base_dir))
    }

    fn prepare_site_logs(&self, context: &panel_proto::SiteLogContext) -> Result<(), AgentError> {
        let username = panel_core::Username::parse(&context.username)
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let domain = panel_core::domain::DomainName::parse(&context.domain)
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let base = Path::new("/var/log/panel");
        let tenant_dir = base.join(username.as_str());
        let site_dir = tenant_dir.join(domain.as_ascii());
        for (path, mode) in [
            (base, 0o755),
            (tenant_dir.as_path(), 0o750),
            (site_dir.as_path(), 0o750),
        ] {
            if !self.runner.path_exists(path) {
                self.runner.create_dir_all(path, mode, 0, 0)?;
            }
        }
        // Strict ACL application: failure must stop apply before nginx -t. The
        // default ACL gives new/rotated logs tenant read access without truncation.
        for path in [&tenant_dir, &site_dir] {
            let acl = if path == &site_dir {
                format!("u:{}:r-x,d:u:{}:r-x", username.as_str(), username.as_str())
            } else {
                format!("u:{}:r-x", username.as_str())
            };
            self.runner
                .run_command("setfacl", &["-m", &acl, &path.to_string_lossy()])?;
        }
        Ok(())
    }

    fn build_config(&self, req: &NginxBaselineApplyRequest) -> NginxBaselineConfig {
        let base_dir = req
            .base_dir
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| self.base_dir.clone())
            .unwrap_or_else(|| get_nginx_base_dir(None));
        let log_dir = req
            .log_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/log/panel"));
        let acme_dir = req
            .acme_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/lib/panel/acme"));

        let cache_dir = if base_dir == std::path::Path::new("/etc/panel/nginx") {
            PathBuf::from("/var/cache/nginx/panel")
        } else {
            base_dir.join("cache")
        };

        NginxBaselineConfig {
            base_dir,
            log_dir,
            acme_dir,
            cache_dir,
            panel_api_port: req
                .panel_api_port
                .unwrap_or(panel_core::production::INTERNAL_API_PORT),
            panel_listen_port: req
                .panel_listen_port
                .unwrap_or(panel_core::production::PUBLIC_PANEL_PORT),
            worker_processes: "auto".to_string(),
            worker_connections: 4096,
        }
    }

    fn test_nginx_syntax(&self, config: &NginxBaselineConfig) -> bool {
        let conf_path_str = nginx_main_config_path(&config.base_dir)
            .to_string_lossy()
            .to_string();
        match self.check_config(&nginx_main_config_path(&config.base_dir)) {
            Ok(_) => {
                info!(
                    "Nginx baseline configuration test passed (-t -c {})",
                    conf_path_str
                );
                true
            }
            Err(e) => {
                warn!("Nginx test failed or binary missing: {e}");
                false
            }
        }
    }

    fn reload_nginx(&self) -> bool {
        if !self.debouncer.should_reload() {
            info!("Nginx reload debounced (coalesced within 500ms)");
            return true;
        }
        match self.runner.run_command("systemctl", &["reload", "nginx"]) {
            Ok(_) => {
                info!("Nginx reloaded successfully via systemctl");
                true
            }
            Err(e) => {
                warn!("systemctl reload nginx failed or not supported in test environment: {e}");
                false
            }
        }
    }

    /// Applies the baseline Nginx configuration and directory structure.
    pub fn apply_baseline(
        &self,
        req: NginxBaselineApplyRequest,
    ) -> Result<NginxBaselineAppliedResponse, AgentError> {
        let _operation = self
            .operation_lock
            .lock()
            .map_err(|_| AgentError::Internal("Kunci konfigurasi Nginx gagal".into()))?;
        let config = self.build_config(&req);
        info!("Installing Nginx baseline to {}", config.base_dir.display());
        if config.base_dir == Path::new("/etc/panel/nginx") {
            self.ensure_host_baseline(
                &config,
                Path::new("/etc/nginx/nginx.conf"),
                Path::new("/etc/nginx/conf.d/00-nusa-panel-sites.conf"),
            )?;
        } else {
            config.install_baseline().map_err(|e| {
                AgentError::InvalidInput(format!("Failed to install Nginx baseline: {e}"))
            })?;
        }

        let test_passed = if req.test_config {
            self.test_nginx_syntax(&config)
        } else {
            true
        };

        let reloaded = if req.reload && test_passed {
            self.reload_nginx()
        } else {
            false
        };

        Ok(NginxBaselineAppliedResponse {
            base_dir: config.base_dir.to_string_lossy().to_string(),
            nginx_conf_path: nginx_main_config_path(&config.base_dir)
                .to_string_lossy()
                .to_string(),
            conf_d_dir: config.conf_d_dir().to_string_lossy().to_string(),
            snippets_dir: config.snippets_dir().to_string_lossy().to_string(),
            sites_available_dir: config.sites_available_dir().to_string_lossy().to_string(),
            sites_enabled_dir: config.sites_enabled_dir().to_string_lossy().to_string(),
            log_dir: config.log_dir.to_string_lossy().to_string(),
            acme_dir: config.acme_dir.to_string_lossy().to_string(),
            test_passed,
            reloaded,
        })
    }

    /// Applies a virtual host configuration atomically with syntax verification and automatic rollback.
    pub fn apply_site(&self, req: SiteApplyRequest) -> Result<SiteAppliedResponse, AgentError> {
        let _operation = self
            .operation_lock
            .lock()
            .map_err(|_| AgentError::Internal("Kunci konfigurasi Nginx gagal".into()))?;
        if let Some(context) = &req.log_context {
            self.prepare_site_logs(context).map_err(|error| {
                AgentError::InvalidInput(format!(
                    "Persiapan direktori log situs gagal sebelum validasi Nginx: {error}"
                ))
            })?;
        }
        let base_dir = req
            .nginx_base_dir
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| self.base_dir.clone())
            .unwrap_or_else(|| get_nginx_base_dir(None));
        let pipeline = NginxApplyPipeline::new(&base_dir);
        let nginx_conf_path = self.prepare_site_config(&base_dir)?;
        let runner = Arc::clone(&self.runner);

        let conf_path_str = nginx_conf_path.to_string_lossy().to_string();
        let syntax_check = move || -> Result<(), String> {
            match runner.run_command("nginx", &["-t", "-c", &conf_path_str]) {
                Ok(out) => {
                    if out.status.success() {
                        Ok(())
                    } else {
                        Err(String::from_utf8_lossy(&out.stderr).to_string())
                    }
                }
                Err(e) => Err(e.to_string()),
            }
        };

        let result = if req.config_content.is_empty() {
            if req.enable {
                if req.test_config {
                    pipeline.enable_site(&req.site_id, syntax_check)
                } else {
                    pipeline.enable_site(&req.site_id, || Ok(()))
                }
            } else if req.test_config {
                pipeline.disable_site(&req.site_id, syntax_check)
            } else {
                pipeline.disable_site(&req.site_id, || Ok(()))
            }
        } else if req.test_config {
            pipeline.apply_site_config(&req.site_id, &req.config_content, req.enable, syntax_check)
        } else {
            pipeline.apply_site_config(&req.site_id, &req.config_content, req.enable, || Ok(()))
        };

        let apply_res = result.map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        let reloaded = if req.reload {
            self.reload_nginx()
        } else {
            false
        };

        Ok(SiteAppliedResponse {
            site_id: apply_res.site_id,
            available_path: apply_res.available_path.to_string_lossy().to_string(),
            enabled_path: apply_res
                .enabled_path
                .map(|p| p.to_string_lossy().to_string()),
            enabled: apply_res.enabled,
            reloaded,
        })
    }

    /// Removes a virtual host configuration with automatic rollback on syntax failure.
    pub fn remove_site(&self, req: SiteRemoveRequest) -> Result<SiteRemovedResponse, AgentError> {
        let _operation = self
            .operation_lock
            .lock()
            .map_err(|_| AgentError::Internal("Kunci konfigurasi Nginx gagal".into()))?;
        let base_dir = req
            .nginx_base_dir
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| self.base_dir.clone())
            .unwrap_or_else(|| get_nginx_base_dir(None));
        let pipeline = NginxApplyPipeline::new(&base_dir);
        let nginx_conf_path = self.prepare_site_config(&base_dir)?;
        let runner = Arc::clone(&self.runner);

        let conf_path_str = nginx_conf_path.to_string_lossy().to_string();
        let syntax_check = move || -> Result<(), String> {
            match runner.run_command("nginx", &["-t", "-c", &conf_path_str]) {
                Ok(out) => {
                    if out.status.success() {
                        Ok(())
                    } else {
                        Err(String::from_utf8_lossy(&out.stderr).to_string())
                    }
                }
                Err(e) => Err(e.to_string()),
            }
        };

        pipeline
            .remove_site(&req.site_id, syntax_check)
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        let reloaded = if req.reload {
            self.reload_nginx()
        } else {
            false
        };

        Ok(SiteRemovedResponse {
            site_id: req.site_id,
            reloaded,
        })
    }

    /// Provisions docroot directory, sets permissions, and writes modern placeholder index.html.
    pub fn provision_site(
        &self,
        req: SiteProvisionRequest,
    ) -> Result<SiteProvisionResponse, AgentError> {
        let username = panel_core::Username::parse(&req.username)
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let uid = panel_core::tenant::TenantUid::parse(req.uid)
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let uid_val = uid.as_u32();

        let web_user = req.web_user.as_deref().unwrap_or("www-data");
        if !matches!(web_user, "www-data" | "nginx") {
            return Err(AgentError::InvalidInput(
                "User worker Nginx harus www-data atau nginx".into(),
            ));
        }
        let directories = site_provision_directories(&req, username.as_str())?;
        let docroot_path = PathBuf::from(&req.docroot);

        // Reject pre-existing symlink components before changing directories/ACLs.
        if let Some(filename) = req.starter.filename() {
            self.check_site_symlinks(&docroot_path.join(filename))?;
        }
        for (index, path) in directories.iter().enumerate() {
            if !self.runner.path_exists(path) {
                self.runner.create_dir_all(
                    path,
                    if index == 0 { 0o700 } else { 0o750 },
                    uid_val,
                    uid_val,
                )?;
            }
        }

        // Every ancestor needs search permission, including the 0750 sites dir.
        // Apply directly: the legacy set_facl helper ignores command failures.
        for path in &directories {
            self.check_site_symlinks(path)?;
            let acl = if path == &docroot_path {
                format!("u:{web_user}:r-x,d:u:{web_user}:r-x")
            } else {
                format!("u:{web_user}:--x")
            };
            let path_arg = path.to_string_lossy();
            let output = self
                .runner
                .run_command(
                    "runuser",
                    &[
                        "-u",
                        username.as_str(),
                        "--",
                        "/usr/bin/setfacl",
                        "-P",
                        "-m",
                        &acl,
                        "--",
                        &path_arg,
                    ],
                )
                .map_err(|error| {
                    AgentError::InvalidInput(format!(
                        "ACL Nginx gagal pada {}: {error}",
                        path.display()
                    ))
                })?;
            if !output.status.success() {
                return Err(AgentError::InvalidInput(format!(
                    "ACL Nginx gagal pada {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
        }

        // Fixed destinations per runtime. Existing project files are never replaced.
        let mut files = Vec::new();
        if let Some(filename) = req.starter.filename() {
            files.push((
                filename,
                req.index_content.clone().unwrap_or_else(|| {
                    panel_webserver::template::render_placeholder_html(&req.username)
                }),
            ));
        }
        if req.starter == panel_proto::SiteStarter::Nodejs {
            files.push((
                "package.json",
                "{\"private\":true,\"scripts\":{\"start\":\"node server.js\"}}\n".into(),
            ));
        }
        if req.starter == panel_proto::SiteStarter::PythonDjango {
            files.push(("myproject/__init__.py", String::new()));
        }
        let mut placeholder_written = false;
        for (filename, content) in files {
            let path = docroot_path.join(filename);
            self.check_site_symlinks(&path)?;
            if self.runner.write_file_if_absent(
                &path,
                content.as_bytes(),
                0o644,
                uid_val,
                uid_val,
            )? {
                placeholder_written = true;
            }
        }

        info!(
            "Site docroot provisioned at {} for tenant {} (UID {})",
            docroot_path.display(),
            username.as_str(),
            uid_val
        );

        Ok(SiteProvisionResponse {
            docroot: req.docroot,
            placeholder_written,
            acls_applied: true,
        })
    }

    fn check_site_symlinks(&self, path: &Path) -> Result<(), AgentError> {
        for component in path.ancestors() {
            if self.runner.read_link(component).is_ok() {
                return Err(AgentError::SecurityViolation(format!(
                    "Symlink tidak diizinkan pada jalur situs: {}",
                    component.display()
                )));
            }
        }
        Ok(())
    }

    /// Deploys an ACME HTTP-01 challenge response file.
    pub fn deploy_acme_challenge(
        &self,
        req: AcmeChallengeDeployRequest,
    ) -> Result<AcmeChallengeDeployedResponse, AgentError> {
        if req.token.is_empty()
            || !req
                .token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(AgentError::SecurityViolation(format!(
                "Invalid ACME challenge token: '{}'",
                req.token
            )));
        }

        let acme_dir = req
            .acme_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/www/acme-challenge"));

        let challenge_dir = acme_dir.join(".well-known").join("acme-challenge");
        self.runner.create_dir_all(&challenge_dir, 0o755, 0, 0)?;

        let challenge_path = challenge_dir.join(&req.token);
        self.runner
            .write_file(&challenge_path, req.key_authorization.as_bytes(), 0o644)?;

        info!(
            "ACME HTTP-01 challenge deployed at {}",
            challenge_path.display()
        );

        Ok(AcmeChallengeDeployedResponse {
            challenge_path: challenge_path.to_string_lossy().to_string(),
        })
    }

    /// Cleans up an ACME HTTP-01 challenge file.
    pub fn cleanup_acme_challenge(
        &self,
        req: AcmeChallengeCleanupRequest,
    ) -> Result<AcmeChallengeCleanedResponse, AgentError> {
        if req.token.is_empty()
            || !req
                .token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(AgentError::SecurityViolation(format!(
                "Invalid ACME challenge token: '{}'",
                req.token
            )));
        }

        let acme_dir = req
            .acme_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/www/acme-challenge"));

        let challenge_path = acme_dir
            .join(".well-known")
            .join("acme-challenge")
            .join(&req.token);

        let removed = if self.runner.path_exists(&challenge_path) {
            self.runner.remove_file(&challenge_path)?;
            true
        } else {
            false
        };

        info!("ACME HTTP-01 challenge cleaned for token {}", req.token);

        Ok(AcmeChallengeCleanedResponse {
            token: req.token,
            removed,
        })
    }

    /// Deploys SSL certificate and private key with strict 0600 mode for the private key.
    pub fn deploy_ssl_cert(
        &self,
        req: SslCertDeployRequest,
    ) -> Result<SslCertDeployedResponse, AgentError> {
        let domain = panel_core::domain::DomainName::parse(&req.domain)
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        let ssl_base_dir = req
            .ssl_base_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/panel/ssl"));

        let cert_dir = ssl_base_dir.join(domain.as_ascii());
        // Directory 0700 for root
        self.runner.create_dir_all(&cert_dir, 0o700, 0, 0)?;

        let cert_path = cert_dir.join("fullchain.pem");
        let key_path = cert_dir.join("privkey.pem");

        // Write fullchain (mode 0644)
        self.runner
            .write_file(&cert_path, req.fullchain_pem.as_bytes(), 0o644)?;

        // Write private key with strict 0600 mode as required by PRD!
        self.runner
            .write_file(&key_path, req.privkey_pem.as_bytes(), 0o600)?;
        crate::platform::relabel(self.runner.as_ref(), &cert_dir)?;

        info!(
            "SSL certificate deployed for domain {} at {}",
            domain.as_unicode(),
            cert_dir.display()
        );

        crate::email::certificate_changed(self.runner.as_ref(), domain.as_ascii())?;
        Ok(SslCertDeployedResponse {
            domain: domain.as_unicode().to_string(),
            cert_path: cert_path.to_string_lossy().to_string(),
            key_path: key_path.to_string_lossy().to_string(),
        })
    }

    /// Deploys an htpasswd basic authentication file with strict 0640 permissions.
    pub fn deploy_site_htpasswd(
        &self,
        req: SiteHtpasswdDeployRequest,
    ) -> Result<SiteHtpasswdDeployedResponse, AgentError> {
        if req.site_id.is_empty()
            || !req
                .site_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(AgentError::SecurityViolation(format!(
                "Invalid site_id for htpasswd: '{}'",
                req.site_id
            )));
        }

        let auth_base_dir = req
            .auth_base_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/panel/nginx/auth"));

        // Directory 0750
        self.runner.create_dir_all(&auth_base_dir, 0o750, 0, 0)?;

        let htpasswd_filename = format!("site_{}.htpasswd", req.site_id);
        let htpasswd_path = auth_base_dir.join(&htpasswd_filename);

        // Mode 0640 as required by PRD
        self.runner
            .write_file(&htpasswd_path, req.htpasswd_content.as_bytes(), 0o640)?;

        info!("Site htpasswd file deployed at {}", htpasswd_path.display());

        Ok(SiteHtpasswdDeployedResponse {
            site_id: req.site_id,
            htpasswd_path: htpasswd_path.to_string_lossy().to_string(),
        })
    }

    /// Removes an htpasswd basic authentication file for a site.
    pub fn remove_site_htpasswd(
        &self,
        req: SiteHtpasswdRemoveRequest,
    ) -> Result<SiteHtpasswdRemovedResponse, AgentError> {
        if req.site_id.is_empty()
            || !req
                .site_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(AgentError::SecurityViolation(format!(
                "Invalid site_id for htpasswd removal: '{}'",
                req.site_id
            )));
        }

        let auth_base_dir = req
            .auth_base_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/panel/nginx/auth"));

        let htpasswd_filename = format!("site_{}.htpasswd", req.site_id);
        let htpasswd_path = auth_base_dir.join(&htpasswd_filename);

        let removed = if self.runner.path_exists(&htpasswd_path) {
            self.runner.remove_file(&htpasswd_path)?;
            true
        } else {
            false
        };

        info!("Site htpasswd file removed for site {}", req.site_id);

        Ok(SiteHtpasswdRemovedResponse {
            site_id: req.site_id,
            removed,
        })
    }

    /// Safely reads access or error logs for a site with pagination/tail limit and query filter.
    pub fn read_site_logs(
        &self,
        req: SiteLogsReadRequest,
    ) -> Result<SiteLogsReadResponse, AgentError> {
        let username = panel_core::Username::parse(&req.username)
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let domain = panel_core::domain::DomainName::parse(&req.site_domain)
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        let is_app_log = req.log_type == "app"
            || req.log_type == "process"
            || req.log_type == "service"
            || req.log_type == "node"
            || req.log_type == "python";

        if req.log_type != "access"
            && req.log_type != "error"
            && req.log_type != "php_error"
            && req.log_type != "php_slow"
            && !is_app_log
        {
            return Err(AgentError::InvalidInput(format!(
                "Invalid log type '{}'. Expected 'access', 'error', 'php_error', 'php_slow', or 'app'",
                req.log_type
            )));
        }

        let base_dir = req
            .log_base_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/log/panel"));

        let log_file = match req.log_type.as_str() {
            "php_error" => {
                let site_php = base_dir
                    .join(username.as_str())
                    .join(domain.as_ascii())
                    .join("php-error.log");
                if self.runner.path_exists(&site_php) {
                    site_php
                } else {
                    find_php_log_file(
                        self.runner.as_ref(),
                        &base_dir.join(username.as_str()),
                        "error",
                    )
                    .unwrap_or(site_php)
                }
            }
            "php_slow" => {
                let site_php = base_dir
                    .join(username.as_str())
                    .join(domain.as_ascii())
                    .join("php-slow.log");
                if self.runner.path_exists(&site_php) {
                    site_php
                } else {
                    find_php_log_file(
                        self.runner.as_ref(),
                        &base_dir.join(username.as_str()),
                        "slow",
                    )
                    .unwrap_or(site_php)
                }
            }
            "app" | "process" | "service" | "node" | "python" => base_dir
                .join(username.as_str())
                .join(domain.as_ascii())
                .join("app.log"),
            _ => base_dir
                .join(username.as_str())
                .join(domain.as_ascii())
                .join(format!("{}.log", req.log_type)),
        };

        if !self.runner.path_exists(&log_file) {
            if let (true, Some(sid)) = (is_app_log, req.site_id.as_deref()) {
                let clean_id = sid.strip_prefix("site_").unwrap_or(sid);
                let service_name = format!("site-{clean_id}.service");
                let lines_limit = req.lines.clamp(1, 1000);
                let lines_str = lines_limit.to_string();
                if let Ok(output) = self.runner.run_command(
                    "journalctl",
                    &[
                        "-u",
                        &service_name,
                        "-n",
                        &lines_str,
                        "--no-pager",
                        "-o",
                        "cat",
                    ],
                ) {
                    let out_str = String::from_utf8_lossy(&output.stdout);
                    let query_lower = req.query.as_ref().map(|q| q.to_lowercase());
                    let matched_lines: Vec<String> = out_str
                        .lines()
                        .filter(|l| {
                            if let Some(ref q) = query_lower {
                                l.to_lowercase().contains(q)
                            } else {
                                true
                            }
                        })
                        .map(|s| s.to_string())
                        .collect();
                    let total = matched_lines.len();
                    return Ok(SiteLogsReadResponse {
                        lines: matched_lines,
                        total_matched: total,
                    });
                }
            }

            return Ok(SiteLogsReadResponse {
                lines: Vec::new(),
                total_matched: 0,
            });
        }

        let content_bytes = self.runner.read_file(&log_file)?;
        let content_str = String::from_utf8_lossy(&content_bytes);

        let lines_limit = req.lines.clamp(1, 1000);
        let query_lower = req.query.as_ref().map(|q| q.to_lowercase());

        let all_matching: Vec<String> = content_str
            .lines()
            .filter(|line| {
                if let Some(ref q) = query_lower {
                    line.to_lowercase().contains(q)
                } else {
                    true
                }
            })
            .map(|s| s.to_string())
            .collect();

        let total_matched = all_matching.len();
        let start_idx = total_matched.saturating_sub(lines_limit);
        let result_lines = all_matching[start_idx..].to_vec();

        Ok(SiteLogsReadResponse {
            lines: result_lines,
            total_matched,
        })
    }
}

fn site_provision_directories(
    req: &SiteProvisionRequest,
    username: &str,
) -> Result<Vec<PathBuf>, AgentError> {
    let home = PathBuf::from(format!("/home/{username}"));
    let sites = home.join("sites");
    let docroot = Path::new(&req.docroot);
    if req.home_dir != format!("/home/{username}")
        || req.docroot.contains('\\')
        || req.docroot.chars().any(char::is_control)
        || req
            .docroot
            .split('/')
            .any(|component| matches!(component, "." | ".."))
    {
        return Err(AgentError::SecurityViolation(
            "Jalur situs tidak valid untuk tenant".into(),
        ));
    }
    let relative = docroot.strip_prefix(&sites).map_err(|_| {
        AgentError::SecurityViolation("Docroot harus berada di direktori sites milik tenant".into())
    })?;
    if relative.as_os_str().is_empty() {
        return Err(AgentError::SecurityViolation(
            "Docroot tidak boleh berupa direktori sites tenant".into(),
        ));
    }
    let mut directories = vec![home, sites.clone()];
    let mut current = sites;
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(AgentError::SecurityViolation(
                "Komponen jalur situs tidak valid".into(),
            ));
        }
        current.push(component);
        directories.push(current.clone());
    }
    Ok(directories)
}

fn find_php_log_file(runner: &dyn SystemRunner, tenant_dir: &Path, kind: &str) -> Option<PathBuf> {
    let suffix = format!("-{kind}.log");
    if let Ok(entries) = runner.read_dir(tenant_dir) {
        for entry in entries {
            let matches = entry
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with("php-fpm-") && name.ends_with(&suffix));
            if matches {
                return Some(entry);
            }
        }
    }
    None
}

// Only the controlled first-line brand may differ; operator edits stay protected.
fn managed_brand_equivalent(existing: &[u8], expected: &str) -> bool {
    existing == expected.as_bytes()
        || ["Qpanel", "Nusa Panel"].iter().any(|brand| {
            existing
                == expected
                    .replacen(
                        "# Managed by QonPanel:",
                        &format!("# Managed by {brand}:"),
                        1,
                    )
                    .as_bytes()
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    use tempfile::TempDir;

    fn provision_request(docroot: &str) -> SiteProvisionRequest {
        SiteProvisionRequest {
            site_id: 1,
            username: "p_alice".into(),
            uid: 10001,
            home_dir: "/home/p_alice".into(),
            docroot: docroot.into(),
            web_user: Some("www-data".into()),
            index_content: Some("website placeholder".into()),
            starter: panel_proto::SiteStarter::Html,
        }
    }

    #[test]
    fn brand_marker_accepts_only_exact_legacy_guard_content() {
        let current = "# Managed by QonPanel: guard.\nserver { return 404; }\n";
        for brand in ["Qpanel", "Nusa Panel", "QonPanel"] {
            let old = current.replace("QonPanel", brand);
            assert!(managed_brand_equivalent(old.as_bytes(), current));
            assert!(!managed_brand_equivalent(
                format!("{old}# custom\n").as_bytes(),
                current
            ));
        }
    }

    #[test]
    fn starters_use_language_filenames_and_preserve_reused_project_files() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(runner.clone());
        let root = "/home/p_alice/sites/reused.test/app";
        let mut request = provision_request(root);
        manager.provision_site(request.clone()).unwrap();
        request.starter = panel_proto::SiteStarter::Nodejs;
        request.index_content = Some("const http = require('http');".into());
        manager.provision_site(request.clone()).unwrap();
        assert_eq!(
            runner
                .read_file(&Path::new(root).join("index.html"))
                .unwrap(),
            b"website placeholder"
        );
        assert!(runner.path_exists(&Path::new(root).join("server.js")));
        assert!(runner.path_exists(&Path::new(root).join("package.json")));
        runner.set_file_content(Path::new(root).join("server.js"), b"user-owned code");
        manager.provision_site(request.clone()).unwrap();
        assert_eq!(
            runner
                .read_file(&Path::new(root).join("server.js"))
                .unwrap(),
            b"user-owned code"
        );
        request.starter = panel_proto::SiteStarter::PythonDjango;
        request.index_content =
            Some("def app(environ, start_response): pass\napplication = app".into());
        manager.provision_site(request).unwrap();
        assert!(runner.path_exists(&Path::new(root).join("myproject/wsgi.py")));
        assert!(runner.path_exists(&Path::new(root).join("myproject/__init__.py")));
    }

    #[test]
    fn provision_grants_search_on_every_ancestor_before_writing_placeholder() {
        for (docroot, expected) in [
            (
                "/home/p_alice/sites/example.test/public",
                vec![
                    "/home/p_alice",
                    "/home/p_alice/sites",
                    "/home/p_alice/sites/example.test",
                    "/home/p_alice/sites/example.test/public",
                ],
            ),
            (
                "/home/p_alice/sites/example.test/app/public",
                vec![
                    "/home/p_alice",
                    "/home/p_alice/sites",
                    "/home/p_alice/sites/example.test",
                    "/home/p_alice/sites/example.test/app",
                    "/home/p_alice/sites/example.test/app/public",
                ],
            ),
        ] {
            let runner = Arc::new(MockSystemRunner::new());
            let manager = NginxManager::new(runner.clone());
            let request = provision_request(docroot);
            let result = manager.provision_site(request.clone()).unwrap();
            assert!(result.acls_applied && result.placeholder_written);
            let actions = runner.recorded_actions();
            let index = actions.iter().position(|action| matches!(action,
                crate::system::MockAction::WriteFile { path, .. } if path.ends_with("index.html")
            )).unwrap();
            for expected_path in expected {
                let path = Path::new(expected_path);
                let acl = if path == Path::new(docroot) {
                    "u:www-data:r-x,d:u:www-data:r-x"
                } else {
                    "u:www-data:--x"
                };
                let position = actions.iter().position(|action| matches!(action,
                    crate::system::MockAction::Command { binary, args }
                    if binary == "runuser" && args.len() == 9 && args[..8] == ["-u", "p_alice", "--", "/usr/bin/setfacl", "-P", "-m", acl, "--"] && Path::new(&args[8]) == path
                )).unwrap_or_else(|| panic!("missing ACL for {}", path.display()));
                assert!(position < index);
            }
            assert!(actions.iter().any(|action| matches!(action,
                crate::system::MockAction::CreateDir { path, mode: 0o700, uid: 10001, gid: 10001 }
                if path == Path::new("/home/p_alice")
            )));
            assert!(
                !actions
                    .iter()
                    .any(|action| matches!(action, crate::system::MockAction::SetFacl { .. })),
                "must not use the helper that suppresses failures"
            );
        }
    }

    #[test]
    fn provision_preserves_existing_directory_modes_owners_and_index() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(runner.clone());
        let request = provision_request("/home/p_alice/sites/example.test/public");
        for path in site_provision_directories(&request, "p_alice").unwrap() {
            runner.add_existing_path(path);
        }
        let index = Path::new(&request.docroot).join("index.html");
        runner.set_file_content(&index, b"tenant website");
        for _ in 0..2 {
            let result = manager.provision_site(request.clone()).unwrap();
            assert!(result.acls_applied && !result.placeholder_written);
        }
        assert_eq!(runner.read_file(&index).unwrap(), b"tenant website");
        assert!(
            runner
                .recorded_actions()
                .iter()
                .all(|action| matches!(action,
                    crate::system::MockAction::Command { binary, .. } if binary == "runuser"
                ))
        );
    }

    #[test]
    fn provision_acl_failure_does_not_write_placeholder_or_report_success() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(runner.clone());
        let sites = Path::new("/home/p_alice").join("sites");
        runner.set_response(
            &format!(
                "runuser -u p_alice -- /usr/bin/setfacl -P -m u:www-data:--x -- {}",
                sites.display()
            ),
            1,
            b"",
            b"Operation not supported",
        );
        let error = manager
            .provision_site(provision_request("/home/p_alice/sites/example.test/public"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("ACL Nginx gagal") && error.contains("Operation not supported"));
        assert!(
            !runner
                .recorded_actions()
                .iter()
                .any(|action| matches!(action, crate::system::MockAction::WriteFile { .. }))
        );
    }

    #[test]
    fn provision_rejects_cross_tenant_traversal_and_invalid_worker_before_actions() {
        for docroot in [
            "/etc/nginx",
            "/home/p_bob/sites/example/public",
            "/home/p_alice/sites",
            "/home/p_alice/sites/../private",
            "/home/p_alice/sites/a/./public",
            "/home/p_alice/sites/a\n/public",
            "/home/p_alice/sites/a\\public",
        ] {
            let runner = Arc::new(MockSystemRunner::new());
            let manager = NginxManager::new(runner.clone());
            assert!(
                manager.provision_site(provision_request(docroot)).is_err(),
                "{docroot}"
            );
            assert!(runner.recorded_actions().is_empty());
        }
        for (home, worker) in [
            ("/home/p_bob", "www-data"),
            ("/home/p_alice", "root"),
            ("/home/p_alice", "www-data:rw-"),
        ] {
            let runner = Arc::new(MockSystemRunner::new());
            let manager = NginxManager::new(runner.clone());
            let mut request = provision_request("/home/p_alice/sites/example.test/public");
            request.home_dir = home.into();
            request.web_user = Some(worker.into());
            assert!(manager.provision_site(request).is_err());
            assert!(runner.recorded_actions().is_empty());
        }
    }

    #[test]
    fn provision_rejects_symlink_ancestors_and_index_before_mutation() {
        for link in [
            "/home/p_alice/sites",
            "/home/p_alice/sites/example.test/public/index.html",
        ] {
            let runner = Arc::new(MockSystemRunner::new());
            runner
                .create_symlink(Path::new("/etc"), Path::new(link))
                .unwrap();
            let count = runner.recorded_actions().len();
            let manager = NginxManager::new(runner.clone());
            assert!(
                manager
                    .provision_site(provision_request("/home/p_alice/sites/example.test/public"))
                    .is_err()
            );
            assert_eq!(count, runner.recorded_actions().len());
        }
    }

    #[test]
    fn provision_supports_nginx_worker_account() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(runner.clone());
        let mut request = provision_request("/home/p_alice/sites/example.test/public");
        request.web_user = Some("nginx".into());
        manager.provision_site(request).unwrap();
        assert!(
            runner
                .get_commands()
                .iter()
                .any(|command| command.contains("u:nginx:r-x,d:u:nginx:r-x"))
        );
    }

    fn log_apply_request(base_dir: &Path) -> SiteApplyRequest {
        SiteApplyRequest {
            site_id: "site_42".into(),
            config_content: "server { listen 80; server_name testing.incogmsg.my.id; access_log /var/log/panel/p_tokoalim/testing.incogmsg.my.id/access.log panel_json; error_log /var/log/panel/p_tokoalim/testing.incogmsg.my.id/error.log warn; }".into(),
            enable: true,
            nginx_base_dir: Some(base_dir.to_string_lossy().into_owned()),
            log_context: Some(panel_proto::SiteLogContext { username: "p_tokoalim".into(), domain: "testing.incogmsg.my.id".into() }),
            test_config: true,
            reload: true,
        }
    }

    #[test]
    fn apply_prepares_site_log_directories_and_acl_before_nginx_validation() {
        let temp = TempDir::new().unwrap();
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::with_base_dir(runner.clone(), temp.path());
        manager.apply_site(log_apply_request(temp.path())).unwrap();
        let actions = runner.recorded_actions();
        let check_index = actions
            .iter()
            .position(|action| {
                matches!(action,
            crate::system::MockAction::Command { binary, .. } if binary == "nginx")
            })
            .unwrap();
        for path in [
            "/var/log/panel/p_tokoalim",
            "/var/log/panel/p_tokoalim/testing.incogmsg.my.id",
        ] {
            let index = actions
                .iter()
                .position(|action| {
                    matches!(action,
                crate::system::MockAction::CreateDir { path: actual, mode: 0o750, uid: 0, gid: 0 }
                if actual == Path::new(path))
                })
                .unwrap();
            assert!(index < check_index, "log directory must precede nginx -t");
            assert!(actions[..check_index].iter().any(|action| matches!(action,
                crate::system::MockAction::Command { binary, args }
                    if binary == "setfacl" && args.last().is_some_and(|arg| Path::new(arg) == Path::new(path)))));
        }
        assert!(temp.path().join("sites-available/site_42.conf").exists());
    }

    #[test]
    fn preparing_logs_preserves_existing_files_and_directory_ownership() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(runner.clone());
        for dir in [
            "/var/log/panel",
            "/var/log/panel/p_tokoalim",
            "/var/log/panel/p_tokoalim/testing.incogmsg.my.id",
        ] {
            runner.add_existing_path(dir);
        }
        for name in ["access.log", "error.log"] {
            runner.set_file_content(
                Path::new(&format!(
                    "/var/log/panel/p_tokoalim/testing.incogmsg.my.id/{name}"
                )),
                b"existing log history\n",
            );
        }
        let context = log_apply_request(Path::new("fixture")).log_context.unwrap();
        manager.prepare_site_logs(&context).unwrap();
        manager.prepare_site_logs(&context).unwrap();
        for name in ["access.log", "error.log"] {
            assert_eq!(
                runner
                    .read_file(Path::new(&format!(
                        "/var/log/panel/p_tokoalim/testing.incogmsg.my.id/{name}"
                    )))
                    .unwrap(),
                b"existing log history\n"
            );
        }
        assert!(
            runner
                .recorded_actions()
                .iter()
                .all(|action| matches!(action,
            crate::system::MockAction::Command { binary, .. } if binary == "setfacl"))
        );
    }

    #[test]
    fn log_context_rejects_traversal_before_any_host_action_and_normalizes_idn() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(runner.clone());
        for (username, domain) in [
            ("../../etc", "example.com"),
            ("p_alice", "../../etc"),
            ("p_alice", "example.com;error_log /etc/passwd"),
        ] {
            assert!(
                manager
                    .prepare_site_logs(&panel_proto::SiteLogContext {
                        username: username.into(),
                        domain: domain.into()
                    })
                    .is_err()
            );
        }
        assert!(runner.recorded_actions().is_empty());
        manager
            .prepare_site_logs(&panel_proto::SiteLogContext {
                username: "p_alice".into(),
                domain: "CAFÉ.example".into(),
            })
            .unwrap();
        assert!(runner.path_exists(Path::new("/var/log/panel/p_alice/xn--caf-dma.example")));
    }

    #[test]
    fn log_acl_failure_stops_before_staging_or_nginx_and_preserves_previous_vhost() {
        let temp = TempDir::new().unwrap();
        let runner = Arc::new(MockSystemRunner::new());
        runner.set_response("setfacl", 1, b"", b"ACL denied");
        let manager = NginxManager::with_base_dir(runner.clone(), temp.path());
        let path = temp.path().join("sites-available/site_42.conf");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "previous working vhost").unwrap();
        let error = manager
            .apply_site(log_apply_request(temp.path()))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Persiapan direktori log situs gagal")
        );
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "previous working vhost"
        );
        assert!(!runner.recorded_actions().iter().any(|action| matches!(action,
            crate::system::MockAction::Command { binary, .. } if binary == "nginx" || binary == "systemctl")));
    }

    const HOST_MAIN: &str = "/etc/nginx/nginx.conf";
    const HOST_BRIDGE: &str = "/etc/nginx/conf.d/00-nusa-panel-sites.conf";
    const DISTRO_MAIN: &[u8] = b"events {}\nhttp { include /etc/nginx/conf.d/*.conf; include /etc/nginx/sites-enabled/*; }\n";

    fn host_fixture() -> (Arc<MockSystemRunner>, NginxManager) {
        let runner = Arc::new(MockSystemRunner::new());
        runner
            .write_file(Path::new(HOST_MAIN), DISTRO_MAIN, 0o644)
            .unwrap();
        runner.set_response("nginx", 0, b"syntax is ok", b"");
        let manager = NginxManager::new(runner.clone());
        (runner, manager)
    }

    fn prepare_host(manager: &NginxManager) -> Result<(), AgentError> {
        manager.ensure_host_baseline(
            &NginxBaselineConfig::default(),
            Path::new(HOST_MAIN),
            Path::new(HOST_BRIDGE),
        )
    }

    #[test]
    fn host_baseline_rejects_unknown_hosts_before_any_site_is_registered() {
        let (runner, manager) = host_fixture();
        prepare_host(&manager).unwrap();
        let guard = String::from_utf8(
            runner
                .read_file(Path::new("/etc/nginx/conf.d/00-qpanel-reject-unknown.conf"))
                .expect("guard must be loaded by system conf.d"),
        )
        .unwrap();
        assert!(guard.contains("listen 80 default_server;"));
        assert!(guard.contains("listen [::]:80 default_server;"));
        assert!(guard.contains("listen 443 ssl default_server;"));
        assert!(guard.contains("ssl_reject_handshake on;"));
        assert_eq!(guard.matches("return 444;").count(), 2);
        assert!(!guard.contains("proxy_pass") && !guard.contains("root "));
        assert_eq!(runner.read_file(Path::new(HOST_MAIN)).unwrap(), DISTRO_MAIN);
    }

    fn stock_default_fixture(runner: &MockSystemRunner) {
        use md5::{Digest, Md5};
        let stock = b"server { listen 80 default_server; server_name _; root /var/www/html; }\n";
        let metadata = format!(
            " /etc/nginx/sites-available/default {:x}\n /etc/nginx/conf.d/default.conf {:x}\n",
            Md5::digest(stock),
            Md5::digest(stock)
        );
        runner.set_response(
            "dpkg-query -W -f=${Conffiles}\n nginx-common",
            0,
            metadata.as_bytes(),
            b"",
        );
        runner.set_file_content("/etc/nginx/sites-available/default", stock);
        runner.set_file_content("/etc/nginx/conf.d/default.conf", stock);
        runner
            .create_symlink(
                Path::new("../sites-available/default"),
                Path::new("/etc/nginx/sites-enabled/default"),
            )
            .unwrap();
    }

    #[test]
    fn host_guard_quarantines_only_checksum_verified_package_defaults() {
        let (runner, manager) = host_fixture();
        stock_default_fixture(&runner);
        let custom = Path::new("/etc/nginx/conf.d/customer.conf");
        runner.set_file_content(custom, b"server { server_name customer.test; }");
        prepare_host(&manager).unwrap();
        assert!(!runner.path_exists(Path::new("/etc/nginx/sites-enabled/default")));
        assert!(!runner.path_exists(Path::new("/etc/nginx/conf.d/default.conf")));
        assert!(runner.path_exists(Path::new("/etc/nginx/sites-available/default")));
        assert_eq!(
            runner.read_file(custom).unwrap(),
            b"server { server_name customer.test; }"
        );
        let moves: Vec<_> = runner
            .recorded_actions()
            .into_iter()
            .filter_map(|action| match action {
                crate::system::MockAction::MovePath { from, to }
                    if from == Path::new("/etc/nginx/sites-enabled/default")
                        || from == Path::new("/etc/nginx/conf.d/default.conf") =>
                {
                    Some(to)
                }
                _ => None,
            })
            .collect();
        assert_eq!(moves.len(), 2);
        for path in moves {
            assert!(runner.path_exists(&path));
            assert!(path.to_string_lossy().contains("qpanel-disabled"));
        }
    }

    #[test]
    fn host_guard_preserves_modified_defaults_and_conflicting_guard_without_mutation() {
        for changed in [
            "/etc/nginx/sites-available/default",
            "/etc/nginx/conf.d/default.conf",
            "/etc/nginx/conf.d/00-qpanel-reject-unknown.conf",
        ] {
            let (runner, manager) = host_fixture();
            stock_default_fixture(&runner);
            runner.set_file_content(changed, b"# operator custom config");
            let files = runner.files.lock().unwrap().clone();
            let links = runner.symlinks.lock().unwrap().clone();
            assert!(prepare_host(&manager).is_err(), "{changed}");
            assert_eq!(*runner.files.lock().unwrap(), files);
            assert_eq!(*runner.symlinks.lock().unwrap(), links);
        }
    }

    #[test]
    fn host_guard_validation_failure_restores_packaged_defaults() {
        let (runner, manager) = host_fixture();
        stock_default_fixture(&runner);
        let files = runner.files.lock().unwrap().clone();
        let links = runner.symlinks.lock().unwrap().clone();
        runner.set_response("nginx", 1, b"", b"duplicate operator default_server");
        assert!(prepare_host(&manager).is_err());
        // MockSystemRunner stores created directory markers in its file map.
        // Empty baseline directories may remain; every original file/link must return.
        let directories: std::collections::HashSet<_> = runner
            .recorded_actions()
            .into_iter()
            .filter_map(|action| match action {
                crate::system::MockAction::CreateDir { path, .. } => Some(path),
                _ => None,
            })
            .collect();
        let mut restored = runner.files.lock().unwrap().clone();
        restored.retain(|path, _| !directories.contains(path));
        assert_eq!(restored, files);
        assert_eq!(*runner.symlinks.lock().unwrap(), links);
    }

    #[test]
    fn host_baseline_accepts_legacy_brand_bridge_without_rewriting_it() {
        for brand in ["Nusa Panel", "Qpanel"] {
            let (runner, manager) = host_fixture();
            let legacy = format!(
                "# Managed by {brand}: tenant websites.\ninclude /etc/panel/nginx/http.conf;\n"
            );
            runner.set_file_content(Path::new(HOST_BRIDGE), legacy.as_bytes());
            let guard = NginxBaselineConfig::default()
                .render_default_catchall()
                .replacen("Managed by QonPanel", &format!("Managed by {brand}"), 1);
            runner.set_file_content(
                Path::new("/etc/nginx/conf.d/00-qpanel-reject-unknown.conf"),
                guard.as_bytes(),
            );
            for _ in 0..2 {
                prepare_host(&manager).unwrap();
                assert_eq!(
                    runner.read_file(Path::new(HOST_BRIDGE)).unwrap(),
                    legacy.as_bytes()
                );
            }
        }
    }

    #[test]
    fn host_baseline_repairs_installer_layout_without_a_panel_main_config() {
        let (runner, manager) = host_fixture();
        let proxy = Path::new("/etc/nginx/sites-available/nusa-panel.conf");
        runner
            .write_file(proxy, b"existing custom public port proxy", 0o644)
            .unwrap();
        prepare_host(&manager).unwrap();
        assert_eq!(runner.read_file(Path::new(HOST_MAIN)).unwrap(), DISTRO_MAIN);
        assert_eq!(
            runner.read_file(proxy).unwrap(),
            b"existing custom public port proxy"
        );
        assert!(!runner.path_exists(Path::new("/etc/panel/nginx/nginx.conf")));
        assert_eq!(
            nginx_main_config_path(Path::new("/etc/panel/nginx")),
            Path::new(HOST_MAIN)
        );
        assert_eq!(
            nginx_main_config_path(Path::new("fixture")),
            Path::new("fixture/nginx.conf")
        );
        let bridge = runner.read_file(Path::new(HOST_BRIDGE)).unwrap();
        assert!(
            String::from_utf8(bridge)
                .unwrap()
                .contains("include /etc/panel/nginx/http.conf;")
        );
        for path in [
            "acme",
            "security-headers",
            "deny-sensitive",
            "fastcgi-params",
        ] {
            assert!(runner.path_exists(&PathBuf::from(format!(
                "/etc/panel/nginx/snippets/{path}.conf"
            ))));
        }
        let actions = runner.recorded_actions();
        assert!(actions.iter().any(|action| matches!(action,
            crate::system::MockAction::Command { binary, args }
            if binary == "nginx" && args == &["-t", "-c", HOST_MAIN]
        )));
        assert!(!actions.iter().any(|action| matches!(action,
            crate::system::MockAction::Command { binary, .. } if binary == "systemctl"
        )));
    }

    #[test]
    fn host_baseline_is_idempotent_and_preserves_custom_snippets() {
        let (runner, manager) = host_fixture();
        let snippet = Path::new("/etc/panel/nginx/snippets/security-headers.conf");
        runner
            .write_file(snippet, b"# operator custom headers", 0o644)
            .unwrap();
        prepare_host(&manager).unwrap();
        let count = runner.recorded_actions().len();
        prepare_host(&manager).unwrap();
        assert_eq!(count, runner.recorded_actions().len());
        assert_eq!(
            runner.read_file(snippet).unwrap(),
            b"# operator custom headers"
        );
    }

    #[test]
    fn production_baseline_rpc_reports_the_system_main_and_validates_before_reload() {
        let (runner, manager) = host_fixture();
        runner.set_response("systemctl", 0, b"", b"");
        let request = NginxBaselineApplyRequest {
            base_dir: Some("/etc/panel/nginx".into()),
            log_dir: None,
            acme_dir: None,
            panel_api_port: None,
            panel_listen_port: Some(19090),
            test_config: true,
            reload: true,
        };
        let result = manager.apply_baseline(request.clone()).unwrap();
        assert_eq!(result.nginx_conf_path, HOST_MAIN);
        assert!(result.test_passed && result.reloaded);
        assert!(!runner.path_exists(Path::new("/etc/panel/nginx/conf.d/10-panel.conf")));
        let before = runner.recorded_actions().len();
        runner.set_response("nginx", 1, b"", b"invalid operator edit");
        let result = manager.apply_baseline(request).unwrap();
        assert!(!result.test_passed && !result.reloaded);
        assert!(
            !runner.recorded_actions()[before..]
                .iter()
                .any(|action| matches!(action,
                    crate::system::MockAction::Command { binary, .. } if binary == "systemctl"
                ))
        );
    }

    #[test]
    fn host_baseline_validation_failure_rolls_back_only_new_files() {
        let (runner, manager) = host_fixture();
        let snippet = Path::new("/etc/panel/nginx/snippets/security-headers.conf");
        runner
            .write_file(snippet, b"# preserve this", 0o644)
            .unwrap();
        runner.set_response("nginx", 1, b"", b"invalid existing configuration");
        assert!(prepare_host(&manager).is_err());
        assert!(!runner.path_exists(Path::new(HOST_BRIDGE)));
        assert!(!runner.path_exists(Path::new("/etc/panel/nginx/http.conf")));
        assert!(!runner.path_exists(Path::new("/etc/panel/nginx/snippets/acme.conf")));
        assert_eq!(runner.read_file(snippet).unwrap(), b"# preserve this");
        assert_eq!(runner.read_file(Path::new(HOST_MAIN)).unwrap(), DISTRO_MAIN);
    }

    #[test]
    fn host_baseline_rejects_missing_main_and_custom_include_layouts_without_mutation() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(runner.clone());
        assert!(prepare_host(&manager).is_err());
        assert!(runner.recorded_actions().is_empty());
        for main in [
            "http { include /operator/custom/*.conf; }",
            "http {\n# include /etc/nginx/conf.d/*.conf;\n}",
        ] {
            runner
                .write_file(Path::new(HOST_MAIN), main.as_bytes(), 0o644)
                .unwrap();
            let count = runner.recorded_actions().len();
            assert!(prepare_host(&manager).is_err());
            assert_eq!(count, runner.recorded_actions().len());
        }
    }

    #[test]
    fn host_baseline_preserves_conflicting_integration_file() {
        let (runner, manager) = host_fixture();
        runner
            .write_file(Path::new(HOST_BRIDGE), b"# unrelated configuration", 0o644)
            .unwrap();
        let count = runner.recorded_actions().len();
        assert!(prepare_host(&manager).is_err());
        assert_eq!(count, runner.recorded_actions().len());
        assert_eq!(
            runner.read_file(Path::new(HOST_BRIDGE)).unwrap(),
            b"# unrelated configuration"
        );
    }

    #[test]
    fn test_apply_baseline_with_mock_runner() {
        let temp = TempDir::new().unwrap();
        let nginx_dir = temp.path().join("nginx");
        let log_dir = temp.path().join("logs");
        let acme_dir = temp.path().join("acme");

        let runner = Arc::new(MockSystemRunner::new());
        // Configure mock responses for nginx -t and systemctl reload
        runner.set_response("nginx", 0, b"syntax is ok", b"");
        runner.set_response("systemctl", 0, b"", b"");

        let manager = NginxManager::new(Arc::clone(&runner) as Arc<dyn SystemRunner>);

        let req = NginxBaselineApplyRequest {
            base_dir: Some(nginx_dir.to_string_lossy().to_string()),
            log_dir: Some(log_dir.to_string_lossy().to_string()),
            acme_dir: Some(acme_dir.to_string_lossy().to_string()),
            panel_api_port: Some(7700),
            panel_listen_port: Some(17890),
            test_config: true,
            reload: true,
        };

        let res = manager.apply_baseline(req).expect("apply baseline");
        assert!(res.test_passed);
        assert!(res.reloaded);
        assert!(nginx_dir.join("nginx.conf").exists());
        assert!(nginx_dir.join("conf.d/00-default.conf").exists());
        assert!(nginx_dir.join("snippets/deny-sensitive.conf").exists());

        // Verify commands recorded in runner
        let actions = runner.recorded_actions();
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn test_deploy_and_cleanup_acme_challenge() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(Arc::clone(&runner) as Arc<dyn SystemRunner>);

        // 1. Deploy challenge
        let deploy_req = AcmeChallengeDeployRequest {
            token: "valid-token-123_abc".to_string(),
            key_authorization: "valid-token-123_abc.account-thumbprint-xyz".to_string(),
            acme_dir: Some("/var/www/acme-challenge".to_string()),
        };
        let deploy_res = manager
            .deploy_acme_challenge(deploy_req)
            .expect("deploy challenge");
        assert!(deploy_res.challenge_path.contains("valid-token-123_abc"));

        // Verify write action
        let actions = runner.recorded_actions();
        let write_action = actions.iter().find(|a| match a {
            crate::system::MockAction::WriteFile {
                path,
                content,
                mode,
            } => {
                path.to_string_lossy().contains("valid-token-123_abc")
                    && content == b"valid-token-123_abc.account-thumbprint-xyz"
                    && *mode == 0o644
            }
            _ => false,
        });
        assert!(
            write_action.is_some(),
            "Challenge file must be written with mode 0644"
        );

        // 2. Reject malicious traversal token
        let bad_req = AcmeChallengeDeployRequest {
            token: "../../../etc/passwd".to_string(),
            key_authorization: "evil".to_string(),
            acme_dir: Some("/var/www/acme-challenge".to_string()),
        };
        assert!(manager.deploy_acme_challenge(bad_req).is_err());
    }

    #[test]
    fn test_deploy_ssl_cert_enforces_0600_key() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(Arc::clone(&runner) as Arc<dyn SystemRunner>);

        let req = SslCertDeployRequest {
            domain: "contoh.id".to_string(),
            fullchain_pem: "-----BEGIN CERTIFICATE-----\nMIIB...\n-----END CERTIFICATE-----\n"
                .to_string(),
            privkey_pem: "-----BEGIN PRIVATE KEY-----\nMIIE...\n-----END PRIVATE KEY-----\n"
                .to_string(),
            ssl_base_dir: Some("/etc/panel/ssl".to_string()),
        };

        let res = manager.deploy_ssl_cert(req).expect("deploy ssl cert");
        assert_eq!(res.domain, "contoh.id");
        assert!(res.cert_path.ends_with("fullchain.pem"));
        assert!(res.key_path.ends_with("privkey.pem"));

        // Verify actions: fullchain written with 0644, privkey written with 0600
        let actions = runner.recorded_actions();
        let cert_action = actions.iter().find(|a| match a {
            crate::system::MockAction::WriteFile {
                path,
                content,
                mode,
            } => {
                path.to_string_lossy().contains("fullchain.pem")
                    && content.starts_with(b"-----BEGIN CERTIFICATE-----")
                    && *mode == 0o644
            }
            _ => false,
        });
        assert!(cert_action.is_some(), "Cert must be written with mode 0644");

        let key_action = actions.iter().find(|a| match a {
            crate::system::MockAction::WriteFile {
                path,
                content,
                mode,
            } => {
                path.to_string_lossy().contains("privkey.pem")
                    && content.starts_with(b"-----BEGIN PRIVATE KEY-----")
                    && *mode == 0o600
            }
            _ => false,
        });
        assert!(
            key_action.is_some(),
            "Private key MUST be written with strict mode 0600"
        );
    }

    #[test]
    fn test_read_site_logs_tail_and_query_filter() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NginxManager::new(Arc::clone(&runner) as Arc<dyn SystemRunner>);

        let log_dir = PathBuf::from("/var/log/panel/p_alice/example.com");
        let access_log_path = log_dir.join("access.log");

        let log_content = "\
{\"time_local\":\"2026-10-03T10:00:00Z\",\"status\":200,\"request_uri\":\"/index.html\"}\n\
{\"time_local\":\"2026-10-03T10:01:00Z\",\"status\":404,\"request_uri\":\"/not-found\"}\n\
{\"time_local\":\"2026-10-03T10:02:00Z\",\"status\":200,\"request_uri\":\"/about\"}\n\
{\"time_local\":\"2026-10-03T10:03:00Z\",\"status\":500,\"request_uri\":\"/server-error\"}\n\
{\"time_local\":\"2026-10-03T10:04:00Z\",\"status\":200,\"request_uri\":\"/contact\"}\n";

        runner.set_file_content(&access_log_path, log_content.as_bytes());

        // 1. Read last 2 lines
        let req = SiteLogsReadRequest {
            username: "p_alice".to_string(),
            site_domain: "example.com".to_string(),
            log_type: "access".to_string(),
            lines: 2,
            query: None,
            log_base_dir: Some("/var/log/panel".to_string()),
            site_id: None,
        };
        let res = manager.read_site_logs(req).expect("read logs");
        assert_eq!(res.lines.len(), 2);
        assert_eq!(res.total_matched, 5);
        assert!(res.lines[0].contains("/server-error"));
        assert!(res.lines[1].contains("/contact"));

        // 2. Query filter for status 404
        let req_filter = SiteLogsReadRequest {
            username: "p_alice".to_string(),
            site_domain: "example.com".to_string(),
            log_type: "access".to_string(),
            lines: 10,
            query: Some("404".to_string()),
            log_base_dir: Some("/var/log/panel".to_string()),
            site_id: None,
        };
        let res_filter = manager
            .read_site_logs(req_filter)
            .expect("read filtered logs");
        assert_eq!(res_filter.lines.len(), 1);
        assert_eq!(res_filter.total_matched, 1);
        assert!(res_filter.lines[0].contains("/not-found"));

        // 3. Non-existent log file returns empty lines without error
        let req_missing = SiteLogsReadRequest {
            username: "p_alice".to_string(),
            site_domain: "nonexistent.com".to_string(),
            log_type: "access".to_string(),
            lines: 10,
            query: None,
            log_base_dir: Some("/var/log/panel".to_string()),
            site_id: None,
        };
        let res_missing = manager
            .read_site_logs(req_missing)
            .expect("read missing logs");
        assert!(res_missing.lines.is_empty());
        assert_eq!(res_missing.total_matched, 0);

        // 4. Invalid log_type rejected
        let req_bad_type = SiteLogsReadRequest {
            username: "p_alice".to_string(),
            site_domain: "example.com".to_string(),
            log_type: "something_else".to_string(),
            lines: 10,
            query: None,
            log_base_dir: Some("/var/log/panel".to_string()),
            site_id: None,
        };
        assert!(manager.read_site_logs(req_bad_type).is_err());
    }
}
