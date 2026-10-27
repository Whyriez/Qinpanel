//! Cgroup v2 resource reader for tenant systemd slices.
//!
//! Reads resource metrics directly from cgroup v2 control files:
//! - `cpu.stat`: CPU usage (usage_usec, user_usec, system_usec, throttling stats).
//! - `memory.current` & `memory.peak`: Current and peak memory consumption in bytes.
//! - `pids.current` & `pids.peak`: Current and peak number of processes/threads.
//! - `io.stat`: Read/write bytes and IO operations across block devices.

use crate::AgentError;
use panel_core::tenant::TenantUid;
use panel_proto::SliceResourceMetrics;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_CGROUP_ROOT: &str = "/sys/fs/cgroup";

#[derive(Debug, Clone)]
pub struct CgroupReader {
    cgroup_root: PathBuf,
}

impl Default for CgroupReader {
    fn default() -> Self {
        Self::new(DEFAULT_CGROUP_ROOT)
    }
}

impl CgroupReader {
    pub fn new(cgroup_root: impl AsRef<Path>) -> Self {
        Self {
            cgroup_root: cgroup_root.as_ref().to_path_buf(),
        }
    }

    /// Read cgroup v2 resource metrics for a tenant UID.
    ///
    /// Validates tenant UID within range 10000..=59999.
    /// If the slice does not exist or has no active processes, returns
    /// default metrics with `exists: false`.
    pub fn read_slice_resources(&self, uid: u32) -> Result<SliceResourceMetrics, AgentError> {
        TenantUid::parse(uid).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        let slice_name = format!("panel-u{uid}.slice");

        // Systemd may place slices directly at root or under system.slice
        let direct_path = self.cgroup_root.join(&slice_name);
        let system_slice_path = self.cgroup_root.join("system.slice").join(&slice_name);

        let slice_dir = if direct_path.is_dir() {
            direct_path
        } else if system_slice_path.is_dir() {
            system_slice_path
        } else {
            return Ok(SliceResourceMetrics {
                uid,
                slice_name,
                exists: false,
                ..Default::default()
            });
        };

        let mut metrics = SliceResourceMetrics {
            uid,
            slice_name,
            exists: true,
            ..Default::default()
        };

        // 1. Read cpu.stat
        let cpu_stat_path = slice_dir.join("cpu.stat");
        if let Ok(content) = fs::read_to_string(&cpu_stat_path) {
            parse_cpu_stat(&content, &mut metrics);
        }

        // 2. Read memory.current
        if let Some(val) = read_u64_file(&slice_dir.join("memory.current")) {
            metrics.memory_current_bytes = val;
        }

        // 3. Read memory.peak (optional in some kernels)
        if let Some(val) = read_u64_file(&slice_dir.join("memory.peak")) {
            metrics.memory_peak_bytes = Some(val);
        }

        // 4. Read pids.current
        if let Some(val) = read_u64_file(&slice_dir.join("pids.current")) {
            metrics.pids_current = val;
        }

        // 5. Read pids.peak (optional)
        if let Some(val) = read_u64_file(&slice_dir.join("pids.peak")) {
            metrics.pids_peak = Some(val);
        }

        // 6. Read io.stat
        let io_stat_path = slice_dir.join("io.stat");
        if let Ok(content) = fs::read_to_string(&io_stat_path) {
            parse_io_stat(&content, &mut metrics);
        }

        Ok(metrics)
    }
}

/// Reads and parses a single integer from a cgroup v2 file.
fn read_u64_file(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse::<u64>().ok()
}

/// Parses key-value pairs from cgroup v2 `cpu.stat`.
fn parse_cpu_stat(content: &str, metrics: &mut SliceResourceMetrics) {
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let key = match parts.next() {
            Some(k) => k,
            None => continue,
        };
        let val_str = match parts.next() {
            Some(v) => v,
            None => continue,
        };
        let val = match val_str.parse::<u64>() {
            Ok(v) => v,
            Err(_) => continue,
        };

        match key {
            "usage_usec" => metrics.cpu_usage_usec = val,
            "user_usec" => metrics.cpu_user_usec = val,
            "system_usec" => metrics.cpu_system_usec = val,
            "nr_periods" => metrics.cpu_nr_periods = val,
            "nr_throttled" => metrics.cpu_nr_throttled = val,
            "throttled_usec" => metrics.cpu_throttled_usec = val,
            _ => {}
        }
    }
}

/// Parses space-separated key=value entries per device line in cgroup v2 `io.stat`.
/// Example: `8:0 rbytes=1048576 wbytes=2097152 rios=120 wios=240 dbytes=0 dios=0`
fn parse_io_stat(content: &str, metrics: &mut SliceResourceMetrics) {
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        // Skip device identifier (e.g. "8:0")
        if parts.next().is_none() {
            continue;
        }

        for item in parts {
            let mut kv = item.splitn(2, '=');
            let key = match kv.next() {
                Some(k) => k,
                None => continue,
            };
            let val_str = match kv.next() {
                Some(v) => v,
                None => continue,
            };
            let val = match val_str.parse::<u64>() {
                Ok(v) => v,
                Err(_) => continue,
            };

            match key {
                "rbytes" => metrics.io_read_bytes = metrics.io_read_bytes.saturating_add(val),
                "wbytes" => metrics.io_write_bytes = metrics.io_write_bytes.saturating_add(val),
                "rios" => metrics.io_read_ops = metrics.io_read_ops.saturating_add(val),
                "wios" => metrics.io_write_ops = metrics.io_write_ops.saturating_add(val),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cgroup_reader_full_metrics() {
        let temp = tempdir().expect("tempdir");
        let slice_dir = temp.path().join("panel-u10001.slice");
        fs::create_dir_all(&slice_dir).expect("create slice dir");

        let cpu_stat = "usage_usec 1500000\nuser_usec 1200000\nsystem_usec 300000\nnr_periods 200\nnr_throttled 5\nthrottled_usec 45000\n";
        fs::write(slice_dir.join("cpu.stat"), cpu_stat).expect("write cpu.stat");

        fs::write(slice_dir.join("memory.current"), "52428800\n").expect("write memory.current");
        fs::write(slice_dir.join("memory.peak"), "104857600\n").expect("write memory.peak");

        fs::write(slice_dir.join("pids.current"), "8\n").expect("write pids.current");
        fs::write(slice_dir.join("pids.peak"), "24\n").expect("write pids.peak");

        let io_stat = "8:0 rbytes=1048576 wbytes=2097152 rios=100 wios=200 dbytes=0 dios=0\n8:16 rbytes=524288 wbytes=0 rios=50 wios=0\n";
        fs::write(slice_dir.join("io.stat"), io_stat).expect("write io.stat");

        let reader = CgroupReader::new(temp.path());
        let metrics = reader.read_slice_resources(10001).expect("read metrics");

        assert_eq!(metrics.uid, 10001);
        assert_eq!(metrics.slice_name, "panel-u10001.slice");
        assert!(metrics.exists);
        assert_eq!(metrics.cpu_usage_usec, 1500000);
        assert_eq!(metrics.cpu_user_usec, 1200000);
        assert_eq!(metrics.cpu_system_usec, 300000);
        assert_eq!(metrics.cpu_nr_periods, 200);
        assert_eq!(metrics.cpu_nr_throttled, 5);
        assert_eq!(metrics.cpu_throttled_usec, 45000);
        assert_eq!(metrics.memory_current_bytes, 52428800);
        assert_eq!(metrics.memory_peak_bytes, Some(104857600));
        assert_eq!(metrics.pids_current, 8);
        assert_eq!(metrics.pids_peak, Some(24));
        assert_eq!(metrics.io_read_bytes, 1572864);
        assert_eq!(metrics.io_write_bytes, 2097152);
        assert_eq!(metrics.io_read_ops, 150);
        assert_eq!(metrics.io_write_ops, 200);
    }

    #[test]
    fn test_cgroup_reader_system_slice_fallback() {
        let temp = tempdir().expect("tempdir");
        let slice_dir = temp.path().join("system.slice").join("panel-u10002.slice");
        fs::create_dir_all(&slice_dir).expect("create system slice dir");

        fs::write(slice_dir.join("memory.current"), "10000\n").expect("write memory.current");

        let reader = CgroupReader::new(temp.path());
        let metrics = reader.read_slice_resources(10002).expect("read metrics");

        assert!(metrics.exists);
        assert_eq!(metrics.memory_current_bytes, 10000);
        assert_eq!(metrics.memory_peak_bytes, None);
    }

    #[test]
    fn test_cgroup_reader_non_existent_slice() {
        let temp = tempdir().expect("tempdir");
        let reader = CgroupReader::new(temp.path());
        let metrics = reader.read_slice_resources(10003).expect("read metrics");

        assert_eq!(metrics.uid, 10003);
        assert!(!metrics.exists);
        assert_eq!(metrics.cpu_usage_usec, 0);
        assert_eq!(metrics.memory_current_bytes, 0);
        assert_eq!(metrics.pids_current, 0);
    }

    #[test]
    fn test_cgroup_reader_invalid_uid_rejected() {
        let reader = CgroupReader::new("/sys/fs/cgroup");
        assert!(reader.read_slice_resources(9999).is_err());
        assert!(reader.read_slice_resources(60000).is_err());
    }

    #[test]
    fn test_cgroup_reader_corrupt_files_resilient() {
        let temp = tempdir().expect("tempdir");
        let slice_dir = temp.path().join("panel-u10004.slice");
        fs::create_dir_all(&slice_dir).expect("create slice dir");

        fs::write(
            slice_dir.join("cpu.stat"),
            "garbage line\nusage_usec not_a_number\n",
        )
        .expect("write corrupt cpu.stat");
        fs::write(slice_dir.join("memory.current"), "invalid_bytes").expect("write corrupt memory");
        fs::write(slice_dir.join("io.stat"), "8:0 rbytes=abc corrupted\n")
            .expect("write corrupt io");

        let reader = CgroupReader::new(temp.path());
        let metrics = reader
            .read_slice_resources(10004)
            .expect("read metrics should not fail");

        assert!(metrics.exists);
        assert_eq!(metrics.cpu_usage_usec, 0);
        assert_eq!(metrics.memory_current_bytes, 0);
        assert_eq!(metrics.io_read_bytes, 0);
    }
}
