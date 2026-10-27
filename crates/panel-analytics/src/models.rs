use serde::{Deserialize, Serialize};

/// Metric point for server/host level resource utilization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerMetricsPoint {
    pub timestamp: i64,
    pub cpu_percent: f64,
    pub cpu_cores: u32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub inodes_used: u64,
    pub inodes_total: u64,
    pub net_rx_bytes_sec: f64,
    pub net_tx_bytes_sec: f64,
    pub io_read_bytes_sec: f64,
    pub io_write_bytes_sec: f64,
}

/// Metric point for an individual tenant's cgroup slice (`panel-u{uid}.slice`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TenantMetricsPoint {
    pub timestamp: i64,
    pub uid: u32,
    pub cpu_usage_usec: u64,
    pub cpu_percent: f64,
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub memory_swap_bytes: Option<u64>,
    pub pids_current: u64,
    pub io_read_bytes_sec: f64,
    pub io_write_bytes_sec: f64,
}

/// Metric point for a system service cgroup (`system.slice/{service}.service`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceMetricsPoint {
    pub timestamp: i64,
    pub service_name: String,
    pub cpu_usage_usec: u64,
    pub cpu_percent: f64,
    pub memory_current_bytes: u64,
    pub pids_current: u64,
    pub io_read_bytes_sec: f64,
    pub io_write_bytes_sec: f64,
}

/// Target resolution interval for rollups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollupResolution {
    Raw,
    OneMinute,
    OneHour,
    OneDay,
}

impl RollupResolution {
    pub fn step_secs(&self) -> i64 {
        match self {
            Self::Raw => 10,
            Self::OneMinute => 60,
            Self::OneHour => 3600,
            Self::OneDay => 86400,
        }
    }
}

/// Aggregated rollup point for server metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerMetricsRollup {
    pub bucket_timestamp: i64,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub cpu_min: f64,
    pub memory_used_avg: f64,
    pub memory_used_max: u64,
    pub swap_used_avg: f64,
    pub swap_used_max: u64,
    pub load1_avg: f64,
    pub load1_max: f64,
    pub disk_used_avg: f64,
    pub disk_used_max: u64,
    pub net_rx_avg: f64,
    pub net_rx_max: f64,
    pub net_tx_avg: f64,
    pub net_tx_max: f64,
    pub io_read_avg: f64,
    pub io_read_max: f64,
    pub io_write_avg: f64,
    pub io_write_max: f64,
    pub sample_count: u32,
}

/// Aggregated rollup point for tenant metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TenantMetricsRollup {
    pub bucket_timestamp: i64,
    pub uid: u32,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub memory_avg: f64,
    pub memory_max: u64,
    pub pids_avg: f64,
    pub pids_max: u64,
    pub io_read_avg: f64,
    pub io_write_avg: f64,
    pub sample_count: u32,
}

/// Aggregated rollup point for service metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceMetricsRollup {
    pub bucket_timestamp: i64,
    pub service_name: String,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub memory_avg: f64,
    pub memory_max: u64,
    pub pids_avg: f64,
    pub pids_max: u64,
    pub io_read_avg: f64,
    pub io_write_avg: f64,
    pub sample_count: u32,
}

/// Retention configuration adhering to PRD 5.6:
/// - 10s raw: 2 hours (7200s)
/// - 1m rollups: 14 days (14 * 86400s)
/// - 1h rollups: 400 days (400 * 86400s)
/// - 1d rollups: indefinite
/// - Security events: 7 days (7 * 86400s)
/// - Database size limit: 1024 MB
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionConfig {
    pub raw_retention_secs: i64,
    pub one_minute_retention_secs: i64,
    pub one_hour_retention_secs: i64,
    pub one_day_retention_secs: Option<i64>,
    #[serde(default = "default_security_events_retention_secs")]
    pub security_events_retention_secs: i64,
    #[serde(default = "default_max_db_size_bytes")]
    pub max_db_size_bytes: Option<i64>,
    #[serde(default)]
    pub auto_vacuum: bool,
}

fn default_security_events_retention_secs() -> i64 {
    7 * 86400
}

fn default_max_db_size_bytes() -> Option<i64> {
    Some(1024 * 1024 * 1024)
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            raw_retention_secs: 2 * 3600,                // 2 hours
            one_minute_retention_secs: 14 * 86400,       // 14 days
            one_hour_retention_secs: 400 * 86400,        // 400 days
            one_day_retention_secs: None,                // Indefinite
            security_events_retention_secs: 7 * 86400,   // 7 days (PRD 5.6)
            max_db_size_bytes: Some(1024 * 1024 * 1024), // 1024 MB
            auto_vacuum: false,
        }
    }
}

/// Single table row count statistic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseTableStat {
    pub table_name: String,
    pub row_count: i64,
}

/// Detailed storage and fragmentation metrics for analytics SQLite database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatabaseStorageStats {
    pub db_size_bytes: i64,
    pub wal_size_bytes: i64,
    pub total_file_size_bytes: i64,
    pub page_size: i64,
    pub page_count: i64,
    pub freelist_count: i64,
    pub free_bytes: i64,
    pub fragmentation_ratio: f64,
    pub max_db_size_bytes: Option<i64>,
    pub is_healthy: bool,
    pub integrity_message: String,
    pub table_stats: Vec<DatabaseTableStat>,
}

/// Comprehensive report after executing a retention maintenance cycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionMaintenanceReport {
    pub timestamp: i64,
    pub raw_records_pruned: usize,
    pub one_minute_records_pruned: usize,
    pub one_hour_records_pruned: usize,
    pub one_day_records_pruned: usize,
    pub security_events_pruned: usize,
    pub checkpoints_pruned: usize,
    pub total_records_pruned: usize,
    pub wal_checkpoint_executed: bool,
    pub vacuum_executed: bool,
    pub db_size_bytes_before: i64,
    pub db_size_bytes_after: i64,
    pub wal_size_bytes_before: i64,
    pub wal_size_bytes_after: i64,
    pub duration_ms: u64,
    pub emergency_pruning_triggered: bool,
}

/// Report after performing SQLite PRAGMA quick_check / integrity_check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrityCheckReport {
    pub is_healthy: bool,
    pub status: String,
    pub checked_at: i64,
    pub details: Vec<String>,
}

/// Report after database corruption recovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryReport {
    pub recovered_at: i64,
    pub backup_path: Option<String>,
    pub success: bool,
    pub message: String,
}

/// Report after performing a metrics collection iteration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricsCollectionReport {
    pub timestamp: i64,
    pub server_collected: bool,
    pub tenants_collected: usize,
    pub services_collected: usize,
}

/// Report after running rollup computation and retention pruning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollupReport {
    pub timestamp: i64,
    pub server_1m_created: usize,
    pub server_1h_created: usize,
    pub server_1d_created: usize,
    pub tenant_1m_created: usize,
    pub tenant_1h_created: usize,
    pub tenant_1d_created: usize,
    pub service_1m_created: usize,
    pub service_1h_created: usize,
    pub service_1d_created: usize,
    pub pruned_records: usize,
}

/// Parsed, structured log entry from Nginx `panel_json` log stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedLogEntry {
    pub site_id: i64,
    pub timestamp: i64,
    pub client_ip: String,
    pub host: String,
    pub method: String,
    pub uri: String,
    pub status: u16,
    pub body_bytes_sent: u64,
    pub request_time_secs: f64,
    pub upstream_response_time_secs: Option<f64>,
    pub upstream_cache_status: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub protocol: Option<String>,
    pub tls_version: Option<String>,
}

/// Checkpoint offset state for crash safety and rotation recovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogCheckpoint {
    pub file_path: String,
    pub site_id: i64,
    pub inode: u64,
    pub file_offset: u64,
    pub last_processed_ts: i64,
    pub updated_at: i64,
}

/// A batch of parsed log lines along with file coordinate updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogBatch {
    pub file_path: String,
    pub site_id: i64,
    pub inode: u64,
    pub new_offset: u64,
    pub entries: Vec<ParsedLogEntry>,
}

/// Configuration parameters for log tailing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogTailerConfig {
    pub poll_interval_ms: u64,
    pub batch_size: usize,
    pub channel_capacity: usize,
    pub checkpoint_interval_ms: u64,
}

impl Default for LogTailerConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 500,
            batch_size: 100,
            channel_capacity: 256,
            checkpoint_interval_ms: 1000,
        }
    }
}

/// Configuration parameters for the multi-file log tailer engine.
#[derive(Debug, Clone)]
pub struct TailerEngineConfig {
    pub watch_directories: Vec<std::path::PathBuf>,
    pub poll_interval: std::time::Duration,
    pub batch_size: usize,
    pub channel_capacity: usize,
}

impl Default for TailerEngineConfig {
    fn default() -> Self {
        Self {
            watch_directories: vec![
                std::path::PathBuf::from("/var/log/panel/sites"),
                std::path::PathBuf::from("/var/log/nginx"),
            ],
            poll_interval: std::time::Duration::from_millis(500),
            batch_size: 100,
            channel_capacity: 256,
        }
    }
}

/// Real-time statistics of the log tailer engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TailerStats {
    pub active_tailers: usize,
    pub total_lines_processed: u64,
    pub total_bytes_processed: u64,
    pub total_batches_processed: u64,
}

/// Generic item count representation for top-K metrics (paths, referrers, countries, browsers, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopItem {
    pub name: String,
    pub count: u64,
}

/// Slow URL entry with average request duration and invocation count.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlowUrlItem {
    pub uri: String,
    pub avg_time_ms: f64,
    pub count: u64,
}

/// Error URL entry with HTTP status code and occurrence count.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorUrlItem {
    pub uri: String,
    pub status: u16,
    pub count: u64,
}

/// 1-Minute aggregated web traffic metric point for an individual site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SiteMetricsPoint {
    pub site_id: i64,
    pub bucket_ts: i64,
    pub total_requests: u64,
    pub total_bytes: u64,
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    pub cache_hit: u64,
    pub cache_miss: u64,
    pub cache_bypass: u64,
    pub cache_other: u64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub latency_avg_ms: f64,
    pub upstream_time_avg_ms: Option<f64>,
    pub unique_visitors: u64,
    pub human_requests: u64,
    pub bot_requests: u64,
    pub http_v1: u64,
    pub http_v2: u64,
    pub http_v3: u64,
    pub tls_v12: u64,
    pub tls_v13: u64,
    pub tls_other: u64,
    pub top_paths: Vec<TopItem>,
    pub top_referrers: Vec<TopItem>,
    pub top_countries: Vec<TopItem>,
    pub top_browsers: Vec<TopItem>,
    pub top_os: Vec<TopItem>,
    pub top_devices: Vec<TopItem>,
    pub top_slow_urls: Vec<SlowUrlItem>,
    pub top_error_urls: Vec<ErrorUrlItem>,
    pub latency_buckets: Vec<u64>,
}

/// Aggregated 1-Hour or 1-Day rollup for an individual site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SiteMetricsRollup {
    pub site_id: i64,
    pub bucket_ts: i64,
    pub total_requests: u64,
    pub total_bytes: u64,
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    pub cache_hit: u64,
    pub cache_miss: u64,
    pub cache_bypass: u64,
    pub cache_other: u64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub latency_avg_ms: f64,
    pub upstream_time_avg_ms: Option<f64>,
    pub unique_visitors: u64,
    pub human_requests: u64,
    pub bot_requests: u64,
    pub http_v1: u64,
    pub http_v2: u64,
    pub http_v3: u64,
    pub tls_v12: u64,
    pub tls_v13: u64,
    pub tls_other: u64,
    pub top_paths: Vec<TopItem>,
    pub top_referrers: Vec<TopItem>,
    pub top_countries: Vec<TopItem>,
    pub top_browsers: Vec<TopItem>,
    pub top_os: Vec<TopItem>,
    pub top_devices: Vec<TopItem>,
    pub top_slow_urls: Vec<SlowUrlItem>,
    pub top_error_urls: Vec<ErrorUrlItem>,
    pub latency_buckets: Vec<u64>,
    pub sample_count: u32,
}

/// Report after running site rollups and site retention pruning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteRollupReport {
    pub timestamp: i64,
    pub site_1h_created: usize,
    pub site_1d_created: usize,
    pub pruned_1m_records: usize,
    pub pruned_1h_records: usize,
}

/// Raw metric point for PHP-FPM pool status (10s interval).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhpFpmPoolMetricPoint {
    pub timestamp: i64,
    pub pool_name: String,
    pub user: Option<String>,
    pub php_version: Option<String>,
    pub active_processes: u32,
    pub idle_processes: u32,
    pub total_processes: u32,
    pub max_active_processes: u32,
    pub max_children_reached: u32,
    pub listen_queue: u32,
    pub max_listen_queue: u32,
    pub listen_queue_len: u32,
    pub slow_requests: u64,
    pub accepted_conn: u64,
}

/// 1-minute rollup for PHP-FPM pool metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhpFpmPoolMetricRollup {
    pub pool_name: String,
    pub bucket_ts: i64,
    pub active_avg: f64,
    pub active_max: u32,
    pub idle_avg: f64,
    pub total_avg: f64,
    pub total_max: u32,
    pub listen_queue_avg: f64,
    pub listen_queue_max: u32,
    pub max_children_reached_max: u32,
    pub slow_requests_delta: u64,
    pub accepted_conn_delta: u64,
    pub sample_count: u32,
}

/// A single frame within a PHP-FPM slowlog stack trace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpSlowlogFrame {
    pub frame_index: usize,
    pub function: String,
    pub file: String,
    pub line: u32,
    pub address: Option<String>,
}

/// A parsed slowlog execution event for a tenant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhpSlowlogEntry {
    pub id: Option<i64>,
    pub timestamp: i64,
    pub pool: String,
    pub pid: u32,
    pub script_filename: String,
    pub top_function: Option<String>,
    pub frames: Vec<PhpSlowlogFrame>,
    pub duration_secs: Option<f64>,
    pub site_id: Option<i64>,
    pub tenant_user: Option<String>,
}

/// Application runtime type for transient cgroup monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    Node,
    Python,
}

impl AppType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
        }
    }
}

impl std::str::FromStr for AppType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "node" | "nodejs" => Ok(Self::Node),
            "python" | "py" => Ok(Self::Python),
            other => Err(format!("Unknown app type: {other}")),
        }
    }
}

/// Raw metric point for Node.js / Python application service (10s interval).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppProcessMetricPoint {
    pub timestamp: i64,
    pub site_id: i64,
    pub app_type: AppType,
    pub service_name: String,
    pub cpu_usage_usec: u64,
    pub cpu_percent: f64,
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub memory_swap_bytes: Option<u64>,
    pub pids_current: u64,
    pub io_read_bytes_sec: f64,
    pub io_write_bytes_sec: f64,
}

/// 1-minute rollup for Node.js / Python application service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppProcessMetricRollup {
    pub site_id: i64,
    pub app_type: AppType,
    pub bucket_ts: i64,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub memory_current_avg: f64,
    pub memory_current_max: u64,
    pub memory_peak_max: Option<u64>,
    pub pids_max: u64,
    pub io_read_avg: f64,
    pub io_write_avg: f64,
    pub sample_count: u32,
}
