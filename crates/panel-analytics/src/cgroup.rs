use crate::error::AnalyticsError;
use crate::models::{ServiceMetricsPoint, TenantMetricsPoint};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Default)]
struct CgroupTrackerState {
    tenant_cpu: HashMap<u32, (u64, Instant)>,
    tenant_io: HashMap<u32, ((u64, u64), Instant)>,
    service_cpu: HashMap<String, (u64, Instant)>,
    service_io: HashMap<String, ((u64, u64), Instant)>,
}

/// Reader for cgroup v2 metrics for tenants (`panel-u{uid}.slice`) and services (`system.slice/{service}.service`).
pub struct CgroupMetricsCollector {
    cgroup_root: PathBuf,
    state: Mutex<CgroupTrackerState>,
}

impl CgroupMetricsCollector {
    pub fn new(cgroup_root: impl AsRef<Path>) -> Self {
        Self {
            cgroup_root: cgroup_root.as_ref().to_path_buf(),
            state: Mutex::new(CgroupTrackerState::default()),
        }
    }

    pub fn default_system() -> Self {
        Self::new("/sys/fs/cgroup")
    }

    /// Locate directory for a tenant's slice.
    fn find_tenant_slice_dir(&self, uid: u32) -> Option<PathBuf> {
        let slice_name = format!("panel-u{uid}.slice");
        let direct = self.cgroup_root.join(&slice_name);
        if direct.is_dir() {
            return Some(direct);
        }
        let under_system = self.cgroup_root.join("system.slice").join(&slice_name);
        if under_system.is_dir() {
            return Some(under_system);
        }
        None
    }

    /// Locate directory for a system service.
    fn find_service_dir(&self, service_name: &str) -> Option<PathBuf> {
        let full_name = if service_name.ends_with(".service") {
            service_name.to_string()
        } else {
            format!("{service_name}.service")
        };

        let under_system = self.cgroup_root.join("system.slice").join(&full_name);
        if under_system.is_dir() {
            return Some(under_system);
        }
        let direct = self.cgroup_root.join(&full_name);
        if direct.is_dir() {
            return Some(direct);
        }
        None
    }

    /// Collect current metrics point for a specific tenant UID.
    pub fn collect_tenant_metrics(
        &self,
        uid: u32,
        timestamp: i64,
    ) -> Result<TenantMetricsPoint, AnalyticsError> {
        let now = Instant::now();
        let Some(slice_dir) = self.find_tenant_slice_dir(uid) else {
            // Slice does not exist or tenant has no active processes
            return Ok(TenantMetricsPoint {
                timestamp,
                uid,
                cpu_usage_usec: 0,
                cpu_percent: 0.0,
                memory_current_bytes: 0,
                memory_peak_bytes: None,
                memory_swap_bytes: None,
                pids_current: 0,
                io_read_bytes_sec: 0.0,
                io_write_bytes_sec: 0.0,
            });
        };

        // 1. CPU
        let cpu_usage_usec = read_cpu_usage_usec(&slice_dir.join("cpu.stat"));

        // 2. Memory
        let memory_current_bytes = read_u64_file(&slice_dir.join("memory.current")).unwrap_or(0);
        let memory_peak_bytes = read_u64_file(&slice_dir.join("memory.peak"));
        let memory_swap_bytes = read_u64_file(&slice_dir.join("memory.swap.current"));

        // 3. PIDs
        let pids_current = read_u64_file(&slice_dir.join("pids.current")).unwrap_or(0);

        // 4. IO
        let (io_rbytes, io_wbytes) = read_io_bytes(&slice_dir.join("io.stat"));

        // Calculate delta CPU % and IO rates
        let mut lock = self.state.lock().map_err(|e| {
            AnalyticsError::System(format!("Mutex lock error on cgroup tracker: {e}"))
        })?;

        let cpu_percent = if let Some((prev_usage, prev_time)) = lock.tenant_cpu.get(&uid) {
            let elapsed_usec = now.duration_since(*prev_time).as_micros() as u64;
            if elapsed_usec > 1000 {
                let delta_usage = cpu_usage_usec.saturating_sub(*prev_usage);
                let pct = (delta_usage as f64 / elapsed_usec as f64) * 100.0;
                pct.max(0.0)
            } else {
                0.0
            }
        } else {
            0.0
        };
        lock.tenant_cpu.insert(uid, (cpu_usage_usec, now));

        let (io_read_bytes_sec, io_write_bytes_sec) =
            if let Some(((prev_r, prev_w), prev_time)) = lock.tenant_io.get(&uid) {
                let elapsed_secs = now.duration_since(*prev_time).as_secs_f64();
                if elapsed_secs > 0.001 {
                    let r_delta = io_rbytes.saturating_sub(*prev_r);
                    let w_delta = io_wbytes.saturating_sub(*prev_w);
                    (r_delta as f64 / elapsed_secs, w_delta as f64 / elapsed_secs)
                } else {
                    (0.0, 0.0)
                }
            } else {
                (0.0, 0.0)
            };
        lock.tenant_io.insert(uid, ((io_rbytes, io_wbytes), now));

        Ok(TenantMetricsPoint {
            timestamp,
            uid,
            cpu_usage_usec,
            cpu_percent,
            memory_current_bytes,
            memory_peak_bytes,
            memory_swap_bytes,
            pids_current,
            io_read_bytes_sec,
            io_write_bytes_sec,
        })
    }

    /// Collect current metrics point for a monitored system service.
    pub fn collect_service_metrics(
        &self,
        service_name: &str,
        timestamp: i64,
    ) -> Result<ServiceMetricsPoint, AnalyticsError> {
        let now = Instant::now();
        let Some(svc_dir) = self.find_service_dir(service_name) else {
            return Ok(ServiceMetricsPoint {
                timestamp,
                service_name: service_name.to_string(),
                cpu_usage_usec: 0,
                cpu_percent: 0.0,
                memory_current_bytes: 0,
                pids_current: 0,
                io_read_bytes_sec: 0.0,
                io_write_bytes_sec: 0.0,
            });
        };

        let cpu_usage_usec = read_cpu_usage_usec(&svc_dir.join("cpu.stat"));
        let memory_current_bytes = read_u64_file(&svc_dir.join("memory.current")).unwrap_or(0);
        let pids_current = read_u64_file(&svc_dir.join("pids.current")).unwrap_or(0);
        let (io_rbytes, io_wbytes) = read_io_bytes(&svc_dir.join("io.stat"));

        let mut lock = self.state.lock().map_err(|e| {
            AnalyticsError::System(format!("Mutex lock error on cgroup tracker: {e}"))
        })?;

        let cpu_percent = if let Some((prev_usage, prev_time)) = lock.service_cpu.get(service_name)
        {
            let elapsed_usec = now.duration_since(*prev_time).as_micros() as u64;
            if elapsed_usec > 1000 {
                let delta_usage = cpu_usage_usec.saturating_sub(*prev_usage);
                let pct = (delta_usage as f64 / elapsed_usec as f64) * 100.0;
                pct.max(0.0)
            } else {
                0.0
            }
        } else {
            0.0
        };
        lock.service_cpu
            .insert(service_name.to_string(), (cpu_usage_usec, now));

        let (io_read_bytes_sec, io_write_bytes_sec) =
            if let Some(((prev_r, prev_w), prev_time)) = lock.service_io.get(service_name) {
                let elapsed_secs = now.duration_since(*prev_time).as_secs_f64();
                if elapsed_secs > 0.001 {
                    let r_delta = io_rbytes.saturating_sub(*prev_r);
                    let w_delta = io_wbytes.saturating_sub(*prev_w);
                    (r_delta as f64 / elapsed_secs, w_delta as f64 / elapsed_secs)
                } else {
                    (0.0, 0.0)
                }
            } else {
                (0.0, 0.0)
            };
        lock.service_io
            .insert(service_name.to_string(), ((io_rbytes, io_wbytes), now));

        Ok(ServiceMetricsPoint {
            timestamp,
            service_name: service_name.to_string(),
            cpu_usage_usec,
            cpu_percent,
            memory_current_bytes,
            pids_current,
            io_read_bytes_sec,
            io_write_bytes_sec,
        })
    }
}

pub(crate) fn read_u64_file(path: &Path) -> Option<u64> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed == "max" {
        return None;
    }
    trimmed.parse::<u64>().ok()
}

pub(crate) fn read_cpu_usage_usec(path: &Path) -> u64 {
    let Ok(content) = fs::read_to_string(path) else {
        return 0;
    };
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        if let (Some("usage_usec"), Some(v)) = (parts.next(), parts.next()) {
            return v.parse::<u64>().unwrap_or(0);
        }
    }
    0
}

pub(crate) fn read_io_bytes(path: &Path) -> (u64, u64) {
    let Ok(content) = fs::read_to_string(path) else {
        return (0, 0);
    };
    let mut total_r = 0u64;
    let mut total_w = 0u64;

    for line in content.lines() {
        for token in line.split_whitespace() {
            if let Some(rest) = token.strip_prefix("rbytes=") {
                total_r += rest.parse::<u64>().unwrap_or(0);
            } else if let Some(rest) = token.strip_prefix("wbytes=") {
                total_w += rest.parse::<u64>().unwrap_or(0);
            }
        }
    }

    (total_r, total_w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cgroup_tenant_collection() {
        let tmp = tempdir().expect("tempdir");
        let cgroup_root = tmp.path();

        let slice_dir = cgroup_root.join("panel-u10001.slice");
        fs::create_dir_all(&slice_dir).expect("create slice");

        fs::write(
            slice_dir.join("cpu.stat"),
            "usage_usec 500000\nuser_usec 400000\nsystem_usec 100000\n",
        )
        .expect("write cpu.stat");
        fs::write(slice_dir.join("memory.current"), "52428800\n").expect("write memory.current");
        fs::write(slice_dir.join("memory.peak"), "62914560\n").expect("write memory.peak");
        fs::write(slice_dir.join("pids.current"), "12\n").expect("write pids.current");
        fs::write(
            slice_dir.join("io.stat"),
            "8:0 rbytes=1048576 wbytes=2097152 rios=100 wios=200\n",
        )
        .expect("write io.stat");

        let collector = CgroupMetricsCollector::new(cgroup_root);
        let m1 = collector
            .collect_tenant_metrics(10001, 1770000000)
            .expect("collect 1");

        assert_eq!(m1.uid, 10001);
        assert_eq!(m1.cpu_usage_usec, 500000);
        assert_eq!(m1.memory_current_bytes, 52428800);
        assert_eq!(m1.memory_peak_bytes, Some(62914560));
        assert_eq!(m1.pids_current, 12);
        assert_eq!(m1.cpu_percent, 0.0); // First sample

        // Second sample with incremented usage
        fs::write(
            slice_dir.join("cpu.stat"),
            "usage_usec 800000\nuser_usec 600000\nsystem_usec 200000\n",
        )
        .expect("write cpu.stat 2");
        std::thread::sleep(std::time::Duration::from_millis(50));

        let m2 = collector
            .collect_tenant_metrics(10001, 1770000010)
            .expect("collect 2");
        assert_eq!(m2.cpu_usage_usec, 800000);
        assert!(m2.cpu_percent > 0.0);
    }

    #[test]
    fn test_cgroup_service_collection() {
        let tmp = tempdir().expect("tempdir");
        let cgroup_root = tmp.path();

        let svc_dir = cgroup_root.join("system.slice").join("nginx.service");
        fs::create_dir_all(&svc_dir).expect("create svc");

        fs::write(svc_dir.join("cpu.stat"), "usage_usec 1200000\n").expect("write cpu.stat");
        fs::write(svc_dir.join("memory.current"), "104857600\n").expect("write memory.current");
        fs::write(svc_dir.join("pids.current"), "4\n").expect("write pids.current");
        fs::write(svc_dir.join("io.stat"), "8:0 rbytes=50000 wbytes=80000\n")
            .expect("write io.stat");

        let collector = CgroupMetricsCollector::new(cgroup_root);
        let m = collector
            .collect_service_metrics("nginx", 1770000000)
            .expect("collect nginx");

        assert_eq!(m.service_name, "nginx");
        assert_eq!(m.cpu_usage_usec, 1200000);
        assert_eq!(m.memory_current_bytes, 104857600);
        assert_eq!(m.pids_current, 4);
    }
}
