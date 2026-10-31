//! Node.js site process management: systemd unit generation, environment file writing,
//! service lifecycle (start, stop, restart, status), HTTP health check probing,
//! and tenant-UID job execution within tenant cgroup slice.

use crate::AgentError;
use crate::system::SystemRunner;
use panel_proto::{
    NodeSiteActionRequest, NodeSiteActionResponse, NodeSiteActionType, NodeSiteApplyRequest,
    NodeSiteApplyResponse, NodeSiteEntrypoint, NodeSiteJobRequest, NodeSiteJobResponse,
    NodeSiteJobType, NodeSiteRemoveRequest, NodeSiteStatusRequest, NodeSiteStatusResponse,
};
use panel_runtime::node::{DEFAULT_NODE_BASE_DIR, normalize_node_version};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const DEFAULT_SYSTEMD_DIR: &str = "/etc/systemd/system";
pub const DEFAULT_ENV_DIR: &str = "/etc/panel/env";

pub(crate) fn validate_runtime_version(version: &str) -> Result<(), AgentError> {
    let version = normalize_node_version(version);
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|c| c.is_ascii_digit())
                || part.parse::<u16>().is_err()
        })
    {
        return Err(AgentError::InvalidInput(
            "Versi runtime harus major.minor.patch tanpa path atau directive".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_unit_path(value: &str) -> Result<(), AgentError> {
    if value.chars().any(char::is_control) || value.contains('%') {
        return Err(AgentError::InvalidInput(
            "Path unit tidak boleh mengandung directive atau specifier".into(),
        ));
    }
    Ok(())
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

pub(crate) fn validate_app_directory(
    username: &str,
    uid: u32,
    path: &str,
) -> Result<(), AgentError> {
    panel_core::tenant::TenantUid::parse(uid)
        .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
    if panel_core::tenant::is_reserved_username(username)
        || !path.starts_with(&format!("/home/{username}/"))
        || path.contains(['\n', '\r', '\0', '%', '\\'])
        || path.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err(AgentError::InvalidInput(
            "Direktori aplikasi harus berada di home tenant dan tidak mengandung directive systemd"
                .into(),
        ));
    }
    Ok(())
}

/// Validates entrypoint configuration.
pub fn validate_entrypoint(entrypoint: &NodeSiteEntrypoint) -> Result<(), AgentError> {
    match entrypoint {
        NodeSiteEntrypoint::NodeScript(script) => {
            if script.trim().is_empty() {
                return Err(AgentError::InvalidInput(
                    "Entrypoint script tidak boleh kosong".into(),
                ));
            }
            if script.starts_with('/') || script.starts_with('\\') {
                return Err(AgentError::InvalidInput(
                    "Entrypoint script harus berupa path relatif terhadap direktori aplikasi"
                        .into(),
                ));
            }
            if script.contains("..")
                || !script
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
            {
                return Err(AgentError::InvalidInput(
                    "Entrypoint script tidak boleh mengandung path traversal ('..')".into(),
                ));
            }
        }
        NodeSiteEntrypoint::NpmStart => {}
        NodeSiteEntrypoint::NpmCustom(subcmd) => {
            if subcmd.trim().is_empty() {
                return Err(AgentError::InvalidInput(
                    "Nama script npm tidak boleh kosong".into(),
                ));
            }
            if !subcmd
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(AgentError::InvalidInput(format!(
                    "Nama script npm '{}' mengandung karakter tidak valid",
                    subcmd
                )));
            }
        }
    }
    Ok(())
}

/// Generates systemd unit content according to PRD section 5.1 specification.
#[allow(clippy::too_many_arguments)]
pub fn generate_node_site_unit(
    site_id: &str,
    username: &str,
    uid: u32,
    node_version: &str,
    app_dir: &str,
    port: u16,
    entrypoint: &NodeSiteEntrypoint,
    auto_restart: bool,
    env_file_path: &str,
    base_runtime_dir: Option<&str>,
) -> Result<String, AgentError> {
    validate_site_id(site_id)?;
    validate_username(username)?;
    validate_entrypoint(entrypoint)?;
    validate_runtime_version(node_version)?;
    validate_app_directory(username, uid, app_dir)?;
    validate_unit_path(env_file_path)?;
    if let Some(base) = base_runtime_dir {
        validate_unit_path(base)?;
    }

    let base_dir = base_runtime_dir.unwrap_or(DEFAULT_NODE_BASE_DIR);
    let norm_ver = normalize_node_version(node_version);
    let bin_dir = format!("{}/{}/bin", base_dir, norm_ver);

    let exec_start = match entrypoint {
        NodeSiteEntrypoint::NodeScript(script) => format!("{}/node {}", bin_dir, script),
        NodeSiteEntrypoint::NpmStart => format!("{}/npm start", bin_dir),
        NodeSiteEntrypoint::NpmCustom(subcmd) => format!("{}/npm run {}", bin_dir, subcmd),
    };

    let restart_policy = if auto_restart {
        "Restart=on-failure\nRestartSec=2"
    } else {
        "Restart=no"
    };

    let unit = format!(
        "[Unit]\n\
Description=Site {site_id} (nodejs)\n\
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
Environment=PORT={port}\n\
Environment=NODE_ENV=production\n\
Environment=PATH={bin_dir}:/usr/local/bin:/usr/bin:/bin\n\
ExecStart={exec_start}\n\
{restart_policy}\n\
LimitNOFILE=65535\n\
NoNewPrivileges=true\n\
ProtectSystem=strict\n\
ReadWritePaths=/home/{username}\n\
TemporaryFileSystem=/home:ro\n\
BindPaths=/home/{username}\n\
PrivateTmp=true\n\
PrivateDevices=true\n\
ProtectKernelTunables=true\n\
ProtectKernelModules=true\n\
ProtectControlGroups=true\n\
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6\n\
\n\
[Install]\n\
WantedBy=multi-user.target\n"
    );

    Ok(unit)
}

/// Generates environment file content from key-value pairs.
pub fn generate_node_site_env(env_vars: &BTreeMap<String, String>) -> Result<String, AgentError> {
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
        // Quote if value contains whitespace or special chars
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

/// Applies a Node.js site by generating environment file and systemd unit,
/// reloading systemd daemon, and enabling/starting the service.
pub fn apply_node_site(
    runner: &dyn SystemRunner,
    req: &NodeSiteApplyRequest,
) -> Result<NodeSiteApplyResponse, AgentError> {
    validate_site_id(&req.site_id)?;
    validate_username(&req.username)?;
    validate_entrypoint(&req.entrypoint)?;
    validate_runtime_version(&req.node_version)?;
    validate_app_directory(&req.username, req.uid, &req.app_dir)?;

    if req.port < 1024 {
        return Err(AgentError::InvalidInput(format!(
            "Port {} harus berupa unprivileged port (>= 1024)",
            req.port
        )));
    }

    let node_binary = format!(
        "{}/{}/bin/node",
        req.base_runtime_dir
            .as_deref()
            .unwrap_or(DEFAULT_NODE_BASE_DIR),
        normalize_node_version(&req.node_version)
    );
    if !runner.path_exists(PathBuf::from(&node_binary).as_path()) {
        return Err(AgentError::InvalidInput(
            "Runtime Node.js belum terpasang; pasang melalui Runtime Manager".into(),
        ));
    }
    let entry = match &req.entrypoint {
        NodeSiteEntrypoint::NodeScript(file) => file.as_str(),
        _ => "package.json",
    };
    if !runner.path_exists(&PathBuf::from(&req.app_dir).join(entry)) {
        return Err(AgentError::InvalidInput(
            "Entrypoint Node.js belum ada; unggah project atau gunakan server.js".into(),
        ));
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

    // 1. Write environment file with permissions 0600
    let env_content = generate_node_site_env(&req.env_vars)?;
    runner.write_file(&env_path, env_content.as_bytes(), 0o600)?;

    // 2. Generate and write systemd unit with permissions 0644
    let unit_content = generate_node_site_unit(
        &req.site_id,
        &req.username,
        req.uid,
        &req.node_version,
        &req.app_dir,
        req.port,
        &req.entrypoint,
        req.auto_restart,
        &env_path_str,
        req.base_runtime_dir.as_deref(),
    )?;
    runner.write_file(&unit_path, unit_content.as_bytes(), 0o644)?;

    // 3. Reload systemd and restart/start service
    runner.run_command("systemctl", &["daemon-reload"])?;
    runner.run_command("systemctl", &["enable", &service_name])?;
    runner.run_command("systemctl", &["restart", &service_name])?;

    Ok(NodeSiteApplyResponse {
        site_id: req.site_id.clone(),
        service_name,
        unit_path: unit_path_str,
        env_path: env_path_str,
        port: req.port,
        active: true,
        message: format!(
            "Service untuk situs {} berhasil dipasang dan dimulai",
            req.site_id
        ),
    })
}

/// Removes a Node.js site by stopping and disabling the service, removing unit & env files,
/// and reloading systemd daemon.
pub fn remove_node_site(
    runner: &dyn SystemRunner,
    req: &NodeSiteRemoveRequest,
) -> Result<NodeSiteActionResponse, AgentError> {
    validate_site_id(&req.site_id)?;

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

    // 1. Stop and disable service
    if runner.path_exists(&unit_path) {
        runner.run_command("systemctl", &["stop", &service_name])?;
        runner.run_command("systemctl", &["disable", &service_name])?;
    } else if runner
        .run_command("systemctl", &["is-active", "--quiet", &service_name])
        .is_ok()
    {
        runner.run_command("systemctl", &["stop", &service_name])?;
    }

    // 2. Remove files
    if runner.path_exists(&unit_path) {
        runner.remove_file(&unit_path)?;
    }
    if runner.path_exists(&env_path) {
        runner.remove_file(&env_path)?;
    }

    // 3. Daemon reload
    runner.run_command("systemctl", &["daemon-reload"])?;

    Ok(NodeSiteActionResponse {
        site_id: req.site_id.clone(),
        action: "remove".to_string(),
        success: true,
        message: format!(
            "Service untuk situs {} berhasil dihentikan dan dihapus",
            req.site_id
        ),
    })
}

/// Executes lifecycle action (start, stop, restart, reload) on a Node.js site service.
pub fn action_node_site(
    runner: &dyn SystemRunner,
    req: &NodeSiteActionRequest,
) -> Result<NodeSiteActionResponse, AgentError> {
    validate_site_id(&req.site_id)?;

    let service_name = format!("site-{}.service", req.site_id);
    let (action_str, cmd_arg) = match req.action {
        NodeSiteActionType::Start => ("start", "start"),
        NodeSiteActionType::Stop => ("stop", "stop"),
        NodeSiteActionType::Restart => ("restart", "restart"),
        NodeSiteActionType::Reload => ("reload", "reload-or-restart"),
    };

    runner.run_command("systemctl", &[cmd_arg, &service_name])?;

    Ok(NodeSiteActionResponse {
        site_id: req.site_id.clone(),
        action: action_str.to_string(),
        success: true,
        message: format!(
            "Aksi '{}' berhasil dieksekusi pada {}",
            action_str, service_name
        ),
    })
}

/// Helper to probe an HTTP port locally with a 2-second timeout.
pub fn probe_http(port: u16, path: &str) -> (bool, Option<u16>, String) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(1500)) {
        Ok(s) => s,
        Err(e) => {
            return (
                false,
                None,
                format!("Koneksi gagal ke 127.0.0.1:{}: {}", port, e),
            );
        }
    };

    let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(1500)));

    let req_str = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUser-Agent: Qinpanel-Probe/1.0\r\nConnection: close\r\n\r\n",
        if path.is_empty() { "/" } else { path },
        port
    );

    let mut stream = stream;
    if let Err(e) = stream.write_all(req_str.as_bytes()) {
        return (false, None, format!("Gagal mengirim request probe: {}", e));
    }

    let mut buf = [0u8; 1024];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 => {
            let resp = String::from_utf8_lossy(&buf[..n]);
            // Parse status code from HTTP/1.x <status>
            let mut parts = resp.split_whitespace();
            let parsed_code = parts
                .next()
                .filter(|ver| ver.starts_with("HTTP/"))
                .and_then(|_| parts.next())
                .and_then(|cs| cs.parse::<u16>().ok());

            if let Some(code) = parsed_code {
                let healthy = code < 500;
                return (
                    healthy,
                    Some(code),
                    format!("HTTP status {} diterima dari probe", code),
                );
            }
            (
                true,
                Some(200),
                "Respons diterima dari upstream".to_string(),
            )
        }
        Ok(_) => (
            false,
            None,
            "Koneksi upstream ditutup tanpa respon".to_string(),
        ),
        Err(e) => (false, None, format!("Gagal membaca respon probe: {}", e)),
    }
}

/// Queries systemd service status and performs health check probe if port is provided.
pub fn status_node_site(
    runner: &dyn SystemRunner,
    req: &NodeSiteStatusRequest,
) -> Result<NodeSiteStatusResponse, AgentError> {
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

    let (healthy, probe_status_code, probe_message) = if is_active {
        if let Some(port) = req.port {
            let path = req.probe_path.as_deref().unwrap_or("/");
            let url = format!("http://127.0.0.1:{port}/{}", path.trim_start_matches('/'));
            let (h, code, msg) = match runner.run_command(
                "curl",
                &[
                    "--noproxy",
                    "*",
                    "--silent",
                    "--show-error",
                    "--connect-timeout",
                    "1",
                    "--max-time",
                    "1",
                    "--output",
                    "/dev/null",
                    "--write-out",
                    "%{http_code}",
                    "--",
                    &url,
                ],
            ) {
                Ok(output) => {
                    let code = String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .parse::<u16>()
                        .ok();
                    (
                        output.status.success() && code.is_some_and(|c| (200..500).contains(&c)),
                        code,
                        "HTTP readiness probe".to_string(),
                    )
                }
                Err(error) => (false, None, error.to_string()),
            };
            (h, code, Some(msg))
        } else {
            (
                true,
                None,
                Some("Service aktif tanpa port probe".to_string()),
            )
        }
    } else {
        (
            false,
            None,
            Some(format!(
                "Service {} sedang dalam status {}",
                service_name, active_state
            )),
        )
    };

    Ok(NodeSiteStatusResponse {
        site_id: req.site_id.clone(),
        service_name,
        active: is_active,
        sub_state,
        pid,
        memory_bytes,
        cpu_usage_usec,
        uptime_secs,
        healthy,
        probe_status_code,
        probe_message,
    })
}

/// Executes a job command (e.g. `npm install`, `npm run build`) under tenant UID in tenant slice.
pub fn run_node_site_job(
    runner: &dyn SystemRunner,
    req: &NodeSiteJobRequest,
) -> Result<NodeSiteJobResponse, AgentError> {
    validate_site_id(&req.site_id)?;
    validate_username(&req.username)?;
    validate_runtime_version(&req.node_version)?;
    validate_app_directory(&req.username, req.uid, &req.app_dir)?;

    let base_dir = req
        .base_runtime_dir
        .as_deref()
        .unwrap_or(DEFAULT_NODE_BASE_DIR);
    let norm_ver = normalize_node_version(&req.node_version);
    let bin_dir = format!("{}/{}/bin", base_dir, norm_ver);

    let (binary, args_owned): (&str, Vec<String>) = match &req.job_type {
        NodeSiteJobType::NpmInstall => ("npm", vec!["install".to_string()]),
        NodeSiteJobType::NpmCi => ("npm", vec!["ci".to_string()]),
        NodeSiteJobType::NpmBuild => ("npm", vec!["run".to_string(), "build".to_string()]),
        NodeSiteJobType::PnpmInstall => ("pnpm", vec!["install".to_string()]),
        NodeSiteJobType::YarnInstall => ("yarn", vec!["install".to_string()]),
        NodeSiteJobType::CustomScript { script_name } => {
            if script_name.trim().is_empty() {
                return Err(AgentError::InvalidInput(
                    "Nama script tidak boleh kosong".into(),
                ));
            }
            if !script_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
            {
                return Err(AgentError::InvalidInput(format!(
                    "Nama script '{}' mengandung karakter tidak valid",
                    script_name
                )));
            }
            ("npm", vec!["run".to_string(), script_name.clone()])
        }
    };

    let start_time = Instant::now();

    // Try executing through systemd-run with user slice and isolation
    let slice_arg = format!("--slice=panel-u{}.slice", req.uid);
    let uid_arg = format!("--uid={}", req.uid);
    let gid_arg = format!("--gid={}", req.uid);
    let workdir_arg = format!("--working-directory={}", req.app_dir);
    let path_env_arg = format!("--setenv=PATH={}:/usr/local/bin:/usr/bin:/bin", bin_dir);
    let node_env_arg = "--setenv=NODE_ENV=development".to_string();

    let mut systemd_args = vec![
        "--quiet",
        "--property=NoNewPrivileges=yes",
        &slice_arg,
        &uid_arg,
        &gid_arg,
        "--pipe",
        "--wait",
        &workdir_arg,
        &path_env_arg,
        &node_env_arg,
        binary,
    ];
    let arg_refs: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();
    systemd_args.extend_from_slice(&arg_refs);

    // Run command
    let res = runner.run_command("systemd-run", &systemd_args);
    let duration_ms = start_time.elapsed().as_millis() as u64;

    match res {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let code = output.status.code().unwrap_or(1);
            Ok(NodeSiteJobResponse {
                site_id: req.site_id.clone(),
                command: format!("{} {}", binary, args_owned.join(" ")),
                exit_code: code,
                success: code == 0,
                stdout,
                stderr,
                duration_ms,
            })
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_versions_cannot_escape_install_root_or_inject_unit_directives() {
        for value in [
            "../../home/p_alice/payload",
            "22.14.0\nUser=root",
            "22.14.0%u",
            "22.14.0/bin/node",
        ] {
            assert!(validate_runtime_version(value).is_err(), "{value}");
        }
        assert!(validate_runtime_version("v22.14.0").is_ok());
        assert!(validate_runtime_version("3.12.9").is_ok());
    }
    use crate::system::{MockAction, MockSystemRunner};

    #[test]
    fn missing_runtime_or_entrypoint_never_installs_a_broken_node_service() {
        let runner = MockSystemRunner::new();
        let request = NodeSiteApplyRequest {
            site_id: "site_1".into(),
            username: "p_alice".into(),
            uid: 10001,
            node_version: "22.14.0".into(),
            app_dir: "/home/p_alice/sites/app".into(),
            port: 20042,
            entrypoint: NodeSiteEntrypoint::NodeScript("server.js".into()),
            env_vars: BTreeMap::new(),
            auto_restart: true,
            env_file_path: None,
            unit_path: None,
            base_runtime_dir: None,
        };
        assert!(apply_node_site(&runner, &request).is_err());
        runner.add_existing_path("/opt/panel/runtimes/node/22.14.0/bin/node");
        assert!(apply_node_site(&runner, &request).is_err());
        assert!(
            runner
                .recorded_actions()
                .iter()
                .all(|action| !matches!(action, MockAction::WriteFile { .. }))
        );
        runner.add_existing_path("/home/p_alice/sites/app/server.js");
        apply_node_site(&runner, &request).unwrap();
        runner.set_response("systemctl stop site-site_1.service", 1, b"", b"stop failed");
        assert!(
            remove_node_site(
                &runner,
                &NodeSiteRemoveRequest {
                    site_id: "site_1".into(),
                    unit_path: None,
                    env_file_path: None
                }
            )
            .is_err()
        );
        assert!(
            runner.path_exists(PathBuf::from("/etc/systemd/system/site-site_1.service").as_path())
        );
    }

    #[test]
    fn test_generate_node_site_unit_matches_prd_5_1() {
        let unit = generate_node_site_unit(
            "site-42",
            "p_alice",
            10001,
            "22.11.0",
            "/home/p_alice/sites/example.com/app",
            20042,
            &NodeSiteEntrypoint::NodeScript("server.js".to_string()),
            true,
            "/etc/panel/env/site-42.env",
            Some("/opt/panel/runtimes/node"),
        )
        .expect("generate unit");

        assert!(unit.contains("Description=Site site-42 (nodejs)"));
        assert!(unit.contains("Wants=panel-agent.service\nAfter=panel-agent.service"));
        assert!(unit.contains("User=p_alice"));
        assert!(unit.contains("Group=p_alice"));
        assert!(unit.contains("Slice=panel-u10001.slice"));
        assert!(unit.contains("WorkingDirectory=/home/p_alice/sites/example.com/app"));
        assert!(unit.contains("EnvironmentFile=-/etc/panel/env/site-42.env"));
        assert!(unit.contains("Environment=PORT=20042"));
        assert!(unit.contains("Environment=NODE_ENV=production"));
        assert!(unit.contains("ExecStart=/opt/panel/runtimes/node/22.11.0/bin/node server.js"));
        assert!(unit.contains("Restart=on-failure\nRestartSec=2"));
        assert!(unit.contains("LimitNOFILE=65535"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("ReadWritePaths=/home/p_alice"));
        assert!(unit.contains("TemporaryFileSystem=/home:ro"));
        assert!(unit.contains("BindPaths=/home/p_alice"));
        assert!(unit.contains("PrivateTmp=true"));
        assert!(unit.contains("PrivateDevices=true"));
        assert!(unit.contains("ProtectKernelTunables=true"));
        assert!(unit.contains("ProtectKernelModules=true"));
        assert!(unit.contains("ProtectControlGroups=true"));
        assert!(unit.contains("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6"));
    }

    #[test]
    fn test_generate_node_site_unit_entrypoints() {
        let unit_npm_start = generate_node_site_unit(
            "site-1",
            "p_bob",
            10002,
            "20.18.0",
            "/home/p_bob/sites/next/app",
            20001,
            &NodeSiteEntrypoint::NpmStart,
            false,
            "/etc/panel/env/site-1.env",
            None,
        )
        .expect("npm start unit");

        assert!(
            unit_npm_start.contains("ExecStart=/opt/panel/runtimes/node/20.18.0/bin/npm start")
        );
        assert!(unit_npm_start.contains("Restart=no"));

        let unit_npm_custom = generate_node_site_unit(
            "site-2",
            "p_bob",
            10002,
            "20.18.0",
            "/home/p_bob/sites/next/app",
            20002,
            &NodeSiteEntrypoint::NpmCustom("preview".to_string()),
            true,
            "/etc/panel/env/site-2.env",
            None,
        )
        .expect("npm custom unit");

        assert!(
            unit_npm_custom
                .contains("ExecStart=/opt/panel/runtimes/node/20.18.0/bin/npm run preview")
        );
    }

    #[test]
    fn test_generate_node_site_env() {
        let mut vars = BTreeMap::new();
        vars.insert("NODE_ENV".to_string(), "production".to_string());
        vars.insert(
            "DATABASE_URL".to_string(),
            "postgres://user:pass@127.0.0.1:5432/db".to_string(),
        );
        vars.insert("APP_TITLE".to_string(), "My Awesome App".to_string());

        let env_content = generate_node_site_env(&vars).expect("generate env");
        assert!(env_content.contains("NODE_ENV=production\n"));
        assert!(env_content.contains("DATABASE_URL=postgres://user:pass@127.0.0.1:5432/db\n"));
        assert!(env_content.contains("APP_TITLE=\"My Awesome App\"\n"));
    }

    #[test]
    fn test_apply_and_remove_node_site_mock() {
        let runner = MockSystemRunner::default();
        let mut env_vars = BTreeMap::new();
        env_vars.insert("PORT".to_string(), "20042".to_string());

        let req_apply = NodeSiteApplyRequest {
            site_id: "site-1".to_string(),
            username: "p_alice".to_string(),
            uid: 10001,
            node_version: "22.14.0".to_string(),
            app_dir: "/home/p_alice/sites/app".to_string(),
            port: 20042,
            entrypoint: NodeSiteEntrypoint::NodeScript("server.js".to_string()),
            env_vars,
            auto_restart: true,
            env_file_path: Some("/tmp/test-env.env".to_string()),
            unit_path: Some("/tmp/test-site.service".to_string()),
            base_runtime_dir: None,
        };

        runner.add_existing_path("/opt/panel/runtimes/node/22.14.0/bin/node");
        runner.add_existing_path("/home/p_alice/sites/app/server.js");
        let res_apply = apply_node_site(&runner, &req_apply).expect("apply node site");
        assert_eq!(res_apply.site_id, "site-1");
        assert_eq!(res_apply.port, 20042);
        assert!(res_apply.active);

        // Verify actions performed
        let actions = runner.actions.lock().unwrap();
        assert!(actions.iter().any(|a| matches!(a, MockAction::WriteFile { path, .. } if path == &PathBuf::from("/tmp/test-env.env"))));
        assert!(actions.iter().any(|a| matches!(a, MockAction::WriteFile { path, .. } if path == &PathBuf::from("/tmp/test-site.service"))));
        assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["daemon-reload"])));
        assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["restart", "site-site-1.service"])));
        drop(actions);

        // Now test remove
        let req_remove = NodeSiteRemoveRequest {
            site_id: "site-1".to_string(),
            unit_path: Some("/tmp/test-site.service".to_string()),
            env_file_path: Some("/tmp/test-env.env".to_string()),
        };
        let res_remove = remove_node_site(&runner, &req_remove).expect("remove node site");
        assert!(res_remove.success);

        let actions2 = runner.actions.lock().unwrap();
        assert!(actions2.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["stop", "site-site-1.service"])));
        assert!(actions2.iter().any(|a| matches!(a, MockAction::RemoveFile { path } if path == &PathBuf::from("/tmp/test-site.service"))));
        assert!(actions2.iter().any(|a| matches!(a, MockAction::RemoveFile { path } if path == &PathBuf::from("/tmp/test-env.env"))));
    }

    #[test]
    fn test_action_node_site_mock() {
        let runner = MockSystemRunner::default();
        let req = NodeSiteActionRequest {
            site_id: "site-42".to_string(),
            action: NodeSiteActionType::Restart,
        };
        let res = action_node_site(&runner, &req).expect("restart node site");
        assert!(res.success);
        assert_eq!(res.action, "restart");

        let actions = runner.actions.lock().unwrap();
        assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["restart", "site-site-42.service"])));
    }

    #[test]
    fn test_status_node_site_mock() {
        let runner = MockSystemRunner::default();
        // Setup mock response for systemctl show
        let show_output = "ActiveState=active\nSubState=running\nMainPID=9876\nMemoryCurrent=67108864\nCPUUsageNSec=500000000\n";
        runner.set_response("systemctl", 0, show_output.as_bytes(), b"");

        let req = NodeSiteStatusRequest {
            site_id: "site-42".to_string(),
            port: None,
            probe_path: None,
        };

        let res = status_node_site(&runner, &req).expect("status node site");
        assert!(res.active);
        assert_eq!(res.sub_state, "running");
        assert_eq!(res.pid, Some(9876));
        assert_eq!(res.memory_bytes, Some(67108864));
        assert_eq!(res.cpu_usage_usec, Some(500000));
        assert!(res.healthy);
    }

    #[test]
    fn test_run_node_site_job_mock() {
        let runner = MockSystemRunner::default();
        runner.set_response(
            "systemd-run",
            0,
            b"up to date, audited 150 packages in 1s\n",
            b"",
        );

        let req = NodeSiteJobRequest {
            site_id: "site-42".to_string(),
            username: "p_alice".to_string(),
            uid: 10001,
            node_version: "22.14.0".to_string(),
            app_dir: "/home/p_alice/sites/app".to_string(),
            job_type: NodeSiteJobType::NpmInstall,
            timeout_secs: Some(60),
            base_runtime_dir: None,
        };

        let res = run_node_site_job(&runner, &req).expect("run node site job");
        assert!(res.success);
        assert_eq!(res.exit_code, 0);
        assert!(res.stdout.contains("audited 150 packages"));
        runner.set_spawn_error("systemd-run");
        runner.actions.lock().expect("actions").clear();
        assert!(run_node_site_job(&runner, &req).is_err());
        let actions = runner.actions.lock().expect("actions");
        assert_eq!(actions.len(), 1);
        assert!(
            matches!(&actions[0], MockAction::Command { binary, .. } if binary == "systemd-run")
        );
    }
}
