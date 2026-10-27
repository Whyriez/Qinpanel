//! Host hardening and prerequisites validation.
//!
//! Provides diagnostic checks and automated hardening enforcement for:
//! - cgroup v2 unified hierarchy with all required controllers (cpu, memory, io, pids).
//! - POSIX ACL support on tenant filesystems.
//! - Filesystem quota support (kernel ext4/xfs quota or fallback scanner).
//! - `/proc` filesystem mounting with `hidepid=2` to prevent tenant cross-process snooping.
//! - Presence of dedicated Linux system group `panel-tenants`.

use crate::AgentError;
use crate::system::SystemRunner;
use panel_proto::{
    HardeningApplyRequest, HardeningApplyResponse, HardeningStatus, PrerequisiteCheckItem,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

pub const DEFAULT_CGROUP_ROOT: &str = "/sys/fs/cgroup";
// /proc/mounts describes the agent's private namespace, not necessarily the host.
pub const DEFAULT_PROC_MOUNTS: &str = "/proc/1/mounts";
pub const DEFAULT_GROUP_FILE: &str = "/etc/group";
pub const PROC_HARDENING_UNIT: &str = "panel-proc-hardening.service";

fn has_hidepid(options: &str) -> bool {
    options
        .split(',')
        .any(|option| matches!(option.trim(), "hidepid=2" | "hidepid=invisible"))
}

pub struct HardeningManager {
    runner: Arc<dyn SystemRunner>,
    cgroup_root: PathBuf,
    proc_mounts: PathBuf,
    group_file: PathBuf,
}

impl HardeningManager {
    pub fn new(runner: Arc<dyn SystemRunner>) -> Self {
        Self {
            runner,
            cgroup_root: PathBuf::from(DEFAULT_CGROUP_ROOT),
            proc_mounts: PathBuf::from(DEFAULT_PROC_MOUNTS),
            group_file: PathBuf::from(DEFAULT_GROUP_FILE),
        }
    }

    pub fn with_paths(
        runner: Arc<dyn SystemRunner>,
        cgroup_root: impl AsRef<Path>,
        proc_mounts: impl AsRef<Path>,
        group_file: impl AsRef<Path>,
    ) -> Self {
        Self {
            runner,
            cgroup_root: cgroup_root.as_ref().to_path_buf(),
            proc_mounts: proc_mounts.as_ref().to_path_buf(),
            group_file: group_file.as_ref().to_path_buf(),
        }
    }

    /// Validates cgroup v2 unified hierarchy and required controllers.
    pub fn check_cgroup_v2(&self) -> PrerequisiteCheckItem {
        let controllers_file = self.cgroup_root.join("cgroup.controllers");
        if !controllers_file.exists() {
            return PrerequisiteCheckItem {
                name: "cgroup_v2".to_string(),
                ok: false,
                message: "cgroup v2 tidak aktif atau hirarki terpadu dinonaktifkan. Aktifkan dengan parameter kernel systemd.unified_cgroup_hierarchy=1 di GRUB.".to_string(),
                details: Some("File cgroup.controllers tidak ditemukan di /sys/fs/cgroup".to_string()),
            };
        }

        match fs::read_to_string(&controllers_file) {
            Ok(content) => {
                let required = ["cpu", "memory", "io", "pids"];
                let available: Vec<&str> = content.split_whitespace().collect();
                let missing: Vec<&&str> = required
                    .iter()
                    .filter(|req| !available.contains(req))
                    .collect();

                if missing.is_empty() {
                    PrerequisiteCheckItem {
                        name: "cgroup_v2".to_string(),
                        ok: true,
                        message: "cgroup v2 aktif dengan kontrol penuh (cpu, memory, io, pids)"
                            .to_string(),
                        details: Some(format!("Available controllers: {}", available.join(" "))),
                    }
                } else {
                    let missing_str = missing.iter().map(|s| **s).collect::<Vec<_>>().join(", ");
                    PrerequisiteCheckItem {
                        name: "cgroup_v2".to_string(),
                        ok: false,
                        message: format!(
                            "cgroup v2 aktif tetapi controller belum lengkap: {missing_str}. Pastikan controller didelegasikan ke cgroup panel."
                        ),
                        details: Some(format!("Missing: {missing_str}")),
                    }
                }
            }
            Err(e) => PrerequisiteCheckItem {
                name: "cgroup_v2".to_string(),
                ok: false,
                message: format!("Gagal membaca status controller cgroup v2: {e}"),
                details: Some(e.to_string()),
            },
        }
    }

    /// Checks POSIX ACL support on filesystem.
    pub fn check_acl(&self) -> PrerequisiteCheckItem {
        match self.runner.run_command("getfacl", &["/"]) {
            Ok(output) if output.status.success() => PrerequisiteCheckItem {
                name: "acl".to_string(),
                ok: true,
                message: "Dukungan POSIX ACL sistem berkas aktif".to_string(),
                details: None,
            },
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                PrerequisiteCheckItem {
                    name: "acl".to_string(),
                    ok: false,
                    message: "POSIX ACL tidak aktif pada sistem berkas. Pastikan filesystem di-mount dengan opsi 'acl' di /etc/fstab.".to_string(),
                    details: Some(stderr.trim().to_string()),
                }
            }
            Err(e) => PrerequisiteCheckItem {
                name: "acl".to_string(),
                ok: false,
                message: format!("Pengecekan POSIX ACL gagal: {e}"),
                details: Some(e.to_string()),
            },
        }
    }

    /// Checks quota subsystem support (kernel ext4/xfs or fallback scanner).
    pub fn check_quota(&self) -> PrerequisiteCheckItem {
        // Try repquota or findmnt
        let is_kernel_quota = self
            .runner
            .run_command("repquota", &["-u", "/"])
            .map(|o| o.status.success())
            .unwrap_or(false)
            || self
                .runner
                .run_command("xfs_quota", &["-x", "-c", "state"])
                .map(|o| o.status.success())
                .unwrap_or(false);

        if is_kernel_quota {
            PrerequisiteCheckItem {
                name: "quota".to_string(),
                ok: true,
                message: "Kuota disk kernel aktif (ext4/xfs)".to_string(),
                details: Some("Kernel filesystem quota is active".to_string()),
            }
        } else {
            PrerequisiteCheckItem {
                name: "quota".to_string(),
                ok: true,
                message: "Kuota kernel tidak aktif; scanner direktori rekursif fallback siap digunakan (rekomendasi: aktifkan usrquota pada /etc/fstab untuk performa optimal)".to_string(),
                details: Some("Using recursive fallback scanner".to_string()),
            }
        }
    }

    /// Checks whether `/proc` is mounted with `hidepid=2` or `hidepid=invisible`.
    pub fn check_hidepid(&self) -> PrerequisiteCheckItem {
        let is_hidepid = if let Ok(mounts) = fs::read_to_string(&self.proc_mounts) {
            mounts.lines().any(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 && parts[1] == "/proc" && parts[2] == "proc" {
                    let opts = parts[3];
                    has_hidepid(opts)
                } else {
                    false
                }
            })
        } else {
            // Fallback to findmnt command
            self.runner
                .run_command(
                    "findmnt",
                    &["--task", "1", "-n", "-o", "OPTIONS", "--target", "/proc"],
                )
                .map(|o| {
                    let opts = String::from_utf8_lossy(&o.stdout);
                    o.status.success() && has_hidepid(opts.trim())
                })
                .unwrap_or(false)
        };

        if is_hidepid {
            PrerequisiteCheckItem {
                name: "hidepid".to_string(),
                ok: true,
                message:
                    "Hardening procfs hidepid=2 aktif (proses tenant lain terisolasi di /proc)"
                        .to_string(),
                details: Some("Host PID 1 procfs mounted with hidepid=2".to_string()),
            }
        } else {
            PrerequisiteCheckItem {
                name: "hidepid".to_string(),
                ok: false,
                message: "procfs hidepid=2 belum aktif atau status host belum dapat diverifikasi. Aktifkan melalui panel; pastikan unit panel-proc-hardening.service terpasang dan host mengizinkan remount procfs.".to_string(),
                details: Some(format!("Host mount source: {}", self.proc_mounts.display())),
            }
        }
    }

    /// Checks whether Linux system group `panel-tenants` exists.
    pub fn check_tenant_group(&self) -> PrerequisiteCheckItem {
        let exists = if let Ok(groups) = fs::read_to_string(&self.group_file) {
            groups.lines().any(|l| l.starts_with("panel-tenants:"))
        } else {
            self.runner
                .run_command("getent", &["group", "panel-tenants"])
                .map(|o| o.status.success())
                .unwrap_or(false)
        };

        if exists {
            PrerequisiteCheckItem {
                name: "tenant_group".to_string(),
                ok: true,
                message: "Grup sistem 'panel-tenants' tersedia".to_string(),
                details: Some("Group panel-tenants exists".to_string()),
            }
        } else {
            PrerequisiteCheckItem {
                name: "tenant_group".to_string(),
                ok: false,
                message: "Grup sistem 'panel-tenants' belum dibuat. Jalankan apply hardening atau 'groupadd panel-tenants'.".to_string(),
                details: Some("Group panel-tenants not found".to_string()),
            }
        }
    }

    /// Runs all diagnostic prerequisite checks.
    pub fn check_all(&self) -> HardeningStatus {
        let cgroup_v2 = self.check_cgroup_v2();
        let acl = self.check_acl();
        let quota = self.check_quota();
        let hidepid = self.check_hidepid();
        let tenant_group = self.check_tenant_group();

        let overall_ok = cgroup_v2.ok && acl.ok && quota.ok && hidepid.ok && tenant_group.ok;

        HardeningStatus {
            cgroup_v2,
            acl,
            quota,
            hidepid,
            tenant_group,
            overall_ok,
        }
    }

    /// Applies requested hardening actions (group creation, hidepid remount).
    pub fn apply(&self, req: HardeningApplyRequest) -> Result<HardeningApplyResponse, AgentError> {
        let mut messages = Vec::new();
        let mut tenant_group_created = false;
        let mut hidepid_applied = false;

        // 1. Ensure panel-tenants group
        if req.ensure_tenant_group {
            let group_check = self.check_tenant_group();
            if !group_check.ok {
                info!("Creating system group 'panel-tenants'...");
                let output = self
                    .runner
                    .run_command("groupadd", &["--system", "panel-tenants"])?;
                if !output.status.success() {
                    let err = String::from_utf8_lossy(&output.stderr);
                    return Err(AgentError::CommandFailed {
                        binary: "groupadd".to_string(),
                        code: output.status.code(),
                        stderr: err.to_string(),
                    });
                }
                tenant_group_created = true;
                messages.push("Grup 'panel-tenants' berhasil dibuat");
            } else {
                messages.push("Grup 'panel-tenants' sudah ada");
            }
        }

        // 2. Enable hidepid=2 on /proc
        if req.enable_hidepid {
            // Persist the explicitly requested policy across reboot. PID 1 starts
            // the fixed oneshot outside this daemon's filesystem namespace.
            self.runner.run_command("systemctl", &["enable", PROC_HARDENING_UNIT])
                .map_err(|e| AgentError::Internal(format!(
                    "Gagal mengaktifkan {PROC_HARDENING_UNIT}. Pasang unit dari release terbaru: {e}"
                )))?;
            let hidepid_check = self.check_hidepid();
            if !hidepid_check.ok {
                info!("Applying host procfs policy through {PROC_HARDENING_UNIT}...");
                let output = self
                    .runner
                    .run_command("systemctl", &["restart", PROC_HARDENING_UNIT])
                    .map_err(|e| AgentError::Internal(format!(
                        "Gagal menerapkan hidepid host melalui {PROC_HARDENING_UNIT}. Periksa journal unit dan izin mount host: {e}"
                    )))?;
                if !output.status.success() {
                    let err = String::from_utf8_lossy(&output.stderr);
                    return Err(AgentError::CommandFailed {
                        binary: "systemctl".to_string(),
                        code: output.status.code(),
                        stderr: err.to_string(),
                    });
                }
                let verified = self.runner.run_command(
                    "findmnt",
                    &["--task", "1", "-n", "-o", "OPTIONS", "--target", "/proc"],
                )?;
                if !verified.status.success()
                    || !has_hidepid(String::from_utf8_lossy(&verified.stdout).trim())
                {
                    return Err(AgentError::Internal(
                        "Unit hardening selesai, tetapi hidepid=2 pada /proc host belum terverifikasi. Periksa journal panel-proc-hardening.service.".to_owned()
                    ));
                }
                hidepid_applied = true;
                messages.push(
                    "procfs host memakai hidepid=2 dan kebijakan diaktifkan untuk boot berikutnya",
                );
            } else {
                messages.push("procfs hidepid=2 sudah aktif");
            }
        }

        let message = if messages.is_empty() {
            "Tidak ada tindakan hardening yang diminta".to_string()
        } else {
            messages.join(". ")
        };

        Ok(HardeningApplyResponse {
            hidepid_applied,
            tenant_group_created,
            message,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    use tempfile::tempdir;

    #[test]
    fn test_cgroup_v2_check_full_and_missing() {
        let temp = tempdir().expect("tempdir");
        let runner = Arc::new(MockSystemRunner::new());

        // 1. Missing cgroup.controllers
        let manager = HardeningManager::with_paths(
            Arc::clone(&runner) as Arc<dyn SystemRunner>,
            temp.path(),
            temp.path().join("mounts"),
            temp.path().join("group"),
        );
        let check = manager.check_cgroup_v2();
        assert!(!check.ok);
        assert!(check.message.contains("tidak aktif"));

        // 2. Partial controllers (missing pids)
        fs::write(temp.path().join("cgroup.controllers"), "cpu memory io\n")
            .expect("write controllers");
        let check_partial = manager.check_cgroup_v2();
        assert!(!check_partial.ok);
        assert!(check_partial.message.contains("pids"));

        // 3. Full controllers
        fs::write(
            temp.path().join("cgroup.controllers"),
            "cpu memory io pids\n",
        )
        .expect("write controllers");
        let check_full = manager.check_cgroup_v2();
        assert!(check_full.ok);
        assert!(check_full.message.contains("kontrol penuh"));
    }

    #[test]
    fn test_hidepid_check_and_apply() {
        let temp = tempdir().expect("tempdir");
        let runner = Arc::new(MockSystemRunner::new());
        let mounts_file = temp.path().join("mounts");
        let group_file = temp.path().join("group");

        // procfs without hidepid
        fs::write(
            &mounts_file,
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n",
        )
        .expect("write mounts");
        fs::write(&group_file, "root:x:0:\n").expect("write group");
        runner.set_response(
            "findmnt --task 1 -n -o OPTIONS --target /proc",
            0,
            b"rw,nosuid,nodev,noexec,hidepid=2,gid=100\n",
            b"",
        );

        let manager = HardeningManager::with_paths(
            Arc::clone(&runner) as Arc<dyn SystemRunner>,
            temp.path(),
            &mounts_file,
            &group_file,
        );

        let hidepid_check = manager.check_hidepid();
        assert!(!hidepid_check.ok);
        assert!(hidepid_check.message.contains("belum aktif"));

        // Apply hardening
        let apply_res = manager
            .apply(HardeningApplyRequest {
                enable_hidepid: true,
                ensure_tenant_group: true,
            })
            .expect("apply hardening");

        assert!(apply_res.hidepid_applied);
        assert!(apply_res.tenant_group_created);

        // Verify actions recorded
        let actions = runner.recorded_actions();
        assert!(actions.iter().any(|a| matches!(a, crate::system::MockAction::Command { binary, .. } if binary == "groupadd")));
        let commands = runner.get_commands();
        assert!(commands.contains(&format!("systemctl enable {PROC_HARDENING_UNIT}")));
        assert!(commands.contains(&format!("systemctl restart {PROC_HARDENING_UNIT}")));
        assert!(!commands.iter().any(|command| command.starts_with("mount ")));

        // Update mounts and group to simulate applied state
        fs::write(
            &mounts_file,
            "proc /proc proc rw,nosuid,nodev,noexec,relatime,hidepid=2 0 0\n",
        )
        .expect("write mounts hidepid");
        fs::write(&group_file, "root:x:0:\npanel-tenants:x:1001:\n").expect("write group tenants");

        let hidepid_check_after = manager.check_hidepid();
        assert!(hidepid_check_after.ok);

        let group_check_after = manager.check_tenant_group();
        assert!(group_check_after.ok);
    }

    #[test]
    fn test_proc_mount_option_tokens_are_exact() {
        for accepted in ["rw,hidepid=2", "hidepid=invisible,gid=100"] {
            assert!(has_hidepid(accepted));
        }
        for rejected in [
            "hidepid=20",
            "x-hidepid=2",
            "hidepid=1",
            "hidepid=invisible-extra",
        ] {
            assert!(!has_hidepid(rejected));
        }
        assert_eq!(DEFAULT_PROC_MOUNTS, "/proc/1/mounts");
    }

    #[test]
    fn test_helper_failure_and_unverified_success_are_reported() {
        let temp = tempdir().expect("tempdir");
        let mounts = temp.path().join("mounts");
        fs::write(&mounts, "proc /proc proc rw,relatime 0 0\n").unwrap();
        for failure in ["enable", "restart", "verification"] {
            let runner = Arc::new(MockSystemRunner::new());
            if failure != "verification" {
                runner.set_response(
                    &format!("systemctl {failure} {PROC_HARDENING_UNIT}"),
                    1,
                    b"",
                    b"permission denied or unit unavailable",
                );
            }
            let manager = HardeningManager::with_paths(
                Arc::clone(&runner) as Arc<dyn SystemRunner>,
                temp.path(),
                &mounts,
                temp.path().join("group"),
            );
            let error = manager
                .apply(HardeningApplyRequest {
                    enable_hidepid: true,
                    ensure_tenant_group: false,
                })
                .expect_err("must not claim host hardening succeeded");
            assert!(error.to_string().contains("panel-proc-hardening.service"));
            assert!(
                !runner
                    .get_commands()
                    .iter()
                    .any(|command| command.starts_with("mount "))
            );
        }
    }

    #[test]
    fn test_existing_host_policy_is_enabled_for_boot_without_remount() {
        let temp = tempdir().unwrap();
        let mounts = temp.path().join("mounts");
        fs::write(&mounts, "proc /proc proc rw,hidepid=invisible 0 0\n").unwrap();
        let runner = Arc::new(MockSystemRunner::new());
        let manager = HardeningManager::with_paths(
            Arc::clone(&runner) as Arc<dyn SystemRunner>,
            temp.path(),
            &mounts,
            temp.path().join("group"),
        );
        let result = manager
            .apply(HardeningApplyRequest {
                enable_hidepid: true,
                ensure_tenant_group: false,
            })
            .unwrap();
        assert!(!result.hidepid_applied);
        assert_eq!(
            runner.get_commands(),
            vec![format!("systemctl enable {PROC_HARDENING_UNIT}")]
        );
    }

    #[test]
    fn test_check_all_aggregation() {
        let temp = tempdir().expect("tempdir");
        let runner = Arc::new(MockSystemRunner::new());
        let mounts_file = temp.path().join("mounts");
        let group_file = temp.path().join("group");

        fs::write(
            temp.path().join("cgroup.controllers"),
            "cpu memory io pids\n",
        )
        .expect("write controllers");
        fs::write(&mounts_file, "proc /proc proc rw,relatime,hidepid=2 0 0\n")
            .expect("write mounts");
        fs::write(&group_file, "panel-tenants:x:1001:\n").expect("write group");

        let manager = HardeningManager::with_paths(
            Arc::clone(&runner) as Arc<dyn SystemRunner>,
            temp.path(),
            &mounts_file,
            &group_file,
        );

        let status = manager.check_all();
        assert!(status.overall_ok);
        assert!(status.cgroup_v2.ok);
        assert!(status.acl.ok);
        assert!(status.quota.ok);
        assert!(status.hidepid.ok);
        assert!(status.tenant_group.ok);
    }
}
