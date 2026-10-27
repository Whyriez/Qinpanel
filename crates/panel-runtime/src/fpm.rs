//! PHP-FPM master and pool domain models, configuration generators, and systemd service templates.
//!
//! Architecture according to PRD 4.4, 4.5, and 5.1:
//! - Master per (tenant, version) running under tenant identity (`User=p_<slug>`, `Group=p_<slug>`).
//! - Strictly sandboxed systemd service: `Slice=panel-u<uid>.slice`, `NoNewPrivileges=true`, `ProtectSystem=strict`,
//!   `TemporaryFileSystem=/home:ro` + `BindPaths=/home/p_<slug>`, `PrivateTmp=true`, `PrivateDevices=true`,
//!   `ProtectKernelTunables=true`, `ProtectKernelModules=true`, `ProtectControlGroups=true`, `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`.
//! - Per-site pools with isolated docroots, hardened `open_basedir`, `disable_functions`, per-site tmp/sessions,
//!   `pm = ondemand`, slowlog, and internal status paths.

use crate::error::RuntimeError;
use crate::php::PhpVersion;
use panel_core::tenant::{TenantUid, is_reserved_username};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_PHP_CONFIG_BASE: &str = "/etc/panel/php";
pub const DEFAULT_PHP_RUNTIME_BASE: &str = "/run/panel/php";
pub const DEFAULT_PHP_LOG_BASE: &str = "/var/log/panel";

pub const DEFAULT_PM_MAX_CHILDREN: u32 = 10;
pub const DEFAULT_PM_PROCESS_IDLE_TIMEOUT_SECS: u32 = 10;
pub const DEFAULT_PM_MAX_REQUESTS: u32 = 500;
pub const DEFAULT_REQUEST_SLOWLOG_TIMEOUT_SECS: u32 = 5;
pub const DEFAULT_STATUS_PATH: &str = "/fpm-status";
pub const DEFAULT_PING_PATH: &str = "/fpm-ping";

pub const DEFAULT_HARDENED_DISABLE_FUNCTIONS: &[&str] = &[
    "exec",
    "passthru",
    "shell_exec",
    "system",
    "proc_open",
    "popen",
    "curl_multi_exec",
    "parse_ini_file",
    "show_source",
];

/// Normalizes path to POSIX format (forward slashes) for configuration files.
pub fn to_posix_str(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Validates that a string contains only safe alphanumeric/hyphen/underscore characters (no newlines, injections, or traversal).
pub fn validate_fpm_identifier(value: &str) -> Result<(), RuntimeError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "Identifier tidak boleh kosong".to_string(),
        ));
    }
    if trimmed.len() > 64 {
        return Err(RuntimeError::InvalidConfig(format!(
            "Identifier terlalu panjang: '{trimmed}'"
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(RuntimeError::InvalidConfig(format!(
            "Identifier mengandung karakter berbahaya: '{trimmed}'"
        )));
    }
    if trimmed.contains("..") {
        return Err(RuntimeError::InvalidConfig(format!(
            "Identifier tidak boleh mengandung traversal: '{trimmed}'"
        )));
    }
    Ok(())
}

/// Configuration context for a tenant's PHP-FPM master instance for a specific PHP version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhpFpmMasterConfig {
    pub username: String,
    pub uid: u32,
    pub version: PhpVersion,
    pub config_base_dir: PathBuf,
    pub runtime_base_dir: PathBuf,
    pub log_base_dir: PathBuf,
}

impl PhpFpmMasterConfig {
    /// Constructs and validates a new PHP-FPM master configuration.
    pub fn new(
        username: impl Into<String>,
        uid: u32,
        version: PhpVersion,
    ) -> Result<Self, RuntimeError> {
        let user = username.into();
        validate_fpm_identifier(&user)?;

        let trimmed = user.trim();
        if is_reserved_username(trimmed) {
            return Err(RuntimeError::InvalidConfig(format!(
                "Username '{trimmed}' tidak diizinkan untuk master FPM tenant"
            )));
        }

        let tenant_uid =
            TenantUid::parse(uid).map_err(|e| RuntimeError::InvalidConfig(e.to_string()))?;

        Ok(Self {
            username: trimmed.to_string(),
            uid: tenant_uid.as_u32(),
            version,
            config_base_dir: PathBuf::from(DEFAULT_PHP_CONFIG_BASE),
            runtime_base_dir: PathBuf::from(DEFAULT_PHP_RUNTIME_BASE),
            log_base_dir: PathBuf::from(DEFAULT_PHP_LOG_BASE),
        })
    }

    /// With custom base directories (useful for hermetic tests).
    pub fn with_custom_dirs(
        mut self,
        config_base: impl AsRef<Path>,
        runtime_base: impl AsRef<Path>,
        log_base: impl AsRef<Path>,
    ) -> Self {
        self.config_base_dir = config_base.as_ref().to_path_buf();
        self.runtime_base_dir = runtime_base.as_ref().to_path_buf();
        self.log_base_dir = log_base.as_ref().to_path_buf();
        self
    }

    /// Systemd service unit name, e.g. "panel-fpm@p_alice-8.3.service".
    pub fn service_name(&self) -> String {
        format!(
            "panel-fpm@{}-{}.service",
            self.username,
            self.version.as_str()
        )
    }

    /// Systemd slice name, e.g. "panel-u10001.slice".
    pub fn slice_name(&self) -> String {
        format!("panel-u{}.slice", self.uid)
    }

    /// Master configuration directory, e.g. "/etc/panel/php/p_alice-8.3".
    pub fn config_dir(&self) -> PathBuf {
        self.config_base_dir
            .join(format!("{}-{}", self.username, self.version.as_str()))
    }

    /// Directory holding pool config files, e.g. "/etc/panel/php/p_alice-8.3/pool.d".
    pub fn pool_dir(&self) -> PathBuf {
        self.config_dir().join("pool.d")
    }

    /// Path to master config file, e.g. "/etc/panel/php/p_alice-8.3/php-fpm.conf".
    pub fn master_conf_path(&self) -> PathBuf {
        self.config_dir().join("php-fpm.conf")
    }

    /// Tenant-owned runtime directory; other tenants cannot create or unlink sockets here.
    pub fn runtime_dir(&self) -> PathBuf {
        self.runtime_base_dir
            .join(format!("{}-{}", self.username, self.version.as_str()))
    }

    /// Path to master PID file, e.g. "/run/panel/php/p_alice-8.3/master.pid".
    pub fn pid_path(&self) -> PathBuf {
        self.runtime_dir().join("master.pid")
    }

    /// Default Unix domain socket path, e.g. "/run/panel/php/p_alice-8.3/fpm.sock".
    pub fn default_socket_path(&self) -> PathBuf {
        self.runtime_dir().join("fpm.sock")
    }

    /// Log directory for this tenant, e.g. "/var/log/panel/p_alice".
    pub fn tenant_log_dir(&self) -> PathBuf {
        self.log_base_dir.join(&self.username)
    }

    /// Path to master error log file, e.g. "/var/log/panel/p_alice/php-fpm-8.3-error.log".
    pub fn error_log_path(&self) -> PathBuf {
        self.tenant_log_dir()
            .join(format!("php-fpm-{}-error.log", self.version.as_str()))
    }

    /// Path to master slow log file, e.g. "/var/log/panel/p_alice/php-fpm-8.3-slow.log".
    pub fn slow_log_path(&self) -> PathBuf {
        self.tenant_log_dir()
            .join(format!("php-fpm-{}-slow.log", self.version.as_str()))
    }

    /// Renders `php-fpm.conf` for the master process.
    pub fn render_master_conf(&self) -> String {
        self.render_master_conf_with_pools(&[self.pool_dir().join("*.conf")])
    }

    /// Renders a validation master with an explicit set of agent-controlled pools.
    pub fn render_master_conf_with_pools(&self, pools: &[PathBuf]) -> String {
        let includes = pools
            .iter()
            .map(|path| format!("include = {}", to_posix_str(path)))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"; Qinpanel - PHP-FPM Master Configuration
; Tenant: {username} (UID: {uid})
; PHP Version: {version}
; Auto-generated by Qinpanel. Do not edit directly.

[global]
pid = {pid_path}
error_log = {error_log_path}
log_level = warning
daemonize = no
process_control_timeout = 10s

{includes}
"#,
            username = self.username,
            uid = self.uid,
            version = self.version.as_str(),
            pid_path = to_posix_str(&self.pid_path()),
            error_log_path = to_posix_str(&self.error_log_path()),
            includes = includes,
        )
    }

    /// Renders the systemd service unit file with sandboxing and cgroup slice attachment.
    pub fn render_systemd_unit(&self) -> String {
        let fpm_bin = to_posix_str(&self.version.fpm_binary_path());
        let master_conf = to_posix_str(&self.master_conf_path());
        let slice = self.slice_name();
        let username = &self.username;
        let config_dir = to_posix_str(&self.config_dir());
        let runtime_dir = to_posix_str(&self.runtime_dir());
        let tenant_log_dir = to_posix_str(&self.tenant_log_dir());

        format!(
            r#"# QonPanel - PHP-FPM Master Service
# Tenant: {username} (UID: {uid})
# PHP Version: {version}
# Auto-generated by Qinpanel. Do not edit directly.

[Unit]
Description=Qinpanel PHP-FPM Master ({username}-{version})
After=network.target
Wants=panel-agent.service
After=panel-agent.service
PartOf={slice}

[Service]
Type=notify
User={username}
Group={username}
Slice={slice}
ExecStart={fpm_bin} --nodaemonize --fpm-config {master_conf}
ExecReload=/bin/kill -USR2 $MAINPID
KillMode=process
Restart=on-failure
RestartSec=3s
LimitNOFILE=65535
RuntimeDirectory=panel/php/{username}-{version}
RuntimeDirectoryMode=0755
RuntimeDirectoryPreserve=restart
LogsDirectory=panel/{username}
LogsDirectoryMode=0750

# Sandbox & Hardening (PRD 4.4 & 5.1)
NoNewPrivileges=true
ProtectSystem=strict
TemporaryFileSystem=/home:ro
BindPaths=/home/{username}
BindReadOnlyPaths=-{config_dir}
ReadWritePaths={runtime_dir} {tenant_log_dir} /home/{username}/tmp
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6

[Install]
WantedBy=multi-user.target
"#,
            username = username,
            uid = self.uid,
            version = self.version.as_str(),
            slice = slice,
            fpm_bin = fpm_bin,
            master_conf = master_conf,
            config_dir = config_dir,
            runtime_dir = runtime_dir,
            tenant_log_dir = tenant_log_dir,
        )
    }
}

/// Configuration context for an isolated PHP-FPM pool (typically one per website).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhpFpmPoolConfig {
    pub pool_name: String,
    pub username: String,
    pub uid: u32,
    pub version: PhpVersion,
    pub socket_path: PathBuf,
    pub docroot: PathBuf,
    pub pm_max_children: u32,
    pub pm_process_idle_timeout_secs: u32,
    pub pm_max_requests: u32,
    pub open_basedir: Vec<PathBuf>,
    pub disable_functions: Vec<String>,
    pub upload_tmp_dir: PathBuf,
    pub session_save_path: PathBuf,
    pub slowlog_path: PathBuf,
    pub request_slowlog_timeout_secs: u32,
    pub status_path: String,
    pub ping_path: String,
    pub error_log_path: PathBuf,
    pub ini_directives: std::collections::BTreeMap<String, String>,
}

impl PhpFpmPoolConfig {
    /// Creates a hardened, default pool configuration for a tenant website.
    pub fn new(
        master: &PhpFpmMasterConfig,
        pool_name: impl Into<String>,
        docroot: impl AsRef<Path>,
    ) -> Result<Self, RuntimeError> {
        let name = pool_name.into();
        validate_fpm_identifier(&name)?;

        let docroot_path = docroot.as_ref().to_path_buf();
        let docroot_posix = to_posix_str(&docroot_path);
        if !docroot_posix.starts_with('/') && !docroot_path.is_absolute() {
            return Err(RuntimeError::InvalidConfig(
                "Docroot pool PHP-FPM harus berupa path absolut".to_string(),
            ));
        }

        let user_home = PathBuf::from(format!("/home/{}", master.username));
        let user_tmp = user_home.join("tmp");

        // Hardened open_basedir: docroot, user tmp, and /tmp
        let open_basedir = vec![
            docroot_path.clone(),
            user_tmp.clone(),
            PathBuf::from("/tmp"),
        ];

        let disable_functions = DEFAULT_HARDENED_DISABLE_FUNCTIONS
            .iter()
            .map(|s| s.to_string())
            .collect();

        Ok(Self {
            pool_name: name,
            username: master.username.clone(),
            uid: master.uid,
            version: master.version,
            socket_path: master.default_socket_path(),
            docroot: docroot_path,
            pm_max_children: DEFAULT_PM_MAX_CHILDREN,
            pm_process_idle_timeout_secs: DEFAULT_PM_PROCESS_IDLE_TIMEOUT_SECS,
            pm_max_requests: DEFAULT_PM_MAX_REQUESTS,
            open_basedir,
            disable_functions,
            upload_tmp_dir: user_tmp.clone(),
            session_save_path: user_tmp,
            slowlog_path: master.slow_log_path(),
            request_slowlog_timeout_secs: DEFAULT_REQUEST_SLOWLOG_TIMEOUT_SECS,
            status_path: DEFAULT_STATUS_PATH.to_string(),
            ping_path: DEFAULT_PING_PATH.to_string(),
            error_log_path: master.error_log_path(),
            ini_directives: panel_core::php::PhpSettings::default().directives(),
        })
    }

    /// Extends or overrides PHP INI directives for this pool.
    pub fn with_ini_directives(
        mut self,
        directives: std::collections::BTreeMap<String, String>,
    ) -> Self {
        self.ini_directives.extend(directives);
        self
    }

    /// Sets custom socket path for dedicated site pools.
    pub fn with_socket_path(mut self, path: impl AsRef<Path>) -> Self {
        self.socket_path = path.as_ref().to_path_buf();
        self
    }

    /// Sets process limits from plan.
    pub fn with_pm_limits(
        mut self,
        max_children: u32,
        idle_timeout_secs: u32,
        max_requests: u32,
    ) -> Self {
        self.pm_max_children = max_children.clamp(1, 200);
        self.pm_process_idle_timeout_secs = idle_timeout_secs.clamp(1, 300);
        self.pm_max_requests = max_requests.clamp(0, 10000);
        self
    }

    /// Path to this pool's `.conf` file inside the master's `pool.d` directory.
    pub fn pool_conf_path(&self, master: &PhpFpmMasterConfig) -> PathBuf {
        master.pool_dir().join(format!("{}.conf", self.pool_name))
    }

    /// Renders the pool configuration file content.
    pub fn render_pool_conf(&self) -> String {
        let open_basedir_str = self
            .open_basedir
            .iter()
            .map(|p| to_posix_str(p))
            .collect::<Vec<_>>()
            .join(":");

        let disable_functions_str = self.disable_functions.join(",");

        let slowlog_section = if self.request_slowlog_timeout_secs > 0 {
            format!(
                r#"slowlog = {slowlog_path}
request_slowlog_timeout = {slowlog_timeout}s"#,
                slowlog_path = to_posix_str(&self.slowlog_path),
                slowlog_timeout = self.request_slowlog_timeout_secs,
            )
        } else {
            "; slowlog is disabled".to_string()
        };

        let mut rendered = format!(
            r#"; Qinpanel - PHP-FPM Pool Configuration
; Pool: {pool_name}
; Tenant: {username} (UID: {uid})
; PHP Version: {version}
; Auto-generated by Qinpanel. Do not edit directly.

[{pool_name}]
user = {username}
group = {username}

listen = {socket_path}
listen.acl_users = www-data
listen.mode = 0660

; Process Manager: ondemand (RAM efficient for idle sites)
pm = ondemand
pm.max_children = {max_children}
pm.process_idle_timeout = {idle_timeout}s
pm.max_requests = {max_requests}

; Internal Healthcheck & Status Paths (PRD 5.1 & 4.4)
pm.status_path = {status_path}
ping.path = {ping_path}
ping.response = pong

; Slowlog Monitoring
{slowlog_section}

; Logging and Output Handling
catch_workers_output = yes
decorate_workers_output = no

; Hardened PHP Directives
php_admin_value[open_basedir] = {open_basedir_str}
php_admin_value[disable_functions] = {disable_functions_str}
php_admin_value[upload_tmp_dir] = {upload_tmp_dir}
php_admin_value[session.save_path] = {session_save_path}
php_admin_flag[log_errors] = on
php_admin_value[error_log] = {error_log_path}
"#,
            pool_name = self.pool_name,
            username = self.username,
            uid = self.uid,
            version = self.version.as_str(),
            socket_path = to_posix_str(&self.socket_path),
            max_children = self.pm_max_children,
            idle_timeout = self.pm_process_idle_timeout_secs,
            max_requests = self.pm_max_requests,
            status_path = self.status_path,
            ping_path = self.ping_path,
            slowlog_section = slowlog_section,
            open_basedir_str = open_basedir_str,
            disable_functions_str = disable_functions_str,
            upload_tmp_dir = to_posix_str(&self.upload_tmp_dir),
            session_save_path = to_posix_str(&self.session_save_path),
            error_log_path = to_posix_str(&self.error_log_path),
        );

        for (k, v) in &self.ini_directives {
            let is_flag = matches!(
                k.as_str(),
                "display_errors"
                    | "display_startup_errors"
                    | "opcache.enable"
                    | "opcache.validate_timestamps"
                    | "allow_url_fopen"
                    | "expose_php"
                    | "short_open_tag"
            );
            if is_flag {
                let flag_val = match v.as_str() {
                    "1" | "on" | "On" | "true" | "True" => "on",
                    _ => "off",
                };
                rendered.push_str(&format!("php_admin_flag[{k}] = {flag_val}\n"));
            } else {
                rendered.push_str(&format!("php_admin_value[{k}] = {v}\n"));
            }
        }

        rendered
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tenant_service_is_enableable_and_uses_private_writable_runtime() {
        let master =
            PhpFpmMasterConfig::new("p_alice", 10001, PhpVersion::parse("8.3").unwrap()).unwrap();
        let unit = master.render_systemd_unit();
        assert!(unit.contains("[Install]\nWantedBy=multi-user.target"));
        assert!(unit.contains("Wants=panel-agent.service\nAfter=panel-agent.service"));
        assert!(unit.contains("RuntimeDirectory=panel/php/p_alice-8.3"));
        assert!(unit.contains("ReadWritePaths=/run/panel/php/p_alice-8.3 "));
        assert_eq!(
            master.default_socket_path(),
            PathBuf::from("/run/panel/php/p_alice-8.3/fpm.sock")
        );
        let pool = PhpFpmPoolConfig::new(&master, "site_1", "/home/p_alice/sites/example/app")
            .unwrap()
            .render_pool_conf();
        assert!(pool.contains("listen.acl_users = www-data"));
        assert!(!pool.contains("listen.group = www-data"));
    }

    #[test]
    fn test_validate_fpm_identifier() {
        assert!(validate_fpm_identifier("p_alice").is_ok());
        assert!(validate_fpm_identifier("site-1").is_ok());
        assert!(validate_fpm_identifier("example.com").is_ok());

        assert!(validate_fpm_identifier("").is_err());
        assert!(validate_fpm_identifier("p_alice; rm -rf /").is_err());
        assert!(validate_fpm_identifier("foo\nbar").is_err());
        assert!(validate_fpm_identifier("../etc/passwd").is_err());
    }

    #[test]
    fn test_fpm_master_config_paths_and_rendering() {
        let master = PhpFpmMasterConfig::new("p_alice", 10001, PhpVersion::Php83).unwrap();
        assert_eq!(master.service_name(), "panel-fpm@p_alice-8.3.service");
        assert_eq!(master.slice_name(), "panel-u10001.slice");
        assert_eq!(
            master.config_dir(),
            PathBuf::from("/etc/panel/php/p_alice-8.3")
        );
        assert_eq!(
            master.master_conf_path(),
            PathBuf::from("/etc/panel/php/p_alice-8.3/php-fpm.conf")
        );
        assert_eq!(
            master.default_socket_path(),
            PathBuf::from("/run/panel/php/p_alice-8.3/fpm.sock")
        );

        let master_conf = master.render_master_conf();
        assert!(master_conf.contains("pid = /run/panel/php/p_alice-8.3/master.pid"));
        assert!(master_conf.contains("error_log = /var/log/panel/p_alice/php-fpm-8.3-error.log"));
        assert!(master_conf.contains("include = /etc/panel/php/p_alice-8.3/pool.d/*.conf"));

        let unit = master.render_systemd_unit();
        assert!(unit.contains("User=p_alice"));
        assert!(unit.contains("Group=p_alice"));
        assert!(unit.contains("Slice=panel-u10001.slice"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("TemporaryFileSystem=/home:ro"));
        assert!(unit.contains("BindPaths=/home/p_alice"));
        assert!(unit.contains(
            "ReadWritePaths=/run/panel/php/p_alice-8.3 /var/log/panel/p_alice /home/p_alice/tmp"
        ));
        assert!(unit.contains("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6"));
    }

    #[test]
    fn test_fpm_pool_config_paths_and_rendering() {
        let master = PhpFpmMasterConfig::new("p_alice", 10001, PhpVersion::Php83).unwrap();
        let pool =
            PhpFpmPoolConfig::new(&master, "site-42", "/home/p_alice/sites/example.com/public")
                .unwrap();

        assert_eq!(
            pool.pool_conf_path(&master),
            PathBuf::from("/etc/panel/php/p_alice-8.3/pool.d/site-42.conf")
        );

        let rendered = pool.render_pool_conf();
        assert!(rendered.contains("[site-42]"));
        assert!(rendered.contains("user = p_alice"));
        assert!(rendered.contains("listen = /run/panel/php/p_alice-8.3/fpm.sock"));
        assert!(rendered.contains("listen.acl_users = www-data"));
        assert!(!rendered.contains("listen.group = www-data"));
        assert!(rendered.contains("listen.mode = 0660"));
        assert!(rendered.contains("pm = ondemand"));
        assert!(rendered.contains("pm.max_children = 10"));
        assert!(rendered.contains("pm.status_path = /fpm-status"));
        assert!(rendered.contains("ping.path = /fpm-ping"));
        assert!(rendered.contains("slowlog = /var/log/panel/p_alice/php-fpm-8.3-slow.log"));
        assert!(rendered.contains("request_slowlog_timeout = 5s"));
        assert!(rendered.contains("php_admin_value[open_basedir] = /home/p_alice/sites/example.com/public:/home/p_alice/tmp:/tmp"));
        assert!(rendered.contains("php_admin_value[disable_functions] = exec,passthru,shell_exec,system,proc_open,popen,curl_multi_exec,parse_ini_file,show_source"));
        assert!(rendered.contains("php_admin_value[memory_limit] = 128M"));
        assert!(rendered.contains("php_admin_flag[display_errors] = off"));
        assert!(rendered.contains("php_admin_flag[opcache.enable] = on"));

        // Custom ini directives override
        let mut custom_directives = std::collections::BTreeMap::new();
        custom_directives.insert("memory_limit".to_string(), "512M".to_string());
        custom_directives.insert("display_errors".to_string(), "1".to_string());
        let custom_pool = pool.with_ini_directives(custom_directives);
        let custom_rendered = custom_pool.render_pool_conf();
        assert!(custom_rendered.contains("php_admin_value[memory_limit] = 512M"));
        assert!(custom_rendered.contains("php_admin_flag[display_errors] = on"));
    }

    #[test]
    fn test_fpm_reserved_user_and_injection_rejected() {
        assert!(PhpFpmMasterConfig::new("root", 10001, PhpVersion::Php83).is_err());
        assert!(PhpFpmMasterConfig::new("p_root", 10001, PhpVersion::Php83).is_err());
        assert!(PhpFpmMasterConfig::new("www-data", 10001, PhpVersion::Php83).is_err());
        assert!(PhpFpmMasterConfig::new("p_alice\nmalicious=1", 10001, PhpVersion::Php83).is_err());
        assert!(PhpFpmMasterConfig::new("p_alice", 9999, PhpVersion::Php83).is_err());
    }
}
