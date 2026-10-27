//! Server hardware detection, automated tuning profiles generation,
//! atomic apply with validation, and automatic/manual rollback.

use crate::AgentError;
use crate::system::SystemRunner;
use panel_proto::{
    AppRuntimeHints, NginxTuningConfig, OptimizerApplyRequest, OptimizerApplyResponse,
    OptimizerProfile, OptimizerProfileType, OptimizerRollbackRequest, OptimizerRollbackResponse,
    OptimizerStatusResponse, PhpFpmTuningConfig, ServerHardwareSpecs, SysctlTuningConfig,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptimizerStateRecord {
    pub active_profile: OptimizerProfileType,
    pub applied_at: u64,
    pub current_settings: OptimizerProfile,
    pub has_backup: bool,
    pub last_backup_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupMetadata {
    pub backed_up_at: u64,
    pub had_sysctl_file: bool,
    pub had_nginx_file: bool,
    pub previous_profile: Option<OptimizerProfileType>,
}

struct SnapshotResult {
    meta: BackupMetadata,
    old_sysctl: Option<Vec<u8>>,
    old_nginx: Option<Vec<u8>>,
}

pub struct OptimizerManager {
    runner: Arc<dyn SystemRunner>,
    base_dir: PathBuf,
    sysctl_conf_path: PathBuf,
    nginx_conf_d: PathBuf,
}

impl OptimizerManager {
    pub fn new(runner: Arc<dyn SystemRunner>) -> Self {
        Self {
            runner,
            base_dir: PathBuf::from("/var/lib/panel/optimizer"),
            sysctl_conf_path: PathBuf::from("/etc/sysctl.d/99-panel-tuning.conf"),
            nginx_conf_d: PathBuf::from("/etc/panel/nginx/conf.d"),
        }
    }

    pub fn with_paths(
        runner: Arc<dyn SystemRunner>,
        base_dir: impl Into<PathBuf>,
        sysctl_conf_path: impl Into<PathBuf>,
        nginx_conf_d: impl Into<PathBuf>,
    ) -> Self {
        Self {
            runner,
            base_dir: base_dir.into(),
            sysctl_conf_path: sysctl_conf_path.into(),
            nginx_conf_d: nginx_conf_d.into(),
        }
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Detect server hardware specs from host /proc and system files.
    pub fn detect_hardware(&self) -> Result<ServerHardwareSpecs, AgentError> {
        let mut vcpu_count = 2u32;
        let mut ram_total_mb = 2048u64;
        let mut ram_free_mb = 1024u64;
        let mut swap_total_mb = 1024u64;
        let mut swap_free_mb = 1024u64;
        let mut kernel_version = "Linux 6.8.0-generic".to_string();
        let mut os_distro = "Ubuntu 24.04 LTS".to_string();

        // 1. Detect vCPU count from /proc/cpuinfo
        let cpuinfo_path = Path::new("/proc/cpuinfo");
        if let Ok(bytes) = self.runner.read_file(cpuinfo_path) {
            let content = String::from_utf8_lossy(&bytes);
            let count = content
                .lines()
                .filter(|l| l.starts_with("processor"))
                .count();
            if count > 0 {
                vcpu_count = count as u32;
            }
        }

        // 2. Detect RAM & Swap from /proc/meminfo
        let meminfo_path = Path::new("/proc/meminfo");
        if let Ok(bytes) = self.runner.read_file(meminfo_path) {
            let content = String::from_utf8_lossy(&bytes);
            for line in content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let key = parts[0].trim_end_matches(':');
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        let mb = kb / 1024;
                        match key {
                            "MemTotal" => ram_total_mb = mb,
                            "MemAvailable" => ram_free_mb = mb,
                            "MemFree" if ram_free_mb == 1024 => ram_free_mb = mb,
                            "SwapTotal" => swap_total_mb = mb,
                            "SwapFree" => swap_free_mb = mb,
                            _ => {}
                        }
                    }
                }
            }
        }

        // 3. Detect Kernel version from /proc/version
        let proc_ver_path = Path::new("/proc/version");
        if let Ok(bytes) = self.runner.read_file(proc_ver_path) {
            let ver_str = String::from_utf8_lossy(&bytes);
            if let Some(first_line) = ver_str.lines().next() {
                kernel_version = first_line.trim().to_string();
            }
        }

        // 4. Detect OS Distro from /etc/os-release
        let os_rel_path = Path::new("/etc/os-release");
        if let Ok(bytes) = self.runner.read_file(os_rel_path) {
            let content = String::from_utf8_lossy(&bytes);
            for line in content.lines() {
                if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                    os_distro = val.trim_matches('"').to_string();
                    break;
                }
            }
        }

        Ok(ServerHardwareSpecs {
            vcpu_count,
            ram_total_mb,
            ram_free_mb,
            swap_total_mb,
            swap_free_mb,
            kernel_version,
            os_distro,
        })
    }

    /// Generate automated tuning profile based on server hardware specs.
    pub fn generate_profile_for_type(
        &self,
        profile_type: OptimizerProfileType,
        specs: &ServerHardwareSpecs,
    ) -> OptimizerProfile {
        match profile_type {
            OptimizerProfileType::Micro => OptimizerProfile {
                profile_type: OptimizerProfileType::Micro,
                display_name: "Profil VPS Mikro (Hemat Memori)".to_string(),
                description: "Dioptimalkan untuk VPS spesifikasi minimal (1-2 vCPU, RAM < 2 GB). Menjaga footprint RAM serendah mungkin dengan batasan proses yang ketat.".to_string(),
                nginx: NginxTuningConfig {
                    worker_processes: "auto".to_string(),
                    worker_connections: 1024,
                    worker_rlimit_nofile: 2048,
                    multi_accept: true,
                    keepalive_timeout_secs: 30,
                    client_body_buffer_size_kb: 16,
                    fastcgi_buffer_size_kb: 16,
                    fastcgi_buffers: "4 16k".to_string(),
                    open_file_cache_max: 2000,
                    open_file_cache_inactive_secs: 20,
                },
                sysctl: SysctlTuningConfig {
                    somaxconn: 1024,
                    tcp_max_syn_backlog: 2048,
                    tcp_tw_reuse: 1,
                    ip_local_port_range: "10240 65535".to_string(),
                    swappiness: 25,
                    file_max: 131072,
                    netdev_max_backlog: 2048,
                },
                php_fpm: PhpFpmTuningConfig {
                    pm_type: "ondemand".to_string(),
                    max_children: 10,
                    start_servers: Some(2),
                    min_spare_servers: Some(1),
                    max_spare_servers: Some(3),
                    process_idle_timeout_secs: 10,
                    max_requests: 500,
                },
                hints: AppRuntimeHints {
                    recommended_node_workers: 1,
                    recommended_node_max_old_space_mb: 256,
                    recommended_python_workers: 2,
                },
            },
            OptimizerProfileType::Standard => OptimizerProfile {
                profile_type: OptimizerProfileType::Standard,
                display_name: "Profil Standar (Seimbang)".to_string(),
                description: "Dioptimalkan untuk VPS 2 vCPU / 2-4 GB RAM. Keseimbangan ideal antara throughput tinggi, waktu respons cepat, dan efisiensi memori.".to_string(),
                nginx: NginxTuningConfig {
                    worker_processes: "auto".to_string(),
                    worker_connections: 2048,
                    worker_rlimit_nofile: 4096,
                    multi_accept: true,
                    keepalive_timeout_secs: 65,
                    client_body_buffer_size_kb: 64,
                    fastcgi_buffer_size_kb: 32,
                    fastcgi_buffers: "8 32k".to_string(),
                    open_file_cache_max: 5000,
                    open_file_cache_inactive_secs: 30,
                },
                sysctl: SysctlTuningConfig {
                    somaxconn: 4096,
                    tcp_max_syn_backlog: 4096,
                    tcp_tw_reuse: 1,
                    ip_local_port_range: "10240 65535".to_string(),
                    swappiness: 15,
                    file_max: 262144,
                    netdev_max_backlog: 4096,
                },
                php_fpm: PhpFpmTuningConfig {
                    pm_type: "ondemand".to_string(),
                    max_children: 25,
                    start_servers: Some(3),
                    min_spare_servers: Some(2),
                    max_spare_servers: Some(5),
                    process_idle_timeout_secs: 15,
                    max_requests: 1000,
                },
                hints: AppRuntimeHints {
                    recommended_node_workers: specs.vcpu_count.max(2),
                    recommended_node_max_old_space_mb: 512,
                    recommended_python_workers: (specs.vcpu_count * 2) + 1,
                },
            },
            OptimizerProfileType::HighPerformance => OptimizerProfile {
                profile_type: OptimizerProfileType::HighPerformance,
                display_name: "Profil Performa Tinggi".to_string(),
                description: "Dioptimalkan untuk server 4-8 vCPU / 4-16 GB RAM. Peningkatan batas worker, antrean koneksi luas, dan cache file agresif untuk lalu lintas padat.".to_string(),
                nginx: NginxTuningConfig {
                    worker_processes: "auto".to_string(),
                    worker_connections: 4096,
                    worker_rlimit_nofile: 8192,
                    multi_accept: true,
                    keepalive_timeout_secs: 65,
                    client_body_buffer_size_kb: 128,
                    fastcgi_buffer_size_kb: 64,
                    fastcgi_buffers: "16 32k".to_string(),
                    open_file_cache_max: 10000,
                    open_file_cache_inactive_secs: 60,
                },
                sysctl: SysctlTuningConfig {
                    somaxconn: 16384,
                    tcp_max_syn_backlog: 16384,
                    tcp_tw_reuse: 1,
                    ip_local_port_range: "10240 65535".to_string(),
                    swappiness: 10,
                    file_max: 1048576,
                    netdev_max_backlog: 8192,
                },
                php_fpm: PhpFpmTuningConfig {
                    pm_type: "ondemand".to_string(),
                    max_children: 60,
                    start_servers: Some(5),
                    min_spare_servers: Some(3),
                    max_spare_servers: Some(10),
                    process_idle_timeout_secs: 20,
                    max_requests: 2000,
                },
                hints: AppRuntimeHints {
                    recommended_node_workers: specs.vcpu_count.max(4),
                    recommended_node_max_old_space_mb: 1024,
                    recommended_python_workers: (specs.vcpu_count * 2) + 1,
                },
            },
            OptimizerProfileType::DedicatedLarge => OptimizerProfile {
                profile_type: OptimizerProfileType::DedicatedLarge,
                display_name: "Profil Dedicated / High Concurrency".to_string(),
                description: "Dioptimalkan untuk dedicated server berkapasitas besar (> 16 GB RAM). Memaksimalkan konkurensi TCP socket, kapasitas pool worker, dan buffer memori.".to_string(),
                nginx: NginxTuningConfig {
                    worker_processes: "auto".to_string(),
                    worker_connections: 8192,
                    worker_rlimit_nofile: 16384,
                    multi_accept: true,
                    keepalive_timeout_secs: 65,
                    client_body_buffer_size_kb: 128,
                    fastcgi_buffer_size_kb: 64,
                    fastcgi_buffers: "32 64k".to_string(),
                    open_file_cache_max: 20000,
                    open_file_cache_inactive_secs: 60,
                },
                sysctl: SysctlTuningConfig {
                    somaxconn: 65535,
                    tcp_max_syn_backlog: 65535,
                    tcp_tw_reuse: 1,
                    ip_local_port_range: "10240 65535".to_string(),
                    swappiness: 10,
                    file_max: 2097152,
                    netdev_max_backlog: 16384,
                },
                php_fpm: PhpFpmTuningConfig {
                    pm_type: "ondemand".to_string(),
                    max_children: 120,
                    start_servers: Some(8),
                    min_spare_servers: Some(4),
                    max_spare_servers: Some(16),
                    process_idle_timeout_secs: 30,
                    max_requests: 4000,
                },
                hints: AppRuntimeHints {
                    recommended_node_workers: specs.vcpu_count.max(8),
                    recommended_node_max_old_space_mb: 2048,
                    recommended_python_workers: (specs.vcpu_count * 2) + 1,
                },
            },
            OptimizerProfileType::Custom => OptimizerProfile {
                profile_type: OptimizerProfileType::Custom,
                display_name: "Profil Kustom".to_string(),
                description: "Konfigurasi kustom sesuai preferensi spesifik administrator.".to_string(),
                nginx: NginxTuningConfig {
                    worker_processes: "auto".to_string(),
                    worker_connections: 2048,
                    worker_rlimit_nofile: 4096,
                    multi_accept: true,
                    keepalive_timeout_secs: 65,
                    client_body_buffer_size_kb: 64,
                    fastcgi_buffer_size_kb: 32,
                    fastcgi_buffers: "8 32k".to_string(),
                    open_file_cache_max: 5000,
                    open_file_cache_inactive_secs: 30,
                },
                sysctl: SysctlTuningConfig {
                    somaxconn: 4096,
                    tcp_max_syn_backlog: 4096,
                    tcp_tw_reuse: 1,
                    ip_local_port_range: "10240 65535".to_string(),
                    swappiness: 15,
                    file_max: 262144,
                    netdev_max_backlog: 4096,
                },
                php_fpm: PhpFpmTuningConfig {
                    pm_type: "ondemand".to_string(),
                    max_children: 25,
                    start_servers: Some(3),
                    min_spare_servers: Some(2),
                    max_spare_servers: Some(5),
                    process_idle_timeout_secs: 15,
                    max_requests: 1000,
                },
                hints: AppRuntimeHints {
                    recommended_node_workers: specs.vcpu_count.max(2),
                    recommended_node_max_old_space_mb: 512,
                    recommended_python_workers: (specs.vcpu_count * 2) + 1,
                },
            },
        }
    }

    /// Automatically decide recommended profile type from server hardware.
    pub fn recommend_profile_type(&self, specs: &ServerHardwareSpecs) -> OptimizerProfileType {
        if specs.ram_total_mb < 2048 {
            OptimizerProfileType::Micro
        } else if specs.ram_total_mb <= 4096 {
            OptimizerProfileType::Standard
        } else if specs.ram_total_mb <= 16384 {
            OptimizerProfileType::HighPerformance
        } else {
            OptimizerProfileType::DedicatedLarge
        }
    }

    /// Read state file if available.
    fn load_state(&self) -> Option<OptimizerStateRecord> {
        let state_path = self.base_dir.join("state.json");
        let bytes = self.runner.read_file(&state_path).ok()?;
        serde_json::from_slice::<OptimizerStateRecord>(&bytes).ok()
    }

    /// Save state file.
    fn save_state(&self, record: &OptimizerStateRecord) -> Result<(), AgentError> {
        let state_path = self.base_dir.join("state.json");
        let bytes = serde_json::to_vec_pretty(record).map_err(|e| {
            AgentError::Internal(format!("Failed to serialize optimizer state: {e}"))
        })?;
        self.runner.create_dir_all(&self.base_dir, 0o755, 0, 0)?;
        self.runner.write_file(&state_path, &bytes, 0o644)?;
        Ok(())
    }

    /// Get current optimizer status and settings comparison.
    pub fn get_status(&self) -> Result<OptimizerStatusResponse, AgentError> {
        let hardware = self.detect_hardware()?;
        let capacity = Some(crate::capacity::collect_capacity(
            self.runner.as_ref(),
            Self::now_secs(),
        ));
        let recommended_profile_type = self.recommend_profile_type(&hardware);
        let recommended_settings =
            self.generate_profile_for_type(recommended_profile_type, &hardware);

        let backup_meta_path = self.base_dir.join("backup").join("meta.json");
        let has_backup = self.runner.path_exists(&backup_meta_path);
        let last_backup_at = if has_backup {
            self.runner
                .read_file(&backup_meta_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<BackupMetadata>(&bytes).ok())
                .map(|meta| meta.backed_up_at)
        } else {
            None
        };

        if let Some(state) = self.load_state() {
            Ok(OptimizerStatusResponse {
                hardware,
                capacity,
                recommended_profile: recommended_profile_type,
                active_profile: Some(state.active_profile),
                applied_at: Some(state.applied_at),
                current_settings: state.current_settings,
                recommended_settings,
                has_backup,
                last_backup_at,
            })
        } else {
            // Default initial profile before optimizer was applied
            let default_current =
                self.generate_profile_for_type(OptimizerProfileType::Standard, &hardware);
            Ok(OptimizerStatusResponse {
                hardware,
                capacity,
                recommended_profile: recommended_profile_type,
                active_profile: None,
                applied_at: None,
                current_settings: default_current,
                recommended_settings,
                has_backup,
                last_backup_at,
            })
        }
    }

    /// Render sysctl config file content from tuning parameters.
    pub fn render_sysctl_conf(profile: &OptimizerProfile) -> String {
        format!(
            "# QonPanel Kernel & Network Tuning Drop-in\n\
             # Profile: {} ({})\n\
             # Auto-generated by Qinpanel Server Optimizer. Do not edit directly.\n\n\
             net.core.somaxconn = {}\n\
             net.ipv4.tcp_max_syn_backlog = {}\n\
             net.ipv4.tcp_tw_reuse = {}\n\
             net.ipv4.ip_local_port_range = {}\n\
             vm.swappiness = {}\n\
             fs.file-max = {}\n\
             net.core.netdev_max_backlog = {}\n",
            profile.display_name,
            profile.profile_type.as_str(),
            profile.sysctl.somaxconn,
            profile.sysctl.tcp_max_syn_backlog,
            profile.sysctl.tcp_tw_reuse,
            profile.sysctl.ip_local_port_range,
            profile.sysctl.swappiness,
            profile.sysctl.file_max,
            profile.sysctl.netdev_max_backlog
        )
    }

    /// Render Nginx conf.d/tuning.conf content from tuning parameters.
    pub fn render_nginx_tuning_conf(profile: &OptimizerProfile) -> String {
        format!(
            "# QonPanel Nginx Performance Tuning Directives\n\
             # Profile: {} ({})\n\
             # Auto-generated by Qinpanel Server Optimizer. Do not edit directly.\n\n\
             client_body_buffer_size {}k;\n\
             fastcgi_buffer_size {}k;\n\
             fastcgi_buffers {};\n\
             open_file_cache max={} inactive={}s;\n\
             open_file_cache_valid 30s;\n\
             open_file_cache_min_uses 2;\n\
             open_file_cache_errors on;\n",
            profile.display_name,
            profile.profile_type.as_str(),
            profile.nginx.client_body_buffer_size_kb,
            profile.nginx.fastcgi_buffer_size_kb,
            profile.nginx.fastcgi_buffers,
            profile.nginx.open_file_cache_max,
            profile.nginx.open_file_cache_inactive_secs
        )
    }

    fn take_backup_snapshot(
        &self,
        backup_dir: &Path,
        previous_profile: Option<OptimizerProfileType>,
        nginx_tuning_path: &Path,
    ) -> Result<SnapshotResult, AgentError> {
        let backup_sysctl_file = backup_dir.join("99-panel-tuning.conf.bak");
        let backup_nginx_file = backup_dir.join("tuning.conf.bak");

        let had_sysctl_file = self.runner.path_exists(&self.sysctl_conf_path);
        let mut old_sysctl_content: Option<Vec<u8>> = None;
        if had_sysctl_file {
            let bytes = self.runner.read_file(&self.sysctl_conf_path)?;
            self.runner.write_file(&backup_sysctl_file, &bytes, 0o644)?;
            old_sysctl_content = Some(bytes);
        }

        let had_nginx_file = self.runner.path_exists(nginx_tuning_path);
        let mut old_nginx_content: Option<Vec<u8>> = None;
        if had_nginx_file {
            let bytes = self.runner.read_file(nginx_tuning_path)?;
            self.runner.write_file(&backup_nginx_file, &bytes, 0o644)?;
            old_nginx_content = Some(bytes);
        }

        let meta = BackupMetadata {
            backed_up_at: Self::now_secs(),
            had_sysctl_file,
            had_nginx_file,
            previous_profile,
        };
        let meta_bytes = serde_json::to_vec_pretty(&meta).map_err(|e| {
            AgentError::Internal(format!("Failed to serialize backup metadata: {e}"))
        })?;
        self.runner
            .write_file(&backup_dir.join("meta.json"), &meta_bytes, 0o644)?;

        Ok(SnapshotResult {
            meta,
            old_sysctl: old_sysctl_content,
            old_nginx: old_nginx_content,
        })
    }

    fn write_tuning_files(
        &self,
        profile: &OptimizerProfile,
        nginx_tuning_path: &Path,
    ) -> Result<(), AgentError> {
        let sysctl_content = Self::render_sysctl_conf(profile);
        if let Some(parent) = self.sysctl_conf_path.parent() {
            self.runner.create_dir_all(parent, 0o755, 0, 0)?;
        }
        self.runner
            .write_file(&self.sysctl_conf_path, sysctl_content.as_bytes(), 0o644)?;

        let nginx_content = Self::render_nginx_tuning_conf(profile);
        self.runner
            .create_dir_all(&self.nginx_conf_d, 0o755, 0, 0)?;
        self.runner
            .write_file(nginx_tuning_path, nginx_content.as_bytes(), 0o644)?;
        Ok(())
    }

    fn rollback_failed_apply(
        &self,
        had_nginx_file: bool,
        old_nginx: Option<&[u8]>,
        had_sysctl_file: bool,
        old_sysctl: Option<&[u8]>,
        nginx_tuning_path: &Path,
    ) {
        if had_nginx_file {
            if let Some(c) = old_nginx {
                let _ = self.runner.write_file(nginx_tuning_path, c, 0o644);
            }
        } else {
            let _ = self.runner.remove_file(nginx_tuning_path);
        }

        if had_sysctl_file {
            if let Some(c) = old_sysctl {
                let _ = self.runner.write_file(&self.sysctl_conf_path, c, 0o644);
            }
        } else {
            let _ = self.runner.remove_file(&self.sysctl_conf_path);
        }
    }

    fn apply_services(&self) -> (bool, bool) {
        let mut sysctl_applied = true;
        if let Err(e) = self.runner.run_command("sysctl", &["--system"]) {
            warn!("sysctl --system warning (may be running in container/test): {e}");
            sysctl_applied = false;
        }

        let mut nginx_reloaded = true;
        if let Err(e) = self.runner.run_command("systemctl", &["reload", "nginx"])
            && let Err(e2) = self.runner.run_command("nginx", &["-s", "reload"])
        {
            warn!("Nginx reload command warning: {e}; fallback: {e2}");
            nginx_reloaded = false;
        }

        (sysctl_applied, nginx_reloaded)
    }

    /// Apply tuning profile atomically with validation and automatic rollback.
    pub fn apply(&self, req: &OptimizerApplyRequest) -> Result<OptimizerApplyResponse, AgentError> {
        let hardware = self.detect_hardware()?;
        let profile = if let Some(ref custom) = req.custom_tuning {
            custom.clone()
        } else {
            self.generate_profile_for_type(req.profile, &hardware)
        };

        let backup_dir = self.base_dir.join("backup");
        self.runner.create_dir_all(&backup_dir, 0o750, 0, 0)?;

        let previous_state = self.load_state();
        let previous_profile = previous_state.as_ref().map(|s| s.active_profile);
        let nginx_tuning_path = self.nginx_conf_d.join("tuning.conf");

        // 1. Snapshot previous files for rollback
        let snapshot =
            self.take_backup_snapshot(&backup_dir, previous_profile, &nginx_tuning_path)?;

        // 2. Render and write new configurations
        self.write_tuning_files(&profile, &nginx_tuning_path)?;

        // 3. Syntax validation: nginx -t
        let nginx_valid = self
            .runner
            .run_command("nginx", &["-t"])
            .map(|out| out.status.success())
            .unwrap_or(false);

        if !nginx_valid {
            warn!(
                "Nginx test failed after applying optimizer profile. Performing automatic rollback."
            );
            self.rollback_failed_apply(
                snapshot.meta.had_nginx_file,
                snapshot.old_nginx.as_deref(),
                snapshot.meta.had_sysctl_file,
                snapshot.old_sysctl.as_deref(),
                &nginx_tuning_path,
            );
            return Err(AgentError::InvalidInput(
                "Nginx configuration validation failed (-t). Tuning changes were automatically rolled back to prevent downtime.".to_string(),
            ));
        }

        // 4. Apply sysctl parameters & reload Nginx
        let (sysctl_applied, nginx_reloaded) = self.apply_services();

        // 5. Record state
        let state_record = OptimizerStateRecord {
            active_profile: req.profile,
            applied_at: Self::now_secs(),
            current_settings: profile,
            has_backup: true,
            last_backup_at: Some(snapshot.meta.backed_up_at),
        };
        self.save_state(&state_record)?;

        info!("Successfully applied optimizer profile: {:?}", req.profile);

        Ok(OptimizerApplyResponse {
            success: true,
            applied_profile: req.profile,
            message: format!(
                "Profil tuning '{}' berhasil diterapkan pada sistem.",
                state_record.current_settings.display_name
            ),
            backup_path: Some(backup_dir.to_string_lossy().to_string()),
            nginx_reloaded,
            sysctl_applied,
        })
    }

    /// Rollback configurations to previous state from backup snapshot.
    pub fn rollback(
        &self,
        _req: &OptimizerRollbackRequest,
    ) -> Result<OptimizerRollbackResponse, AgentError> {
        let backup_dir = self.base_dir.join("backup");
        let meta_path = backup_dir.join("meta.json");

        if !self.runner.path_exists(&meta_path) {
            return Err(AgentError::InvalidInput(
                "Tidak ditemukan snapshot backup konfigurasi untuk melakukan rollback.".to_string(),
            ));
        }

        let meta_bytes = self.runner.read_file(&meta_path)?;
        let meta: BackupMetadata = serde_json::from_slice(&meta_bytes)
            .map_err(|e| AgentError::Internal(format!("Failed to parse backup metadata: {e}")))?;

        let backup_sysctl_file = backup_dir.join("99-panel-tuning.conf.bak");
        let backup_nginx_file = backup_dir.join("tuning.conf.bak");
        let nginx_tuning_path = self.nginx_conf_d.join("tuning.conf");

        // Restore Nginx
        if meta.had_nginx_file && self.runner.path_exists(&backup_nginx_file) {
            let bytes = self.runner.read_file(&backup_nginx_file)?;
            self.runner.write_file(&nginx_tuning_path, &bytes, 0o644)?;
        } else if self.runner.path_exists(&nginx_tuning_path) {
            let _ = self.runner.remove_file(&nginx_tuning_path);
        }

        // Restore Sysctl
        if meta.had_sysctl_file && self.runner.path_exists(&backup_sysctl_file) {
            let bytes = self.runner.read_file(&backup_sysctl_file)?;
            self.runner
                .write_file(&self.sysctl_conf_path, &bytes, 0o644)?;
        } else if self.runner.path_exists(&self.sysctl_conf_path) {
            let _ = self.runner.remove_file(&self.sysctl_conf_path);
        }

        // Apply sysctl & reload nginx
        let _ = self.runner.run_command("sysctl", &["--system"]);
        let _ = self.runner.run_command("systemctl", &["reload", "nginx"]);

        let hardware = self.detect_hardware()?;
        let restored_profile = meta.previous_profile;

        let restored_settings = if let Some(p) = restored_profile {
            self.generate_profile_for_type(p, &hardware)
        } else {
            self.generate_profile_for_type(OptimizerProfileType::Standard, &hardware)
        };

        let now = Self::now_secs();
        let state_record = OptimizerStateRecord {
            active_profile: restored_profile.unwrap_or(OptimizerProfileType::Standard),
            applied_at: now,
            current_settings: restored_settings,
            has_backup: false,
            last_backup_at: None,
        };
        let _ = self.save_state(&state_record);

        // Remove old backup metadata so rollback cannot be triggered twice consecutively
        let _ = self.runner.remove_file(&meta_path);
        let _ = self.runner.remove_file(&backup_sysctl_file);
        let _ = self.runner.remove_file(&backup_nginx_file);

        info!("Successfully rolled back optimizer configuration.");

        Ok(OptimizerRollbackResponse {
            success: true,
            restored_profile,
            message: "Konfigurasi server berhasil di-rollback ke snapshot sebelumnya.".to_string(),
            restored_at: now,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;

    #[test]
    fn test_detect_hardware_defaults() {
        let runner = Arc::new(MockSystemRunner::new());
        let mgr = OptimizerManager::new(runner);

        let specs = mgr.detect_hardware().expect("detect specs");
        assert!(specs.vcpu_count >= 1);
        assert!(specs.ram_total_mb > 0);
        assert!(!specs.kernel_version.is_empty());
        assert!(!specs.os_distro.is_empty());
    }

    #[test]
    fn test_detect_hardware_from_proc_files() {
        let runner = Arc::new(MockSystemRunner::new());
        let cpuinfo = "processor\t: 0\nprocessor\t: 1\nprocessor\t: 2\nprocessor\t: 3\n";
        let meminfo = "MemTotal:        8388608 kB\nMemFree:         4194304 kB\nMemAvailable:    5242880 kB\nSwapTotal:       2097152 kB\nSwapFree:        2097152 kB\n";
        let version = "Linux version 6.8.0-45-generic (buildd@lcy02-amd64-001)\n";
        let os_rel = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\nVERSION_ID=\"24.04\"\n";

        runner
            .write_file(Path::new("/proc/cpuinfo"), cpuinfo.as_bytes(), 0o644)
            .unwrap();
        runner
            .write_file(Path::new("/proc/meminfo"), meminfo.as_bytes(), 0o644)
            .unwrap();
        runner
            .write_file(Path::new("/proc/version"), version.as_bytes(), 0o644)
            .unwrap();
        runner
            .write_file(Path::new("/etc/os-release"), os_rel.as_bytes(), 0o644)
            .unwrap();

        let mgr = OptimizerManager::new(runner);
        let specs = mgr.detect_hardware().expect("detect specs");
        assert_eq!(specs.vcpu_count, 4);
        assert_eq!(specs.ram_total_mb, 8192);
        assert_eq!(specs.ram_free_mb, 5120);
        assert_eq!(specs.swap_total_mb, 2048);
        assert_eq!(specs.os_distro, "Ubuntu 24.04 LTS");
        assert!(specs.kernel_version.contains("6.8.0-45-generic"));
    }

    #[test]
    fn test_profile_recommendation_and_generation() {
        let runner = Arc::new(MockSystemRunner::new());
        let mgr = OptimizerManager::new(runner);

        let micro_specs = ServerHardwareSpecs {
            vcpu_count: 1,
            ram_total_mb: 1024,
            ram_free_mb: 512,
            swap_total_mb: 512,
            swap_free_mb: 512,
            kernel_version: "Linux 6.8".to_string(),
            os_distro: "Ubuntu".to_string(),
        };
        assert_eq!(
            mgr.recommend_profile_type(&micro_specs),
            OptimizerProfileType::Micro
        );
        let micro_p = mgr.generate_profile_for_type(OptimizerProfileType::Micro, &micro_specs);
        assert_eq!(micro_p.nginx.worker_connections, 1024);
        assert_eq!(micro_p.sysctl.somaxconn, 1024);
        assert_eq!(micro_p.php_fpm.max_children, 10);

        let std_specs = ServerHardwareSpecs {
            ram_total_mb: 3840,
            vcpu_count: 2,
            ..micro_specs.clone()
        };
        assert_eq!(
            mgr.recommend_profile_type(&std_specs),
            OptimizerProfileType::Standard
        );

        let perf_specs = ServerHardwareSpecs {
            ram_total_mb: 8192,
            vcpu_count: 4,
            ..micro_specs.clone()
        };
        assert_eq!(
            mgr.recommend_profile_type(&perf_specs),
            OptimizerProfileType::HighPerformance
        );

        let ded_specs = ServerHardwareSpecs {
            ram_total_mb: 32768,
            vcpu_count: 16,
            ..micro_specs
        };
        assert_eq!(
            mgr.recommend_profile_type(&ded_specs),
            OptimizerProfileType::DedicatedLarge
        );
    }

    #[test]
    fn test_render_sysctl_and_nginx_configs() {
        let runner = Arc::new(MockSystemRunner::new());
        let mgr = OptimizerManager::new(runner);
        let specs = mgr.detect_hardware().unwrap();
        let profile = mgr.generate_profile_for_type(OptimizerProfileType::Standard, &specs);

        let sysctl = OptimizerManager::render_sysctl_conf(&profile);
        assert!(sysctl.contains("net.core.somaxconn = 4096"));
        assert!(sysctl.contains("vm.swappiness = 15"));

        let nginx = OptimizerManager::render_nginx_tuning_conf(&profile);
        assert!(nginx.contains("client_body_buffer_size 64k"));
        assert!(nginx.contains("fastcgi_buffers 8 32k"));
        assert!(nginx.contains("open_file_cache max=5000"));
    }

    #[test]
    fn test_apply_tuning_success_and_state() {
        let runner = Arc::new(MockSystemRunner::new());
        let temp_dir = PathBuf::from("/tmp/test_optimizer_apply");
        let sysctl_path = temp_dir.join("etc/sysctl.d/99-panel-tuning.conf");
        let nginx_conf_d = temp_dir.join("etc/panel/nginx/conf.d");
        let base_dir = temp_dir.join("var/lib/panel/optimizer");

        let mgr = OptimizerManager::with_paths(
            runner.clone(),
            base_dir.clone(),
            sysctl_path.clone(),
            nginx_conf_d.clone(),
        );

        let status_before = mgr.get_status().expect("status before");
        assert!(status_before.active_profile.is_none());

        let req = OptimizerApplyRequest {
            profile: OptimizerProfileType::Standard,
            custom_tuning: None,
        };
        let res = mgr.apply(&req).expect("apply standard");
        assert!(res.success);
        assert_eq!(res.applied_profile, OptimizerProfileType::Standard);

        // Verify config files are written
        assert!(runner.path_exists(&sysctl_path));
        let tuning_nginx = nginx_conf_d.join("tuning.conf");
        assert!(runner.path_exists(&tuning_nginx));

        let status_after = mgr.get_status().expect("status after");
        assert_eq!(
            status_after.active_profile,
            Some(OptimizerProfileType::Standard)
        );
        assert!(status_after.has_backup);
    }

    #[test]
    fn test_apply_tuning_syntax_error_automatic_rollback() {
        let runner = Arc::new(MockSystemRunner::new());
        // Set mock command for nginx to fail (-t fails)
        runner.set_response(
            "nginx -t",
            1,
            &[],
            b"nginx: [emerg] invalid parameter in /etc/panel/nginx/conf.d/tuning.conf",
        );

        let temp_dir = PathBuf::from("/tmp/test_optimizer_syntax_error");
        let sysctl_path = temp_dir.join("sysctl.conf");
        let nginx_conf_d = temp_dir.join("conf.d");
        let base_dir = temp_dir.join("state");

        let mgr = OptimizerManager::with_paths(
            runner.clone(),
            base_dir,
            sysctl_path.clone(),
            nginx_conf_d.clone(),
        );

        let req = OptimizerApplyRequest {
            profile: OptimizerProfileType::HighPerformance,
            custom_tuning: None,
        };
        let err = mgr.apply(&req).unwrap_err();
        match err {
            AgentError::InvalidInput(msg) => {
                assert!(msg.contains("validation failed (-t)"));
                assert!(msg.contains("automatically rolled back"));
            }
            other => panic!("Expected InvalidInput error, got: {other:?}"),
        }

        // New files should NOT exist because they were rolled back
        assert!(!runner.path_exists(&nginx_conf_d.join("tuning.conf")));
        assert!(!runner.path_exists(&sysctl_path));
    }

    #[test]
    fn test_rollback_restores_previous_configuration() {
        let runner = Arc::new(MockSystemRunner::new());
        let temp_dir = PathBuf::from("/tmp/test_optimizer_rollback");
        let sysctl_path = temp_dir.join("sysctl.conf");
        let nginx_conf_d = temp_dir.join("conf.d");
        let base_dir = temp_dir.join("state");

        let mgr = OptimizerManager::with_paths(
            runner.clone(),
            base_dir,
            sysctl_path.clone(),
            nginx_conf_d.clone(),
        );

        // Apply Micro
        let req1 = OptimizerApplyRequest {
            profile: OptimizerProfileType::Micro,
            custom_tuning: None,
        };
        mgr.apply(&req1).expect("apply micro");

        // Apply Standard (creates backup of Micro)
        let req2 = OptimizerApplyRequest {
            profile: OptimizerProfileType::Standard,
            custom_tuning: None,
        };
        mgr.apply(&req2).expect("apply standard");

        let status2 = mgr.get_status().unwrap();
        assert_eq!(status2.active_profile, Some(OptimizerProfileType::Standard));

        // Trigger rollback
        let rb_res = mgr
            .rollback(&OptimizerRollbackRequest {})
            .expect("rollback");
        assert!(rb_res.success);
        assert_eq!(rb_res.restored_profile, Some(OptimizerProfileType::Micro));

        let status_rb = mgr.get_status().unwrap();
        assert_eq!(status_rb.active_profile, Some(OptimizerProfileType::Micro));
    }
}
