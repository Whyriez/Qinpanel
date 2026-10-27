//! Node.js & Python process metrics collector via cgroup v2.
//!
//! Tracks transient application services (`site-{id}.service` or custom units):
//! - CPU utilization delta percentage
//! - Current, peak, and swap memory usage
//! - Current task/process count (`pids.current`)
//! - Disk I/O read and write rates (`io.stat`)

use crate::cgroup::{read_cpu_usage_usec, read_io_bytes, read_u64_file};
use crate::error::AnalyticsError;
use crate::models::{AppProcessMetricPoint, AppType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Default)]
struct AppTrackerState {
    cpu: HashMap<String, (u64, Instant)>,
    io: HashMap<String, ((u64, u64), Instant)>,
}

/// Collector for Node.js and Python application service metrics from cgroup v2.
pub struct AppProcessMetricsCollector {
    cgroup_root: PathBuf,
    state: Mutex<AppTrackerState>,
}

impl AppProcessMetricsCollector {
    pub fn new(cgroup_root: impl AsRef<Path>) -> Self {
        Self {
            cgroup_root: cgroup_root.as_ref().to_path_buf(),
            state: Mutex::new(AppTrackerState::default()),
        }
    }

    pub fn default_system() -> Self {
        Self::new("/sys/fs/cgroup")
    }

    /// Resolves the cgroup directory path for an application service unit.
    pub fn find_service_dir(&self, service_name: &str) -> Option<PathBuf> {
        let full_name = if service_name.ends_with(".service") {
            service_name.to_string()
        } else {
            format!("{service_name}.service")
        };

        // 1. Direct under system.slice (most common systemd placement)
        let under_system = self.cgroup_root.join("system.slice").join(&full_name);
        if under_system.is_dir() {
            return Some(under_system);
        }

        // 2. Direct under cgroup root
        let direct = self.cgroup_root.join(&full_name);
        if direct.is_dir() {
            return Some(direct);
        }

        // 3. Under slice matching site/user if tenant slices are structured
        let under_system_slice = self.cgroup_root.join("system.slice");
        if let Ok(entries) = std::fs::read_dir(&under_system_slice) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let candidate = p.join(&full_name);
                    if candidate.is_dir() {
                        return Some(candidate);
                    }
                }
            }
        }

        None
    }

    /// Collects metrics point for an application service.
    pub fn collect_app_metrics(
        &self,
        site_id: i64,
        app_type: AppType,
        service_name: &str,
        timestamp: i64,
    ) -> Result<AppProcessMetricPoint, AnalyticsError> {
        let now = Instant::now();
        let tracker_key = format!("{}:{}:{}", site_id, app_type.as_str(), service_name);

        let Some(svc_dir) = self.find_service_dir(service_name) else {
            // Service not running or cgroup not present
            return Ok(AppProcessMetricPoint {
                timestamp,
                site_id,
                app_type,
                service_name: service_name.to_string(),
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
        let cpu_usage_usec = read_cpu_usage_usec(&svc_dir.join("cpu.stat"));

        // 2. Memory
        let memory_current_bytes = read_u64_file(&svc_dir.join("memory.current")).unwrap_or(0);
        let memory_peak_bytes = read_u64_file(&svc_dir.join("memory.peak"));
        let memory_swap_bytes = read_u64_file(&svc_dir.join("memory.swap.current"));

        // 3. PIDs
        let pids_current = read_u64_file(&svc_dir.join("pids.current")).unwrap_or(0);

        // 4. I/O
        let (io_rbytes, io_wbytes) = read_io_bytes(&svc_dir.join("io.stat"));

        let mut lock = self.state.lock().map_err(|e| {
            AnalyticsError::System(format!("Mutex lock error on app cgroup tracker: {e}"))
        })?;

        // Calculate CPU delta percentage
        let cpu_percent = if let Some((prev_usage, prev_time)) = lock.cpu.get(&tracker_key) {
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
        lock.cpu.insert(tracker_key.clone(), (cpu_usage_usec, now));

        // Calculate I/O rate
        let (io_read_bytes_sec, io_write_bytes_sec) =
            if let Some(((prev_r, prev_w), prev_time)) = lock.io.get(&tracker_key) {
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
        lock.io.insert(tracker_key, ((io_rbytes, io_wbytes), now));

        Ok(AppProcessMetricPoint {
            timestamp,
            site_id,
            app_type,
            service_name: service_name.to_string(),
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
}
