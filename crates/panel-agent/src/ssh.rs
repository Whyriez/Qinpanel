//! SshManager: Manages host-level root-owned authorized_keys files,
//! sshd drop-in configuration (/etc/ssh/sshd_config.d/50-panel.conf),
//! and tenant shell policy enforcement (none | sftp | full).
//!
//! Rule 4.5 / P6-T5: "file dikelola panel di lokasi root-owned"
//! Location: /etc/panel/ssh/authorized_keys/<linux_username> (mode 0644, root:root)
//!
//! Rule 5.4 / P6-T6: "Kebijakan shell: none | sftp | full per plan/tenant;
//! drop-in sshd untuk grup panel-tenants (SFTP-only via internal-sftp + chroot);
//! validasi sshd -t sebelum reload; uji bahwa akses SSH admin tidak pernah terkunci."

use crate::AgentError;
use crate::system::SystemRunner;
use panel_core::{ShellPolicy, TenantUid, Username};
use panel_proto::{
    SshDropinApplyRequest, SshDropinApplyResponse, SshKeysSyncRequest, SshKeysSyncResponse,
    SshPolicyApplyRequest, SshPolicyApplyResponse,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

pub const DEFAULT_SSH_KEYS_DIR: &str = "/etc/panel/ssh/authorized_keys";
pub const DEFAULT_SSHD_DROPIN_PATH: &str = "/etc/ssh/sshd_config.d/50-panel.conf";

/// Secure default OpenSSH drop-in configuration.
/// Only matches `panel-tenants` and `panel-sftp`.
/// Non-tenant users (e.g. admin, root) are never matched and retain unrestricted access.
pub const DEFAULT_SSHD_DROPIN_CONTENT: &str = r#"# ==============================================================================
# QonPanel OpenSSH Drop-in Configuration
# File: /etc/ssh/sshd_config.d/50-panel.conf
# Managed by panel-agent. DO NOT EDIT DIRECTLY.
# ==============================================================================

Subsystem sftp internal-sftp

# Tenant group confinement - applies exclusively to members of panel-tenants
Match Group panel-tenants
    AuthorizedKeysFile /etc/panel/ssh/authorized_keys/%u
    X11Forwarding no
    AllowTcpForwarding no
    AllowAgentForwarding no

# SFTP-only confinement - restricts to internal-sftp, chroots to /home, disables PTY
Match Group panel-sftp
    ForceCommand internal-sftp
    ChrootDirectory /home
    PermitTTY no
    X11Forwarding no
    AllowTcpForwarding no
    AllowAgentForwarding no
"#;

#[derive(Clone)]
pub struct SshManager {
    runner: Arc<dyn SystemRunner>,
    base_dir: PathBuf,
    dropin_path: PathBuf,
}

impl SshManager {
    /// No caller-controlled paths, no private key reads, no daemon/reload or scan.
    /// Only keys named by the effective default sshd configuration are shown.
    pub fn host_key_fingerprints(
        &self,
    ) -> Result<Vec<panel_core::ssh::SshHostKeyFingerprint>, AgentError> {
        let output = self.runner.run_command("sshd", &["-T"])?;
        if !output.status.success() || output.stdout.len() > 256 * 1024 {
            return Err(AgentError::Internal(
                "Identitas OpenSSH tidak tersedia.".into(),
            ));
        }
        let config = String::from_utf8(output.stdout)
            .map_err(|_| AgentError::Internal("Konfigurasi OpenSSH tidak valid.".into()))?;
        let mut keys = Vec::new();
        for (private, public, algorithm) in [
            (
                "/etc/ssh/ssh_host_ed25519_key",
                "/etc/ssh/ssh_host_ed25519_key.pub",
                "ssh-ed25519",
            ),
            (
                "/etc/ssh/ssh_host_ecdsa_key",
                "/etc/ssh/ssh_host_ecdsa_key.pub",
                "ecdsa-sha2-nistp256",
            ),
            (
                "/etc/ssh/ssh_host_rsa_key",
                "/etc/ssh/ssh_host_rsa_key.pub",
                "ssh-rsa",
            ),
        ] {
            if !config.lines().any(|line| {
                let mut tokens = line.split_whitespace();
                tokens.next() == Some("hostkey")
                    && tokens.next() == Some(private)
                    && tokens.next().is_none()
            }) {
                continue;
            }
            let Ok(bytes) = self.runner.read_file(Path::new(public)) else {
                continue;
            };
            if bytes.len() > 16 * 1024 {
                continue;
            }
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let Ok(key) = panel_core::ssh::parse_ssh_public_key(text) else {
                continue;
            };
            if key.key_type != algorithm {
                continue;
            }
            keys.push(panel_core::ssh::SshHostKeyFingerprint {
                key_type: key.key_type,
                fingerprint_sha256: key.fingerprint_sha256,
            });
        }
        Ok(keys)
    }

    pub fn new(runner: Arc<dyn SystemRunner>) -> Self {
        Self {
            runner,
            base_dir: PathBuf::from(DEFAULT_SSH_KEYS_DIR),
            dropin_path: PathBuf::from(DEFAULT_SSHD_DROPIN_PATH),
        }
    }

    pub fn with_base_dir(runner: Arc<dyn SystemRunner>, base_dir: impl Into<PathBuf>) -> Self {
        Self {
            runner,
            base_dir: base_dir.into(),
            dropin_path: PathBuf::from(DEFAULT_SSHD_DROPIN_PATH),
        }
    }

    pub fn with_paths(
        runner: Arc<dyn SystemRunner>,
        base_dir: impl Into<PathBuf>,
        dropin_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            runner,
            base_dir: base_dir.into(),
            dropin_path: dropin_path.into(),
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn dropin_path(&self) -> &Path {
        &self.dropin_path
    }

    /// Atomically syncs authorized_keys for a given tenant linux user.
    pub fn sync_authorized_keys(
        &self,
        req: &SshKeysSyncRequest,
    ) -> Result<SshKeysSyncResponse, AgentError> {
        // Validate linux username strictly
        let validated_user = Username::parse(&req.linux_username)
            .map_err(|e| AgentError::InvalidInput(format!("Invalid linux username: {}", e)))?;
        let username_str = validated_user.as_str();

        // Validate that keys do not contain internal newline characters
        for key in &req.keys {
            if key.contains('\n') || key.contains('\r') {
                return Err(AgentError::InvalidInput(
                    "SSH key entry contains newline characters".to_string(),
                ));
            }
        }

        // Ensure base directory exists
        if !self.base_dir.exists() {
            fs::create_dir_all(&self.base_dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&self.base_dir, fs::Permissions::from_mode(0o755));
            }
        }

        let target_path = self.base_dir.join(username_str);
        let temp_path =
            self.base_dir
                .join(format!("{}.tmp.{}", username_str, uuid::Uuid::new_v4()));

        // Write content atomically: temp file -> sync -> set permissions -> rename
        {
            let mut file = fs::File::create(&temp_path)?;
            for key in &req.keys {
                let trimmed = key.trim();
                if !trimmed.is_empty() {
                    writeln!(file, "{}", trimmed)?;
                }
            }
            file.flush()?;
            file.sync_all()?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o644));
        }

        fs::rename(&temp_path, &target_path)?;

        info!(
            "Synchronized {} SSH public key(s) to root-owned file {} for tenant {}",
            req.keys.len(),
            target_path.display(),
            username_str
        );

        Ok(SshKeysSyncResponse {
            success: true,
            path: target_path.display().to_string(),
            count: req.keys.len(),
        })
    }

    /// Validates OpenSSH daemon configuration syntax via `sshd -t`.
    pub fn validate_sshd_config(&self, config_file: Option<&Path>) -> Result<(), AgentError> {
        let path_str;
        let args: Vec<&str> = if let Some(path) = config_file {
            path_str = path.to_string_lossy().to_string();
            vec!["-t", "-f", &path_str]
        } else {
            vec!["-t"]
        };

        match self.runner.run_command("sshd", &args) {
            Ok(output) => {
                if output.status.success() {
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    Err(AgentError::InvalidInput(format!(
                        "sshd -t syntax validation failed: {}",
                        stderr.trim()
                    )))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Applies the OpenSSH drop-in configuration safely:
    /// writes to temporary file -> validates syntax with `sshd -t` -> atomic rename -> reloads ssh service.
    /// If validation fails, changes are discarded to ensure admin SSH access is never locked out.
    pub fn apply_sshd_dropin(
        &self,
        req: &SshDropinApplyRequest,
    ) -> Result<SshDropinApplyResponse, AgentError> {
        let content = req
            .custom_content
            .as_deref()
            .unwrap_or(DEFAULT_SSHD_DROPIN_CONTENT);

        // Ensure parent directory (/etc/ssh/sshd_config.d) exists
        if let Some(parent) = self.dropin_path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o755));
            }
        }

        let temp_path = if let Some(parent) = self.dropin_path.parent() {
            parent.join(format!("50-panel.conf.tmp.{}", uuid::Uuid::new_v4()))
        } else {
            PathBuf::from(format!("50-panel.conf.tmp.{}", uuid::Uuid::new_v4()))
        };

        // 1. Write content to temp file
        {
            let mut file = fs::File::create(&temp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
            file.sync_all()?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o644));
        }

        // 2. Validate syntax with `sshd -t` BEFORE replacing the active drop-in
        if let Err(e) = self.validate_sshd_config(Some(&temp_path)) {
            let _ = fs::remove_file(&temp_path);
            return Err(AgentError::SecurityViolation(format!(
                "Validasi sintaks sshd -t gagal. Perubahan dibatalkan untuk mencegah penguncian akses SSH: {}",
                e
            )));
        }

        // 3. Move atomically
        fs::rename(&temp_path, &self.dropin_path)?;

        // 4. Ensure system supplemental groups exist
        let _ = self
            .runner
            .run_command("groupadd", &["--system", "panel-tenants"]);
        let _ = self
            .runner
            .run_command("groupadd", &["--system", "panel-sftp"]);

        // 5. Reload ssh service safely
        let reload_res = self.runner.run_command("systemctl", &["reload", "ssh"]);
        let reloaded =
            reload_res.is_ok() && reload_res.map(|o| o.status.success()).unwrap_or(false);

        info!(
            "Applied sshd drop-in configuration to {} (reloaded: {})",
            self.dropin_path.display(),
            reloaded
        );

        Ok(SshDropinApplyResponse {
            success: true,
            path: self.dropin_path.display().to_string(),
            validated: true,
            reloaded,
            message: "Konfigurasi drop-in sshd berhasil diterapkan dan divalidasi".to_string(),
        })
    }

    /// Applies the shell policy (`none`, `sftp`, `full`) to a Linux tenant user.
    pub fn apply_user_shell_policy(
        &self,
        req: &SshPolicyApplyRequest,
    ) -> Result<SshPolicyApplyResponse, AgentError> {
        let validated_user = Username::parse(&req.linux_username)
            .map_err(|e| AgentError::InvalidInput(format!("Invalid linux username: {}", e)))?;
        let _uid = TenantUid::parse(req.uid)
            .map_err(|e| AgentError::InvalidInput(format!("Invalid UID: {}", e)))?;
        let username_str = validated_user.as_str();

        let policy = ShellPolicy::parse(&req.policy)
            .map_err(|e| AgentError::InvalidInput(format!("Invalid shell policy: {}", e)))?;

        let shell_path = policy.as_shell_path();

        let sftp_group_member = match policy {
            ShellPolicy::None => {
                // Shell is /usr/sbin/nologin; strip from panel-sftp, keep panel-tenants
                self.runner.run_command(
                    "usermod",
                    &["-s", shell_path, "-G", "panel-tenants", username_str],
                )?;
                false
            }
            ShellPolicy::Sftp => {
                // Ensure panel-sftp group exists
                let _ = self
                    .runner
                    .run_command("groupadd", &["--system", "panel-sftp"]);
                // Shell is /usr/lib/openssh/sftp-server; add to panel-sftp and panel-tenants
                self.runner.run_command(
                    "usermod",
                    &[
                        "-s",
                        shell_path,
                        "-G",
                        "panel-tenants,panel-sftp",
                        username_str,
                    ],
                )?;
                true
            }
            ShellPolicy::Full => {
                // Shell is /bin/bash; keep only in panel-tenants (stripped from panel-sftp)
                self.runner.run_command(
                    "usermod",
                    &["-s", shell_path, "-G", "panel-tenants", username_str],
                )?;
                false
            }
        };

        info!(
            "Applied shell policy '{}' (shell: {}, sftp_group: {}) for tenant {}",
            policy, shell_path, sftp_group_member, username_str
        );

        Ok(SshPolicyApplyResponse {
            success: true,
            shell: shell_path.to_string(),
            sftp_group_member,
            message: format!("Kebijakan shell '{}' berhasil diterapkan", policy),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod host_identity_tests {
    use super::*;
    use crate::system::MockSystemRunner;
    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl fixture";

    #[test]
    fn configured_public_host_identity_is_read_only() {
        let runner = Arc::new(MockSystemRunner::new());
        runner.set_response(
            "sshd -T",
            0,
            b"port 2222\nhostkey /etc/ssh/ssh_host_ed25519_key\n",
            b"",
        );
        runner.set_file_content("/etc/ssh/ssh_host_ed25519_key.pub", KEY.as_bytes());
        let manager = SshManager::new(runner.clone());
        let keys = manager.host_key_fingerprints().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(
            keys[0].fingerprint_sha256,
            panel_core::ssh::parse_ssh_public_key(KEY)
                .unwrap()
                .fingerprint_sha256
        );
        assert_eq!(runner.get_commands(), vec!["sshd -T"]);
        assert!(
            runner
                .recorded_actions()
                .iter()
                .all(|a| matches!(a, crate::system::MockAction::Command { .. }))
        );
    }

    #[test]
    fn custom_unconfigured_missing_and_malformed_keys_are_not_advertised() {
        let runner = Arc::new(MockSystemRunner::new());
        runner.set_file_content("/etc/ssh/ssh_host_ed25519_key.pub", KEY.as_bytes());
        runner.set_response(
            "sshd -T",
            0,
            b"hostkey /arbitrary/private\nhostkey /etc/ssh/ssh_host_rsa_key\n",
            b"",
        );
        let manager = SshManager::new(runner.clone());
        assert!(manager.host_key_fingerprints().unwrap().is_empty());
        runner.set_response(
            "sshd -T",
            0,
            b"hostkey /etc/ssh/ssh_host_ed25519_key\n",
            b"",
        );
        runner.set_file_content("/etc/ssh/ssh_host_ed25519_key.pub", b"invalid key");
        assert!(manager.host_key_fingerprints().unwrap().is_empty());
        runner.set_response(
            "sshd -T",
            1,
            b"hostkey /etc/ssh/ssh_host_ed25519_key\n",
            b"bad config",
        );
        assert!(manager.host_key_fingerprints().is_err());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    use tempfile::tempdir;

    #[test]
    fn test_sync_authorized_keys_success() {
        let tmp = tempdir().expect("tempdir");
        let runner = Arc::new(MockSystemRunner::new());
        let manager = SshManager::with_base_dir(runner, tmp.path());

        let req = SshKeysSyncRequest {
            linux_username: "p_alice".to_string(),
            keys: vec![
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl key1".to_string(),
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl key2".to_string(),
            ],
        };

        let res = manager.sync_authorized_keys(&req).expect("sync keys");
        assert!(res.success);
        assert_eq!(res.count, 2);

        let target_file = tmp.path().join("p_alice");
        assert!(target_file.exists());
        let content = fs::read_to_string(&target_file).expect("read keys");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_sync_empty_keys_clears_file() {
        let tmp = tempdir().expect("tempdir");
        let runner = Arc::new(MockSystemRunner::new());
        let manager = SshManager::with_base_dir(runner, tmp.path());

        let req = SshKeysSyncRequest {
            linux_username: "p_bob".to_string(),
            keys: vec![],
        };

        let res = manager.sync_authorized_keys(&req).expect("sync empty");
        assert!(res.success);
        assert_eq!(res.count, 0);

        let target_file = tmp.path().join("p_bob");
        assert!(target_file.exists());
        let content = fs::read_to_string(&target_file).expect("read keys");
        assert!(content.trim().is_empty());
    }

    #[test]
    fn test_reject_invalid_username_or_path_traversal() {
        let tmp = tempdir().expect("tempdir");
        let runner = Arc::new(MockSystemRunner::new());
        let manager = SshManager::with_base_dir(runner, tmp.path());

        let req_traversal = SshKeysSyncRequest {
            linux_username: "../etc/shadow".to_string(),
            keys: vec!["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI... key".to_string()],
        };
        assert!(manager.sync_authorized_keys(&req_traversal).is_err());

        let req_invalid_prefix = SshKeysSyncRequest {
            linux_username: "alice".to_string(),
            keys: vec!["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI... key".to_string()],
        };
        assert!(manager.sync_authorized_keys(&req_invalid_prefix).is_err());
    }

    #[test]
    fn test_reject_newline_injection() {
        let tmp = tempdir().expect("tempdir");
        let runner = Arc::new(MockSystemRunner::new());
        let manager = SshManager::with_base_dir(runner, tmp.path());

        let req_injection = SshKeysSyncRequest {
            linux_username: "p_charlie".to_string(),
            keys: vec![
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...\ncommand=\"rm -rf /\" ssh-rsa ..."
                    .to_string(),
            ],
        };
        assert!(manager.sync_authorized_keys(&req_injection).is_err());
    }

    #[test]
    fn test_apply_sshd_dropin_success_with_validation_and_reload() {
        let tmp_base = tempdir().expect("tempdir");
        let tmp_dropin_dir = tempdir().expect("tempdir");
        let dropin_file = tmp_dropin_dir.path().join("50-panel.conf");

        let runner = Arc::new(MockSystemRunner::new());
        runner.set_response("sshd", 0, b"syntax is ok", b"");
        runner.set_response("groupadd", 0, b"", b"");
        runner.set_response("systemctl", 0, b"", b"");

        let manager = SshManager::with_paths(runner.clone(), tmp_base.path(), &dropin_file);

        let req = SshDropinApplyRequest::default();
        let res = manager.apply_sshd_dropin(&req).expect("apply dropin");

        assert!(res.success);
        assert!(res.validated);
        assert!(res.reloaded);
        assert!(dropin_file.exists());

        let content = fs::read_to_string(&dropin_file).expect("read dropin");
        assert!(content.contains("Match Group panel-tenants"));
        assert!(content.contains("Match Group panel-sftp"));
        assert!(content.contains("AuthorizedKeysFile /etc/panel/ssh/authorized_keys/%u"));
        assert!(content.contains("ForceCommand internal-sftp"));
    }

    #[test]
    fn test_apply_sshd_dropin_aborts_on_syntax_error_and_preserves_system() {
        let tmp_base = tempdir().expect("tempdir");
        let tmp_dropin_dir = tempdir().expect("tempdir");
        let dropin_file = tmp_dropin_dir.path().join("50-panel.conf");

        // Write an initial valid configuration
        fs::write(&dropin_file, b"# Initial valid dropin").expect("initial write");

        let runner = Arc::new(MockSystemRunner::new());
        // Configure sshd to fail syntax check
        runner.set_response(
            "sshd",
            1,
            b"",
            b"line 5: Bad configuration option: InvalidKeyword",
        );

        let manager = SshManager::with_paths(runner.clone(), tmp_base.path(), &dropin_file);

        let req = SshDropinApplyRequest {
            custom_content: Some("InvalidKeyword value".to_string()),
        };
        let err = manager.apply_sshd_dropin(&req).unwrap_err();

        assert!(err.to_string().contains("Validasi sintaks sshd -t gagal"));
        // Ensure initial file is NOT overwritten
        let current_content = fs::read_to_string(&dropin_file).expect("read");
        assert_eq!(current_content, "# Initial valid dropin");
    }

    #[test]
    fn test_admin_ssh_not_locked_out_when_tenant_dropin_applied() {
        // Explicit test verifying PRD requirement:
        // "uji bahwa akses SSH admin tidak pernah terkunci"
        //
        // 1. Configuration uses `Match Group panel-tenants` and `Match Group panel-sftp`.
        // 2. An admin user (root, ubuntu, sysadmin) belongs to groups like `sudo` or `wheel`,
        //    NOT `panel-tenants` or `panel-sftp`.
        // 3. OpenSSH evaluates rules sequentially and only applies Match blocks when the group matches.
        let content = DEFAULT_SSHD_DROPIN_CONTENT;

        // Verify structure: no global restrictions outside Match blocks
        let lines_before_match: Vec<&str> = content
            .lines()
            .take_while(|l| !l.starts_with("Match Group panel-tenants"))
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .collect();

        // The only directive before Match is Subsystem definition
        for line in &lines_before_match {
            assert!(
                line.starts_with("Subsystem sftp"),
                "Only subsystem configuration allowed globally: {}",
                line
            );
        }

        // Verify Match blocks are strictly scoped
        assert!(content.contains("Match Group panel-tenants"));
        assert!(content.contains("Match Group panel-sftp"));

        // Admin does NOT have their AuthorizedKeysFile overridden
        // because AuthorizedKeysFile is indented under `Match Group panel-tenants`
        let lines: Vec<&str> = content.lines().collect();
        let auth_key_idx = lines
            .iter()
            .position(|l| l.contains("AuthorizedKeysFile"))
            .expect("has AuthorizedKeysFile");
        let prev_line = lines[auth_key_idx - 1];
        assert!(
            prev_line.contains("Match Group panel-tenants"),
            "AuthorizedKeysFile must be scoped strictly under Match Group panel-tenants"
        );
    }

    #[test]
    fn test_shell_policy_transitions() {
        let runner = Arc::new(MockSystemRunner::new());
        runner.set_response("usermod", 0, b"", b"");
        runner.set_response("groupadd", 0, b"", b"");

        let manager = SshManager::new(runner.clone());

        // 1. None
        let req_none = SshPolicyApplyRequest {
            linux_username: "p_tenant1".to_string(),
            uid: 10001,
            policy: "none".to_string(),
        };
        let res_none = manager
            .apply_user_shell_policy(&req_none)
            .expect("apply none");
        assert_eq!(res_none.shell, "/usr/sbin/nologin");
        assert!(!res_none.sftp_group_member);

        // 2. Sftp
        let req_sftp = SshPolicyApplyRequest {
            linux_username: "p_tenant1".to_string(),
            uid: 10001,
            policy: "sftp".to_string(),
        };
        let res_sftp = manager
            .apply_user_shell_policy(&req_sftp)
            .expect("apply sftp");
        assert_eq!(res_sftp.shell, "/usr/lib/openssh/sftp-server");
        assert!(res_sftp.sftp_group_member);

        // 3. Full
        let req_full = SshPolicyApplyRequest {
            linux_username: "p_tenant1".to_string(),
            uid: 10001,
            policy: "full".to_string(),
        };
        let res_full = manager
            .apply_user_shell_policy(&req_full)
            .expect("apply full");
        assert_eq!(res_full.shell, "/bin/bash");
        assert!(!res_full.sftp_group_member);

        // Check runner actions
        let actions = runner.recorded_actions();
        assert!(actions.len() >= 4);
    }
}
