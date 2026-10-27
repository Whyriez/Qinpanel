//! Cron job systemd timer & service unit management, run-now execution, and log retrieval.

use crate::AgentError;
use crate::system::SystemRunner;
use panel_proto::{
    CronJobApplyRequest, CronJobApplyResponse, CronJobLogsReadRequest, CronJobLogsReadResponse,
    CronJobRemoveRequest, CronJobRemoveResponse, CronJobRunNowRequest, CronJobRunNowResponse,
};
use std::path::PathBuf;
use std::time::Instant;

pub const DEFAULT_SYSTEMD_UNIT_DIR: &str = "/etc/systemd/system";
pub const DEFAULT_ENV_DIR: &str = "/etc/panel/env";

/// Validates identifier string like site_id or job_name.
pub fn validate_site_id(site_id: &str) -> Result<(), AgentError> {
    if site_id.trim().is_empty() {
        return Err(AgentError::InvalidInput(
            "Site ID tidak boleh kosong".into(),
        ));
    }
    if !site_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AgentError::InvalidInput(format!(
            "Site ID '{}' mengandung karakter tidak valid",
            site_id
        )));
    }
    Ok(())
}

/// Validates tenant linux username.
pub fn validate_username(username: &str) -> Result<(), AgentError> {
    if username.trim().is_empty() {
        return Err(AgentError::InvalidInput(
            "Username tidak boleh kosong".into(),
        ));
    }
    if !username.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
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

/// Validates cron command input.
pub fn validate_command(command: &str) -> Result<(), AgentError> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(AgentError::InvalidInput(
            "Command cron tidak boleh kosong".into(),
        ));
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(AgentError::InvalidInput(
            "Command cron tidak boleh mengandung karakter baris baru (newline)".into(),
        ));
    }
    if trimmed.contains('\0') {
        return Err(AgentError::InvalidInput(
            "Command cron tidak boleh mengandung karakter null byte".into(),
        ));
    }
    Ok(())
}

/// Configuration parameters for generating a systemd cron service unit.
#[derive(Debug, Clone)]
pub struct CronServiceUnitConfig<'a> {
    pub site_id: &'a str,
    pub job_id: i64,
    pub name: &'a str,
    pub username: &'a str,
    pub uid: u32,
    pub command: &'a str,
    pub work_dir: &'a str,
    pub timeout_secs: u64,
    pub env_file_path: &'a str,
}

/// Generates systemd oneshot service unit content for a cron job.
pub fn generate_cron_service_unit(
    config: &CronServiceUnitConfig<'_>,
) -> Result<String, AgentError> {
    validate_site_id(config.site_id)?;
    validate_username(config.username)?;
    validate_command(config.command)?;

    let clean_name = config.name.replace(['\n', '\r'], " ");
    let site_id = config.site_id;
    let job_id = config.job_id;
    let username = config.username;
    let uid = config.uid;
    let command = config.command;
    let work_dir = config.work_dir;
    let timeout_secs = config.timeout_secs;
    let env_file_path = config.env_file_path;

    let unit = format!(
        "[Unit]\n\
Description=Cron Job: {clean_name} (Site {site_id}, Job {job_id})\n\
After=network.target\n\
\n\
[Service]\n\
Type=oneshot\n\
User={username}\n\
Group={username}\n\
Slice=panel-u{uid}.slice\n\
WorkingDirectory={work_dir}\n\
EnvironmentFile=-{env_file_path}\n\
Environment=PATH=/usr/local/bin:/usr/bin:/bin\n\
ExecStart={command}\n\
TimeoutStartSec={timeout_secs}\n\
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
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6\n"
    );

    Ok(unit)
}

/// Generates systemd timer unit content for a cron job.
pub fn generate_cron_timer_unit(
    site_id: &str,
    job_id: i64,
    name: &str,
    systemd_calendar: &str,
) -> Result<String, AgentError> {
    validate_site_id(site_id)?;
    let clean_name = name.replace(['\n', '\r'], " ");
    let service_name = format!("site-{site_id}-cron-{job_id}.service");

    let unit = format!(
        "[Unit]\n\
Description=Timer for Cron Job: {clean_name} (Site {site_id}, Job {job_id})\n\
\n\
[Timer]\n\
OnCalendar={systemd_calendar}\n\
Persistent=true\n\
Unit={service_name}\n\
\n\
[Install]\n\
WantedBy=timers.target\n"
    );

    Ok(unit)
}

/// Writes service and timer units, reloads systemd, and enables/disables the timer.
pub fn apply_cron_job(
    runner: &dyn SystemRunner,
    req: &CronJobApplyRequest,
) -> Result<CronJobApplyResponse, AgentError> {
    validate_site_id(&req.site_id)?;
    validate_username(&req.username)?;
    validate_command(&req.command)?;

    let unit_dir = req.unit_dir.as_deref().unwrap_or(DEFAULT_SYSTEMD_UNIT_DIR);
    let env_file_path = req
        .env_file_path
        .clone()
        .unwrap_or_else(|| format!("{}/site-{}.env", DEFAULT_ENV_DIR, req.site_id));
    let work_dir = req
        .work_dir
        .clone()
        .unwrap_or_else(|| format!("/home/{}", req.username));

    let service_name = format!("site-{}-cron-{}.service", req.site_id, req.job_id);
    let timer_name = format!("site-{}-cron-{}.timer", req.site_id, req.job_id);

    let service_content = generate_cron_service_unit(&CronServiceUnitConfig {
        site_id: &req.site_id,
        job_id: req.job_id,
        name: &req.name,
        username: &req.username,
        uid: req.uid,
        command: &req.command,
        work_dir: &work_dir,
        timeout_secs: req.timeout_secs,
        env_file_path: &env_file_path,
    })?;

    let timer_content =
        generate_cron_timer_unit(&req.site_id, req.job_id, &req.name, &req.systemd_calendar)?;

    let service_path = PathBuf::from(unit_dir).join(&service_name);
    let timer_path = PathBuf::from(unit_dir).join(&timer_name);

    runner.write_file(&service_path, service_content.as_bytes(), 0o644)?;
    runner.write_file(&timer_path, timer_content.as_bytes(), 0o644)?;

    // Reload systemd daemon
    runner.run_command("systemctl", &["daemon-reload"])?;

    if req.enabled {
        runner.run_command("systemctl", &["enable", "--now", &timer_name])?;
    } else {
        let _ = runner.run_command("systemctl", &["disable", "--now", &timer_name]);
    }

    Ok(CronJobApplyResponse {
        site_id: req.site_id.clone(),
        job_id: req.job_id,
        service_name,
        timer_name,
        enabled: req.enabled,
        success: true,
        message: if req.enabled {
            "Cron timer berhasil diaktifkan".to_string()
        } else {
            "Cron timer disimpan dalam status nonaktif".to_string()
        },
    })
}

/// Disables timer and removes both timer and service unit files.
pub fn remove_cron_job(
    runner: &dyn SystemRunner,
    req: &CronJobRemoveRequest,
) -> Result<CronJobRemoveResponse, AgentError> {
    validate_site_id(&req.site_id)?;

    let unit_dir = req.unit_dir.as_deref().unwrap_or(DEFAULT_SYSTEMD_UNIT_DIR);
    let service_name = format!("site-{}-cron-{}.service", req.site_id, req.job_id);
    let timer_name = format!("site-{}-cron-{}.timer", req.site_id, req.job_id);

    // Disable timer (ignore failure if not enabled)
    let _ = runner.run_command("systemctl", &["disable", "--now", &timer_name]);

    let timer_path = PathBuf::from(unit_dir).join(&timer_name);
    let service_path = PathBuf::from(unit_dir).join(&service_name);

    if runner.path_exists(&timer_path) {
        let _ = runner.remove_file(&timer_path);
    }
    if runner.path_exists(&service_path) {
        let _ = runner.remove_file(&service_path);
    }

    let _ = runner.run_command("systemctl", &["daemon-reload"]);

    Ok(CronJobRemoveResponse {
        site_id: req.site_id.clone(),
        job_id: req.job_id,
        success: true,
        message: format!("Cron job {} berhasil dihapus", req.job_id),
    })
}

/// Runs a cron job immediately via systemd service execution and returns output.
pub fn run_now_cron_job(
    runner: &dyn SystemRunner,
    req: &CronJobRunNowRequest,
) -> Result<CronJobRunNowResponse, AgentError> {
    validate_site_id(&req.site_id)?;
    validate_username(&req.username)?;
    validate_command(&req.command)?;

    let service_name = format!("site-{}-cron-{}.service", req.site_id, req.job_id);

    let start = Instant::now();
    let output_res = runner.run_command("systemctl", &["start", "--wait", &service_name]);
    let duration_ms = start.elapsed().as_millis() as u64;

    let (exit_code, success, mut stderr) = match output_res {
        Ok(out) => {
            let code = out.status.code().unwrap_or(0);
            (
                code,
                out.status.success(),
                String::from_utf8_lossy(&out.stderr).to_string(),
            )
        }
        Err(e) => (-1, false, e.to_string()),
    };

    // Retrieve recent journal output for the service
    let stdout = match runner.run_command(
        "journalctl",
        &["-u", &service_name, "-n", "50", "--no-pager"],
    ) {
        Ok(log_out) => String::from_utf8_lossy(&log_out.stdout).to_string(),
        Err(e) => {
            if stderr.is_empty() {
                stderr = e.to_string();
            }
            String::new()
        }
    };

    Ok(CronJobRunNowResponse {
        site_id: req.site_id.clone(),
        job_id: req.job_id,
        exit_code,
        stdout,
        stderr,
        duration_ms,
        success,
    })
}

/// Reads journalctl logs for a specific cron job service.
pub fn read_cron_job_logs(
    runner: &dyn SystemRunner,
    req: &CronJobLogsReadRequest,
) -> Result<CronJobLogsReadResponse, AgentError> {
    validate_site_id(&req.site_id)?;

    let service_name = format!("site-{}-cron-{}.service", req.site_id, req.job_id);
    let lines_str = req.lines.min(500).to_string();

    let output = runner.run_command(
        "journalctl",
        &["-u", &service_name, "-n", &lines_str, "--no-pager"],
    )?;

    let raw = String::from_utf8_lossy(&output.stdout);
    let logs: Vec<String> = raw
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(CronJobLogsReadResponse {
        site_id: req.site_id.clone(),
        job_id: req.job_id,
        logs,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::system::{MockAction, MockSystemRunner};

    #[test]
    fn test_generate_cron_service_unit() {
        let unit = generate_cron_service_unit(&CronServiceUnitConfig {
            site_id: "site-42",
            job_id: 7,
            name: "Laravel Scheduler",
            username: "p_alice",
            uid: 10001,
            command: "php artisan schedule:run",
            work_dir: "/home/p_alice/sites/laravel.test",
            timeout_secs: 1800,
            env_file_path: "/etc/panel/env/site-site-42.env",
        })
        .unwrap();

        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("Description=Cron Job: Laravel Scheduler (Site site-42, Job 7)"));
        assert!(unit.contains("Type=oneshot"));
        assert!(unit.contains("User=p_alice"));
        assert!(unit.contains("Group=p_alice"));
        assert!(unit.contains("Slice=panel-u10001.slice"));
        assert!(unit.contains("WorkingDirectory=/home/p_alice/sites/laravel.test"));
        assert!(unit.contains("EnvironmentFile=-/etc/panel/env/site-site-42.env"));
        assert!(unit.contains("ExecStart=php artisan schedule:run"));
        assert!(unit.contains("TimeoutStartSec=1800"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("PrivateTmp=true"));
    }

    #[test]
    fn test_generate_cron_timer_unit() {
        let timer = generate_cron_timer_unit("site-42", 7, "Laravel Scheduler", "*-*-* *:00/15:00")
            .unwrap();

        assert!(unit_contains(&timer, "[Unit]"));
        assert!(unit_contains(
            &timer,
            "Description=Timer for Cron Job: Laravel Scheduler (Site site-42, Job 7)"
        ));
        assert!(unit_contains(&timer, "[Timer]"));
        assert!(unit_contains(&timer, "OnCalendar=*-*-* *:00/15:00"));
        assert!(unit_contains(&timer, "Persistent=true"));
        assert!(unit_contains(&timer, "Unit=site-site-42-cron-7.service"));
        assert!(unit_contains(&timer, "WantedBy=timers.target"));
    }

    fn unit_contains(haystack: &str, needle: &str) -> bool {
        haystack.contains(needle)
    }

    #[test]
    fn test_apply_and_remove_cron_job_lifecycle() {
        let runner = MockSystemRunner::default();
        let unit_dir = "/tmp/systemd";

        let apply_req = CronJobApplyRequest {
            site_id: "site-1".to_string(),
            job_id: 10,
            name: "Nightly Cleanup".to_string(),
            username: "p_bob".to_string(),
            uid: 10002,
            command: "python manage.py clearsessions".to_string(),
            schedule: "0 2 * * *".to_string(),
            systemd_calendar: "*-*-* 02:00:00".to_string(),
            work_dir: Some("/home/p_bob/sites/django".to_string()),
            timeout_secs: 600,
            enabled: true,
            unit_dir: Some(unit_dir.to_string()),
            env_file_path: None,
        };

        let res = apply_cron_job(&runner, &apply_req).unwrap();
        assert_eq!(res.service_name, "site-site-1-cron-10.service");
        assert_eq!(res.timer_name, "site-site-1-cron-10.timer");
        assert!(res.enabled);

        // Verify actions
        let actions = runner.actions.lock().unwrap();
        let wrote_service = actions.iter().any(|a| matches!(a, MockAction::WriteFile { path, .. } if path.to_str().unwrap().contains("site-site-1-cron-10.service")));
        let wrote_timer = actions.iter().any(|a| matches!(a, MockAction::WriteFile { path, .. } if path.to_str().unwrap().contains("site-site-1-cron-10.timer")));
        let reloaded = actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["daemon-reload"]));
        let enabled = actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["enable", "--now", "site-site-1-cron-10.timer"]));

        assert!(wrote_service);
        assert!(wrote_timer);
        assert!(reloaded);
        assert!(enabled);
        drop(actions);

        // Remove cron job
        let rem_req = CronJobRemoveRequest {
            site_id: "site-1".to_string(),
            job_id: 10,
            unit_dir: Some(unit_dir.to_string()),
        };

        let rem_res = remove_cron_job(&runner, &rem_req).unwrap();
        assert!(rem_res.success);

        let actions_after = runner.actions.lock().unwrap();
        let disabled = actions_after.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "systemctl" && args == &["disable", "--now", "site-site-1-cron-10.timer"]));
        assert!(disabled);
    }
}
