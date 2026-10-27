//! Python site process management: systemd unit generation, environment file writing,
//! Unix domain socket management, service lifecycle (start, stop, restart, redeploy, status),
//! runtime version management via `uv`, and tenant-UID job execution within tenant cgroup slice.

use crate::AgentError;
use crate::system::SystemRunner;
use panel_proto::{
    PythonInstalledVersionDto, PythonRuntimeActionResponse, PythonRuntimeInstallRequest,
    PythonRuntimeListResponse, PythonRuntimeRemoveRequest, PythonSiteActionRequest,
    PythonSiteActionResponse, PythonSiteActionType, PythonSiteApplyRequest,
    PythonSiteApplyResponse, PythonSiteEntrypoint, PythonSiteFramework, PythonSiteJobRequest,
    PythonSiteJobResponse, PythonSiteJobType, PythonSiteRemoveRequest, PythonSiteServer,
    PythonSiteStatusRequest, PythonSiteStatusResponse,
};
use panel_runtime::python::{
    DEFAULT_PYTHON_BASE_DIR, DEFAULT_PYTHON_SOCKET_DIR, DEFAULT_UV_BIN, build_python_exec_start,
    build_uv_pip_install_pyproject_cmd, build_uv_pip_install_requirements_cmd,
    build_uv_python_install_cmd, build_uv_venv_cmd, list_installed_python_versions,
    normalize_python_version,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const DEFAULT_SYSTEMD_DIR: &str = "/etc/systemd/system";
pub const DEFAULT_ENV_DIR: &str = "/etc/panel/env";
#[cfg(test)]
#[path = "python_cancel_tests.rs"]
mod cancellation_tests;
pub(crate) static PYTHON_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn install_python_runtime_connected<T>(
    runner: std::sync::Arc<dyn SystemRunner>,
    req: PythonRuntimeInstallRequest,
    rpc: &mut panel_proto::RpcStream<T>,
) -> Result<PythonRuntimeActionResponse, AgentError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    crate::runtime_install::connected(rpc, &PYTHON_INSTALL_LOCK, move |cancel| {
        install_python_runtime_cancellable(runner.as_ref(), &req, &cancel)
    })
    .await
}

/// Validates site ID string (alphanumeric, hyphens, underscores).
pub fn validate_site_id(site_id: &str) -> Result<(), AgentError> {
    if site_id.is_empty() {
        return Err(AgentError::InvalidInput(
            "ID situs tidak boleh kosong".into(),
        ));
    }
    if !site_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AgentError::InvalidInput(format!(
            "ID situs '{}' mengandung karakter ilegal",
            site_id
        )));
    }
    Ok(())
}

/// Validates username format.
pub fn validate_username(username: &str) -> Result<(), AgentError> {
    if username.is_empty() {
        return Err(AgentError::InvalidInput(
            "Username tidak boleh kosong".into(),
        ));
    }
    let first = username.chars().next().unwrap_or('_');
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(AgentError::InvalidInput(format!(
            "Username '{}' harus diawali huruf atau underscore",
            username
        )));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AgentError::InvalidInput(format!(
            "Username '{}' mengandung karakter tidak valid",
            username
        )));
    }
    Ok(())
}

/// Validates entrypoint configuration.
pub fn validate_entrypoint(entrypoint: &PythonSiteEntrypoint) -> Result<(), AgentError> {
    match entrypoint {
        PythonSiteEntrypoint::Wsgi(ep) | PythonSiteEntrypoint::Asgi(ep) => {
            if ep.trim().is_empty() {
                return Err(AgentError::InvalidInput(
                    "Entrypoint Python tidak boleh kosong".into(),
                ));
            }
            if ep.contains(';')
                || ep.contains('&')
                || ep.contains('|')
                || ep.contains('`')
                || ep.contains('$')
            {
                return Err(AgentError::InvalidInput(
                    "Entrypoint Python mengandung karakter shell berbahaya".into(),
                ));
            }
            if !ep.contains(':') && !ep.ends_with(".py") {
                return Err(AgentError::InvalidInput(
                    "Entrypoint Python harus berformat 'module:app' atau 'file.py'".into(),
                ));
            }
        }
        PythonSiteEntrypoint::CustomCommand(cmd) => {
            if cmd.trim().is_empty() {
                return Err(AgentError::InvalidInput(
                    "Custom command tidak boleh kosong".into(),
                ));
            }
            if cmd.contains(';')
                || cmd.contains('&')
                || cmd.contains('|')
                || cmd.contains('`')
                || cmd.contains('$')
            {
                return Err(AgentError::InvalidInput(
                    "Custom command mengandung karakter shell berbahaya".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Validates Unix domain socket path.
pub fn validate_socket_path(socket_path: &str) -> Result<(), AgentError> {
    let clean = socket_path.strip_prefix("unix:").unwrap_or(socket_path);
    if clean.trim().is_empty() {
        return Err(AgentError::InvalidInput(
            "Path Unix socket tidak boleh kosong".into(),
        ));
    }
    if !clean.starts_with('/') {
        return Err(AgentError::InvalidInput(format!(
            "Path Unix socket '{}' harus berupa absolute path",
            clean
        )));
    }
    if clean.contains("..") {
        return Err(AgentError::InvalidInput(
            "Path Unix socket tidak boleh mengandung '..'".into(),
        ));
    }
    Ok(())
}

fn validate_site_socket(site_id: &str, socket_path: &str) -> Result<(), AgentError> {
    validate_socket_path(socket_path)?;
    let expected = format!("/run/panel/python/{site_id}/app.sock");
    if socket_path.strip_prefix("unix:").unwrap_or(socket_path) != expected {
        return Err(AgentError::InvalidInput(format!(
            "Socket Python harus memakai {expected}"
        )));
    }
    Ok(())
}

/// Generates systemd service unit file content for a Python website process.
/// Complies with PRD Section 5.1 sandboxing rules:
/// - Runs under tenant UID/GID and tenant cgroup slice (`panel-u<uid>.slice`)
/// - `ProtectSystem=strict`, `NoNewPrivileges=true`, `PrivateTmp=true`, `PrivateDevices=true`
/// - `LimitNOFILE=65535`, `Restart=on-failure`, `RestartSec=2`
/// - Writable tenant home and a private per-site runtime directory
/// - Socket access for Nginx through an ACL applied by the tenant
#[allow(clippy::too_many_arguments)]
pub fn generate_python_site_unit(
    site_id: &str,
    username: &str,
    uid: u32,
    python_version: &str,
    app_dir: &str,
    venv_dir: &str,
    socket_path: &str,
    _framework: PythonSiteFramework,
    server_type: PythonSiteServer,
    entrypoint: &PythonSiteEntrypoint,
    workers: u32,
    auto_restart: bool,
    env_file_path: &str,
    base_runtime_dir: Option<&str>,
) -> Result<String, AgentError> {
    validate_site_id(site_id)?;
    validate_username(username)?;
    validate_entrypoint(entrypoint)?;
    validate_site_socket(site_id, socket_path)?;

    let base_dir = base_runtime_dir.unwrap_or(DEFAULT_PYTHON_BASE_DIR);
    let norm_ver = normalize_python_version(python_version);
    let bin_dir = format!("{}/{}/bin", base_dir, norm_ver);

    let clean_socket = socket_path.strip_prefix("unix:").unwrap_or(socket_path);
    let runtime_path = format!("/run/panel/python/{site_id}");

    let ep_str = match entrypoint {
        PythonSiteEntrypoint::Wsgi(s) | PythonSiteEntrypoint::Asgi(s) => s.as_str(),
        PythonSiteEntrypoint::CustomCommand(cmd) => cmd.as_str(),
    };

    let exec_start = match entrypoint {
        PythonSiteEntrypoint::CustomCommand(cmd) => cmd.clone(),
        _ => {
            let srv = match server_type {
                PythonSiteServer::GunicornWsgi => {
                    panel_runtime::python::PythonServerType::GunicornWsgi
                }
                PythonSiteServer::UvicornAsgi => {
                    panel_runtime::python::PythonServerType::UvicornAsgi
                }
                PythonSiteServer::GunicornUvicornWorker => {
                    panel_runtime::python::PythonServerType::GunicornUvicornWorker
                }
            };
            build_python_exec_start(venv_dir, srv, ep_str, workers, clean_socket)
        }
    };

    let restart_policy = if auto_restart {
        "Restart=on-failure\nRestartSec=2"
    } else {
        "Restart=no"
    };

    let unit = format!(
        "[Unit]\n\
Description=Site {site_id} (python)\n\
After=network.target\n\
Wants=panel-agent.service\n\
After=panel-agent.service\n\
\n\
[Service]\n\
User={username}\n\
Group={username}\n\
Slice=panel-u{uid}.slice\n\
WorkingDirectory={app_dir}\n\
EnvironmentFile=-{env_file_path}\n\
Environment=PYTHONUNBUFFERED=1\n\
Environment=VIRTUAL_ENV={venv_dir}\n\
Environment=PATH={venv_dir}/bin:{bin_dir}:/usr/local/bin:/usr/bin:/bin\n\
ExecStart={exec_start}\n\
{restart_policy}\n\
LimitNOFILE=65535\n\
NoNewPrivileges=true\n\
ProtectSystem=strict\n\
ReadWritePaths=/home/{username} {runtime_path}\n\
TemporaryFileSystem=/home:ro\n\
BindPaths=/home/{username}\n\
PrivateTmp=true\n\
PrivateDevices=true\n\
ProtectKernelTunables=true\n\
ProtectKernelModules=true\n\
ProtectControlGroups=true\n\
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6\n\
RuntimeDirectory=panel/python/{site_id}\n\
RuntimeDirectoryMode=0750\n\
ExecStartPre=/usr/bin/setfacl -m u:www-data:--x,d:u:www-data:rwX {runtime_path}\n\
UMask=0007\n\
\n\
[Install]\n\
WantedBy=multi-user.target\n"
    );

    Ok(unit)
}

/// Generates environment file content from key-value pairs.
pub fn generate_python_site_env(env_vars: &BTreeMap<String, String>) -> Result<String, AgentError> {
    let mut out = String::new();
    for (k, v) in env_vars {
        if k.trim().is_empty() {
            continue;
        }
        if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(AgentError::InvalidInput(format!(
                "Nama environment variable '{}' tidak valid",
                k
            )));
        }
        if v.contains(' ') || v.contains('\t') || v.contains('\n') || v.contains('"') {
            let escaped = v
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            out.push_str(&format!("{}=\"{}\"\n", k, escaped));
        } else {
            out.push_str(&format!("{}={}\n", k, v));
        }
    }
    Ok(out)
}

fn resolve_python_binary(
    runner: &dyn SystemRunner,
    version_dir: &Path,
) -> Result<String, AgentError> {
    let children = runner.read_dir(version_dir).unwrap_or_default();
    panel_runtime::python::python_binary_candidates(version_dir, &children)
        .into_iter()
        .find(|candidate| runner.path_exists(candidate))
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .ok_or_else(|| {
            AgentError::InvalidInput(
                "Runtime Python belum terpasang; pasang melalui Runtime Manager".into(),
            )
        })
}

fn prepare_python_environment(
    runner: &dyn SystemRunner,
    req: &PythonSiteApplyRequest,
) -> Result<(), AgentError> {
    let version = normalize_python_version(&req.python_version);
    let version_dir = Path::new(
        req.base_runtime_dir
            .as_deref()
            .unwrap_or(DEFAULT_PYTHON_BASE_DIR),
    )
    .join(&version);
    let python = resolve_python_binary(runner, &version_dir)?;
    let prefix = vec![
        "--quiet".to_string(),
        "--pipe".into(),
        "--wait".into(),
        "--collect".into(),
        format!("--uid={}", req.uid),
        format!("--gid={}", req.uid),
        format!("--slice=panel-u{}.slice", req.uid),
        format!("--working-directory={}", req.app_dir),
        "--property=RuntimeMaxSec=30s".into(),
        "--property=TimeoutStopSec=2s".into(),
        "--property=NoNewPrivileges=yes".into(),
        format!("--setenv=UV_CACHE_DIR=/home/{}/.cache/uv", req.username),
    ];
    let execute = |arguments: Vec<String>| -> Result<(), AgentError> {
        let mut args = prefix.clone();
        args.extend(arguments);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = runner.run_command("systemd-run", &args)?;
        if !result.status.success() {
            return Err(AgentError::InvalidInput(
                "Persiapan virtual environment Python gagal".into(),
            ));
        }
        Ok(())
    };
    let venv_python = format!("{}/bin/python", req.venv_dir);
    if !runner.path_exists(Path::new(&venv_python)) {
        execute(build_uv_venv_cmd(DEFAULT_UV_BIN, &req.venv_dir, &python))?;
    }
    let packages: &[&str] = match req.server_type {
        PythonSiteServer::GunicornWsgi => &["gunicorn"],
        PythonSiteServer::UvicornAsgi => &["uvicorn"],
        PythonSiteServer::GunicornUvicornWorker => &["gunicorn", "uvicorn"],
    };
    let missing: Vec<_> = packages
        .iter()
        .filter(|name| !runner.path_exists(Path::new(&format!("{}/bin/{name}", req.venv_dir))))
        .copied()
        .collect();
    if !missing.is_empty() {
        let mut args = vec![
            DEFAULT_UV_BIN.to_string(),
            "pip".into(),
            "install".into(),
            "--python".into(),
            venv_python,
        ];
        args.extend(missing.into_iter().map(str::to_string));
        execute(args)?;
    }
    Ok(())
}

/// Applies a Python site by ensuring socket directory exists, generating env file and unit,
/// reloading systemd daemon, and enabling/starting the service.
pub fn apply_python_site(
    runner: &dyn SystemRunner,
    req: &PythonSiteApplyRequest,
) -> Result<PythonSiteApplyResponse, AgentError> {
    validate_site_id(&req.site_id)?;
    validate_username(&req.username)?;
    validate_entrypoint(&req.entrypoint)?;
    validate_site_socket(&req.site_id, &req.socket_path)?;
    crate::node_site::validate_app_directory(&req.username, req.uid, &req.app_dir)?;
    crate::node_site::validate_app_directory(&req.username, req.uid, &req.venv_dir)?;

    if let PythonSiteEntrypoint::Wsgi(entry) | PythonSiteEntrypoint::Asgi(entry) = &req.entrypoint {
        let module = entry.split(':').next().unwrap_or("");
        let relative = if module.ends_with(".py") {
            module.to_string()
        } else {
            format!("{}.py", module.replace('.', "/"))
        };
        if module.is_empty()
            || module.contains("..")
            || module.starts_with('/')
            || module.starts_with('\\')
            || !runner.path_exists(&Path::new(&req.app_dir).join(relative))
        {
            return Err(AgentError::InvalidInput(
                "Entrypoint Python belum ada; unggah project atau gunakan starter standar".into(),
            ));
        }
    }
    crate::system::ensure_runtime_parent_access(runner, &req.username)?;
    prepare_python_environment(runner, req)?;
    let service_name = format!("site-{}.service", req.site_id);
    let unit_path_str = req
        .unit_path
        .clone()
        .unwrap_or_else(|| format!("{}/{}", DEFAULT_SYSTEMD_DIR, service_name));
    let unit_path = PathBuf::from(&unit_path_str);

    let env_path_str = req
        .env_file_path
        .clone()
        .unwrap_or_else(|| format!("{}/site-{}.env", DEFAULT_ENV_DIR, req.site_id));
    let env_path = PathBuf::from(&env_path_str);

    // Ensure socket base directory exists
    let socket_dir = Path::new(DEFAULT_PYTHON_SOCKET_DIR);
    if !runner.path_exists(socket_dir) {
        runner.create_dir_all(socket_dir, 0o755, 0, 0)?;
    }
    crate::platform::relabel(runner, socket_dir)?;

    // 1. Write environment file with permissions 0600
    let env_content = generate_python_site_env(&req.env_vars)?;
    runner.write_file(&env_path, env_content.as_bytes(), 0o600)?;

    // 2. Generate and write systemd unit with permissions 0644
    let unit_content = generate_python_site_unit(
        &req.site_id,
        &req.username,
        req.uid,
        &req.python_version,
        &req.app_dir,
        &req.venv_dir,
        &req.socket_path,
        req.framework,
        req.server_type,
        &req.entrypoint,
        req.workers,
        req.auto_restart,
        &env_path_str,
        req.base_runtime_dir.as_deref(),
    )?;
    runner.write_file(&unit_path, unit_content.as_bytes(), 0o644)?;

    // 3. Reload systemd and restart/start service
    runner.run_command("systemctl", &["daemon-reload"])?;
    runner.run_command("systemctl", &["enable", &service_name])?;
    runner.run_command("systemctl", &["restart", &service_name])?;

    Ok(PythonSiteApplyResponse {
        site_id: req.site_id.clone(),
        service_name,
        unit_path: unit_path_str,
        env_path: env_path_str,
        socket_path: req.socket_path.clone(),
        active: true,
        message: format!(
            "Service untuk situs Python {} berhasil dipasang dan dimulai",
            req.site_id
        ),
    })
}

/// Removes a Python site by stopping and disabling the service, removing unit & env files,
/// removing unix socket if present, and reloading systemd daemon.
pub fn remove_python_site(
    runner: &dyn SystemRunner,
    req: &PythonSiteRemoveRequest,
) -> Result<PythonSiteActionResponse, AgentError> {
    validate_site_id(&req.site_id)?;

    if let Some(socket) = req.socket_path.as_deref() {
        let socket = socket.strip_prefix("unix:").unwrap_or(socket);
        let canonical = format!("/run/panel/python/{}/app.sock", req.site_id);
        let legacy = format!("/run/panel/python/{}.sock", req.site_id);
        if socket != canonical && socket != legacy {
            return Err(AgentError::InvalidInput(
                "Socket penghapusan harus milik situs".into(),
            ));
        }
    }
    let service_name = format!("site-{}.service", req.site_id);
    let unit_path_str = req
        .unit_path
        .clone()
        .unwrap_or_else(|| format!("{}/{}", DEFAULT_SYSTEMD_DIR, service_name));
    let unit_path = PathBuf::from(&unit_path_str);

    let env_path_str = req
        .env_file_path
        .clone()
        .unwrap_or_else(|| format!("{}/site-{}.env", DEFAULT_ENV_DIR, req.site_id));
    let env_path = PathBuf::from(&env_path_str);

    if runner.path_exists(&unit_path) {
        runner.run_command("systemctl", &["stop", &service_name])?;
        runner.run_command("systemctl", &["disable", &service_name])?;
    } else if runner
        .run_command("systemctl", &["is-active", "--quiet", &service_name])
        .is_ok()
    {
        runner.run_command("systemctl", &["stop", &service_name])?;
    }

    if runner.path_exists(&unit_path) {
        runner.remove_file(&unit_path)?;
    }
    if runner.path_exists(&env_path) {
        runner.remove_file(&env_path)?;
    }

    if let Some(ref sock) = req.socket_path {
        let clean = sock.strip_prefix("unix:").unwrap_or(sock);
        let sock_path = PathBuf::from(clean);
        if runner.path_exists(&sock_path) {
            runner.remove_file(&sock_path)?;
        }
    }

    runner.run_command("systemctl", &["daemon-reload"])?;

    Ok(PythonSiteActionResponse {
        site_id: req.site_id.clone(),
        action: "remove".to_string(),
        success: true,
        message: format!("Situs Python {} berhasil dihapus", req.site_id),
    })
}

/// Executes a service action (start, stop, restart, redeploy) on a Python site.
pub fn action_python_site(
    runner: &dyn SystemRunner,
    req: &PythonSiteActionRequest,
) -> Result<PythonSiteActionResponse, AgentError> {
    validate_site_id(&req.site_id)?;

    let service_name = format!("site-{}.service", req.site_id);
    let (action_str, cmd) = match req.action {
        PythonSiteActionType::Start => ("start", "start"),
        PythonSiteActionType::Stop => ("stop", "stop"),
        PythonSiteActionType::Restart => ("restart", "restart"),
        PythonSiteActionType::Redeploy => ("redeploy", "restart"),
    };

    let output = runner.run_command("systemctl", &[cmd, &service_name])?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AgentError::CommandFailed {
            binary: "systemctl".into(),
            code: output.status.code(),
            stderr: stderr.to_string(),
        });
    }

    Ok(PythonSiteActionResponse {
        site_id: req.site_id.clone(),
        action: action_str.to_string(),
        success: true,
        message: format!(
            "Aksi '{}' pada {} berhasil dijalankan",
            action_str, service_name
        ),
    })
}

/// Queries systemd service status and inspects Unix socket existence.
pub fn status_python_site(
    runner: &dyn SystemRunner,
    req: &PythonSiteStatusRequest,
) -> Result<PythonSiteStatusResponse, AgentError> {
    validate_site_id(&req.site_id)?;

    let service_name = format!("site-{}.service", req.site_id);
    let output = runner.run_command(
        "systemctl",
        &[
            "show",
            &service_name,
            "--property=ActiveState,SubState,MainPID,MemoryCurrent,CPUUsageNSec,ActiveEnterTimestampMonotonic",
        ],
    )?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut active_state = "inactive".to_string();
    let mut sub_state = "dead".to_string();
    let mut pid: Option<u32> = None;
    let mut memory_bytes: Option<u64> = None;
    let mut cpu_usage_usec: Option<u64> = None;
    let mut active_enter_mono_usec: Option<u64> = None;

    for line in stdout.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k {
                "ActiveState" => active_state = v.to_string(),
                "SubState" => sub_state = v.to_string(),
                "MainPID" => {
                    if let Some(p) = v.parse::<u32>().ok().filter(|&p| p > 0) {
                        pid = Some(p);
                    }
                }
                "MemoryCurrent" => {
                    if let Some(m) = v.parse::<u64>().ok().filter(|&m| m < u64::MAX) {
                        memory_bytes = Some(m);
                    }
                }
                "CPUUsageNSec" => {
                    if let Ok(c) = v.parse::<u64>() {
                        cpu_usage_usec = Some(c / 1000);
                    }
                }
                "ActiveEnterTimestampMonotonic" => {
                    active_enter_mono_usec = v.parse::<u64>().ok().filter(|&m| m > 0);
                }
                _ => {}
            }
        }
    }

    let is_active = active_state == "active";
    let uptime_secs = if is_active {
        active_enter_mono_usec.map(|mono_usec| {
            let sys_uptime_secs = std::fs::read_to_string("/proc/uptime")
                .ok()
                .and_then(|content| content.split_whitespace().next()?.parse::<f64>().ok());
            if let Some(sys_uptime_secs) = sys_uptime_secs {
                let start_secs = mono_usec as f64 / 1_000_000.0;
                let diff = sys_uptime_secs - start_secs;
                if diff >= 0.0 {
                    return diff as u64;
                }
            }
            mono_usec / 1_000_000
        })
    } else {
        None
    };

    // Socket probe verification
    let mut socket_exists = false;
    let mut probe_message = None;

    if let Some(ref sock) = req.socket_path {
        let clean = sock.strip_prefix("unix:").unwrap_or(sock);
        let path = PathBuf::from(clean);
        socket_exists = runner.path_exists(&path);
        if socket_exists {
            probe_message = Some("Unix socket aktif dan terdeteksi di filesystem".to_string());
        } else if is_active {
            probe_message = Some("Service aktif namun socket belum terbentuk".to_string());
        }
    }

    let healthy = if let Some(socket) = req.socket_path.as_deref() {
        if is_active && socket_exists {
            // Probe with the webserver identity so socket ACL errors cannot look healthy.
            match runner.run_command(
                "runuser",
                &[
                    "-u",
                    "www-data",
                    "--",
                    "curl",
                    "--noproxy",
                    "*",
                    "--silent",
                    "--show-error",
                    "--connect-timeout",
                    "1",
                    "--max-time",
                    "2",
                    "--unix-socket",
                    socket.strip_prefix("unix:").unwrap_or(socket),
                    "--output",
                    "/dev/null",
                    "--write-out",
                    "%{http_code}",
                    "http://localhost/",
                ],
            ) {
                Ok(output) => {
                    let status = String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .parse::<u16>()
                        .ok();
                    output.status.success() && status.is_some_and(|code| (200..500).contains(&code))
                }
                Err(error) => {
                    probe_message = Some(error.to_string());
                    false
                }
            }
        } else {
            false
        }
    } else {
        is_active
    };

    Ok(PythonSiteStatusResponse {
        site_id: req.site_id.clone(),
        service_name,
        active: is_active,
        sub_state,
        pid,
        memory_bytes,
        cpu_usage_usec,
        uptime_secs,
        socket_exists,
        healthy,
        probe_message,
    })
}

/// Executes a Python site job (e.g. `uv venv`, `uv pip install`) under tenant UID in tenant cgroup slice.
pub fn run_python_site_job(
    runner: &dyn SystemRunner,
    req: &PythonSiteJobRequest,
) -> Result<PythonSiteJobResponse, AgentError> {
    validate_site_id(&req.site_id)?;
    validate_username(&req.username)?;

    let base_dir = req
        .base_runtime_dir
        .as_deref()
        .unwrap_or(DEFAULT_PYTHON_BASE_DIR);
    let norm_ver = normalize_python_version(&req.python_version);
    let python_bin = if matches!(req.job_type, PythonSiteJobType::UvVenvCreate) {
        resolve_python_binary(runner, &Path::new(base_dir).join(&norm_ver))?
    } else {
        String::new()
    };

    let start_time = Instant::now();

    let args_owned: Vec<String> = match &req.job_type {
        PythonSiteJobType::UvVenvCreate => {
            build_uv_venv_cmd(DEFAULT_UV_BIN, &req.venv_dir, &python_bin)
        }
        PythonSiteJobType::UvPipInstallRequirements { req_file } => {
            let req_path = req_file.as_deref().unwrap_or("requirements.txt");
            let full_req_path = if req_path.starts_with('/') {
                req_path.to_string()
            } else {
                format!("{}/{}", req.app_dir, req_path)
            };
            build_uv_pip_install_requirements_cmd(DEFAULT_UV_BIN, &req.venv_dir, &full_req_path)
        }
        PythonSiteJobType::UvPipInstallPyproject => {
            build_uv_pip_install_pyproject_cmd(DEFAULT_UV_BIN, &req.venv_dir, &req.app_dir)
        }
        PythonSiteJobType::CustomScript { script_name } => {
            if script_name.trim().is_empty() {
                return Err(AgentError::InvalidInput(
                    "Nama script tidak boleh kosong".into(),
                ));
            }
            if !script_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
            {
                return Err(AgentError::InvalidInput(format!(
                    "Nama script '{}' mengandung karakter tidak valid",
                    script_name
                )));
            }
            let venv_python = format!("{}/bin/python", req.venv_dir);
            vec![venv_python, script_name.clone()]
        }
    };

    let binary = &args_owned[0];
    let slice_arg = format!("--slice=panel-u{}.slice", req.uid);
    let uid_arg = format!("--uid={}", req.uid);
    let gid_arg = format!("--gid={}", req.uid);
    let workdir_arg = format!("--working-directory={}", req.app_dir);
    let venv_path_arg = format!(
        "--setenv=PATH={}/bin:{}/{}/bin:/usr/local/bin:/usr/bin:/bin",
        req.venv_dir, base_dir, norm_ver
    );
    let venv_env_arg = format!("--setenv=VIRTUAL_ENV={}", req.venv_dir);

    let mut systemd_args = vec![
        "--quiet",
        &slice_arg,
        &uid_arg,
        &gid_arg,
        "--pipe",
        "--wait",
        &workdir_arg,
        &venv_path_arg,
        &venv_env_arg,
        binary.as_str(),
    ];

    for arg in &args_owned[1..] {
        systemd_args.push(arg.as_str());
    }

    let output = runner.run_command("systemd-run", &systemd_args);
    let duration_ms = start_time.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let code = out.status.code().unwrap_or(-1);
            let success = out.status.success();

            Ok(PythonSiteJobResponse {
                site_id: req.site_id.clone(),
                command: args_owned.join(" "),
                exit_code: code,
                success,
                stdout,
                stderr,
                duration_ms,
            })
        }
        Err(e) => Ok(PythonSiteJobResponse {
            site_id: req.site_id.clone(),
            command: args_owned.join(" "),
            exit_code: 1,
            success: false,
            stdout: String::new(),
            stderr: format!("Eksekusi job gagal: {}", e),
            duration_ms,
        }),
    }
}

/// Lists installed Python runtimes on disk.
pub fn list_python_runtimes(
    runner: &dyn SystemRunner,
    req: &panel_proto::PythonRuntimeListRequest,
) -> Result<PythonRuntimeListResponse, AgentError> {
    let base_dir_str = req.base_dir.as_deref().unwrap_or(DEFAULT_PYTHON_BASE_DIR);
    let base_dir = Path::new(base_dir_str);

    if !runner.path_exists(base_dir) {
        return Ok(PythonRuntimeListResponse {
            installed: Vec::new(),
        });
    }

    let versions = list_installed_python_versions(base_dir)
        .map_err(|e| AgentError::Internal(format!("Gagal membaca daftar Python runtime: {}", e)))?;

    let dtos = versions
        .into_iter()
        .map(|v| PythonInstalledVersionDto {
            version: v.version,
            install_path: v.install_path,
            bin_path: v.bin_path,
            is_default: v.is_default,
        })
        .collect();

    Ok(PythonRuntimeListResponse { installed: dtos })
}

/// Installs a Python runtime version via `uv python install <version> --install-dir <dir>`.
pub fn install_python_runtime(
    runner: &dyn SystemRunner,
    req: &PythonRuntimeInstallRequest,
) -> Result<PythonRuntimeActionResponse, AgentError> {
    install_python_runtime_cancellable(runner, req, &tokio_util::sync::CancellationToken::new())
}

fn install_python_runtime_cancellable(
    runner: &dyn SystemRunner,
    req: &PythonRuntimeInstallRequest,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<PythonRuntimeActionResponse, AgentError> {
    if req.base_dir.is_some() {
        return Err(AgentError::InvalidInput(
            "Direktori runtime Python dikelola agent".into(),
        ));
    }
    let base_dir_str = req.base_dir.as_deref().unwrap_or(DEFAULT_PYTHON_BASE_DIR);
    let norm_ver = normalize_python_version(&req.version);
    panel_runtime::python::validate_python_version(&norm_ver)
        .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
    let target_dir = format!("{}/{}", base_dir_str, norm_ver);

    if cancel.is_cancelled() {
        return Err(AgentError::Internal("Pemasangan Python dibatalkan".into()));
    }
    // Never remove an existing runtime on cancellation or re-run its installation.
    if runner.path_exists(Path::new(&target_dir)) {
        return Err(AgentError::InvalidInput(
            "Direktori runtime Python sudah ada; muat ulang inventaris sebelum memasang kembali"
                .into(),
        ));
    }
    let uv = crate::uv::ensure(runner, cancel)?;
    let mut cmd = build_uv_python_install_cmd(uv, &norm_ver, Some(&target_dir));
    cmd.extend([
        "--python-downloads-json-url".into(),
        panel_runtime::python::PYTHON_DOWNLOADS_JSON.into(),
    ]);
    let args_ref: Vec<&str> = cmd[1..].iter().map(|s| s.as_str()).collect();

    let result = runner.run_cancellable_python(&args_ref, cancel);
    if cancel.is_cancelled() || !matches!(&result, Ok(output) if output.status.success()) {
        // The child has exited and been reaped. Clean only this new managed target.
        if runner.path_exists(Path::new(&target_dir)) {
            runner.remove_dir_all(Path::new(&target_dir))?;
        }
    }
    if cancel.is_cancelled() {
        return Err(AgentError::Internal("Pemasangan Python dibatalkan".into()));
    }
    let output = result.map_err(|error| {
        AgentError::Internal(format!(
            "Gagal menjalankan uv untuk memasang Python {norm_ver}: {error}"
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AgentError::CommandFailed {
            binary: cmd[0].clone(),
            code: output.status.code(),
            stderr: stderr.to_string(),
        });
    }

    Ok(PythonRuntimeActionResponse {
        version: norm_ver.clone(),
        success: true,
        path: target_dir,
        message: format!("Python {} berhasil diinstall via uv", norm_ver),
    })
}

/// Removes a Python runtime version from disk.
pub fn remove_python_runtime(
    runner: &dyn SystemRunner,
    req: &PythonRuntimeRemoveRequest,
) -> Result<PythonRuntimeActionResponse, AgentError> {
    let base_dir_str = req.base_dir.as_deref().unwrap_or(DEFAULT_PYTHON_BASE_DIR);
    let norm_ver = normalize_python_version(&req.version);
    panel_runtime::python::validate_python_version(&norm_ver)
        .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
    let target_dir = format!("{}/{}", base_dir_str, norm_ver);
    let target_path = PathBuf::from(&target_dir);

    if runner.path_exists(&target_path) {
        runner.remove_dir_all(&target_path)?;
    }

    Ok(PythonRuntimeActionResponse {
        version: norm_ver.clone(),
        success: true,
        path: target_dir,
        message: format!("Python {} berhasil dihapus", norm_ver),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{MockAction, MockSystemRunner};

    #[test]
    fn runtime_install_rejects_path_and_option_versions_before_commands() {
        let runner = MockSystemRunner::new();
        for version in ["../../etc", "--help", "3.14.0/../../x", "3.14.0\nextra"] {
            let request = PythonRuntimeInstallRequest {
                version: version.into(),
                base_dir: None,
            };
            assert!(install_python_runtime(&runner, &request).is_err());
            assert!(
                remove_python_runtime(
                    &runner,
                    &PythonRuntimeRemoveRequest {
                        version: version.into(),
                        base_dir: None
                    }
                )
                .is_err()
            );
        }
        assert!(runner.recorded_actions().is_empty());
        runner.add_existing_path(crate::uv::UV_BIN);
        runner.set_response(crate::uv::UV_BIN, 0, b"installed", b"");
        assert!(
            install_python_runtime(
                &runner,
                &PythonRuntimeInstallRequest {
                    version: "3.14.10".into(),
                    base_dir: None
                }
            )
            .unwrap()
            .success
        );
        let commands = runner.get_commands();
        assert!(commands.iter().any(|command| command.contains(
            "--python-downloads-json-url https://raw.githubusercontent.com/astral-sh/uv/"
        )));
    }

    #[test]
    fn python_environment_setup_is_bounded_and_runs_as_tenant_before_unit_install() {
        let runner = MockSystemRunner::new();
        let request = PythonSiteApplyRequest {
            site_id: "site_1".into(),
            username: "p_alice".into(),
            uid: 10001,
            python_version: "3.12.9".into(),
            app_dir: "/home/p_alice/sites/app".into(),
            venv_dir: "/home/p_alice/sites/app/.venv".into(),
            socket_path: "/run/panel/python/site_1/app.sock".into(),
            framework: PythonSiteFramework::Generic,
            server_type: PythonSiteServer::GunicornWsgi,
            entrypoint: PythonSiteEntrypoint::Wsgi("app:app".into()),
            workers: 1,
            env_vars: BTreeMap::new(),
            auto_restart: true,
            env_file_path: None,
            unit_path: None,
            base_runtime_dir: None,
        };
        let invalid_socket = PythonSiteApplyRequest {
            socket_path: "/run/panel/python/site_1/custom.sock".into(),
            ..request.clone()
        };
        assert!(apply_python_site(&runner, &invalid_socket).is_err());
        assert!(runner.recorded_actions().is_empty());
        assert!(apply_python_site(&runner, &request).is_err());
        runner.add_existing_path("/home/p_alice/sites/app/app.py");
        assert!(apply_python_site(&runner, &request).is_err());
        runner.add_existing_path("/opt/panel/runtimes/python/3.12.9/bin/python3");
        runner.set_response("systemd-run", 1, b"", b"dependency setup failed");
        assert!(apply_python_site(&runner, &request).is_err());
        assert!(
            runner
                .recorded_actions()
                .iter()
                .all(|action| !matches!(action, MockAction::WriteFile { .. }))
        );
        runner.set_response("systemd-run", 0, b"ready", b"");
        runner.set_response("setfacl", 1, b"", b"parent ACL denied");
        assert!(apply_python_site(&runner, &request).is_err());
        assert!(
            !runner
                .get_commands()
                .iter()
                .any(|cmd| cmd.starts_with("systemctl"))
        );
        runner.set_response("setfacl", 0, b"", b"");
        apply_python_site(&runner, &request).unwrap();
        let commands = runner.get_commands();
        assert!(
            commands
                .iter()
                .any(|command| command.contains("--uid=10001")
                    && command.contains("RuntimeMaxSec=30s")
                    && command.contains("uv venv"))
        );
        assert!(
            commands
                .iter()
                .any(|command| command.contains("--uid=10001")
                    && command.contains("pip install --python")
                    && command.ends_with("gunicorn"))
        );
        let unit = String::from_utf8(
            runner
                .read_file(Path::new("/etc/systemd/system/site-site_1.service"))
                .unwrap(),
        )
        .unwrap();
        assert!(unit.contains("RuntimeDirectory=panel/python/site_1"));
        assert!(unit.contains("Wants=panel-agent.service\nAfter=panel-agent.service"));
        assert!(unit.contains("ExecStartPre=/usr/bin/setfacl -m u:www-data:--x,d:u:www-data:rwX /run/panel/python/site_1"));
        assert!(
            remove_python_site(
                &runner,
                &PythonSiteRemoveRequest {
                    site_id: "site_1".into(),
                    socket_path: Some("/etc/shadow".into()),
                    unit_path: None,
                    env_file_path: None
                }
            )
            .is_err()
        );
    }

    #[test]
    fn test_generate_python_site_unit_django() {
        let unit = generate_python_site_unit(
            "site-py1",
            "p_alice",
            10001,
            "3.12.9",
            "/home/p_alice/sites/django.test/app",
            "/home/p_alice/sites/django.test/.venv",
            "/run/panel/python/site-py1/app.sock",
            PythonSiteFramework::Django,
            PythonSiteServer::GunicornWsgi,
            &PythonSiteEntrypoint::Wsgi("myproject.wsgi:application".to_string()),
            3,
            true,
            "/etc/panel/env/site-py1.env",
            None,
        )
        .expect("generate django unit");

        assert!(unit.contains("Description=Site site-py1 (python)"));
        assert!(unit.contains("User=p_alice"));
        assert!(unit.contains("Group=p_alice"));
        assert!(unit.contains("Slice=panel-u10001.slice"));
        assert!(unit.contains("Environment=PYTHONUNBUFFERED=1"));
        assert!(unit.contains("Environment=VIRTUAL_ENV=/home/p_alice/sites/django.test/.venv"));
        assert!(unit.contains("EnvironmentFile=-/etc/panel/env/site-py1.env"));
        assert!(unit.contains("ExecStart=/home/p_alice/sites/django.test/.venv/bin/gunicorn --workers 3 --bind unix:/run/panel/python/site-py1/app.sock myproject.wsgi:application"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("LimitNOFILE=65535"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("ReadWritePaths=/home/p_alice /run/panel/python"));
        assert!(unit.contains("RuntimeDirectory=panel/python"));
        assert!(unit.contains("RuntimeDirectoryMode=0750"));
        assert!(unit.contains("UMask=0007"));
    }

    #[test]
    fn test_generate_python_site_unit_fastapi() {
        let unit = generate_python_site_unit(
            "site-py2",
            "p_bob",
            10002,
            "3.11.11",
            "/home/p_bob/sites/fastapi.test/app",
            "/home/p_bob/sites/fastapi.test/.venv",
            "/run/panel/python/site-py2/app.sock",
            PythonSiteFramework::FastApi,
            PythonSiteServer::UvicornAsgi,
            &PythonSiteEntrypoint::Asgi("main:app".to_string()),
            4,
            false,
            "/etc/panel/env/site-py2.env",
            None,
        )
        .expect("generate fastapi unit");

        assert!(unit.contains("ExecStart=/home/p_bob/sites/fastapi.test/.venv/bin/uvicorn --workers 4 --uds /run/panel/python/site-py2/app.sock main:app"));
        assert!(unit.contains("Restart=no"));
    }

    #[test]
    fn test_generate_python_site_env() {
        let mut vars = BTreeMap::new();
        vars.insert(
            "DJANGO_SETTINGS_MODULE".to_string(),
            "mysite.settings".to_string(),
        );
        vars.insert(
            "DATABASE_URL".to_string(),
            "postgresql://user:pass@127.0.0.1:5432/db".to_string(),
        );
        vars.insert(
            "SECRET_KEY".to_string(),
            "super secret key with spaces".to_string(),
        );

        let env_content = generate_python_site_env(&vars).expect("generate env");
        assert!(env_content.contains("DJANGO_SETTINGS_MODULE=mysite.settings\n"));
        assert!(env_content.contains("DATABASE_URL=postgresql://user:pass@127.0.0.1:5432/db\n"));
        assert!(env_content.contains("SECRET_KEY=\"super secret key with spaces\"\n"));
    }

    #[test]
    fn test_apply_and_remove_python_site_mock() {
        let runner = MockSystemRunner::default();
        let mut env_vars = BTreeMap::new();
        env_vars.insert("ENV".to_string(), "production".to_string());

        let req_apply = PythonSiteApplyRequest {
            site_id: "site-py1".to_string(),
            username: "p_alice".to_string(),
            uid: 10001,
            python_version: "3.12.9".to_string(),
            app_dir: "/home/p_alice/sites/app".to_string(),
            venv_dir: "/home/p_alice/sites/app/.venv".to_string(),
            socket_path: "/run/panel/python/site-py1/app.sock".to_string(),
            framework: PythonSiteFramework::Flask,
            server_type: PythonSiteServer::GunicornWsgi,
            entrypoint: PythonSiteEntrypoint::Wsgi("app:app".to_string()),
            workers: 3,
            env_vars,
            auto_restart: true,
            env_file_path: Some("/tmp/test-py.env".to_string()),
            unit_path: Some("/tmp/test-py.service".to_string()),
            base_runtime_dir: None,
        };

        runner.add_existing_path("/opt/panel/runtimes/python/3.12.9/bin/python3");
        runner.add_existing_path("/home/p_alice/sites/app/wsgi.py");
        runner.add_existing_path("/home/p_alice/sites/app/app.py");
        let res_apply = apply_python_site(&runner, &req_apply).expect("apply python site");
        assert_eq!(res_apply.site_id, "site-py1");
        assert_eq!(res_apply.socket_path, "/run/panel/python/site-py1/app.sock");
        assert!(res_apply.active);

        let actions = runner.actions.lock().unwrap();
        assert!(actions.iter().any(|a| matches!(a, MockAction::WriteFile { path, .. } if path == &PathBuf::from("/tmp/test-py.env"))));
        assert!(actions.iter().any(|a| matches!(a, MockAction::WriteFile { path, .. } if path == &PathBuf::from("/tmp/test-py.service"))));
        assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["daemon-reload"])));
        assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["restart", "site-site-py1.service"])));
        drop(actions);

        let req_remove = PythonSiteRemoveRequest {
            site_id: "site-py1".to_string(),
            socket_path: Some("/run/panel/python/site-py1/app.sock".to_string()),
            unit_path: Some("/tmp/test-py.service".to_string()),
            env_file_path: Some("/tmp/test-py.env".to_string()),
        };

        let res_remove = remove_python_site(&runner, &req_remove).expect("remove python site");
        assert!(res_remove.success);

        let actions2 = runner.actions.lock().unwrap();
        assert!(actions2.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["stop", "site-site-py1.service"])));
        assert!(actions2.iter().any(|a| matches!(a, MockAction::RemoveFile { path } if path == &PathBuf::from("/tmp/test-py.service"))));
        assert!(actions2.iter().any(|a| matches!(a, MockAction::RemoveFile { path } if path == &PathBuf::from("/tmp/test-py.env"))));
    }

    #[test]
    fn test_action_python_site_mock() {
        let runner = MockSystemRunner::default();
        let req = PythonSiteActionRequest {
            site_id: "site-py42".to_string(),
            action: PythonSiteActionType::Restart,
        };

        let res = action_python_site(&runner, &req).expect("restart python site");
        assert_eq!(res.action, "restart");
        assert!(res.success);

        let actions = runner.actions.lock().unwrap();
        assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["restart", "site-site-py42.service"])));
    }

    #[test]
    fn test_run_python_site_job_mock() {
        let runner = MockSystemRunner::default();
        let req = PythonSiteJobRequest {
            site_id: "site-py1".to_string(),
            username: "p_alice".to_string(),
            uid: 10001,
            python_version: "3.12.9".to_string(),
            app_dir: "/home/p_alice/sites/app".to_string(),
            venv_dir: "/home/p_alice/sites/app/.venv".to_string(),
            job_type: PythonSiteJobType::UvPipInstallRequirements { req_file: None },
            timeout_secs: Some(300),
            base_runtime_dir: None,
        };

        let res = run_python_site_job(&runner, &req).expect("run uv pip install job");
        assert!(res.success);
        assert!(res.command.contains("pip install -r"));

        let actions = runner.actions.lock().unwrap();
        assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl-run" || (binary == "systemd-run" && args.iter().any(|arg| arg.contains("panel-u10001.slice"))))));
    }
}
