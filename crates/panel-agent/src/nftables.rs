//! Nftables per-UID loopback firewall isolation manager.
//!
//! Enforces network isolation across tenants by preventing cross-tenant
//! loopback connections on tenant application port ranges (20000..=29999).
//! Only the site owner's UID, Nginx reverse proxy (www-data: uid 33), and root (uid 0)
//! can access a tenant application port on 127.0.0.1.

use crate::AgentError;
use crate::system::SystemRunner;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

pub const DEFAULT_NFTABLES_CONF_PATH: &str = "/etc/nftables.d/panel-loopback-guard.nft";
pub const DEFAULT_TABLE_NAME: &str = "panel_loopback_guard";
pub const TENANT_PORT_RANGE_START: u16 = 20000;
pub const TENANT_PORT_RANGE_END: u16 = 29999;
pub const NGINX_SYSTEM_UID: u32 = 33; // standard www-data UID on Debian/Ubuntu
pub const ROOT_SYSTEM_UID: u32 = 0;

/// Mapping of a tenant application loopback port to its owner UID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantPortBinding {
    pub site_id: u64,
    pub tenant_uid: u32,
    pub port: u16,
}

/// Status of the nftables loopback isolation firewall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NftablesStatus {
    pub enabled: bool,
    pub table_name: String,
    pub active_rules_count: usize,
    pub protected_ports_count: usize,
    pub last_synced_at: u64,
}

pub struct NftablesManager {
    runner: Arc<dyn SystemRunner>,
    rules_path: PathBuf,
    table_name: String,
}

impl NftablesManager {
    pub fn new(runner: Arc<dyn SystemRunner>) -> Self {
        Self {
            runner,
            rules_path: PathBuf::from(DEFAULT_NFTABLES_CONF_PATH),
            table_name: DEFAULT_TABLE_NAME.to_string(),
        }
    }

    pub fn with_path(runner: Arc<dyn SystemRunner>, rules_path: impl AsRef<Path>) -> Self {
        Self {
            runner,
            rules_path: rules_path.as_ref().to_path_buf(),
            table_name: DEFAULT_TABLE_NAME.to_string(),
        }
    }

    /// Render nftables configuration file content from tenant port bindings.
    pub fn render_rules(&self, bindings: &[TenantPortBinding]) -> String {
        let mut out = String::new();
        out.push_str("#!/usr/sbin/nft -f\n");
        out.push_str("# QonPanel Loopback Per-UID Isolation Guard (P10-T4)\n");
        out.push_str("# Prevents tenant B from connecting to tenant A's local loopback port.\n\n");

        // Both commands are in the same nft transaction as the replacement.
        // Reapplying must remove stale UID grants instead of appending rules.
        out.push_str(&format!(
            "add table inet {}\nflush table inet {}\n",
            self.table_name, self.table_name
        ));
        out.push_str(&format!("table inet {} {{\n", self.table_name));
        let mut sorted_bindings = bindings.to_vec();
        sorted_bindings.sort_by_key(|b| b.port);
        out.push_str("    chain tenant_source {\n");
        for b in &sorted_bindings {
            out.push_str(&format!(
                "        tcp sport {} meta skuid {} return;\n",
                b.port, b.tenant_uid
            ));
        }
        out.push_str("        drop;\n    }\n");
        out.push_str("    chain output {\n");
        out.push_str("        type filter hook output priority 0; policy accept;\n\n");
        // Check the responding socket before the privileged Nginx/root client
        // exceptions. Returning from the subchain still checks destination UID.
        out.push_str(&format!(
            "        oif \"lo\" tcp sport {}-{} jump tenant_source;\n",
            TENANT_PORT_RANGE_START, TENANT_PORT_RANGE_END
        ));
        out.push_str("        # Allow root (uid 0) and Nginx reverse proxy (www-data: uid 33) to access all tenant apps\n");
        out.push_str(&format!(
            "        oif \"lo\" tcp dport {}-{} meta skuid {{ {}, {} }} accept;\n\n",
            TENANT_PORT_RANGE_START, TENANT_PORT_RANGE_END, ROOT_SYSTEM_UID, NGINX_SYSTEM_UID
        ));

        out.push_str("        # Specific tenant application ports: allow only site owner UID\n");
        for b in &sorted_bindings {
            out.push_str(&format!(
                "        # site_{} port {}\n        oif \"lo\" tcp dport {} meta skuid {} accept;\n",
                b.site_id, b.port, b.port, b.tenant_uid
            ));
        }

        out.push_str("\n        # Reject cross-tenant port access on loopback with TCP reset\n");
        out.push_str(&format!(
            "        oif \"lo\" tcp dport {}-{} reject with tcp reset;\n",
            TENANT_PORT_RANGE_START, TENANT_PORT_RANGE_END
        ));

        out.push_str("    }\n");
        out.push_str("}\n");

        out
    }

    /// Validate syntax of nftables configuration using `nft -c -f`.
    pub fn validate_rules(&self, rules: &str) -> Result<(), AgentError> {
        // Create parent directory if needed
        if let Some(parent) = self.rules_path.parent() {
            self.runner.create_dir_all(parent, 0o750, 0, 0)?;
        }

        let temp_check_path = self.rules_path.with_extension("tmp");
        self.runner
            .write_file(&temp_check_path, rules.as_bytes(), 0o640)?;

        let temp_path_str = temp_check_path.to_string_lossy().to_string();
        let check_res = self
            .runner
            .run_command("nft", &["-c", "-f", &temp_path_str]);

        let _ = self.runner.remove_file(&temp_check_path);

        match check_res {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                warn!(target: "panel_agent::nftables", "nftables syntax check failed: {}", err_msg);
                Err(AgentError::CommandFailed {
                    binary: format!("nft -c -f {}", temp_path_str),
                    code: output.status.code(),
                    stderr: err_msg.to_string(),
                })
            }
            Err(e) => {
                warn!(target: "panel_agent::nftables", "Failed to invoke nft binary: {}", e);
                Err(e)
            }
        }
    }

    /// Synchronize tenant port bindings and apply nftables configuration.
    pub fn sync_bindings(
        &self,
        bindings: &[TenantPortBinding],
    ) -> Result<NftablesStatus, AgentError> {
        let mut ports = std::collections::HashSet::new();
        for binding in bindings {
            panel_core::tenant::TenantUid::parse(binding.tenant_uid)
                .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
            if !(TENANT_PORT_RANGE_START..=TENANT_PORT_RANGE_END).contains(&binding.port)
                || !ports.insert(binding.port)
            {
                return Err(AgentError::InvalidInput(
                    "Pemetaan port tenant harus unik dan dalam rentang terisolasi".into(),
                ));
            }
        }
        let rules = self.render_rules(bindings);
        self.validate_rules(&rules)?;

        if let Some(parent) = self.rules_path.parent() {
            self.runner.create_dir_all(parent, 0o750, 0, 0)?;
        }

        self.runner
            .write_file(&self.rules_path, rules.as_bytes(), 0o640)?;

        let rules_path_str = self.rules_path.to_string_lossy().to_string();
        let apply_res = self.runner.run_command("nft", &["-f", &rules_path_str]);

        match apply_res {
            Ok(output) if output.status.success() => {
                info!(
                    target: "panel_agent::nftables",
                    "Applied nftables loopback guard with {} bindings",
                    bindings.len()
                );
                let epoch = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                Ok(NftablesStatus {
                    enabled: true,
                    table_name: self.table_name.clone(),
                    active_rules_count: 2 * bindings.len() + 4,
                    protected_ports_count: bindings.len(),
                    last_synced_at: epoch,
                })
            }
            Ok(output) => {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                Err(AgentError::CommandFailed {
                    binary: format!("nft -f {}", rules_path_str),
                    code: output.status.code(),
                    stderr: err_msg.to_string(),
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Inspect current status of the nftables loopback guard.
    pub fn get_status(&self, bindings_count: usize) -> NftablesStatus {
        let exists = self.runner.path_exists(&self.rules_path);
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        NftablesStatus {
            enabled: exists,
            table_name: self.table_name.clone(),
            active_rules_count: if exists { 2 * bindings_count + 4 } else { 0 },
            protected_ports_count: bindings_count,
            last_synced_at: epoch,
        }
    }

    /// Flush and disable loopback isolation rules.
    pub fn flush_rules(&self) -> Result<(), AgentError> {
        let cmd = format!("delete table inet {}", self.table_name);
        let _ = self.runner.run_command("nft", &[&cmd]);
        if self.runner.path_exists(&self.rules_path) {
            let _ = self.runner.remove_file(&self.rules_path);
        }
        Ok(())
    }

    /// Synchronize bindings from proto request.
    pub fn sync_from_proto(
        &self,
        req: &panel_proto::NftablesSyncRequest,
    ) -> Result<panel_proto::NftablesSyncResponse, AgentError> {
        let bindings: Vec<TenantPortBinding> = req
            .bindings
            .iter()
            .map(|b| TenantPortBinding {
                site_id: b.site_id,
                tenant_uid: b.tenant_uid,
                port: b.port,
            })
            .collect();
        let status = self.sync_bindings(&bindings)?;
        Ok(panel_proto::NftablesSyncResponse {
            success: status.enabled,
            rules_count: status.active_rules_count,
            message: format!(
                "Synced {} loopback isolation rules",
                status.active_rules_count
            ),
        })
    }

    /// Retrieve status from proto request.
    pub fn get_status_proto(
        &self,
        _req: &panel_proto::NftablesStatusRequest,
    ) -> panel_proto::NftablesStatusResponse {
        let status = self.get_status(0);
        panel_proto::NftablesStatusResponse {
            status: panel_proto::NftablesStatusDto {
                enabled: status.enabled,
                table_name: status.table_name,
                active_rules_count: status.active_rules_count,
                protected_ports_count: status.protected_ports_count,
                last_synced_at: status.last_synced_at,
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;

    #[test]
    fn test_render_nftables_rules_content() {
        let runner = Arc::new(MockSystemRunner::new());
        let mgr = NftablesManager::new(runner);

        let bindings = vec![
            TenantPortBinding {
                site_id: 1,
                tenant_uid: 10001,
                port: 20001,
            },
            TenantPortBinding {
                site_id: 2,
                tenant_uid: 10002,
                port: 20002,
            },
        ];

        let rules = mgr.render_rules(&bindings);

        assert!(rules.contains("table inet panel_loopback_guard"));
        assert!(rules.contains("chain output"));
        assert!(rules.contains("meta skuid { 0, 33 } accept"));
        assert!(rules.contains("oif \"lo\" tcp dport 20001 meta skuid 10001 accept"));
        assert!(rules.contains("oif \"lo\" tcp dport 20002 meta skuid 10002 accept"));
        assert!(rules.contains(
            "add table inet panel_loopback_guard\nflush table inet panel_loopback_guard"
        ));
        assert!(rules.contains("tcp sport 20001 meta skuid 10001 return;"));
        assert!(rules.contains("tcp sport 20002 meta skuid 10002 return;"));
        assert!(rules.contains("chain tenant_source {"));
        assert!(rules.contains("        drop;\n    }"));
        assert!(
            rules.find("jump tenant_source").unwrap()
                < rules.find("meta skuid { 0, 33 } accept").unwrap()
        );
        assert!(!rules.contains("meta skuid 10001 accept;\n    }"));
        assert!(rules.contains("reject with tcp reset"));
    }

    #[test]
    fn test_sync_bindings_mock_success() {
        let runner = Arc::new(MockSystemRunner::new());
        runner.set_response("nft", 0, b"", b"");

        let conf_path = PathBuf::from("/tmp/nftables.nft");
        let mgr = NftablesManager::with_path(runner.clone(), &conf_path);

        let bindings = vec![TenantPortBinding {
            site_id: 10,
            tenant_uid: 10005,
            port: 20050,
        }];

        let status = mgr.sync_bindings(&bindings);
        assert!(status.is_ok());
        let st = status.unwrap();
        assert!(st.enabled);
        assert_eq!(st.protected_ports_count, 1);
        assert_eq!(st.active_rules_count, 6);
    }

    #[test]
    fn contradictory_or_privileged_bindings_fail_before_writing_rules() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = NftablesManager::new(runner.clone());
        let valid = TenantPortBinding {
            site_id: 1,
            tenant_uid: 10001,
            port: 20050,
        };
        for invalid in [
            TenantPortBinding {
                tenant_uid: 0,
                ..valid.clone()
            },
            TenantPortBinding {
                port: 7700,
                ..valid.clone()
            },
        ] {
            assert!(manager.sync_bindings(&[invalid]).is_err());
        }
        assert!(
            manager
                .sync_bindings(&[
                    valid.clone(),
                    TenantPortBinding {
                        site_id: 2,
                        tenant_uid: 10002,
                        ..valid
                    }
                ])
                .is_err()
        );
        assert!(runner.recorded_actions().is_empty());
    }
}
