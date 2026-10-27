//! `panelctl uninstall` command handlers for clean panel uninstallation.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::Path;
use tracing::info;

fn remove_panel_proxy(purge: bool) -> Result<(), Box<dyn Error>> {
    let _ = purge;
    #[cfg(unix)]
    if cfg!(target_os = "linux") {
        let available = Path::new("/etc/nginx/sites-available/nusa-panel.conf");
        let enabled = Path::new("/etc/nginx/sites-enabled/nusa-panel.conf");
        let bridge = Path::new("/etc/nginx/conf.d/00-nusa-panel-sites.conf");
        let bridge_contents = if purge && bridge.exists() {
            Some(std::fs::read(bridge)?)
        } else {
            None
        };
        let contents = if available.exists() {
            Some(std::fs::read(available)?)
        } else {
            None
        };
        let link = std::fs::read_link(enabled).ok();
        let enabled_contents = if link.is_none() && enabled.is_file() {
            Some(std::fs::read(enabled)?)
        } else {
            None
        };
        if contents.is_none()
            && link.is_none()
            && enabled_contents.is_none()
            && bridge_contents.is_none()
        {
            return Ok(());
        }
        if bridge_contents.is_some() {
            std::fs::remove_file(bridge)?;
        }
        if enabled.symlink_metadata().is_ok() {
            std::fs::remove_file(enabled)?;
        }
        if available.exists() {
            std::fs::remove_file(available)?;
        }
        let syntax = panel_core::production::system_command("nginx")
            .arg("-t")
            .status();
        let active = panel_core::production::system_command("systemctl")
            .args(["is-active", "--quiet", "nginx"])
            .status();
        let valid = matches!(syntax, Ok(status) if status.success());
        let reloaded = if valid && matches!(active, Ok(status) if status.success()) {
            panel_core::production::system_command("systemctl")
                .args(["reload", "nginx"])
                .status()
                .is_ok_and(|status| status.success())
        } else {
            valid
        };
        if !reloaded {
            if let Some(bytes) = bridge_contents {
                std::fs::write(bridge, bytes)?;
            }
            if let Some(bytes) = contents {
                std::fs::write(available, bytes)?;
            }
            if let Some(target) = link {
                std::os::unix::fs::symlink(target, enabled)?;
            }
            if let Some(bytes) = enabled_contents {
                std::fs::write(enabled, bytes)?;
            }
            return Err(
                "Nginx validation/reload failed; panel proxy restored and uninstall aborted".into(),
            );
        }
    }
    Ok(())
}

/// Options for `panelctl uninstall`.
#[derive(Debug, Clone, Default)]
pub struct UninstallCliOptions {
    /// Purge all panel databases, configuration files, and backup snapshots
    pub purge: bool,
    /// Automatically confirm uninstallation prompt
    pub yes: bool,
    /// Output result as formatted JSON
    pub json: bool,
}

/// Detailed result of the uninstall operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallResult {
    pub success: bool,
    pub services_stopped: bool,
    pub unit_files_removed: bool,
    pub binaries_removed: bool,
    pub data_purged: bool,
    pub message: String,
}

fn stop_and_remove_systemd_services() -> (bool, bool) {
    if !cfg!(target_os = "linux") {
        return (false, false);
    }

    let _ = std::process::Command::new("systemctl")
        .args([
            "stop",
            "panel-api.service",
            "panel-agent.service",
            "panel-proc-hardening.service",
        ])
        .status();

    let _ = std::process::Command::new("systemctl")
        .args([
            "disable",
            "panel-api.service",
            "panel-agent.service",
            "panel-proc-hardening.service",
        ])
        .status();

    let mut unit_removed = false;
    for unit_file in &[
        "/etc/systemd/system/panel-api.service",
        "/etc/systemd/system/panel-agent.service",
        "/etc/systemd/system/panel-proc-hardening.service",
    ] {
        let p = Path::new(unit_file);
        if p.exists() {
            let _ = std::fs::remove_file(p);
            unit_removed = true;
        }
    }

    let _ = std::process::Command::new("systemctl")
        .arg("daemon-reload")
        .status();

    (true, unit_removed)
}

fn remove_installed_binaries() -> bool {
    let mut removed = false;
    let bin_dir = Path::new("/usr/local/bin");
    for bin_name in &["panel-api", "panel-agent", "panelctl"] {
        let bin_path = bin_dir.join(bin_name);
        if bin_path.exists() {
            let _ = std::fs::remove_file(&bin_path);
            removed = true;
        }
    }
    let _ = std::fs::remove_dir_all("/run/panel");
    removed
}

fn purge_data_and_users(purge: bool) -> bool {
    if !purge {
        info!("Direktori data (/var/lib/panel, /var/backups/panel) dipertahankan (--keep-data).");
        return false;
    }

    info!("Membersihkan seluruh direktori data dan konfigurasi Qinpanel (--purge)...");
    let mut purged = false;
    for dir_path in &[
        "/var/lib/panel",
        "/var/backups/panel",
        "/etc/panel",
        "/var/log/panel",
    ] {
        let p = Path::new(dir_path);
        if p.exists() {
            let _ = std::fs::remove_dir_all(p);
            purged = true;
        }
    }

    if cfg!(target_os = "linux") {
        let _ = std::process::Command::new("userdel")
            .arg("panel-api")
            .status();
        let _ = std::process::Command::new("groupdel").arg("panel").status();
    }
    purged
}

/// Executes clean uninstallation of Qinpanel.
pub async fn run_uninstall(
    options: &UninstallCliOptions,
) -> Result<UninstallResult, Box<dyn Error>> {
    info!("Memulai proses pencopotan pemasangan (uninstall) Qinpanel...");

    remove_panel_proxy(options.purge)?;
    let (services_stopped, unit_files_removed) = stop_and_remove_systemd_services();
    let binaries_removed = remove_installed_binaries();
    let data_purged = purge_data_and_users(options.purge);

    let res = UninstallResult {
        success: true,
        services_stopped,
        unit_files_removed,
        binaries_removed,
        data_purged,
        message: if options.purge {
            "Qinpanel berhasil dicopot sepenuhnya beserta seluruh data dan konfigurasi.".to_string()
        } else {
            "Qinpanel berhasil dicopot. Berkas basis data dan cadangan dipertahankan.".to_string()
        },
    };

    if options.json {
        let json_str = serde_json::to_string_pretty(&res)?;
        println!("{}", json_str);
    } else {
        println!(
            "================================================================================"
        );
        println!("                     HASIL PENCAPUTAN QONPANEL                                ");
        println!(
            "================================================================================"
        );
        println!("\x1b[32m[SUKSES]\x1b[0m {}", res.message);
        println!(
            "Layanan Dihentikan : {}",
            if res.services_stopped {
                "Ya"
            } else {
                "Tidak (Bukan Linux)"
            }
        );
        println!(
            "Unit Systemd       : {}",
            if res.unit_files_removed {
                "Dihapus"
            } else {
                "Tidak ditemukan"
            }
        );
        println!(
            "Biner Dihapus      : {}",
            if res.binaries_removed {
                "Ya"
            } else {
                "Tidak ditemukan"
            }
        );
        println!(
            "Status Data        : {}",
            if res.data_purged {
                "Dibersihkan (--purge)"
            } else {
                "Dipertahankan di /var/lib/panel"
            }
        );
        println!(
            "================================================================================"
        );
    }

    Ok(res)
}
