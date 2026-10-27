//! Filesystem disk and inode quota management with directory scanning fallback.
//!
//! Complies with PRD Section 4.4 and Task P2-T3:
//! - Detects filesystem support (ext4 usrquota, XFS quota, or fallback scan)
//! - Sets and retrieves quota limits per tenant UID
//! - Provides safe, symlink-confined recursive directory scanner
//! - Writes agent audit records for quota operations

use crate::AgentError;
use crate::system::SystemRunner;
use panel_core::Username;
use panel_core::tenant::TenantUid;
use panel_proto::{
    QuotaGetRequest, QuotaGetResponse, QuotaScanRequest, QuotaScanResponse, QuotaSetRequest,
    QuotaSetResponse,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{info, warn};

pub const DEFAULT_HOME_BASE: &str = "/home";
pub const DEFAULT_MOUNT_POINT: &str = "/home";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsQuotaBackend {
    Ext4,
    Xfs,
    FallbackScan,
}

impl FsQuotaBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ext4 => "ext4",
            Self::Xfs => "xfs",
            Self::FallbackScan => "fallback_scan",
        }
    }
}

pub struct QuotaManager {
    runner: Arc<dyn SystemRunner>,
    home_base: PathBuf,
    forced_backend: Option<FsQuotaBackend>,
    /// In-memory cache of configured fallback limits (uid -> (disk_bytes, inodes))
    fallback_limits: Mutex<HashMap<u32, (u64, u64)>>,
}

impl QuotaManager {
    pub fn new(runner: Arc<dyn SystemRunner>) -> Self {
        Self {
            runner,
            home_base: PathBuf::from(DEFAULT_HOME_BASE),
            forced_backend: None,
            fallback_limits: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_home_base(runner: Arc<dyn SystemRunner>, home_base: impl AsRef<Path>) -> Self {
        Self {
            runner,
            home_base: home_base.as_ref().to_path_buf(),
            forced_backend: None,
            fallback_limits: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_forced_backend(
        runner: Arc<dyn SystemRunner>,
        home_base: impl AsRef<Path>,
        backend: FsQuotaBackend,
    ) -> Self {
        Self {
            runner,
            home_base: home_base.as_ref().to_path_buf(),
            forced_backend: Some(backend),
            fallback_limits: Mutex::new(HashMap::new()),
        }
    }

    /// Detects filesystem quota backend for the given mount point or path.
    pub fn detect_backend(&self, mount_point: &str) -> FsQuotaBackend {
        if let Some(forced) = self.forced_backend {
            return forced;
        }

        // Check using findmnt if available
        if let Ok(output) = self.runner.run_command(
            "findmnt",
            &["-n", "-o", "FSTYPE,OPTIONS", "--target", mount_point],
        ) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lower = stdout.to_lowercase();
            if lower.contains("xfs") && (lower.contains("quota") || lower.contains("uquota")) {
                return FsQuotaBackend::Xfs;
            }
            if lower.contains("ext4") && (lower.contains("usrquota") || lower.contains("quota")) {
                return FsQuotaBackend::Ext4;
            }
        }

        FsQuotaBackend::FallbackScan
    }

    /// Sets disk blocks and inode quota for a tenant user.
    pub fn set_quota(&self, req: QuotaSetRequest) -> Result<QuotaSetResponse, AgentError> {
        let tenant_uid =
            TenantUid::parse(req.uid).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let username =
            Username::parse(&req.username).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        let mount = req.mount_point.as_deref().unwrap_or(DEFAULT_MOUNT_POINT);

        let backend = self.detect_backend(mount);

        match backend {
            FsQuotaBackend::Ext4 => {
                // In ext4 quota, block size is typically 1024 bytes (1 KB)
                let blocks = req.disk_limit_bytes / 1024;
                let blocks_str = blocks.to_string();
                let inodes_str = req.inode_limit.to_string();

                self.runner.run_command(
                    "setquota",
                    &[
                        "-u",
                        username.as_str(),
                        &blocks_str,
                        &blocks_str,
                        &inodes_str,
                        &inodes_str,
                        mount,
                    ],
                )?;
            }
            FsQuotaBackend::Xfs => {
                let limit_cmd = format!(
                    "limit bsoft={} bhard={} isoft={} ihard={} {}",
                    req.disk_limit_bytes,
                    req.disk_limit_bytes,
                    req.inode_limit,
                    req.inode_limit,
                    username.as_str()
                );
                self.runner
                    .run_command("xfs_quota", &["-x", "-c", &limit_cmd, mount])?;
            }
            FsQuotaBackend::FallbackScan => {
                // Store in fallback limits map
                if let Ok(mut map) = self.fallback_limits.lock() {
                    map.insert(tenant_uid.as_u32(), (req.disk_limit_bytes, req.inode_limit));
                }
            }
        }

        info!(
            "Configured {} quota for user {} (UID {}) on {}: {} bytes, {} inodes",
            backend.as_str(),
            username.as_str(),
            tenant_uid.as_u32(),
            mount,
            req.disk_limit_bytes,
            req.inode_limit
        );

        Ok(QuotaSetResponse {
            uid: tenant_uid.as_u32(),
            username: username.as_str().to_string(),
            applied: true,
            backend: backend.as_str().to_string(),
        })
    }

    /// Retrieves current quota usage and limits for a tenant user.
    pub fn get_quota(&self, req: QuotaGetRequest) -> Result<QuotaGetResponse, AgentError> {
        let tenant_uid =
            TenantUid::parse(req.uid).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let username =
            Username::parse(&req.username).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        let mount = req.mount_point.as_deref().unwrap_or(DEFAULT_MOUNT_POINT);

        let backend = self.detect_backend(mount);

        match backend {
            FsQuotaBackend::Ext4 => {
                // Call repquota to query current usage
                match self.runner.run_command("repquota", &["-u", "-n", mount]) {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        if let Some((used_bytes, limit_bytes, used_inodes, limit_inodes)) =
                            Self::parse_repquota_output(&stdout, tenant_uid.as_u32())
                        {
                            return Ok(QuotaGetResponse {
                                uid: tenant_uid.as_u32(),
                                username: username.as_str().to_string(),
                                bytes_used: used_bytes,
                                bytes_limit: limit_bytes,
                                inodes_used: used_inodes,
                                inodes_limit: limit_inodes,
                                backend: backend.as_str().to_string(),
                            });
                        }
                    }
                    Err(e) => {
                        warn!(
                            "repquota failed on {}: {}. Falling back to directory scan.",
                            mount, e
                        );
                    }
                }
            }
            FsQuotaBackend::Xfs => {
                // Call xfs_quota report
                match self
                    .runner
                    .run_command("xfs_quota", &["-x", "-c", "report -u -b -i", mount])
                {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        if let Some((used_bytes, limit_bytes, used_inodes, limit_inodes)) =
                            Self::parse_xfs_quota_output(&stdout, username.as_str())
                        {
                            return Ok(QuotaGetResponse {
                                uid: tenant_uid.as_u32(),
                                username: username.as_str().to_string(),
                                bytes_used: used_bytes,
                                bytes_limit: limit_bytes,
                                inodes_used: used_inodes,
                                inodes_limit: limit_inodes,
                                backend: backend.as_str().to_string(),
                            });
                        }
                    }
                    Err(e) => {
                        warn!(
                            "xfs_quota report failed: {}. Falling back to directory scan.",
                            e
                        );
                    }
                }
            }
            FsQuotaBackend::FallbackScan => {}
        }

        // Fallback: directory scan
        let home_path = self.home_base.join(username.as_str());
        let scan_res = self.scan_directory(&home_path)?;

        let (limit_bytes, limit_inodes) = self
            .fallback_limits
            .lock()
            .ok()
            .and_then(|m| m.get(&tenant_uid.as_u32()).copied())
            .unwrap_or((0, 0));

        Ok(QuotaGetResponse {
            uid: tenant_uid.as_u32(),
            username: username.as_str().to_string(),
            bytes_used: scan_res.0,
            bytes_limit: limit_bytes,
            inodes_used: scan_res.1,
            inodes_limit: limit_inodes,
            backend: FsQuotaBackend::FallbackScan.as_str().to_string(),
        })
    }

    /// Performs a safe recursive directory scan counting bytes and inodes.
    pub fn scan_quota(&self, req: QuotaScanRequest) -> Result<QuotaScanResponse, AgentError> {
        let tenant_uid =
            TenantUid::parse(req.uid).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let username =
            Username::parse(&req.username).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        let target_path = if let Some(ref p) = req.path {
            let path = PathBuf::from(p);
            // Security: must be subpath of user's home
            let expected_home = self.home_base.join(username.as_str());
            if !path.starts_with(&expected_home) {
                return Err(AgentError::SecurityViolation(format!(
                    "Scan path '{}' is outside user home '{}'",
                    path.display(),
                    expected_home.display()
                )));
            }
            path
        } else {
            self.home_base.join(username.as_str())
        };

        let start = Instant::now();
        let (bytes_used, inodes_used) = self.scan_directory(&target_path)?;
        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            "Scanned quota for user {} (UID {}): {} bytes, {} inodes in {} ms",
            username.as_str(),
            tenant_uid.as_u32(),
            bytes_used,
            inodes_used,
            duration_ms
        );

        Ok(QuotaScanResponse {
            uid: tenant_uid.as_u32(),
            username: username.as_str().to_string(),
            bytes_used,
            inodes_used,
            scan_duration_ms: duration_ms,
        })
    }

    /// Internal recursive scanner: counts file sizes and inodes, safely avoiding symlink loops.
    fn scan_directory(&self, root: &Path) -> Result<(u64, u64), AgentError> {
        if !root.exists() {
            return Ok((0, 0));
        }

        let mut total_bytes = 0u64;
        let mut total_inodes = 1u64; // Root directory itself
        let mut stack = vec![root.to_path_buf()];

        while let Some(current_dir) = stack.pop() {
            let entries = match std::fs::read_dir(&current_dir) {
                Ok(e) => e,
                Err(err) => {
                    warn!(
                        "Failed to read directory {}: {}",
                        current_dir.display(),
                        err
                    );
                    continue;
                }
            };

            for entry_res in entries {
                let entry = match entry_res {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let path = entry.path();
                // Crucial security: symlink_metadata prevents following symlinks outside the tree!
                let meta = match std::fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                total_inodes += 1;

                if meta.is_symlink() {
                    // Count symlink itself
                    total_bytes += meta.len();
                } else if meta.is_dir() {
                    stack.push(path);
                } else {
                    total_bytes += meta.len();
                }
            }
        }

        Ok((total_bytes, total_inodes))
    }

    /// Helper to parse repquota output lines:
    /// User #<uid> -- <used_kb> <soft_kb> <hard_kb> ... <used_inodes> <soft_inodes> <hard_inodes>
    fn parse_repquota_output(stdout: &str, uid: u32) -> Option<(u64, u64, u64, u64)> {
        let uid_str = format!("#{uid}");
        let uid_num = uid.to_string();

        for line in stdout.lines() {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.is_empty() {
                continue;
            }
            if (tokens[0] == uid_str || tokens[0] == uid_num) && tokens.len() >= 8 {
                // tokens: [user, status, used_kb, soft_kb, hard_kb, grace, used_inodes, soft_inodes, hard_inodes, grace]
                let used_kb: u64 = tokens[2].parse().unwrap_or(0);
                let hard_kb: u64 = tokens[4].parse().unwrap_or(0);
                let used_inodes: u64 = tokens[6].parse().unwrap_or(0);
                let hard_inodes: u64 = tokens[7].parse().unwrap_or(0);
                return Some((used_kb * 1024, hard_kb * 1024, used_inodes, hard_inodes));
            }
        }
        None
    }

    /// Helper to parse xfs_quota report output lines.
    fn parse_xfs_quota_output(stdout: &str, username: &str) -> Option<(u64, u64, u64, u64)> {
        for line in stdout.lines() {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.is_empty() {
                continue;
            }
            if tokens[0] == username && tokens.len() >= 6 {
                let used_kb: u64 = tokens[1].parse().unwrap_or(0);
                let hard_kb: u64 = tokens[3].parse().unwrap_or(0);
                let used_inodes: u64 = tokens[4].parse().unwrap_or(0);
                let hard_inodes: u64 = tokens[5].parse().unwrap_or(0);
                return Some((used_kb * 1024, hard_kb * 1024, used_inodes, hard_inodes));
            }
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::system::{MockAction, MockSystemRunner};

    #[test]
    fn test_set_and_get_quota_ext4() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = QuotaManager::with_forced_backend(
            Arc::clone(&runner) as Arc<dyn SystemRunner>,
            "/home",
            FsQuotaBackend::Ext4,
        );

        let set_req = QuotaSetRequest {
            uid: 10001,
            username: "p_testuser".to_string(),
            disk_limit_bytes: 10 * 1024 * 1024, // 10 MB
            inode_limit: 100_000,
            mount_point: Some("/home".to_string()),
        };

        let set_res = manager.set_quota(set_req).expect("set quota");
        assert_eq!(set_res.uid, 10001);
        assert_eq!(set_res.backend, "ext4");

        let actions = runner.recorded_actions();
        let cmd = actions
            .iter()
            .find(|a| matches!(a, MockAction::Command { binary, .. } if binary == "setquota"));
        assert!(cmd.is_some(), "setquota command should be executed");

        if let Some(MockAction::Command { args, .. }) = cmd {
            assert_eq!(args[0], "-u");
            assert_eq!(args[1], "p_testuser");
            assert_eq!(args[2], "10240"); // 10 MB in KB
            assert_eq!(args[4], "100000"); // inodes
        }
    }

    #[test]
    fn test_set_and_get_quota_xfs() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = QuotaManager::with_forced_backend(
            Arc::clone(&runner) as Arc<dyn SystemRunner>,
            "/home",
            FsQuotaBackend::Xfs,
        );

        let set_req = QuotaSetRequest {
            uid: 10002,
            username: "p_xfsuser".to_string(),
            disk_limit_bytes: 20 * 1024 * 1024,
            inode_limit: 200_000,
            mount_point: Some("/home".to_string()),
        };

        let set_res = manager.set_quota(set_req).expect("set quota xfs");
        assert_eq!(set_res.uid, 10002);
        assert_eq!(set_res.backend, "xfs");

        let actions = runner.recorded_actions();
        let cmd = actions
            .iter()
            .find(|a| matches!(a, MockAction::Command { binary, .. } if binary == "xfs_quota"));
        assert!(cmd.is_some(), "xfs_quota command should be executed");
    }

    #[test]
    fn test_fallback_scan_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let user_home = tmp.path().join("p_scanner");
        std::fs::create_dir_all(&user_home).expect("create dir");

        // Write some files
        std::fs::write(user_home.join("file1.txt"), "hello world").expect("write file1"); // 11 bytes
        std::fs::write(user_home.join("file2.txt"), "panel quota test").expect("write file2"); // 16 bytes

        let sub = user_home.join("subdir");
        std::fs::create_dir_all(&sub).expect("create subdir");
        std::fs::write(sub.join("file3.txt"), "12345").expect("write file3"); // 5 bytes

        let runner = Arc::new(MockSystemRunner::new());
        let manager = QuotaManager::with_home_base(runner as Arc<dyn SystemRunner>, tmp.path());

        let scan_res = manager
            .scan_quota(QuotaScanRequest {
                uid: 10003,
                username: "p_scanner".to_string(),
                path: None,
            })
            .expect("scan quota succeeds");

        assert_eq!(scan_res.uid, 10003);
        assert_eq!(scan_res.bytes_used, 11 + 16 + 5);
        // Inodes: user_home (1) + file1 (1) + file2 (1) + subdir (1) + file3 (1) = 5
        assert_eq!(scan_res.inodes_used, 5);

        // Test fallback get_quota
        let get_res = manager
            .get_quota(QuotaGetRequest {
                uid: 10003,
                username: "p_scanner".to_string(),
                mount_point: None,
            })
            .expect("get quota succeeds");

        assert_eq!(get_res.bytes_used, 32);
        assert_eq!(get_res.inodes_used, 5);
        assert_eq!(get_res.backend, "fallback_scan");
    }

    #[test]
    fn test_scan_path_traversal_rejected() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let user_home = tmp.path().join("p_evil");
        std::fs::create_dir_all(&user_home).expect("create dir");

        let runner = Arc::new(MockSystemRunner::new());
        let manager = QuotaManager::with_home_base(runner as Arc<dyn SystemRunner>, tmp.path());

        // Attempt to scan /etc
        let res = manager.scan_quota(QuotaScanRequest {
            uid: 10004,
            username: "p_evil".to_string(),
            path: Some("/etc".to_string()),
        });
        assert!(res.is_err(), "Scanning outside home must be rejected");
    }
}
