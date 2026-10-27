//! `panel-agent`: Privileged root daemon executing typed system operations.

pub mod ai;
pub mod audit;
pub mod backups;
pub mod capacity;
pub mod cloudflare;
pub mod connectivity;
pub mod cron;
pub mod crypto;
pub mod database_web;
pub mod databases;
pub mod dns01;
pub mod email;
pub mod fs_manager;
pub mod fs_worker;
pub mod git_deploy;
pub mod hardening;
mod installation;
pub mod nftables;
pub mod node;
pub mod node_site;
pub mod optimizer;
pub mod panel_tls;
pub mod panel_update;
pub mod php;
pub mod platform;
pub mod proxy;
pub mod pty;
pub mod python_site;
pub mod quota;
pub mod reliability;
pub mod resource;
mod runtime_install;
pub mod server_backup;
pub mod slice;
pub mod ssh;
pub mod storage_managed;
pub mod storage_secret;
pub mod system;
pub mod user;
mod uv;
pub mod webserver;

use panel_proto::{
    PROTOCOL_VERSION, ProtoError, RequestPayload, ResponsePayload, RpcResponse, RpcStream,
    VersionInfo,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, warn};

#[cfg(unix)]
pub type IpcStream = tokio::net::UnixStream;
#[cfg(not(unix))]
pub type IpcStream = tokio::net::TcpStream;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Protocol error: {0}")]
    Proto(#[from] ProtoError),
    #[error("Authentication/Authorization failed: unauthorized peer {0}")]
    UnauthorizedPeer(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Security violation: {0}")]
    SecurityViolation(String),
    #[error("Command {binary} failed with exit code {code:?}: {stderr}")]
    CommandFailed {
        binary: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Agent daemon configuration.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub socket_path: PathBuf,
    pub allowed_uid: Option<u32>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/run/panel/agent.sock"),
            allowed_uid: None,
        }
    }
}

pub struct AgentServer {
    config: AgentConfig,
    runner: Arc<dyn system::SystemRunner>,
    user_manager: Arc<user::UserLifecycleManager>,
    slice_manager: Arc<slice::SliceManager>,
    quota_manager: Arc<quota::QuotaManager>,
    resource_reader: Arc<resource::CgroupReader>,
    hardening_manager: Arc<hardening::HardeningManager>,
    webserver_manager: Arc<webserver::NginxManager>,
    fs_manager: Arc<fs_manager::FsManager>,
    pty_manager: Arc<pty::PtySessionManager>,
    ssh_manager: Arc<ssh::SshManager>,
    optimizer_manager: Arc<optimizer::OptimizerManager>,
    nftables_manager: Arc<nftables::NftablesManager>,
    reliability_manager: Arc<reliability::ReliabilityManager>,
}

fn create_default_pty_manager() -> Arc<pty::PtySessionManager> {
    #[cfg(unix)]
    let backend: Arc<dyn pty::PtyBackend> = Arc::new(pty::UnixPtyBackend::new());
    #[cfg(not(unix))]
    let backend: Arc<dyn pty::PtyBackend> = Arc::new(pty::MockPtyBackend::new());

    let mgr = Arc::new(pty::PtySessionManager::new(
        pty::PtyManagerConfig::default(),
        backend,
    ));
    std::mem::drop(mgr.start_idle_reaper(std::time::Duration::from_secs(30)));
    mgr
}

impl AgentServer {
    pub fn new(config: AgentConfig) -> Self {
        let runner = Arc::new(system::RealSystemRunner);
        let user_manager = Arc::new(user::UserLifecycleManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let slice_manager = Arc::new(slice::SliceManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let quota_manager = Arc::new(quota::QuotaManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let resource_reader = Arc::new(resource::CgroupReader::default());
        let hardening_manager = Arc::new(hardening::HardeningManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let webserver_manager = Arc::new(webserver::NginxManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let fs_manager = Arc::new(fs_manager::FsManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let pty_manager = create_default_pty_manager();
        let ssh_manager = Arc::new(ssh::SshManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let optimizer_manager = Arc::new(optimizer::OptimizerManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let nftables_manager = Arc::new(nftables::NftablesManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let reliability_manager = Arc::new(reliability::ReliabilityManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>,
            "/etc/panel",
            Arc::clone(&nftables_manager),
        ));
        Self {
            config,
            runner,
            user_manager,
            slice_manager,
            quota_manager,
            resource_reader,
            hardening_manager,
            webserver_manager,
            fs_manager,
            pty_manager,
            ssh_manager,
            optimizer_manager,
            nftables_manager,
            reliability_manager,
        }
    }

    pub fn with_user_manager(
        config: AgentConfig,
        user_manager: Arc<user::UserLifecycleManager>,
    ) -> Self {
        let runner = Arc::new(system::RealSystemRunner);
        let slice_manager = Arc::new(slice::SliceManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let quota_manager = Arc::new(quota::QuotaManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let resource_reader = Arc::new(resource::CgroupReader::default());
        let hardening_manager = Arc::new(hardening::HardeningManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let webserver_manager = Arc::new(webserver::NginxManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let fs_manager = Arc::new(fs_manager::FsManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let pty_manager = create_default_pty_manager();
        let ssh_manager = Arc::new(ssh::SshManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let optimizer_manager = Arc::new(optimizer::OptimizerManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let nftables_manager = Arc::new(nftables::NftablesManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let reliability_manager = Arc::new(reliability::ReliabilityManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>,
            "/etc/panel",
            Arc::clone(&nftables_manager),
        ));
        Self {
            config,
            runner,
            user_manager,
            slice_manager,
            quota_manager,
            resource_reader,
            hardening_manager,
            webserver_manager,
            fs_manager,
            pty_manager,
            ssh_manager,
            optimizer_manager,
            nftables_manager,
            reliability_manager,
        }
    }

    pub fn with_managers(
        config: AgentConfig,
        user_manager: Arc<user::UserLifecycleManager>,
        slice_manager: Arc<slice::SliceManager>,
        quota_manager: Arc<quota::QuotaManager>,
        resource_reader: Arc<resource::CgroupReader>,
        hardening_manager: Arc<hardening::HardeningManager>,
    ) -> Self {
        let runner = Arc::new(system::RealSystemRunner);
        let webserver_manager = Arc::new(webserver::NginxManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let fs_manager = Arc::new(fs_manager::FsManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let pty_manager = create_default_pty_manager();
        let ssh_manager = Arc::new(ssh::SshManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let optimizer_manager = Arc::new(optimizer::OptimizerManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let nftables_manager = Arc::new(nftables::NftablesManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let reliability_manager = Arc::new(reliability::ReliabilityManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>,
            "/etc/panel",
            Arc::clone(&nftables_manager),
        ));
        Self {
            config,
            runner,
            user_manager,
            slice_manager,
            quota_manager,
            resource_reader,
            hardening_manager,
            webserver_manager,
            fs_manager,
            pty_manager,
            ssh_manager,
            optimizer_manager,
            nftables_manager,
            reliability_manager,
        }
    }

    pub fn with_all_managers(
        config: AgentConfig,
        user_manager: Arc<user::UserLifecycleManager>,
        slice_manager: Arc<slice::SliceManager>,
        quota_manager: Arc<quota::QuotaManager>,
        resource_reader: Arc<resource::CgroupReader>,
        hardening_manager: Arc<hardening::HardeningManager>,
        webserver_manager: Arc<webserver::NginxManager>,
    ) -> Self {
        let runner = Arc::clone(webserver_manager.runner());
        let fs_manager = Arc::new(fs_manager::FsManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let pty_manager = create_default_pty_manager();
        let ssh_manager = Arc::new(ssh::SshManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let optimizer_manager = Arc::new(optimizer::OptimizerManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let nftables_manager = Arc::new(nftables::NftablesManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>
        ));
        let reliability_manager = Arc::new(reliability::ReliabilityManager::new(
            Arc::clone(&runner) as Arc<dyn system::SystemRunner>,
            "/etc/panel",
            Arc::clone(&nftables_manager),
        ));
        Self {
            config,
            runner,
            user_manager,
            slice_manager,
            quota_manager,
            resource_reader,
            hardening_manager,
            webserver_manager,
            fs_manager,
            pty_manager,
            ssh_manager,
            optimizer_manager,
            nftables_manager,
            reliability_manager,
        }
    }

    pub fn with_fs_manager(mut self, fs_manager: Arc<fs_manager::FsManager>) -> Self {
        self.fs_manager = fs_manager;
        self
    }

    pub fn with_quota_manager(mut self, quota_manager: Arc<quota::QuotaManager>) -> Self {
        self.quota_manager = quota_manager;
        self
    }

    pub fn with_pty_manager(mut self, pty_manager: Arc<pty::PtySessionManager>) -> Self {
        self.pty_manager = pty_manager;
        self
    }

    pub fn with_ssh_manager(mut self, ssh_manager: Arc<ssh::SshManager>) -> Self {
        self.ssh_manager = ssh_manager;
        self
    }

    pub fn with_optimizer_manager(
        mut self,
        optimizer_manager: Arc<optimizer::OptimizerManager>,
    ) -> Self {
        self.optimizer_manager = optimizer_manager;
        self
    }

    pub fn optimizer_manager(&self) -> &Arc<optimizer::OptimizerManager> {
        &self.optimizer_manager
    }

    pub fn with_nftables_manager(
        mut self,
        nftables_manager: Arc<nftables::NftablesManager>,
    ) -> Self {
        self.nftables_manager = nftables_manager;
        self
    }

    pub fn nftables_manager(&self) -> &Arc<nftables::NftablesManager> {
        &self.nftables_manager
    }

    pub fn with_reliability_manager(
        mut self,
        reliability_manager: Arc<reliability::ReliabilityManager>,
    ) -> Self {
        self.reliability_manager = reliability_manager;
        self
    }

    pub fn reliability_manager(&self) -> &Arc<reliability::ReliabilityManager> {
        &self.reliability_manager
    }

    pub fn pty_manager(&self) -> &Arc<pty::PtySessionManager> {
        &self.pty_manager
    }

    pub fn ssh_manager(&self) -> &Arc<ssh::SshManager> {
        &self.ssh_manager
    }

    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// Verifies peer credentials over Unix Domain Socket using SO_PEERCRED on Unix.
    fn verify_peer(&self, stream: &IpcStream) -> Result<(), AgentError> {
        #[cfg(unix)]
        {
            if let Some(expected_uid) = self.config.allowed_uid {
                let cred = stream.peer_cred()?;
                if cred.uid() != expected_uid {
                    return Err(AgentError::UnauthorizedPeer(format!(
                        "Peer UID {} does not match expected allowed UID {}",
                        cred.uid(),
                        expected_uid
                    )));
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = stream;
        }
        Ok(())
    }

    /// Starts the server listening on the configured socket path.
    pub async fn run(
        self: Arc<Self>,
        shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<(), AgentError> {
        self.run_listener(shutdown_rx).await
    }

    #[cfg(unix)]
    fn bind_unix_listener(
        socket_path: &std::path::Path,
    ) -> Result<tokio::net::UnixListener, AgentError> {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt, chown};
        let production = socket_path == Path::new("/run/panel/agent.sock");
        let gid = if production {
            let output = panel_core::production::system_command("getent")
                .args(["group", "panel"])
                .output()?;
            let group = String::from_utf8_lossy(&output.stdout);
            Some(
                group
                    .split(':')
                    .nth(2)
                    .and_then(|id| id.parse::<u32>().ok())
                    .filter(|_| output.status.success())
                    .ok_or_else(|| {
                        AgentError::SecurityViolation("Missing system group panel".to_string())
                    })?,
            )
        } else {
            None
        };

        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
            if production {
                if std::fs::symlink_metadata(parent)?.file_type().is_symlink() {
                    return Err(AgentError::SecurityViolation(
                        "Runtime directory is a symlink".to_string(),
                    ));
                }
                chown(parent, Some(0), gid)?;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o750))?;
            }
        }

        match std::fs::symlink_metadata(socket_path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() {
                    return Err(AgentError::SecurityViolation(
                        "Refusing to remove a non-socket IPC path".to_string(),
                    ));
                }
                if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        "Agent socket is already listening",
                    )
                    .into());
                }
                std::fs::remove_file(socket_path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let listener = tokio::net::UnixListener::bind(socket_path)?;
        let secured = (|| -> std::io::Result<()> {
            if production {
                chown(socket_path, Some(0), gid)?;
            }
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))
        })();
        if let Err(error) = secured {
            drop(listener);
            let _ = std::fs::remove_file(socket_path);
            return Err(error.into());
        }
        info!("Agent listening on Unix socket: {}", socket_path.display());
        Ok(listener)
    }

    #[cfg(unix)]
    #[allow(clippy::cognitive_complexity)]
    async fn run_listener(
        self: Arc<Self>,
        mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<(), AgentError> {
        let socket_path = &self.config.socket_path;
        let listener = Self::bind_unix_listener(socket_path)?;

        loop {
            tokio::select! {
                accept_res = listener.accept() => {
                    if let Ok((stream, _addr)) = accept_res {
                        let server = Arc::clone(&self);
                        tokio::spawn(async move {
                            if let Err(e) = server.handle_connection(stream).await {
                                warn!("Error handling agent client connection: {}", e);
                            }
                        });
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Agent received shutdown signal, shutting down listener...");
                    break;
                }
            }
        }

        if socket_path.exists() {
            let _ = std::fs::remove_file(socket_path);
        }

        Ok(())
    }

    #[cfg(not(unix))]
    #[allow(clippy::cognitive_complexity)]
    async fn run_listener(
        self: Arc<Self>,
        mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<(), AgentError> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let local_addr = listener.local_addr()?;

        if let Some(parent) = self.config.socket_path.parent().filter(|p| !p.exists()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.config.socket_path, local_addr.port().to_string())?;

        info!(
            "Agent listening on fallback TCP loopback: {} (development mode)",
            local_addr
        );

        loop {
            tokio::select! {
                accept_res = listener.accept() => {
                    if let Ok((stream, _addr)) = accept_res {
                        let server = Arc::clone(&self);
                        tokio::spawn(async move {
                            if let Err(e) = server.handle_connection(stream).await {
                                warn!("Error handling agent client connection: {}", e);
                            }
                        });
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Agent received shutdown signal, shutting down listener...");
                    break;
                }
            }
        }

        if self.config.socket_path.exists() {
            let _ = std::fs::remove_file(&self.config.socket_path);
        }

        Ok(())
    }

    #[allow(clippy::cognitive_complexity)]
    async fn handle_connection(&self, stream: IpcStream) -> Result<(), AgentError> {
        self.verify_peer(&stream)?;
        let mut rpc = RpcStream::new(stream);

        loop {
            let req = match rpc.recv_request().await {
                Ok(req) => req,
                Err(ProtoError::ConnectionClosed) => break,
                Err(e) => return Err(AgentError::Proto(e)),
            };

            let (action, target) = match &req.payload {
                RequestPayload::ServerBackup(_) => ("server.backup".into(), "host".into()),
                RequestPayload::PanelUpdate(_) => ("panel.update".into(), "host".into()),
                RequestPayload::Email(_) => ("email.operation".into(), "email".into()),
                RequestPayload::ManagedStorage(_) => ("storage.managed".into(), "storage".into()),
                RequestPayload::StorageSecret(_) => ("storage.credential".into(), "storage".into()),
                RequestPayload::AiSecret(_) => ("system.ai.secret".into(), "ai".into()),
                RequestPayload::Cloudflare(_) => {
                    ("cloudflare.operation".into(), "cloudflare".into())
                }
                RequestPayload::InstallWithProgress(operation) => {
                    ("installation.stream".to_string(), format!("{operation:?}"))
                }
                RequestPayload::Ping => ("ping".to_string(), "system".to_string()),
                RequestPayload::Version => ("version".to_string(), "system".to_string()),
                RequestPayload::UserCreate(d) => ("user.create".to_string(), d.username.clone()),
                RequestPayload::UserSuspend(d) => ("user.suspend".to_string(), d.username.clone()),
                RequestPayload::UserUnsuspend(d) => {
                    ("user.unsuspend".to_string(), d.username.clone())
                }
                RequestPayload::UserDelete(d) => ("user.delete".to_string(), d.username.clone()),
                RequestPayload::SliceApply(d) => {
                    ("slice.apply".to_string(), format!("panel-u{}.slice", d.uid))
                }
                RequestPayload::SliceRemove(d) => (
                    "slice.remove".to_string(),
                    format!("panel-u{}.slice", d.uid),
                ),
                RequestPayload::QuotaSet(d) => ("quota.set".to_string(), d.username.clone()),
                RequestPayload::QuotaGet(d) => ("quota.get".to_string(), d.username.clone()),
                RequestPayload::QuotaScan(d) => ("quota.scan".to_string(), d.username.clone()),
                RequestPayload::ResourceRead(d) => (
                    "resource.read".to_string(),
                    format!("panel-u{}.slice", d.uid),
                ),
                RequestPayload::HardeningCheck => {
                    ("hardening.check".to_string(), "host".to_string())
                }
                RequestPayload::HardeningApply(_) => {
                    ("hardening.apply".to_string(), "host".to_string())
                }
                RequestPayload::NginxBaselineApply(d) => (
                    "nginx.baseline_apply".to_string(),
                    d.base_dir
                        .clone()
                        .unwrap_or_else(|| "/etc/panel/nginx".to_string()),
                ),
                RequestPayload::SiteApply(d) => ("nginx.site_apply".to_string(), d.site_id.clone()),
                RequestPayload::SiteRemove(d) => {
                    ("nginx.site_remove".to_string(), d.site_id.clone())
                }
                RequestPayload::SiteHtpasswdDeploy(d) => {
                    ("nginx.htpasswd_deploy".to_string(), d.site_id.clone())
                }
                RequestPayload::SiteHtpasswdRemove(d) => {
                    ("nginx.htpasswd_remove".to_string(), d.site_id.clone())
                }
                RequestPayload::SiteProvision(d) => {
                    ("nginx.site_provision".to_string(), d.username.clone())
                }
                RequestPayload::AcmeChallengeDeploy(d) => {
                    ("acme.challenge_deploy".to_string(), d.token.clone())
                }
                RequestPayload::AcmeChallengeCleanup(d) => {
                    ("acme.challenge_cleanup".to_string(), d.token.clone())
                }
                RequestPayload::SslCertDeploy(d) => {
                    ("ssl.cert_deploy".to_string(), d.domain.clone())
                }
                RequestPayload::Dns01(_) => ("dns01".into(), "dns01".into()),
                RequestPayload::PanelTls(_) => ("panel.tls".into(), "panel".into()),
                RequestPayload::SiteLogsRead(d) => (
                    "nginx.site_logs_read".to_string(),
                    format!("{}/{}", d.username, d.site_domain),
                ),
                RequestPayload::PhpRepoSetup(_) => {
                    ("php.repo_setup".to_string(), "sury_ppa".to_string())
                }
                RequestPayload::PhpRuntimeList(_) => {
                    ("php.runtime_list".to_string(), "host".to_string())
                }
                RequestPayload::PhpRuntimeInstall(d) => {
                    ("php.runtime_install".to_string(), d.version.clone())
                }
                RequestPayload::PhpRuntimeRemove(d) => {
                    ("php.runtime_remove".to_string(), d.version.clone())
                }
                RequestPayload::PhpExtensionList(d) => {
                    ("php.extension_list".to_string(), d.version.clone())
                }
                RequestPayload::PhpExtensionInstall(d) => (
                    "php.extension_install".to_string(),
                    format!("php{}-{}", d.version, d.extension),
                ),
                RequestPayload::PhpExtensionToggle(d) => (
                    "php.extension_toggle".to_string(),
                    format!("php{}-{}", d.version, d.extension),
                ),
                RequestPayload::PhpExtensionRemove(d) => (
                    "php.extension_remove".to_string(),
                    format!("php{}-{}", d.version, d.extension),
                ),
                RequestPayload::PhpFpmMasterEnsure(d) => (
                    "php.fpm_master_ensure".to_string(),
                    format!("{}-{}", d.username, d.version),
                ),
                RequestPayload::PhpFpmPoolApply(d) => (
                    "php.fpm_pool_apply".to_string(),
                    format!("{}-{}/{}", d.username, d.version, d.pool_name),
                ),
                RequestPayload::PhpFpmPoolRemove(d) => (
                    "php.fpm_pool_remove".to_string(),
                    format!("{}-{}/{}", d.username, d.version, d.pool_name),
                ),
                RequestPayload::PhpFpmMasterStatus(d) => (
                    "php.fpm_master_status".to_string(),
                    format!("{}-{}", d.username, d.version),
                ),
                RequestPayload::PhpFpmMasterReload(d) => (
                    "php.fpm_master_reload".to_string(),
                    format!("{}-{}", d.username, d.version),
                ),
                RequestPayload::PhpFpmMasterRestart(d) => (
                    "php.fpm_master_restart".to_string(),
                    format!("{}-{}", d.username, d.version),
                ),
                RequestPayload::PhpFpmHealthCheck(d) => (
                    "php.fpm_health_check".to_string(),
                    format!("{}-{}/{}", d.username, d.version, d.pool_name),
                ),
                RequestPayload::PhpFpmPoolStatus(d) => (
                    "php.fpm_pool_status".to_string(),
                    format!("{}-{}/{}", d.username, d.version, d.pool_name),
                ),
                RequestPayload::PhpLogsRead(d) => (
                    "php.logs_read".to_string(),
                    format!("{}-{}/{}", d.username, d.version, d.log_type),
                ),
                RequestPayload::FsList(d) => {
                    ("fs.list".to_string(), format!("{}/{}", d.username, d.path))
                }
                RequestPayload::FsStat(d) => {
                    ("fs.stat".to_string(), format!("{}/{}", d.username, d.path))
                }
                RequestPayload::FsMkdir(d) => {
                    ("fs.mkdir".to_string(), format!("{}/{}", d.username, d.path))
                }
                RequestPayload::FsRename(d) => (
                    "fs.rename".to_string(),
                    format!("{}:{}->{}", d.username, d.source, d.destination),
                ),
                RequestPayload::FsCopy(d) => (
                    "fs.copy".to_string(),
                    format!("{}:{}->{}", d.username, d.source, d.destination),
                ),
                RequestPayload::FsTrash(d) => {
                    ("fs.trash".to_string(), format!("{}/{}", d.username, d.path))
                }
                RequestPayload::FsTrashList(d) => ("fs.trash_list".to_string(), d.username.clone()),
                RequestPayload::FsTrashRestore(d) => (
                    "fs.trash_restore".to_string(),
                    format!("{}/{}", d.username, d.trash_id),
                ),
                RequestPayload::FsTrashEmpty(d) => {
                    ("fs.trash_empty".to_string(), d.username.clone())
                }
                RequestPayload::FsDelete(d) => (
                    "fs.delete".to_string(),
                    format!("{}/{}", d.username, d.path),
                ),
                RequestPayload::FsUploadInit(d) => (
                    "fs.upload_init".to_string(),
                    format!("{}:{}/{}", d.username, d.target_path, d.file_name),
                ),
                RequestPayload::FsUploadGetSession(d) => (
                    "fs.upload_get_session".to_string(),
                    format!("{}:{}", d.username, d.upload_id),
                ),
                RequestPayload::FsUploadAppendChunk(d) => (
                    "fs.upload_append_chunk".to_string(),
                    format!("{}:{}:{}", d.username, d.upload_id, d.offset),
                ),
                RequestPayload::FsReadRange(d) => (
                    "fs.read_range".to_string(),
                    format!("{}:{}:{}-{}", d.username, d.path, d.start, d.length),
                ),
                RequestPayload::FsZipDirectory(d) => (
                    "fs.zip_directory".to_string(),
                    format!("{}:{}", d.username, d.path),
                ),
                RequestPayload::FsArchiveCompress(d) => (
                    "fs.archive_compress".to_string(),
                    format!("{}:{}", d.username, d.destination),
                ),
                RequestPayload::FsArchiveExtract(d) => (
                    "fs.archive_extract".to_string(),
                    format!("{}:{}", d.username, d.archive_path),
                ),
                RequestPayload::FsChmod(d) => (
                    "fs.chmod".to_string(),
                    format!("{}:{}:{:04o}", d.username, d.path, d.mode),
                ),
                RequestPayload::FsChmodRecursive(d) => (
                    "fs.chmod_recursive".to_string(),
                    format!(
                        "{}:{}:{:04o}:{:04o}",
                        d.username, d.path, d.file_mode, d.dir_mode
                    ),
                ),
                RequestPayload::FsChown(d) => (
                    "fs.chown".to_string(),
                    format!("{}:{}:{:?}:{:?}", d.username, d.path, d.uid, d.gid),
                ),
                RequestPayload::FsReadFile(d) => (
                    "fs.read_file".to_string(),
                    format!("{}:{}", d.username, d.path),
                ),
                RequestPayload::FsWriteAtomic(d) => (
                    "fs.write_atomic".to_string(),
                    format!("{}:{}:{}b", d.username, d.path, d.data.len()),
                ),
                RequestPayload::FsWriteEditor(d) => (
                    "fs.write_editor".to_string(),
                    format!("{}:{}:{}b", d.username, d.path, d.data.len()),
                ),
                RequestPayload::FsSearch(d) => (
                    "fs.search".to_string(),
                    format!("{}:{}:{}", d.username, d.path, d.query),
                ),
                RequestPayload::PtyOpen(d) => (
                    "pty.open".to_string(),
                    format!("{}:{:?}", d.username, d.cwd),
                ),
                RequestPayload::PtyInput(d) => ("pty.input".to_string(), d.session_id.clone()),
                RequestPayload::PtyReadOutput(d) => {
                    ("pty.read_output".to_string(), d.session_id.clone())
                }
                RequestPayload::PtyResize(d) => (
                    "pty.resize".to_string(),
                    format!("{}:{}x{}", d.session_id, d.rows, d.cols),
                ),
                RequestPayload::PtyClose(d) => ("pty.close".to_string(), d.session_id.clone()),
                RequestPayload::PtyList(d) => ("pty.list".to_string(), format!("{:?}", d.username)),
                RequestPayload::SshKeysSync(d) => {
                    ("ssh.sync_keys".to_string(), d.linux_username.clone())
                }
                RequestPayload::SshPolicyApply(d) => (
                    "ssh.policy_apply".to_string(),
                    format!("{}:{}", d.linux_username, d.policy),
                ),
                RequestPayload::SshDropinApply(_) => (
                    "ssh.dropin_apply".to_string(),
                    "/etc/ssh/sshd_config.d/50-panel.conf".to_string(),
                ),
                RequestPayload::NodeRuntimeList(_) => {
                    ("node.runtime_list".to_string(), "host".to_string())
                }
                RequestPayload::NodeRuntimeInstall(d) => {
                    ("node.runtime_install".to_string(), d.version.clone())
                }
                RequestPayload::NodeRuntimeRemove(d) => {
                    ("node.runtime_remove".to_string(), d.version.clone())
                }
                RequestPayload::NodeRuntimeCorepack(d) => {
                    ("node.runtime_corepack".to_string(), d.version.clone())
                }
                RequestPayload::NodeSiteApply(d) => {
                    ("node.site_apply".to_string(), d.site_id.clone())
                }
                RequestPayload::NodeSiteRemove(d) => {
                    ("node.site_remove".to_string(), d.site_id.clone())
                }
                RequestPayload::NodeSiteAction(d) => (
                    format!("node.site_{:?}", d.action).to_lowercase(),
                    d.site_id.clone(),
                ),
                RequestPayload::NodeSiteStatus(d) => {
                    ("node.site_status".to_string(), d.site_id.clone())
                }
                RequestPayload::NodeSiteJob(d) => (
                    "node.site_job".to_string(),
                    format!("{}:{:?}", d.site_id, d.job_type),
                ),
                RequestPayload::PythonRuntimeList(_) => {
                    ("python.runtime_list".to_string(), "host".to_string())
                }
                RequestPayload::PythonRuntimeInstall(d) => {
                    ("python.runtime_install".to_string(), d.version.clone())
                }
                RequestPayload::PythonRuntimeRemove(d) => {
                    ("python.runtime_remove".to_string(), d.version.clone())
                }
                RequestPayload::PythonSiteApply(d) => {
                    ("python.site_apply".to_string(), d.site_id.clone())
                }
                RequestPayload::PythonSiteRemove(d) => {
                    ("python.site_remove".to_string(), d.site_id.clone())
                }
                RequestPayload::PythonSiteAction(d) => (
                    format!("python.site_{:?}", d.action).to_lowercase(),
                    d.site_id.clone(),
                ),
                RequestPayload::PythonSiteStatus(d) => {
                    ("python.site_status".to_string(), d.site_id.clone())
                }
                RequestPayload::PythonSiteJob(d) => (
                    "python.site_job".to_string(),
                    format!("{}:{:?}", d.site_id, d.job_type),
                ),
                RequestPayload::ProxyUpstreamProbe(d) => {
                    ("proxy.upstream_probe".to_string(), d.upstream_url.clone())
                }
                RequestPayload::SiteEnvEncrypt(d) => {
                    ("site.env_encrypt".to_string(), d.site_id.clone())
                }
                RequestPayload::SiteEnvDecrypt(d) => {
                    ("site.env_decrypt".to_string(), d.site_id.clone())
                }
                RequestPayload::SiteEnvApply(d) => {
                    ("site.env_apply".to_string(), d.site_id.clone())
                }
                RequestPayload::CronJobApply(d) => (
                    "cron.job_apply".to_string(),
                    format!("{}:{}", d.site_id, d.job_id),
                ),
                RequestPayload::CronJobRemove(d) => (
                    "cron.job_remove".to_string(),
                    format!("{}:{}", d.site_id, d.job_id),
                ),
                RequestPayload::CronJobRunNow(d) => (
                    "cron.job_run_now".to_string(),
                    format!("{}:{}", d.site_id, d.job_id),
                ),
                RequestPayload::CronJobLogsRead(d) => (
                    "cron.job_logs_read".to_string(),
                    format!("{}:{}", d.site_id, d.job_id),
                ),
                RequestPayload::SiteDeployKeyGenerate(d) => {
                    ("git.key_generate".to_string(), d.site_id.clone())
                }
                RequestPayload::SiteGitDeploy(d) => (
                    "git.deploy".to_string(),
                    format!("{}:{}", d.site_id, d.release_id),
                ),
                RequestPayload::SiteGitRollback(d) => (
                    "git.rollback".to_string(),
                    format!("{}:{}", d.site_id, d.target_release_id),
                ),
                RequestPayload::SiteGitReleasesList(d) => {
                    ("git.releases_list".to_string(), d.base_dir.clone())
                }
                RequestPayload::DatabaseServerStatus(_) => {
                    ("database.server_status".to_string(), "host".to_string())
                }
                RequestPayload::DatabaseWebStatus => ("database.web_status".into(), "host".into()),
                RequestPayload::DatabaseWebCredentials(d) => (
                    "database.web_access".into(),
                    format!("{}:{}", d.engine, d.name),
                ),
                RequestPayload::DatabaseServerInstall(d) => {
                    ("database.server_install".to_string(), d.engine.clone())
                }
                RequestPayload::DatabaseServerAction(d) => {
                    (format!("database.server_{}", d.action), d.engine.clone())
                }
                RequestPayload::DatabaseCreate(d) => (
                    "database.create".to_string(),
                    format!("{}:{}", d.engine, d.name),
                ),
                RequestPayload::DatabaseDelete(d) => (
                    "database.delete".to_string(),
                    format!("{}:{}", d.engine, d.name),
                ),
                RequestPayload::DatabaseRotatePassword(d) => (
                    "database.rotate_password".to_string(),
                    format!("{}:{}", d.engine, d.name),
                ),
                RequestPayload::DatabaseUpdateRemoteAccess(d) => (
                    "database.update_remote_access".to_string(),
                    format!("{}:{}", d.engine, d.name),
                ),
                RequestPayload::DatabaseExport(d) => (
                    "database.export".to_string(),
                    format!("{}:{}", d.engine, d.name),
                ),
                RequestPayload::DatabaseImport(d) => (
                    "database.import".to_string(),
                    format!("{}:{}", d.engine, d.name),
                ),
                RequestPayload::DatabaseSizeCheck(d) => (
                    "database.size_check".to_string(),
                    format!("{}:{}", d.engine, d.name),
                ),
                RequestPayload::BackupCreate(d) => (
                    "backup.create".to_string(),
                    format!("{}:{}", d.site_id, d.filename),
                ),
                RequestPayload::BackupRestore(d) => (
                    "backup.restore".to_string(),
                    format!("{}:{}", d.site_id, d.backup_filepath),
                ),
                RequestPayload::BackupDelete(d) => {
                    ("backup.delete".to_string(), d.filepath.clone())
                }
                RequestPayload::OptimizerGetStatus(_) => {
                    ("optimizer.get_status".to_string(), "system".to_string())
                }
                RequestPayload::OptimizerApply(d) => (
                    "optimizer.apply".to_string(),
                    d.profile.as_str().to_string(),
                ),
                RequestPayload::OptimizerRollback(_) => {
                    ("optimizer.rollback".to_string(), "system".to_string())
                }
                RequestPayload::NftablesSync(_) => {
                    ("nftables.sync".to_string(), "loopback_guard".to_string())
                }
                RequestPayload::NftablesStatus(_) => {
                    ("nftables.status".to_string(), "loopback_guard".to_string())
                }
                RequestPayload::ConnectivityInspect(_) => {
                    ("connectivity.inspect".into(), "fixed-service".into())
                }
                RequestPayload::SshHostKeys => ("ssh.host_keys".into(), "local-openssh".into()),
                RequestPayload::SystemDoctorCheck(_) => {
                    ("system.doctor_check".to_string(), "system".to_string())
                }
                RequestPayload::SystemDoctorFix(_) => {
                    ("system.doctor_fix".to_string(), "system".to_string())
                }
                RequestPayload::SystemReconcile(_) => {
                    ("system.reconcile".to_string(), "system".to_string())
                }
                RequestPayload::SystemBackupCreate(d) => (
                    "system.backup_create".to_string(),
                    d.output_filename
                        .clone()
                        .unwrap_or_else(|| "auto".to_string()),
                ),
                RequestPayload::SystemBackupList(_) => {
                    ("system.backup_list".to_string(), "system".to_string())
                }
                RequestPayload::SystemBackupRestore(d) => (
                    "system.backup_restore".to_string(),
                    d.backup_filepath.clone(),
                ),
            };

            let response = match req.payload {
                RequestPayload::SshHostKeys => {
                    let manager = Arc::clone(&self.ssh_manager);
                    match tokio::task::spawn_blocking(move || manager.host_key_fingerprints()).await
                    {
                        Ok(Ok(keys)) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SshHostKeys(keys))
                        }
                        _ => RpcResponse::err(&req.request_id, "Identitas OpenSSH tidak tersedia."),
                    }
                }
                RequestPayload::ConnectivityInspect(target) => RpcResponse::ok(
                    &req.request_id,
                    ResponsePayload::ConnectivityInspected(connectivity::inspect(target).await),
                ),
                RequestPayload::ManagedStorage(operation) => {
                    let runner = Arc::clone(&self.runner);
                    match tokio::task::spawn_blocking(move || {
                        storage_managed::execute(runner.as_ref(), operation)
                    })
                    .await
                    {
                        Ok(Ok(value)) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::ManagedStorage(value))
                        }
                        _ => RpcResponse::err(
                            &req.request_id,
                            "Layanan storage tidak dapat diproses. Periksa agent dan journal layanan.",
                        ),
                    }
                }
                RequestPayload::StorageSecret(operation) => {
                    match storage_secret::execute(self.runner.as_ref(), operation) {
                        Ok(value) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::StorageSecret(value))
                        }
                        Err(_) => RpcResponse::err(
                            &req.request_id,
                            "Credential storage tidak dapat diproses.",
                        ),
                    }
                }
                RequestPayload::AiSecret(operation) => {
                    match ai::secret(self.runner.as_ref(), operation) {
                        Ok(value) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::AiSecret(value))
                        }
                        Err(_) => RpcResponse::err(&req.request_id, "Secret AI gagal diproses."),
                    }
                }
                RequestPayload::Email(operation) => {
                    let runner = Arc::clone(&self.runner);
                    match tokio::task::spawn_blocking(move || {
                        email::execute(runner.as_ref(), operation)
                    })
                    .await
                    {
                        Ok(Ok(value)) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::Email(value))
                        }
                        Ok(Err(error)) => RpcResponse::err(&req.request_id, error.to_string()),
                        Err(_) => RpcResponse::err(
                            &req.request_id,
                            "Operasi email terhenti; periksa status sebelum mencoba kembali",
                        ),
                    }
                }

                RequestPayload::ServerBackup(operation) => {
                    let runner = Arc::clone(&self.runner);
                    match tokio::task::spawn_blocking(move || {
                        server_backup::execute(runner.as_ref(), operation)
                    })
                    .await
                    {
                        Ok(Ok(value)) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::ServerBackup(value))
                        }
                        Ok(Err(error)) => RpcResponse::err(&req.request_id, error.to_string()),
                        Err(_) => {
                            RpcResponse::err(&req.request_id, "Status backup tidak tersedia.")
                        }
                    }
                }
                RequestPayload::PanelUpdate(operation) => {
                    let runner = Arc::clone(&self.runner);
                    match tokio::task::spawn_blocking(move || {
                        panel_update::execute(runner.as_ref(), operation)
                    })
                    .await
                    {
                        Ok(Ok(value)) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PanelUpdate(value))
                        }
                        Ok(Err(error)) => RpcResponse::err(&req.request_id, error.to_string()),
                        Err(_) => RpcResponse::err(
                            &req.request_id,
                            "Status updater tidak tersedia; periksa sebelum mencoba kembali.",
                        ),
                    }
                }
                RequestPayload::Dns01(operation) => {
                    static DNS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
                    let _guard = DNS_LOCK.lock().await;
                    let runner = Arc::clone(&self.runner);
                    match tokio::task::spawn_blocking(move || {
                        dns01::execute(runner.as_ref(), operation)
                    })
                    .await
                    {
                        Ok(Ok(value)) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::Dns01(value))
                        }
                        Ok(Err(e)) => RpcResponse::err(&req.request_id, e.to_string()),
                        Err(_) => RpcResponse::err(
                            &req.request_id,
                            "Operasi DNS-01 terputus; periksa jurnal cleanup.",
                        ),
                    }
                }
                RequestPayload::Cloudflare(operation) => {
                    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
                    let _guard = LOCK.lock().await;
                    let runner = Arc::clone(&self.runner);
                    match tokio::task::spawn_blocking(move || {
                        cloudflare::execute(runner.as_ref(), operation)
                    })
                    .await
                    {
                        Ok(Ok(value)) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::Cloudflare(value))
                        }
                        Ok(Err(error)) => RpcResponse::err(&req.request_id, error.to_string()),
                        Err(_) => RpcResponse::err(
                            &req.request_id,
                            "Operasi Cloudflare terhenti. Periksa status sebelum mencoba kembali.",
                        ),
                    }
                }
                RequestPayload::InstallWithProgress(operation) => {
                    match installation::run(
                        Arc::clone(&self.runner),
                        operation,
                        &mut rpc,
                        &req.request_id,
                    )
                    .await
                    {
                        Ok(payload) => RpcResponse::ok(&req.request_id, payload),
                        Err(error) => RpcResponse::err(&req.request_id, error.to_string()),
                    }
                }
                RequestPayload::Ping => RpcResponse::ok(&req.request_id, ResponsePayload::Pong),
                RequestPayload::Version => {
                    let info = VersionInfo {
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        proto_version: PROTOCOL_VERSION,
                        os: std::env::consts::OS.to_string(),
                        arch: std::env::consts::ARCH.to_string(),
                    };
                    RpcResponse::ok(&req.request_id, ResponsePayload::Version(info))
                }
                RequestPayload::UserCreate(data) => match self.user_manager.create_user(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::UserCreated(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::UserSuspend(data) => match self.user_manager.suspend_user(data) {
                    Ok(res) => {
                        RpcResponse::ok(&req.request_id, ResponsePayload::UserSuspended(res))
                    }
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::UserUnsuspend(data) => {
                    match self.user_manager.unsuspend_user(data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::UserUnsuspended(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::UserDelete(data) => match self.user_manager.delete_user(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::UserDeleted(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::SliceApply(data) => match self.slice_manager.apply_slice(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::SliceApplied(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::SliceRemove(data) => match self.slice_manager.remove_slice(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::SliceRemoved(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::QuotaSet(data) => match self.quota_manager.set_quota(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::QuotaSet(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::QuotaGet(data) => match self.quota_manager.get_quota(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::QuotaGet(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::QuotaScan(data) => match self.quota_manager.scan_quota(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::QuotaScan(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::ResourceRead(data) => {
                    match self.resource_reader.read_slice_resources(data.uid) {
                        Ok(metrics) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::ResourceRead(panel_proto::ResourceReadResponse {
                                metrics,
                            }),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::HardeningCheck => RpcResponse::ok(
                    &req.request_id,
                    ResponsePayload::HardeningStatus(Box::new(self.hardening_manager.check_all())),
                ),
                RequestPayload::HardeningApply(data) => match self.hardening_manager.apply(data) {
                    Ok(res) => {
                        RpcResponse::ok(&req.request_id, ResponsePayload::HardeningApplied(res))
                    }
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::NginxBaselineApply(data) => {
                    match self.webserver_manager.apply_baseline(data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::NginxBaselineApplied(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteApply(data) => match self.webserver_manager.apply_site(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::SiteApplied(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::SiteRemove(data) => {
                    match self.webserver_manager.remove_site(data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SiteRemoved(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteHtpasswdDeploy(data) => {
                    match self.webserver_manager.deploy_site_htpasswd(data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SiteHtpasswdDeployed(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteHtpasswdRemove(data) => {
                    match self.webserver_manager.remove_site_htpasswd(data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SiteHtpasswdRemoved(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteProvision(data) => {
                    match self.webserver_manager.provision_site(data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SiteProvisioned(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::AcmeChallengeDeploy(data) => {
                    match self.webserver_manager.deploy_acme_challenge(data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::AcmeChallengeDeployed(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::AcmeChallengeCleanup(data) => {
                    match self.webserver_manager.cleanup_acme_challenge(data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::AcmeChallengeCleaned(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SslCertDeploy(data) => {
                    match self.webserver_manager.deploy_ssl_cert(data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SslCertDeployed(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PanelTls(data) => match self.webserver_manager.panel_tls(data) {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::PanelTls(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::SiteLogsRead(data) => {
                    match self.webserver_manager.read_site_logs(data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SiteLogsRead(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpRepoSetup(_) => {
                    match php::setup_php_repository(self.runner.as_ref()) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PhpRepoSetup(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpRuntimeList(_) => {
                    match php::list_php_runtimes(self.runner.as_ref()) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PhpRuntimeList(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpRuntimeInstall(data) => {
                    match php::install_php_runtime(self.runner.as_ref(), &data.version) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpRuntimeInstalled(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpRuntimeRemove(data) => {
                    match php::remove_php_runtime(self.runner.as_ref(), &data.version) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpRuntimeRemoved(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpExtensionList(data) => {
                    match php::list_php_extensions(self.runner.as_ref(), &data.version) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PhpExtensionList(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpExtensionInstall(data) => {
                    match php::install_php_extension(
                        self.runner.as_ref(),
                        &data.version,
                        &data.extension,
                    ) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpExtensionAction(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpExtensionToggle(data) => {
                    match php::toggle_php_extension(
                        self.runner.as_ref(),
                        &data.version,
                        &data.extension,
                        data.enabled,
                    ) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpExtensionAction(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpExtensionRemove(data) => {
                    match php::remove_php_extension(
                        self.runner.as_ref(),
                        &data.version,
                        &data.extension,
                    ) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpExtensionAction(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpFpmMasterEnsure(data) => {
                    match php::ensure_php_fpm_master(self.runner.as_ref(), data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpFpmMasterEnsure(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpFpmPoolApply(data) => {
                    match php::apply_php_fpm_pool(self.runner.as_ref(), data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PhpFpmPoolApply(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpFpmPoolRemove(data) => {
                    match php::remove_php_fpm_pool(self.runner.as_ref(), data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PhpFpmPoolRemove(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpFpmMasterStatus(data) => {
                    match php::get_php_fpm_master_status(self.runner.as_ref(), data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpFpmMasterStatus(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpFpmMasterReload(data) => {
                    match php::reload_php_fpm_master(self.runner.as_ref(), data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpFpmMasterAction(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpFpmMasterRestart(data) => {
                    match php::restart_php_fpm_master(self.runner.as_ref(), data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpFpmMasterAction(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpFpmHealthCheck(data) => {
                    match php::check_php_fpm_health(self.runner.as_ref(), data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PhpFpmHealthChecked(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpFpmPoolStatus(data) => {
                    match php::get_php_fpm_pool_status(self.runner.as_ref(), data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PhpFpmPoolStatus(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PhpLogsRead(data) => {
                    match php::read_php_logs(self.runner.as_ref(), data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PhpLogsRead(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsList(data) => match self.fs_manager.list_dir(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsList(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsStat(data) => match self.fs_manager.stat(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsStat(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsMkdir(data) => match self.fs_manager.mkdir(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsAction(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsRename(data) => match self.fs_manager.rename(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsAction(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsCopy(data) => match self.fs_manager.copy(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsAction(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsTrash(data) => match self.fs_manager.trash(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsTrash(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsTrashList(data) => match self.fs_manager.trash_list(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsTrashList(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsTrashRestore(data) => {
                    match self.fs_manager.trash_restore(data).await {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::FsTrashRestore(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsTrashEmpty(data) => match self.fs_manager.trash_empty(data).await
                {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsTrashEmpty(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsDelete(data) => match self.fs_manager.delete(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsAction(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsUploadInit(data) => {
                    match self.fs_manager.upload_init(data).await {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::FsUploadInit(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsUploadGetSession(data) => {
                    match self.fs_manager.upload_get_session(data).await {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::FsUploadGetSession(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsUploadAppendChunk(data) => {
                    match self.fs_manager.upload_append_chunk(data).await {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::FsUploadAppendChunk(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsReadRange(data) => match self.fs_manager.read_range(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsReadRange(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsZipDirectory(data) => {
                    match self.fs_manager.zip_directory(data).await {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::FsZipDirectory(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsArchiveCompress(data) => {
                    match self.fs_manager.archive_compress(data).await {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::FsArchiveCompress(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsArchiveExtract(data) => {
                    match self.fs_manager.archive_extract(data).await {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::FsArchiveExtract(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsChmod(data) => match self.fs_manager.chmod(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsChmod(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsChmodRecursive(data) => {
                    match self.fs_manager.chmod_recursive(data).await {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::FsChmodRecursive(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsChown(data) => match self.fs_manager.chown(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsChown(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsReadFile(data) => match self.fs_manager.read_file(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsReadFile(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::FsWriteAtomic(data) => {
                    match self.fs_manager.write_atomic(data).await {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::FsWriteAtomic(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsWriteEditor(data) => {
                    match self.fs_manager.write_editor(data).await {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::FsWriteEditor(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::FsSearch(data) => match self.fs_manager.search(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::FsSearch(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::PtyOpen(data) => match self.pty_manager.open_session(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::PtyOpen(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::PtyInput(data) => match self.pty_manager.write_input(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::PtyInput(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::PtyReadOutput(data) => {
                    match self.pty_manager.read_output(data).await {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PtyReadOutput(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PtyResize(data) => match self.pty_manager.resize(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::PtyResize(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::PtyClose(data) => match self.pty_manager.close(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::PtyClose(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::PtyList(data) => match self.pty_manager.list_sessions(data).await {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::PtyList(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::SshKeysSync(data) => match self
                    .ssh_manager
                    .sync_authorized_keys(&data)
                {
                    Ok(res) => RpcResponse::ok(&req.request_id, ResponsePayload::SshKeysSync(res)),
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::SshPolicyApply(data) => {
                    match self.ssh_manager.apply_user_shell_policy(&data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SshPolicyApply(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SshDropinApply(data) => {
                    match self.ssh_manager.apply_sshd_dropin(&data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SshDropinApply(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeRuntimeList(data) => {
                    match node::list_node_runtimes(self.runner.as_ref(), data.base_dir.as_deref()) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::NodeRuntimeList(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeRuntimeInstall(data) => {
                    match node::install_node_runtime_connected(
                        Arc::clone(&self.runner),
                        data,
                        &mut rpc,
                    )
                    .await
                    {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::NodeRuntimeInstalled(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeRuntimeRemove(data) => {
                    let _guard = node::NODE_INSTALL_LOCK.lock().await;
                    match node::remove_node_runtime(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::NodeRuntimeRemoved(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeRuntimeCorepack(data) => {
                    let _guard = node::NODE_INSTALL_LOCK.lock().await;
                    match node::enable_node_corepack(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::NodeRuntimeCorepack(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeSiteApply(data) => {
                    match node_site::apply_node_site(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::NodeSiteApplied(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeSiteRemove(data) => {
                    match node_site::remove_node_site(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::NodeSiteRemoved(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeSiteAction(data) => {
                    match node_site::action_node_site(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::NodeSiteAction(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeSiteStatus(data) => {
                    match node_site::status_node_site(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::NodeSiteStatus(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NodeSiteJob(data) => {
                    match node_site::run_node_site_job(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::NodeSiteJob(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PythonRuntimeList(data) => {
                    match python_site::list_python_runtimes(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PythonRuntimeList(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PythonRuntimeInstall(data) => {
                    match python_site::install_python_runtime_connected(
                        std::sync::Arc::clone(&self.runner),
                        data,
                        &mut rpc,
                    )
                    .await
                    {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PythonRuntimeInstalled(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PythonRuntimeRemove(data) => {
                    let _guard = python_site::PYTHON_INSTALL_LOCK.lock().await;
                    match python_site::remove_python_runtime(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PythonRuntimeRemoved(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PythonSiteApply(data) => {
                    match python_site::apply_python_site(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PythonSiteApplied(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PythonSiteRemove(data) => {
                    match python_site::remove_python_site(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::PythonSiteRemoved(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PythonSiteAction(data) => {
                    match python_site::action_python_site(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PythonSiteAction(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PythonSiteStatus(data) => {
                    match python_site::status_python_site(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PythonSiteStatus(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::PythonSiteJob(data) => {
                    match python_site::run_python_site_job(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::PythonSiteJob(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::ProxyUpstreamProbe(data) => {
                    let res = proxy::probe_upstream(&data).await;
                    RpcResponse::ok(&req.request_id, ResponsePayload::ProxyUpstreamProbed(res))
                }
                RequestPayload::SiteEnvEncrypt(data) => {
                    match crypto::get_or_create_master_key(self.runner.as_ref(), None) {
                        Ok(master_key) => {
                            match crypto::encrypt_env_vars(&master_key, &data.env_vars) {
                                Ok(encrypted_vars) => RpcResponse::ok(
                                    &req.request_id,
                                    ResponsePayload::SiteEnvEncrypted(
                                        panel_proto::SiteEnvEncryptResponse {
                                            site_id: data.site_id,
                                            encrypted_vars,
                                        },
                                    ),
                                ),
                                Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                            }
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteEnvDecrypt(data) => {
                    match crypto::get_or_create_master_key(self.runner.as_ref(), None) {
                        Ok(master_key) => {
                            match crypto::decrypt_env_vars(&master_key, &data.encrypted_vars) {
                                Ok(env_vars) => RpcResponse::ok(
                                    &req.request_id,
                                    ResponsePayload::SiteEnvDecrypted(
                                        panel_proto::SiteEnvDecryptResponse {
                                            site_id: data.site_id,
                                            env_vars,
                                        },
                                    ),
                                ),
                                Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                            }
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteEnvApply(data) => {
                    match crypto::apply_site_env_file(
                        self.runner.as_ref(),
                        &data.site_id,
                        &data.username,
                        data.env_file_path.as_deref(),
                        &data.encrypted_vars,
                        None,
                    ) {
                        Ok(path) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SiteEnvApplied(panel_proto::SiteEnvApplyResponse {
                                site_id: data.site_id,
                                env_file_path: path,
                                success: true,
                                message: "Variabel lingkungan terenkripsi berhasil dipasang"
                                    .to_string(),
                            }),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::CronJobApply(data) => {
                    match cron::apply_cron_job(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::CronJobApplied(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::CronJobRemove(data) => {
                    match cron::remove_cron_job(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::CronJobRemoved(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::CronJobRunNow(data) => {
                    match cron::run_now_cron_job(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::CronJobRunNow(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::CronJobLogsRead(data) => {
                    match cron::read_cron_job_logs(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::CronJobLogsRead(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteDeployKeyGenerate(data) => {
                    match git_deploy::generate_deploy_key(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SiteDeployKeyGenerated(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteGitDeploy(data) => {
                    match git_deploy::execute_git_deploy(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SiteGitDeployed(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteGitRollback(data) => {
                    match git_deploy::execute_git_rollback(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SiteGitRollbacked(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SiteGitReleasesList(data) => {
                    match git_deploy::list_git_releases(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SiteGitReleasesList(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseServerStatus(data) => {
                    match databases::get_database_server_status(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::DatabaseServerStatus(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseWebStatus => RpcResponse::ok(
                    &req.request_id,
                    ResponsePayload::DatabaseWebStatus(database_web::status(self.runner.as_ref())),
                ),
                RequestPayload::DatabaseWebCredentials(data) => {
                    match database_web::credentials(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::DatabaseWebCredentials(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseServerInstall(data) => {
                    match databases::install_database_server(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::DatabaseServerInstalled(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseServerAction(data) => {
                    match databases::action_database_server(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::DatabaseServerAction(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseCreate(data) => {
                    match databases::create_database(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::DatabaseCreated(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseDelete(data) => {
                    match databases::delete_database(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::DatabaseDeleted(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseRotatePassword(data) => {
                    match databases::rotate_database_password(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::DatabasePasswordRotated(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseUpdateRemoteAccess(data) => {
                    match databases::update_remote_access(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::DatabaseRemoteAccessUpdated(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseExport(data) => {
                    match databases::export_database(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::DatabaseExported(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseImport(data) => {
                    match databases::import_database(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::DatabaseImported(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::DatabaseSizeCheck(data) => {
                    match databases::check_database_size(self.runner.as_ref(), &data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::DatabaseSizeChecked(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::BackupCreate(data) => {
                    match backups::create_backup(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::BackupCreated(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::BackupRestore(data) => {
                    match backups::restore_backup(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::BackupRestored(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::BackupDelete(data) => {
                    match backups::delete_backup(self.runner.as_ref(), &data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::BackupDeleted(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::OptimizerGetStatus(_) => {
                    match self.optimizer_manager.get_status() {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::OptimizerStatus(Box::new(res)),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::OptimizerApply(data) => match self.optimizer_manager.apply(&data) {
                    Ok(res) => {
                        RpcResponse::ok(&req.request_id, ResponsePayload::OptimizerApplied(res))
                    }
                    Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                },
                RequestPayload::OptimizerRollback(data) => {
                    match self.optimizer_manager.rollback(&data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::OptimizerRollbacked(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NftablesSync(data) => {
                    match self.nftables_manager.sync_from_proto(&data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::NftablesSynced(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::NftablesStatus(data) => {
                    let res = self.nftables_manager.get_status_proto(&data);
                    RpcResponse::ok(&req.request_id, ResponsePayload::NftablesStatus(res))
                }
                RequestPayload::SystemDoctorCheck(data) => {
                    match self.reliability_manager.check_doctor(&data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SystemDoctorChecked(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SystemDoctorFix(data) => {
                    match self.reliability_manager.fix_doctor(&data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SystemDoctorFixed(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SystemReconcile(data) => {
                    match self.reliability_manager.reconcile(&data) {
                        Ok(res) => {
                            RpcResponse::ok(&req.request_id, ResponsePayload::SystemReconciled(res))
                        }
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SystemBackupCreate(data) => {
                    match self.reliability_manager.backup_create(&data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SystemBackupCreated(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SystemBackupList(data) => {
                    match self.reliability_manager.backup_list(&data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SystemBackupListed(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
                RequestPayload::SystemBackupRestore(data) => {
                    match self.reliability_manager.backup_restore(&data) {
                        Ok(res) => RpcResponse::ok(
                            &req.request_id,
                            ResponsePayload::SystemBackupRestored(res),
                        ),
                        Err(e) => RpcResponse::err(&req.request_id, e.to_string()),
                    }
                }
            };

            let audit_record = audit::AgentAuditRecord::new(
                "panel-api",
                action,
                target,
                if response.success { "success" } else { "error" },
                Some(format!("req_id:{}", req.request_id)),
            );
            audit::record_audit(&audit_record);

            rpc.send_response(&response).await?;
        }

        Ok(())
    }
}

pub async fn run_agent_socket(path: impl AsRef<Path>) -> Result<(), AgentError> {
    let config = AgentConfig {
        socket_path: path.as_ref().to_path_buf(),
        allowed_uid: None,
    };
    let (tx, rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::new(config));
    tokio::select! {
        res = server.run(rx) => res,
        _ = tokio::signal::ctrl_c() => {
            let _ = tx.send(());
            Ok(())
        }
    }
}

#[cfg(all(test, unix))]
mod ipc_security_tests {
    use super::*;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    #[tokio::test]
    async fn socket_mode_is_explicit_and_live_socket_is_preserved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("agent.sock");
        let listener = AgentServer::bind_unix_listener(&path).expect("bind");
        let metadata = std::fs::symlink_metadata(&path).expect("metadata");
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.mode() & 0o7777, 0o660);
        assert!(AgentServer::bind_unix_listener(&path).is_err());
        assert_eq!(
            std::fs::metadata(&path).expect("metadata").ino(),
            metadata.ino()
        );
        drop(listener);
    }

    #[tokio::test]
    async fn refuses_to_unlink_regular_file_or_symlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("agent.sock");
        std::fs::write(&path, "preserve").expect("write");
        assert!(AgentServer::bind_unix_listener(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "preserve");
        let link = dir.path().join("link.sock");
        std::os::unix::fs::symlink(&path, &link).expect("symlink");
        assert!(AgentServer::bind_unix_listener(&link).is_err());
        assert!(
            link.symlink_metadata()
                .expect("metadata")
                .file_type()
                .is_symlink()
        );
    }
}
