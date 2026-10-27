use crate::error::AnalyticsError;
use crate::models::{
    AppProcessMetricPoint, AppProcessMetricRollup, AppType, DatabaseStorageStats,
    DatabaseTableStat, IntegrityCheckReport, LogCheckpoint, PhpFpmPoolMetricPoint,
    PhpFpmPoolMetricRollup, PhpSlowlogEntry, PhpSlowlogFrame, RecoveryReport, RetentionConfig,
    RetentionMaintenanceReport, RollupResolution, ServerMetricsPoint, ServerMetricsRollup,
    ServiceMetricsPoint, ServiceMetricsRollup, SiteMetricsPoint, SiteMetricsRollup,
    SiteRollupReport, TenantMetricsPoint, TenantMetricsRollup,
};
use crate::rollup::SiteMetricsRollupProcessor;
use crate::security::{
    BruteForceIncident, ProbingPathStat, SecurityEvent, SecurityOverviewSummary, TopSecurityIp,
    calculate_ip_risk_level, classify_probing_path,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Row, Sqlite};
use std::path::Path;
use std::str::FromStr;

#[derive(Clone)]
pub struct AnalyticsStorage {
    pool: Pool<Sqlite>,
    write_pool: Pool<Sqlite>,
    db_path: Option<String>,
}

impl AnalyticsStorage {
    /// Create new storage instance connected to SQLite at specified path.
    pub async fn connect(db_path: impl AsRef<Path>) -> Result<Self, AnalyticsError> {
        let path_str = db_path.as_ref().to_string_lossy().to_string();
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{path_str}"))
            .map_err(AnalyticsError::Database)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(15));

        let write_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect_with(options.clone())
            .await
            .map_err(AnalyticsError::Database)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect_with(options.pragma("query_only", "ON"))
            .await
            .map_err(AnalyticsError::Database)?;

        let storage = Self {
            pool,
            write_pool,
            db_path: Some(path_str),
        };
        storage.init_schema().await?;
        Ok(storage)
    }

    /// Create in-memory storage for unit testing.
    pub async fn connect_in_memory() -> Result<Self, AnalyticsError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .map_err(AnalyticsError::Database)?;

        let storage = Self {
            write_pool: pool.clone(),
            pool,
            db_path: None,
        };
        storage.init_schema().await?;
        Ok(storage)
    }

    /// Single-writer pool for explicit maintenance and fixture writes.
    pub fn write_pool(&self) -> &Pool<Sqlite> {
        &self.write_pool
    }

    pub fn db_path(&self) -> Option<&str> {
        self.db_path.as_deref()
    }

    /// Latest raw sample: available immediately, without waiting for minute rollups.
    pub async fn latest_server_point(&self) -> Result<Option<ServerMetricsPoint>, AnalyticsError> {
        let row = sqlx::query("SELECT * FROM metrics_server_raw ORDER BY timestamp DESC LIMIT 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;
        Ok(row.map(|r| ServerMetricsPoint {
            timestamp: r.get("timestamp"),
            cpu_percent: r.get("cpu_percent"),
            cpu_cores: r.get::<i64, _>("cpu_cores") as u32,
            memory_used_bytes: r.get::<i64, _>("memory_used_bytes") as u64,
            memory_total_bytes: r.get::<i64, _>("memory_total_bytes") as u64,
            swap_used_bytes: r.get::<i64, _>("swap_used_bytes") as u64,
            swap_total_bytes: r.get::<i64, _>("swap_total_bytes") as u64,
            load1: r.get("load1"),
            load5: r.get("load5"),
            load15: r.get("load15"),
            disk_used_bytes: r.get::<i64, _>("disk_used_bytes") as u64,
            disk_total_bytes: r.get::<i64, _>("disk_total_bytes") as u64,
            inodes_used: r.get::<i64, _>("inodes_used") as u64,
            inodes_total: r.get::<i64, _>("inodes_total") as u64,
            net_rx_bytes_sec: r.get("net_rx_bytes_sec"),
            net_tx_bytes_sec: r.get("net_tx_bytes_sec"),
            io_read_bytes_sec: r.get("io_read_bytes_sec"),
            io_write_bytes_sec: r.get("io_write_bytes_sec"),
        }))
    }

    /// Initialize database schema.
    pub async fn init_schema(&self) -> Result<(), AnalyticsError> {
        let schema = r#"
        -- 1. Server metrics: Raw (10s)
        CREATE TABLE IF NOT EXISTS metrics_server_raw (
            timestamp INTEGER NOT NULL PRIMARY KEY,
            cpu_percent REAL NOT NULL,
            cpu_cores INTEGER NOT NULL,
            memory_used_bytes INTEGER NOT NULL,
            memory_total_bytes INTEGER NOT NULL,
            swap_used_bytes INTEGER NOT NULL,
            swap_total_bytes INTEGER NOT NULL,
            load1 REAL NOT NULL,
            load5 REAL NOT NULL,
            load15 REAL NOT NULL,
            disk_used_bytes INTEGER NOT NULL,
            disk_total_bytes INTEGER NOT NULL,
            inodes_used INTEGER NOT NULL,
            inodes_total INTEGER NOT NULL,
            net_rx_bytes_sec REAL NOT NULL,
            net_tx_bytes_sec REAL NOT NULL,
            io_read_bytes_sec REAL NOT NULL,
            io_write_bytes_sec REAL NOT NULL
        );

        -- 2. Server metrics: 1-minute rollup
        CREATE TABLE IF NOT EXISTS metrics_server_1m (
            bucket_ts INTEGER NOT NULL PRIMARY KEY,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            cpu_min REAL NOT NULL,
            memory_used_avg REAL NOT NULL,
            memory_used_max INTEGER NOT NULL,
            swap_used_avg REAL NOT NULL,
            swap_used_max INTEGER NOT NULL,
            load1_avg REAL NOT NULL,
            load1_max REAL NOT NULL,
            disk_used_avg REAL NOT NULL,
            disk_used_max INTEGER NOT NULL,
            net_rx_avg REAL NOT NULL,
            net_rx_max REAL NOT NULL,
            net_tx_avg REAL NOT NULL,
            net_tx_max REAL NOT NULL,
            io_read_avg REAL NOT NULL,
            io_read_max REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            io_write_max REAL NOT NULL,
            sample_count INTEGER NOT NULL
        );

        -- 3. Server metrics: 1-hour rollup
        CREATE TABLE IF NOT EXISTS metrics_server_1h (
            bucket_ts INTEGER NOT NULL PRIMARY KEY,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            cpu_min REAL NOT NULL,
            memory_used_avg REAL NOT NULL,
            memory_used_max INTEGER NOT NULL,
            swap_used_avg REAL NOT NULL,
            swap_used_max INTEGER NOT NULL,
            load1_avg REAL NOT NULL,
            load1_max REAL NOT NULL,
            disk_used_avg REAL NOT NULL,
            disk_used_max INTEGER NOT NULL,
            net_rx_avg REAL NOT NULL,
            net_rx_max REAL NOT NULL,
            net_tx_avg REAL NOT NULL,
            net_tx_max REAL NOT NULL,
            io_read_avg REAL NOT NULL,
            io_read_max REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            io_write_max REAL NOT NULL,
            sample_count INTEGER NOT NULL
        );

        -- 4. Server metrics: 1-day rollup
        CREATE TABLE IF NOT EXISTS metrics_server_1d (
            bucket_ts INTEGER NOT NULL PRIMARY KEY,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            cpu_min REAL NOT NULL,
            memory_used_avg REAL NOT NULL,
            memory_used_max INTEGER NOT NULL,
            swap_used_avg REAL NOT NULL,
            swap_used_max INTEGER NOT NULL,
            load1_avg REAL NOT NULL,
            load1_max REAL NOT NULL,
            disk_used_avg REAL NOT NULL,
            disk_used_max INTEGER NOT NULL,
            net_rx_avg REAL NOT NULL,
            net_rx_max REAL NOT NULL,
            net_tx_avg REAL NOT NULL,
            net_tx_max REAL NOT NULL,
            io_read_avg REAL NOT NULL,
            io_read_max REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            io_write_max REAL NOT NULL,
            sample_count INTEGER NOT NULL
        );

        -- 5. Tenant metrics: Raw (10s)
        CREATE TABLE IF NOT EXISTS metrics_tenant_raw (
            timestamp INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            cpu_usage_usec INTEGER NOT NULL,
            cpu_percent REAL NOT NULL,
            memory_current_bytes INTEGER NOT NULL,
            memory_peak_bytes INTEGER,
            memory_swap_bytes INTEGER,
            pids_current INTEGER NOT NULL,
            io_read_bytes_sec REAL NOT NULL,
            io_write_bytes_sec REAL NOT NULL,
            PRIMARY KEY (timestamp, uid)
        );
        CREATE INDEX IF NOT EXISTS idx_tenant_raw_uid_ts ON metrics_tenant_raw (uid, timestamp);

        -- 6. Tenant metrics: 1-minute rollup
        CREATE TABLE IF NOT EXISTS metrics_tenant_1m (
            bucket_ts INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            memory_avg REAL NOT NULL,
            memory_max INTEGER NOT NULL,
            pids_avg REAL NOT NULL,
            pids_max INTEGER NOT NULL,
            io_read_avg REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (bucket_ts, uid)
        );
        CREATE INDEX IF NOT EXISTS idx_tenant_1m_uid_ts ON metrics_tenant_1m (uid, bucket_ts);

        -- 7. Tenant metrics: 1-hour rollup
        CREATE TABLE IF NOT EXISTS metrics_tenant_1h (
            bucket_ts INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            memory_avg REAL NOT NULL,
            memory_max INTEGER NOT NULL,
            pids_avg REAL NOT NULL,
            pids_max INTEGER NOT NULL,
            io_read_avg REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (bucket_ts, uid)
        );
        CREATE INDEX IF NOT EXISTS idx_tenant_1h_uid_ts ON metrics_tenant_1h (uid, bucket_ts);

        -- 8. Tenant metrics: 1-day rollup
        CREATE TABLE IF NOT EXISTS metrics_tenant_1d (
            bucket_ts INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            memory_avg REAL NOT NULL,
            memory_max INTEGER NOT NULL,
            pids_avg REAL NOT NULL,
            pids_max INTEGER NOT NULL,
            io_read_avg REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (bucket_ts, uid)
        );
        CREATE INDEX IF NOT EXISTS idx_tenant_1d_uid_ts ON metrics_tenant_1d (uid, bucket_ts);

        -- 9. Service metrics: Raw (10s)
        CREATE TABLE IF NOT EXISTS metrics_service_raw (
            timestamp INTEGER NOT NULL,
            service_name TEXT NOT NULL,
            cpu_usage_usec INTEGER NOT NULL,
            cpu_percent REAL NOT NULL,
            memory_current_bytes INTEGER NOT NULL,
            pids_current INTEGER NOT NULL,
            io_read_bytes_sec REAL NOT NULL,
            io_write_bytes_sec REAL NOT NULL,
            PRIMARY KEY (timestamp, service_name)
        );
        CREATE INDEX IF NOT EXISTS idx_service_raw_name_ts ON metrics_service_raw (service_name, timestamp);

        -- 10. Service metrics: 1-minute rollup
        CREATE TABLE IF NOT EXISTS metrics_service_1m (
            bucket_ts INTEGER NOT NULL,
            service_name TEXT NOT NULL,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            memory_avg REAL NOT NULL,
            memory_max INTEGER NOT NULL,
            pids_avg REAL NOT NULL,
            pids_max INTEGER NOT NULL,
            io_read_avg REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (bucket_ts, service_name)
        );
        CREATE INDEX IF NOT EXISTS idx_service_1m_name_ts ON metrics_service_1m (service_name, bucket_ts);

        -- 11. Service metrics: 1-hour rollup
        CREATE TABLE IF NOT EXISTS metrics_service_1h (
            bucket_ts INTEGER NOT NULL,
            service_name TEXT NOT NULL,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            memory_avg REAL NOT NULL,
            memory_max INTEGER NOT NULL,
            pids_avg REAL NOT NULL,
            pids_max INTEGER NOT NULL,
            io_read_avg REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (bucket_ts, service_name)
        );
        CREATE INDEX IF NOT EXISTS idx_service_1h_name_ts ON metrics_service_1h (service_name, bucket_ts);

        -- 12. Service metrics: 1-day rollup
        CREATE TABLE IF NOT EXISTS metrics_service_1d (
            bucket_ts INTEGER NOT NULL,
            service_name TEXT NOT NULL,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            memory_avg REAL NOT NULL,
            memory_max INTEGER NOT NULL,
            pids_avg REAL NOT NULL,
            pids_max INTEGER NOT NULL,
            io_read_avg REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (bucket_ts, service_name)
        );
        CREATE INDEX IF NOT EXISTS idx_service_1d_name_ts ON metrics_service_1d (service_name, bucket_ts);

        -- Checkpoints table
        CREATE TABLE IF NOT EXISTS analytics_checkpoints (
            key TEXT PRIMARY KEY,
            last_timestamp INTEGER NOT NULL
        );

        -- Log tailer checkpoints table (P9-T2)
        CREATE TABLE IF NOT EXISTS log_tailer_checkpoints (
            file_path TEXT PRIMARY KEY,
            site_id INTEGER NOT NULL,
            inode INTEGER NOT NULL DEFAULT 0,
            file_offset INTEGER NOT NULL DEFAULT 0,
            last_processed_ts INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        -- 13. Site traffic metrics: 1-minute rollup (P9-T3)
        CREATE TABLE IF NOT EXISTS site_metrics_1m (
            site_id INTEGER NOT NULL,
            bucket_ts INTEGER NOT NULL,
            total_requests INTEGER NOT NULL,
            total_bytes INTEGER NOT NULL,
            status_2xx INTEGER NOT NULL,
            status_3xx INTEGER NOT NULL,
            status_4xx INTEGER NOT NULL,
            status_5xx INTEGER NOT NULL,
            cache_hit INTEGER NOT NULL,
            cache_miss INTEGER NOT NULL,
            cache_bypass INTEGER NOT NULL,
            cache_other INTEGER NOT NULL,
            latency_p50_ms REAL NOT NULL,
            latency_p95_ms REAL NOT NULL,
            latency_p99_ms REAL NOT NULL,
            latency_avg_ms REAL NOT NULL,
            upstream_time_avg_ms REAL,
            unique_visitors INTEGER NOT NULL,
            human_requests INTEGER NOT NULL,
            bot_requests INTEGER NOT NULL,
            http_v1 INTEGER NOT NULL,
            http_v2 INTEGER NOT NULL,
            http_v3 INTEGER NOT NULL,
            tls_v12 INTEGER NOT NULL,
            tls_v13 INTEGER NOT NULL,
            tls_other INTEGER NOT NULL,
            top_paths_json TEXT NOT NULL,
            top_referrers_json TEXT NOT NULL,
            top_countries_json TEXT NOT NULL,
            top_browsers_json TEXT NOT NULL,
            top_os_json TEXT NOT NULL,
            top_devices_json TEXT NOT NULL,
            top_slow_urls_json TEXT NOT NULL,
            top_error_urls_json TEXT NOT NULL,
            latency_buckets_json TEXT NOT NULL,
            PRIMARY KEY (site_id, bucket_ts)
        );
        CREATE INDEX IF NOT EXISTS idx_site_1m_site_ts ON site_metrics_1m (site_id, bucket_ts);
        CREATE INDEX IF NOT EXISTS idx_site_1m_ts ON site_metrics_1m (bucket_ts);

        -- 14. Site traffic metrics: 1-hour rollup (P9-T3)
        CREATE TABLE IF NOT EXISTS site_metrics_1h (
            site_id INTEGER NOT NULL,
            bucket_ts INTEGER NOT NULL,
            total_requests INTEGER NOT NULL,
            total_bytes INTEGER NOT NULL,
            status_2xx INTEGER NOT NULL,
            status_3xx INTEGER NOT NULL,
            status_4xx INTEGER NOT NULL,
            status_5xx INTEGER NOT NULL,
            cache_hit INTEGER NOT NULL,
            cache_miss INTEGER NOT NULL,
            cache_bypass INTEGER NOT NULL,
            cache_other INTEGER NOT NULL,
            latency_p50_ms REAL NOT NULL,
            latency_p95_ms REAL NOT NULL,
            latency_p99_ms REAL NOT NULL,
            latency_avg_ms REAL NOT NULL,
            upstream_time_avg_ms REAL,
            unique_visitors INTEGER NOT NULL,
            human_requests INTEGER NOT NULL,
            bot_requests INTEGER NOT NULL,
            http_v1 INTEGER NOT NULL,
            http_v2 INTEGER NOT NULL,
            http_v3 INTEGER NOT NULL,
            tls_v12 INTEGER NOT NULL,
            tls_v13 INTEGER NOT NULL,
            tls_other INTEGER NOT NULL,
            top_paths_json TEXT NOT NULL,
            top_referrers_json TEXT NOT NULL,
            top_countries_json TEXT NOT NULL,
            top_browsers_json TEXT NOT NULL,
            top_os_json TEXT NOT NULL,
            top_devices_json TEXT NOT NULL,
            top_slow_urls_json TEXT NOT NULL,
            top_error_urls_json TEXT NOT NULL,
            latency_buckets_json TEXT NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (site_id, bucket_ts)
        );
        CREATE INDEX IF NOT EXISTS idx_site_1h_site_ts ON site_metrics_1h (site_id, bucket_ts);
        CREATE INDEX IF NOT EXISTS idx_site_1h_ts ON site_metrics_1h (bucket_ts);

        -- 15. Site traffic metrics: 1-day rollup (P9-T3)
        CREATE TABLE IF NOT EXISTS site_metrics_1d (
            site_id INTEGER NOT NULL,
            bucket_ts INTEGER NOT NULL,
            total_requests INTEGER NOT NULL,
            total_bytes INTEGER NOT NULL,
            status_2xx INTEGER NOT NULL,
            status_3xx INTEGER NOT NULL,
            status_4xx INTEGER NOT NULL,
            status_5xx INTEGER NOT NULL,
            cache_hit INTEGER NOT NULL,
            cache_miss INTEGER NOT NULL,
            cache_bypass INTEGER NOT NULL,
            cache_other INTEGER NOT NULL,
            latency_p50_ms REAL NOT NULL,
            latency_p95_ms REAL NOT NULL,
            latency_p99_ms REAL NOT NULL,
            latency_avg_ms REAL NOT NULL,
            upstream_time_avg_ms REAL,
            unique_visitors INTEGER NOT NULL,
            human_requests INTEGER NOT NULL,
            bot_requests INTEGER NOT NULL,
            http_v1 INTEGER NOT NULL,
            http_v2 INTEGER NOT NULL,
            http_v3 INTEGER NOT NULL,
            tls_v12 INTEGER NOT NULL,
            tls_v13 INTEGER NOT NULL,
            tls_other INTEGER NOT NULL,
            top_paths_json TEXT NOT NULL,
            top_referrers_json TEXT NOT NULL,
            top_countries_json TEXT NOT NULL,
            top_browsers_json TEXT NOT NULL,
            top_os_json TEXT NOT NULL,
            top_devices_json TEXT NOT NULL,
            top_slow_urls_json TEXT NOT NULL,
            top_error_urls_json TEXT NOT NULL,
            latency_buckets_json TEXT NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (site_id, bucket_ts)
        );
        CREATE INDEX IF NOT EXISTS idx_site_1d_site_ts ON site_metrics_1d (site_id, bucket_ts);
        CREATE INDEX IF NOT EXISTS idx_site_1d_ts ON site_metrics_1d (bucket_ts);

        -- 16. PHP-FPM pool metrics: Raw (10s) (P9-T4)
        CREATE TABLE IF NOT EXISTS metrics_php_fpm_raw (
            timestamp INTEGER NOT NULL,
            pool_name TEXT NOT NULL,
            user TEXT,
            php_version TEXT,
            active_processes INTEGER NOT NULL,
            idle_processes INTEGER NOT NULL,
            total_processes INTEGER NOT NULL,
            max_active_processes INTEGER NOT NULL,
            max_children_reached INTEGER NOT NULL,
            listen_queue INTEGER NOT NULL,
            max_listen_queue INTEGER NOT NULL,
            listen_queue_len INTEGER NOT NULL,
            slow_requests INTEGER NOT NULL,
            accepted_conn INTEGER NOT NULL,
            PRIMARY KEY (timestamp, pool_name)
        );
        CREATE INDEX IF NOT EXISTS idx_php_fpm_raw_pool_ts ON metrics_php_fpm_raw (pool_name, timestamp);

        -- 17. PHP-FPM pool metrics: 1-minute rollup (P9-T4)
        CREATE TABLE IF NOT EXISTS metrics_php_fpm_1m (
            bucket_ts INTEGER NOT NULL,
            pool_name TEXT NOT NULL,
            active_avg REAL NOT NULL,
            active_max INTEGER NOT NULL,
            idle_avg REAL NOT NULL,
            total_avg REAL NOT NULL,
            total_max INTEGER NOT NULL,
            listen_queue_avg REAL NOT NULL,
            listen_queue_max INTEGER NOT NULL,
            max_children_reached_max INTEGER NOT NULL,
            slow_requests_delta INTEGER NOT NULL,
            accepted_conn_delta INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (bucket_ts, pool_name)
        );
        CREATE INDEX IF NOT EXISTS idx_php_fpm_1m_pool_ts ON metrics_php_fpm_1m (pool_name, bucket_ts);

        -- 18. Application process metrics (Node / Python): Raw (10s) (P9-T4)
        CREATE TABLE IF NOT EXISTS metrics_app_process_raw (
            timestamp INTEGER NOT NULL,
            site_id INTEGER NOT NULL,
            app_type TEXT NOT NULL,
            service_name TEXT NOT NULL,
            cpu_usage_usec INTEGER NOT NULL,
            cpu_percent REAL NOT NULL,
            memory_current_bytes INTEGER NOT NULL,
            memory_peak_bytes INTEGER,
            memory_swap_bytes INTEGER,
            pids_current INTEGER NOT NULL,
            io_read_bytes_sec REAL NOT NULL,
            io_write_bytes_sec REAL NOT NULL,
            PRIMARY KEY (timestamp, site_id, app_type)
        );
        CREATE INDEX IF NOT EXISTS idx_app_proc_raw_site_ts ON metrics_app_process_raw (site_id, timestamp);

        -- 19. Application process metrics (Node / Python): 1-minute rollup (P9-T4)
        CREATE TABLE IF NOT EXISTS metrics_app_process_1m (
            bucket_ts INTEGER NOT NULL,
            site_id INTEGER NOT NULL,
            app_type TEXT NOT NULL,
            cpu_avg REAL NOT NULL,
            cpu_max REAL NOT NULL,
            memory_current_avg REAL NOT NULL,
            memory_current_max INTEGER NOT NULL,
            memory_peak_max INTEGER,
            pids_max INTEGER NOT NULL,
            io_read_avg REAL NOT NULL,
            io_write_avg REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            PRIMARY KEY (bucket_ts, site_id, app_type)
        );
        CREATE INDEX IF NOT EXISTS idx_app_proc_1m_site_ts ON metrics_app_process_1m (site_id, bucket_ts);

        -- 20. PHP-FPM slowlog events (P9-T4)
        CREATE TABLE IF NOT EXISTS php_slowlog_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            pool TEXT NOT NULL,
            pid INTEGER NOT NULL,
            script_filename TEXT NOT NULL,
            top_function TEXT,
            frames_json TEXT NOT NULL,
            duration_secs REAL,
            site_id INTEGER,
            tenant_user TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_slowlog_pool_ts ON php_slowlog_events (pool, timestamp);
        CREATE INDEX IF NOT EXISTS idx_slowlog_site_ts ON php_slowlog_events (site_id, timestamp);
        CREATE INDEX IF NOT EXISTS idx_slowlog_user_ts ON php_slowlog_events (tenant_user, timestamp);

        -- 21. Site security events (P9-T8)
        CREATE TABLE IF NOT EXISTS site_security_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            site_id INTEGER NOT NULL,
            timestamp INTEGER NOT NULL,
            client_ip TEXT NOT NULL,
            method TEXT NOT NULL,
            uri TEXT NOT NULL,
            status INTEGER NOT NULL,
            user_agent TEXT,
            country TEXT,
            event_type TEXT NOT NULL,
            threat_score INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sec_events_site_ts ON site_security_events (site_id, timestamp);
        CREATE INDEX IF NOT EXISTS idx_sec_events_site_ip ON site_security_events (site_id, client_ip);
        CREATE INDEX IF NOT EXISTS idx_sec_events_site_type ON site_security_events (site_id, event_type);
        "#;

        sqlx::query(schema)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok(())
    }

    /// Insert a server raw metric point.
    pub async fn insert_server_point(&self, pt: &ServerMetricsPoint) -> Result<(), AnalyticsError> {
        let query = r#"
        INSERT OR REPLACE INTO metrics_server_raw (
            timestamp, cpu_percent, cpu_cores, memory_used_bytes, memory_total_bytes,
            swap_used_bytes, swap_total_bytes, load1, load5, load15,
            disk_used_bytes, disk_total_bytes, inodes_used, inodes_total,
            net_rx_bytes_sec, net_tx_bytes_sec, io_read_bytes_sec, io_write_bytes_sec
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        sqlx::query(query)
            .bind(pt.timestamp)
            .bind(pt.cpu_percent)
            .bind(pt.cpu_cores as i64)
            .bind(pt.memory_used_bytes as i64)
            .bind(pt.memory_total_bytes as i64)
            .bind(pt.swap_used_bytes as i64)
            .bind(pt.swap_total_bytes as i64)
            .bind(pt.load1)
            .bind(pt.load5)
            .bind(pt.load15)
            .bind(pt.disk_used_bytes as i64)
            .bind(pt.disk_total_bytes as i64)
            .bind(pt.inodes_used as i64)
            .bind(pt.inodes_total as i64)
            .bind(pt.net_rx_bytes_sec)
            .bind(pt.net_tx_bytes_sec)
            .bind(pt.io_read_bytes_sec)
            .bind(pt.io_write_bytes_sec)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok(())
    }

    /// Insert batch of tenant raw metric points.
    pub async fn insert_tenant_points(
        &self,
        pts: &[TenantMetricsPoint],
    ) -> Result<(), AnalyticsError> {
        if pts.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AnalyticsError::Database)?;
        let query = r#"
        INSERT OR REPLACE INTO metrics_tenant_raw (
            timestamp, uid, cpu_usage_usec, cpu_percent, memory_current_bytes,
            memory_peak_bytes, memory_swap_bytes, pids_current,
            io_read_bytes_sec, io_write_bytes_sec
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        for pt in pts {
            sqlx::query(query)
                .bind(pt.timestamp)
                .bind(pt.uid as i64)
                .bind(pt.cpu_usage_usec as i64)
                .bind(pt.cpu_percent)
                .bind(pt.memory_current_bytes as i64)
                .bind(pt.memory_peak_bytes.map(|v| v as i64))
                .bind(pt.memory_swap_bytes.map(|v| v as i64))
                .bind(pt.pids_current as i64)
                .bind(pt.io_read_bytes_sec)
                .bind(pt.io_write_bytes_sec)
                .execute(&mut *tx)
                .await
                .map_err(AnalyticsError::Database)?;
        }

        tx.commit().await.map_err(AnalyticsError::Database)?;
        Ok(())
    }

    /// Insert batch of service raw metric points.
    pub async fn insert_service_points(
        &self,
        pts: &[ServiceMetricsPoint],
    ) -> Result<(), AnalyticsError> {
        if pts.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AnalyticsError::Database)?;
        let query = r#"
        INSERT OR REPLACE INTO metrics_service_raw (
            timestamp, service_name, cpu_usage_usec, cpu_percent,
            memory_current_bytes, pids_current, io_read_bytes_sec, io_write_bytes_sec
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        for pt in pts {
            sqlx::query(query)
                .bind(pt.timestamp)
                .bind(&pt.service_name)
                .bind(pt.cpu_usage_usec as i64)
                .bind(pt.cpu_percent)
                .bind(pt.memory_current_bytes as i64)
                .bind(pt.pids_current as i64)
                .bind(pt.io_read_bytes_sec)
                .bind(pt.io_write_bytes_sec)
                .execute(&mut *tx)
                .await
                .map_err(AnalyticsError::Database)?;
        }

        tx.commit().await.map_err(AnalyticsError::Database)?;
        Ok(())
    }

    /// Execute 1-minute rollup for completed buckets up to `before_timestamp`.
    pub async fn rollup_1m(
        &self,
        before_timestamp: i64,
    ) -> Result<(usize, usize, usize), AnalyticsError> {
        let max_bucket = (before_timestamp / 60) * 60;

        // 1. Server 1m rollup
        let s_query = r#"
        INSERT OR REPLACE INTO metrics_server_1m (
            bucket_ts, cpu_avg, cpu_max, cpu_min,
            memory_used_avg, memory_used_max, swap_used_avg, swap_used_max,
            load1_avg, load1_max, disk_used_avg, disk_used_max,
            net_rx_avg, net_rx_max, net_tx_avg, net_tx_max,
            io_read_avg, io_read_max, io_write_avg, io_write_max,
            sample_count
        )
        SELECT
            (timestamp / 60) * 60 AS b_ts,
            AVG(cpu_percent), MAX(cpu_percent), MIN(cpu_percent),
            AVG(memory_used_bytes), MAX(memory_used_bytes),
            AVG(swap_used_bytes), MAX(swap_used_bytes),
            AVG(load1), MAX(load1),
            AVG(disk_used_bytes), MAX(disk_used_bytes),
            AVG(net_rx_bytes_sec), MAX(net_rx_bytes_sec),
            AVG(net_tx_bytes_sec), MAX(net_tx_bytes_sec),
            AVG(io_read_bytes_sec), MAX(io_read_bytes_sec),
            AVG(io_write_bytes_sec), MAX(io_write_bytes_sec),
            COUNT(*)
        FROM metrics_server_raw
        WHERE timestamp < ?
        GROUP BY b_ts
        "#;
        let s_res = sqlx::query(s_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        // 2. Tenant 1m rollup
        let t_query = r#"
        INSERT OR REPLACE INTO metrics_tenant_1m (
            bucket_ts, uid, cpu_avg, cpu_max,
            memory_avg, memory_max, pids_avg, pids_max,
            io_read_avg, io_write_avg, sample_count
        )
        SELECT
            (timestamp / 60) * 60 AS b_ts,
            uid,
            AVG(cpu_percent), MAX(cpu_percent),
            AVG(memory_current_bytes), MAX(memory_current_bytes),
            AVG(pids_current), MAX(pids_current),
            AVG(io_read_bytes_sec), AVG(io_write_bytes_sec),
            COUNT(*)
        FROM metrics_tenant_raw
        WHERE timestamp < ?
        GROUP BY b_ts, uid
        "#;
        let t_res = sqlx::query(t_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        // 3. Service 1m rollup
        let svc_query = r#"
        INSERT OR REPLACE INTO metrics_service_1m (
            bucket_ts, service_name, cpu_avg, cpu_max,
            memory_avg, memory_max, pids_avg, pids_max,
            io_read_avg, io_write_avg, sample_count
        )
        SELECT
            (timestamp / 60) * 60 AS b_ts,
            service_name,
            AVG(cpu_percent), MAX(cpu_percent),
            AVG(memory_current_bytes), MAX(memory_current_bytes),
            AVG(pids_current), MAX(pids_current),
            AVG(io_read_bytes_sec), AVG(io_write_bytes_sec),
            COUNT(*)
        FROM metrics_service_raw
        WHERE timestamp < ?
        GROUP BY b_ts, service_name
        "#;
        let svc_res = sqlx::query(svc_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok((
            s_res.rows_affected() as usize,
            t_res.rows_affected() as usize,
            svc_res.rows_affected() as usize,
        ))
    }

    /// Execute 1-hour rollup for completed buckets up to `before_timestamp`.
    pub async fn rollup_1h(
        &self,
        before_timestamp: i64,
    ) -> Result<(usize, usize, usize), AnalyticsError> {
        let max_bucket = (before_timestamp / 3600) * 3600;

        let s_query = r#"
        INSERT OR REPLACE INTO metrics_server_1h (
            bucket_ts, cpu_avg, cpu_max, cpu_min,
            memory_used_avg, memory_used_max, swap_used_avg, swap_used_max,
            load1_avg, load1_max, disk_used_avg, disk_used_max,
            net_rx_avg, net_rx_max, net_tx_avg, net_tx_max,
            io_read_avg, io_read_max, io_write_avg, io_write_max,
            sample_count
        )
        SELECT
            (bucket_ts / 3600) * 3600 AS b_ts,
            AVG(cpu_avg), MAX(cpu_max), MIN(cpu_min),
            AVG(memory_used_avg), MAX(memory_used_max),
            AVG(swap_used_avg), MAX(swap_used_max),
            AVG(load1_avg), MAX(load1_max),
            AVG(disk_used_avg), MAX(disk_used_max),
            AVG(net_rx_avg), MAX(net_rx_max),
            AVG(net_tx_avg), MAX(net_tx_max),
            AVG(io_read_avg), MAX(io_read_max),
            AVG(io_write_avg), MAX(io_write_max),
            SUM(sample_count)
        FROM metrics_server_1m
        WHERE bucket_ts < ?
        GROUP BY b_ts
        "#;
        let s_res = sqlx::query(s_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let t_query = r#"
        INSERT OR REPLACE INTO metrics_tenant_1h (
            bucket_ts, uid, cpu_avg, cpu_max,
            memory_avg, memory_max, pids_avg, pids_max,
            io_read_avg, io_write_avg, sample_count
        )
        SELECT
            (bucket_ts / 3600) * 3600 AS b_ts,
            uid,
            AVG(cpu_avg), MAX(cpu_max),
            AVG(memory_avg), MAX(memory_max),
            AVG(pids_avg), MAX(pids_max),
            AVG(io_read_avg), AVG(io_write_avg),
            SUM(sample_count)
        FROM metrics_tenant_1m
        WHERE bucket_ts < ?
        GROUP BY b_ts, uid
        "#;
        let t_res = sqlx::query(t_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let svc_query = r#"
        INSERT OR REPLACE INTO metrics_service_1h (
            bucket_ts, service_name, cpu_avg, cpu_max,
            memory_avg, memory_max, pids_avg, pids_max,
            io_read_avg, io_write_avg, sample_count
        )
        SELECT
            (bucket_ts / 3600) * 3600 AS b_ts,
            service_name,
            AVG(cpu_avg), MAX(cpu_max),
            AVG(memory_avg), MAX(memory_max),
            AVG(pids_avg), MAX(pids_max),
            AVG(io_read_avg), AVG(io_write_avg),
            SUM(sample_count)
        FROM metrics_service_1m
        WHERE bucket_ts < ?
        GROUP BY b_ts, service_name
        "#;
        let svc_res = sqlx::query(svc_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok((
            s_res.rows_affected() as usize,
            t_res.rows_affected() as usize,
            svc_res.rows_affected() as usize,
        ))
    }

    /// Execute 1-day rollup for completed buckets up to `before_timestamp`.
    pub async fn rollup_1d(
        &self,
        before_timestamp: i64,
    ) -> Result<(usize, usize, usize), AnalyticsError> {
        let max_bucket = (before_timestamp / 86400) * 86400;

        let s_query = r#"
        INSERT OR REPLACE INTO metrics_server_1d (
            bucket_ts, cpu_avg, cpu_max, cpu_min,
            memory_used_avg, memory_used_max, swap_used_avg, swap_used_max,
            load1_avg, load1_max, disk_used_avg, disk_used_max,
            net_rx_avg, net_rx_max, net_tx_avg, net_tx_max,
            io_read_avg, io_read_max, io_write_avg, io_write_max,
            sample_count
        )
        SELECT
            (bucket_ts / 86400) * 86400 AS b_ts,
            AVG(cpu_avg), MAX(cpu_max), MIN(cpu_min),
            AVG(memory_used_avg), MAX(memory_used_max),
            AVG(swap_used_avg), MAX(swap_used_max),
            AVG(load1_avg), MAX(load1_max),
            AVG(disk_used_avg), MAX(disk_used_max),
            AVG(net_rx_avg), MAX(net_rx_max),
            AVG(net_tx_avg), MAX(net_tx_max),
            AVG(io_read_avg), MAX(io_read_max),
            AVG(io_write_avg), MAX(io_write_max),
            SUM(sample_count)
        FROM metrics_server_1h
        WHERE bucket_ts < ?
        GROUP BY b_ts
        "#;
        let s_res = sqlx::query(s_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let t_query = r#"
        INSERT OR REPLACE INTO metrics_tenant_1d (
            bucket_ts, uid, cpu_avg, cpu_max,
            memory_avg, memory_max, pids_avg, pids_max,
            io_read_avg, io_write_avg, sample_count
        )
        SELECT
            (bucket_ts / 86400) * 86400 AS b_ts,
            uid,
            AVG(cpu_avg), MAX(cpu_max),
            AVG(memory_avg), MAX(memory_max),
            AVG(pids_avg), MAX(pids_max),
            AVG(io_read_avg), AVG(io_write_avg),
            SUM(sample_count)
        FROM metrics_tenant_1h
        WHERE bucket_ts < ?
        GROUP BY b_ts, uid
        "#;
        let t_res = sqlx::query(t_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let svc_query = r#"
        INSERT OR REPLACE INTO metrics_service_1d (
            bucket_ts, service_name, cpu_avg, cpu_max,
            memory_avg, memory_max, pids_avg, pids_max,
            io_read_avg, io_write_avg, sample_count
        )
        SELECT
            (bucket_ts / 86400) * 86400 AS b_ts,
            service_name,
            AVG(cpu_avg), MAX(cpu_max),
            AVG(memory_avg), MAX(memory_max),
            AVG(pids_avg), MAX(pids_max),
            AVG(io_read_avg), AVG(io_write_avg),
            SUM(sample_count)
        FROM metrics_service_1h
        WHERE bucket_ts < ?
        GROUP BY b_ts, service_name
        "#;
        let svc_res = sqlx::query(svc_query)
            .bind(max_bucket)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok((
            s_res.rows_affected() as usize,
            t_res.rows_affected() as usize,
            svc_res.rows_affected() as usize,
        ))
    }

    /// Prune expired metrics according to configured retention periods.
    pub async fn prune_expired(
        &self,
        current_timestamp: i64,
        retention: &RetentionConfig,
    ) -> Result<usize, AnalyticsError> {
        let mut total_deleted = 0usize;

        // 1. Raw records (default 2 hours)
        let raw_cutoff = current_timestamp - retention.raw_retention_secs;
        let q1 = sqlx::query("DELETE FROM metrics_server_raw WHERE timestamp < ?")
            .bind(raw_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q2 = sqlx::query("DELETE FROM metrics_tenant_raw WHERE timestamp < ?")
            .bind(raw_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q3 = sqlx::query("DELETE FROM metrics_service_raw WHERE timestamp < ?")
            .bind(raw_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_fpm_raw = sqlx::query("DELETE FROM metrics_php_fpm_raw WHERE timestamp < ?")
            .bind(raw_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_app_raw = sqlx::query("DELETE FROM metrics_app_process_raw WHERE timestamp < ?")
            .bind(raw_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        total_deleted += (q1.rows_affected()
            + q2.rows_affected()
            + q3.rows_affected()
            + q_fpm_raw.rows_affected()
            + q_app_raw.rows_affected()) as usize;

        // 2. 1-minute records (default 14 days)
        let m1_cutoff = current_timestamp - retention.one_minute_retention_secs;
        let q4 = sqlx::query("DELETE FROM metrics_server_1m WHERE bucket_ts < ?")
            .bind(m1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q5 = sqlx::query("DELETE FROM metrics_tenant_1m WHERE bucket_ts < ?")
            .bind(m1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q6 = sqlx::query("DELETE FROM metrics_service_1m WHERE bucket_ts < ?")
            .bind(m1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_fpm_1m = sqlx::query("DELETE FROM metrics_php_fpm_1m WHERE bucket_ts < ?")
            .bind(m1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_app_1m = sqlx::query("DELETE FROM metrics_app_process_1m WHERE bucket_ts < ?")
            .bind(m1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_slow = sqlx::query("DELETE FROM php_slowlog_events WHERE timestamp < ?")
            .bind(m1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        total_deleted += (q4.rows_affected()
            + q5.rows_affected()
            + q6.rows_affected()
            + q_fpm_1m.rows_affected()
            + q_app_1m.rows_affected()
            + q_slow.rows_affected()) as usize;

        // 3. 1-hour records (default 400 days)
        let h1_cutoff = current_timestamp - retention.one_hour_retention_secs;
        let q7 = sqlx::query("DELETE FROM metrics_server_1h WHERE bucket_ts < ?")
            .bind(h1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q8 = sqlx::query("DELETE FROM metrics_tenant_1h WHERE bucket_ts < ?")
            .bind(h1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q9 = sqlx::query("DELETE FROM metrics_service_1h WHERE bucket_ts < ?")
            .bind(h1_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        total_deleted += (q7.rows_affected() + q8.rows_affected() + q9.rows_affected()) as usize;

        // 4. 1-day records (optional)
        if let Some(d1_secs) = retention.one_day_retention_secs {
            let d1_cutoff = current_timestamp - d1_secs;
            let q10 = sqlx::query("DELETE FROM metrics_server_1d WHERE bucket_ts < ?")
                .bind(d1_cutoff)
                .execute(&self.write_pool)
                .await
                .map_err(AnalyticsError::Database)?;
            let q11 = sqlx::query("DELETE FROM metrics_tenant_1d WHERE bucket_ts < ?")
                .bind(d1_cutoff)
                .execute(&self.write_pool)
                .await
                .map_err(AnalyticsError::Database)?;
            let q12 = sqlx::query("DELETE FROM metrics_service_1d WHERE bucket_ts < ?")
                .bind(d1_cutoff)
                .execute(&self.write_pool)
                .await
                .map_err(AnalyticsError::Database)?;
            total_deleted +=
                (q10.rows_affected() + q11.rows_affected() + q12.rows_affected()) as usize;
        }

        // 5. Security events (default 7 days)
        let sec_cutoff = current_timestamp - 7 * 86400;
        let q_sec = sqlx::query("DELETE FROM site_security_events WHERE timestamp < ?")
            .bind(sec_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        total_deleted += q_sec.rows_affected() as usize;

        Ok(total_deleted)
    }

    // ==========================================
    // Data Lifecycle & Retention Management (P9-T9)
    // ==========================================

    /// Measure actual on-disk database and WAL file sizes in bytes.
    /// Falls back to SQLite page math for in-memory or unreadable files.
    pub async fn get_database_file_sizes(&self) -> (i64, i64) {
        if let Some(path) = &self.db_path {
            let db_size = match tokio::fs::metadata(path).await {
                Ok(m) => m.len() as i64,
                Err(_) => self.estimate_db_size_from_pages().await,
            };
            let wal_path = format!("{path}-wal");
            let wal_size = tokio::fs::metadata(&wal_path)
                .await
                .map(|m| m.len() as i64)
                .unwrap_or(0);
            (db_size, wal_size)
        } else {
            let db_size = self.estimate_db_size_from_pages().await;
            (db_size, 0)
        }
    }

    async fn estimate_db_size_from_pages(&self) -> i64 {
        let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(4096);
        let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
        page_count.saturating_mul(page_size)
    }

    /// Truncate and flush WAL journal pages back into the main SQLite database file.
    pub async fn wal_checkpoint_truncate(&self) -> Result<(), AnalyticsError> {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        Ok(())
    }

    /// Rebuild database file to reclaim deleted pages and reduce disk fragmentation.
    pub async fn vacuum(&self) -> Result<(), AnalyticsError> {
        sqlx::query("VACUUM")
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        Ok(())
    }

    /// Perform SQLite quick integrity check (`PRAGMA quick_check`).
    pub async fn check_integrity(&self) -> Result<IntegrityCheckReport, AnalyticsError> {
        let rows = sqlx::query("PRAGMA quick_check")
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut details = Vec::new();
        for r in rows {
            let msg: String = r.get(0);
            details.push(msg);
        }

        let is_healthy = details.len() == 1 && details.first().map(|s| s.as_str()) == Some("ok");
        let status = if is_healthy {
            "ok".to_string()
        } else {
            "corrupted".to_string()
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        Ok(IntegrityCheckReport {
            is_healthy,
            status,
            checked_at: now,
            details,
        })
    }

    /// Retrieve table row counts and storage metrics across all tables in analytics.db.
    pub async fn get_storage_stats(&self) -> Result<DatabaseStorageStats, AnalyticsError> {
        let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(4096);
        let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
        let freelist_count: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        let free_bytes = freelist_count.saturating_mul(page_size);
        let fragmentation_ratio = if page_count > 0 {
            freelist_count as f64 / page_count as f64
        } else {
            0.0
        };

        let (db_size_bytes, wal_size_bytes) = self.get_database_file_sizes().await;
        let total_file_size_bytes = db_size_bytes.saturating_add(wal_size_bytes);

        let integrity = self.check_integrity().await?;

        // Tables list defined statically
        let table_names = [
            "metrics_server_raw",
            "metrics_server_1m",
            "metrics_server_1h",
            "metrics_server_1d",
            "metrics_tenant_raw",
            "metrics_tenant_1m",
            "metrics_tenant_1h",
            "metrics_tenant_1d",
            "metrics_service_raw",
            "metrics_service_1m",
            "metrics_service_1h",
            "metrics_service_1d",
            "site_metrics_1m",
            "site_metrics_1h",
            "site_metrics_1d",
            "metrics_php_fpm_raw",
            "metrics_php_fpm_1m",
            "metrics_app_process_raw",
            "metrics_app_process_1m",
            "php_slowlog_events",
            "site_security_events",
            "log_tailer_checkpoints",
            "analytics_checkpoints",
        ];

        let mut table_stats = Vec::with_capacity(table_names.len());
        for name in table_names {
            let sql = format!("SELECT COUNT(*) FROM {name}");
            let count: i64 = sqlx::query_scalar(&sql)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);
            table_stats.push(DatabaseTableStat {
                table_name: name.to_string(),
                row_count: count,
            });
        }

        Ok(DatabaseStorageStats {
            db_size_bytes,
            wal_size_bytes,
            total_file_size_bytes,
            page_size,
            page_count,
            freelist_count,
            free_bytes,
            fragmentation_ratio,
            max_db_size_bytes: None,
            is_healthy: integrity.is_healthy,
            integrity_message: integrity.status,
            table_stats,
        })
    }

    /// Run full maintenance cycle: pruning raw/1m/1h/1d metrics, security events,
    /// cleaning orphaned checkpoints, handling DB limits with emergency pruning,
    /// executing PRAGMA wal_checkpoint(TRUNCATE), and optional VACUUM.
    async fn prune_raw_records(&self, cutoff: i64) -> Result<usize, AnalyticsError> {
        let q1 = sqlx::query("DELETE FROM metrics_server_raw WHERE timestamp < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q2 = sqlx::query("DELETE FROM metrics_tenant_raw WHERE timestamp < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q3 = sqlx::query("DELETE FROM metrics_service_raw WHERE timestamp < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_fpm_raw = sqlx::query("DELETE FROM metrics_php_fpm_raw WHERE timestamp < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_app_raw = sqlx::query("DELETE FROM metrics_app_process_raw WHERE timestamp < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        Ok((q1.rows_affected()
            + q2.rows_affected()
            + q3.rows_affected()
            + q_fpm_raw.rows_affected()
            + q_app_raw.rows_affected()) as usize)
    }

    async fn prune_1m_records(&self, cutoff: i64) -> Result<usize, AnalyticsError> {
        let q4 = sqlx::query("DELETE FROM metrics_server_1m WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q5 = sqlx::query("DELETE FROM metrics_tenant_1m WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q6 = sqlx::query("DELETE FROM metrics_service_1m WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_fpm_1m = sqlx::query("DELETE FROM metrics_php_fpm_1m WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_app_1m = sqlx::query("DELETE FROM metrics_app_process_1m WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_slow = sqlx::query("DELETE FROM php_slowlog_events WHERE timestamp < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_site_1m = sqlx::query("DELETE FROM site_metrics_1m WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        Ok((q4.rows_affected()
            + q5.rows_affected()
            + q6.rows_affected()
            + q_fpm_1m.rows_affected()
            + q_app_1m.rows_affected()
            + q_slow.rows_affected()
            + q_site_1m.rows_affected()) as usize)
    }

    async fn prune_1h_records(&self, cutoff: i64) -> Result<usize, AnalyticsError> {
        let q7 = sqlx::query("DELETE FROM metrics_server_1h WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q8 = sqlx::query("DELETE FROM metrics_tenant_1h WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q9 = sqlx::query("DELETE FROM metrics_service_1h WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_site_1h = sqlx::query("DELETE FROM site_metrics_1h WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        Ok((q7.rows_affected()
            + q8.rows_affected()
            + q9.rows_affected()
            + q_site_1h.rows_affected()) as usize)
    }

    async fn prune_1d_records(&self, cutoff: i64) -> Result<usize, AnalyticsError> {
        let q10 = sqlx::query("DELETE FROM metrics_server_1d WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q11 = sqlx::query("DELETE FROM metrics_tenant_1d WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q12 = sqlx::query("DELETE FROM metrics_service_1d WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let q_site_1d = sqlx::query("DELETE FROM site_metrics_1d WHERE bucket_ts < ?")
            .bind(cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        Ok((q10.rows_affected()
            + q11.rows_affected()
            + q12.rows_affected()
            + q_site_1d.rows_affected()) as usize)
    }

    async fn run_emergency_pruning(&self, current_ts: i64) {
        let emerg_raw = current_ts - 1800;
        let _ = sqlx::query("DELETE FROM metrics_server_raw WHERE timestamp < ?")
            .bind(emerg_raw)
            .execute(&self.write_pool)
            .await;
        let _ = sqlx::query("DELETE FROM metrics_tenant_raw WHERE timestamp < ?")
            .bind(emerg_raw)
            .execute(&self.write_pool)
            .await;
        let _ = sqlx::query("DELETE FROM metrics_service_raw WHERE timestamp < ?")
            .bind(emerg_raw)
            .execute(&self.write_pool)
            .await;
        let emerg_1m = current_ts - 3 * 86400;
        let _ = sqlx::query("DELETE FROM site_metrics_1m WHERE bucket_ts < ?")
            .bind(emerg_1m)
            .execute(&self.write_pool)
            .await;
    }

    pub async fn run_maintenance(
        &self,
        current_ts: i64,
        config: &RetentionConfig,
        force_vacuum: bool,
    ) -> Result<RetentionMaintenanceReport, AnalyticsError> {
        let start_time = std::time::Instant::now();
        let (db_size_before, wal_size_before) = self.get_database_file_sizes().await;

        let raw_pruned = self
            .prune_raw_records(current_ts - config.raw_retention_secs)
            .await?;
        let m1_pruned = self
            .prune_1m_records(current_ts - config.one_minute_retention_secs)
            .await?;
        let h1_pruned = self
            .prune_1h_records(current_ts - config.one_hour_retention_secs)
            .await?;
        let d1_pruned = match config.one_day_retention_secs {
            Some(d1_secs) => self.prune_1d_records(current_ts - d1_secs).await?,
            None => 0,
        };

        // 5. Security events pruning (PRD 5.6: default 7 days)
        let sec_cutoff = current_ts - config.security_events_retention_secs;
        let q_sec = sqlx::query("DELETE FROM site_security_events WHERE timestamp < ?")
            .bind(sec_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let sec_pruned = q_sec.rows_affected() as usize;

        // 6. Prune stale log checkpoints (inactive > 30 days)
        let chk_cutoff = current_ts - 30 * 86400;
        let q_chk = sqlx::query("DELETE FROM log_tailer_checkpoints WHERE updated_at < ?")
            .bind(chk_cutoff)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        let checkpoints_pruned = q_chk.rows_affected() as usize;

        // 7. Check database limits & perform emergency pruning if needed
        let mut emergency_pruning_triggered = false;
        if let Some(max_size) = config.max_db_size_bytes {
            let total_current = db_size_before.saturating_add(wal_size_before);
            if total_current > max_size {
                emergency_pruning_triggered = true;
                self.run_emergency_pruning(current_ts).await;
            }
        }

        // 8. WAL Checkpoint truncate
        let _ = self.wal_checkpoint_truncate().await;

        // 9. VACUUM execution
        let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
        let freelist_count: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
        let frag_ratio = if page_count > 0 {
            freelist_count as f64 / page_count as f64
        } else {
            0.0
        };

        let vacuum_executed = (force_vacuum || config.auto_vacuum || frag_ratio > 0.20)
            && self.vacuum().await.is_ok();

        let (db_size_after, wal_size_after) = self.get_database_file_sizes().await;
        let duration_ms = start_time.elapsed().as_millis() as u64;

        let total_pruned =
            raw_pruned + m1_pruned + h1_pruned + d1_pruned + sec_pruned + checkpoints_pruned;

        Ok(RetentionMaintenanceReport {
            timestamp: current_ts,
            raw_records_pruned: raw_pruned,
            one_minute_records_pruned: m1_pruned,
            one_hour_records_pruned: h1_pruned,
            one_day_records_pruned: d1_pruned,
            security_events_pruned: sec_pruned,
            checkpoints_pruned,
            total_records_pruned: total_pruned,
            wal_checkpoint_executed: true,
            vacuum_executed,
            db_size_bytes_before: db_size_before,
            db_size_bytes_after: db_size_after,
            wal_size_bytes_before: wal_size_before,
            wal_size_bytes_after: wal_size_after,
            duration_ms,
            emergency_pruning_triggered,
        })
    }

    /// Safely recover from database corruption by moving corrupted files aside
    /// and instantiating a clean database with fresh schema.
    pub async fn recover_corrupted_database(
        db_path: &str,
    ) -> Result<RecoveryReport, AnalyticsError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let backup_path = format!("{db_path}.corrupted.{now}");

        if tokio::fs::try_exists(db_path).await.unwrap_or(false) {
            let _ = tokio::fs::rename(db_path, &backup_path).await;
        }

        let wal_path = format!("{db_path}-wal");
        if tokio::fs::try_exists(&wal_path).await.unwrap_or(false) {
            let wal_backup = format!("{db_path}-wal.corrupted.{now}");
            let _ = tokio::fs::rename(&wal_path, wal_backup).await;
        }

        let shm_path = format!("{db_path}-shm");
        if tokio::fs::try_exists(&shm_path).await.unwrap_or(false) {
            let shm_backup = format!("{db_path}-shm.corrupted.{now}");
            let _ = tokio::fs::rename(&shm_path, shm_backup).await;
        }

        // Reconnect and initialize fresh schema
        let _new_storage = Self::connect(db_path).await?;

        Ok(RecoveryReport {
            recovered_at: now,
            backup_path: Some(backup_path),
            success: true,
            message: "Database korup berhasil dicadangkan dan database analitik baru berhasil diinisialisasi"
                .to_string(),
        })
    }

    /// Query server rollups within time range.
    pub async fn query_server_rollups(
        &self,
        start_ts: i64,
        end_ts: i64,
        resolution: RollupResolution,
    ) -> Result<Vec<ServerMetricsRollup>, AnalyticsError> {
        let table = match resolution {
            RollupResolution::Raw | RollupResolution::OneMinute => "metrics_server_1m",
            RollupResolution::OneHour => "metrics_server_1h",
            RollupResolution::OneDay => "metrics_server_1d",
        };

        let sql = format!(
            "SELECT * FROM {table} WHERE bucket_ts >= ? AND bucket_ts <= ? ORDER BY bucket_ts ASC"
        );
        let rows = sqlx::query(&sql)
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(ServerMetricsRollup {
                bucket_timestamp: row.get("bucket_ts"),
                cpu_avg: row.get("cpu_avg"),
                cpu_max: row.get("cpu_max"),
                cpu_min: row.get("cpu_min"),
                memory_used_avg: row.get("memory_used_avg"),
                memory_used_max: row.get::<i64, _>("memory_used_max") as u64,
                swap_used_avg: row.get("swap_used_avg"),
                swap_used_max: row.get::<i64, _>("swap_used_max") as u64,
                load1_avg: row.get("load1_avg"),
                load1_max: row.get("load1_max"),
                disk_used_avg: row.get("disk_used_avg"),
                disk_used_max: row.get::<i64, _>("disk_used_max") as u64,
                net_rx_avg: row.get("net_rx_avg"),
                net_rx_max: row.get("net_rx_max"),
                net_tx_avg: row.get("net_tx_avg"),
                net_tx_max: row.get("net_tx_max"),
                io_read_avg: row.get("io_read_avg"),
                io_read_max: row.get("io_read_max"),
                io_write_avg: row.get("io_write_avg"),
                io_write_max: row.get("io_write_max"),
                sample_count: row.get::<i64, _>("sample_count") as u32,
            });
        }

        Ok(results)
    }

    /// Query tenant rollups within time range.
    pub async fn query_tenant_rollups(
        &self,
        uid: u32,
        start_ts: i64,
        end_ts: i64,
        resolution: RollupResolution,
    ) -> Result<Vec<TenantMetricsRollup>, AnalyticsError> {
        let table = match resolution {
            RollupResolution::Raw | RollupResolution::OneMinute => "metrics_tenant_1m",
            RollupResolution::OneHour => "metrics_tenant_1h",
            RollupResolution::OneDay => "metrics_tenant_1d",
        };

        let sql = format!(
            "SELECT * FROM {table} WHERE uid = ? AND bucket_ts >= ? AND bucket_ts <= ? ORDER BY bucket_ts ASC"
        );
        let rows = sqlx::query(&sql)
            .bind(uid as i64)
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(TenantMetricsRollup {
                bucket_timestamp: row.get("bucket_ts"),
                uid: row.get::<i64, _>("uid") as u32,
                cpu_avg: row.get("cpu_avg"),
                cpu_max: row.get("cpu_max"),
                memory_avg: row.get("memory_avg"),
                memory_max: row.get::<i64, _>("memory_max") as u64,
                pids_avg: row.get("pids_avg"),
                pids_max: row.get::<i64, _>("pids_max") as u64,
                io_read_avg: row.get("io_read_avg"),
                io_write_avg: row.get("io_write_avg"),
                sample_count: row.get::<i64, _>("sample_count") as u32,
            });
        }

        Ok(results)
    }

    /// Query service rollups within time range.
    pub async fn query_service_rollups(
        &self,
        service_name: &str,
        start_ts: i64,
        end_ts: i64,
        resolution: RollupResolution,
    ) -> Result<Vec<ServiceMetricsRollup>, AnalyticsError> {
        let table = match resolution {
            RollupResolution::Raw | RollupResolution::OneMinute => "metrics_service_1m",
            RollupResolution::OneHour => "metrics_service_1h",
            RollupResolution::OneDay => "metrics_service_1d",
        };

        let sql = format!(
            "SELECT * FROM {table} WHERE service_name = ? AND bucket_ts >= ? AND bucket_ts <= ? ORDER BY bucket_ts ASC"
        );
        let rows = sqlx::query(&sql)
            .bind(service_name)
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(ServiceMetricsRollup {
                bucket_timestamp: row.get("bucket_ts"),
                service_name: row.get("service_name"),
                cpu_avg: row.get("cpu_avg"),
                cpu_max: row.get("cpu_max"),
                memory_avg: row.get("memory_avg"),
                memory_max: row.get::<i64, _>("memory_max") as u64,
                pids_avg: row.get("pids_avg"),
                pids_max: row.get::<i64, _>("pids_max") as u64,
                io_read_avg: row.get("io_read_avg"),
                io_write_avg: row.get("io_write_avg"),
                sample_count: row.get::<i64, _>("sample_count") as u32,
            });
        }

        Ok(results)
    }

    /// Retrieve stored checkpoint for a log file path.
    pub async fn get_log_checkpoint(
        &self,
        file_path: &str,
    ) -> Result<Option<LogCheckpoint>, AnalyticsError> {
        let query = "SELECT file_path, site_id, inode, file_offset, last_processed_ts, updated_at FROM log_tailer_checkpoints WHERE file_path = ?";
        let row = sqlx::query(query)
            .bind(file_path)
            .fetch_optional(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        if let Some(r) = row {
            let inode_i64: i64 = r.try_get("inode").map_err(AnalyticsError::Database)?;
            let offset_i64: i64 = r.try_get("file_offset").map_err(AnalyticsError::Database)?;
            Ok(Some(LogCheckpoint {
                file_path: r.try_get("file_path").map_err(AnalyticsError::Database)?,
                site_id: r.try_get("site_id").map_err(AnalyticsError::Database)?,
                inode: inode_i64.max(0) as u64,
                file_offset: offset_i64.max(0) as u64,
                last_processed_ts: r
                    .try_get("last_processed_ts")
                    .map_err(AnalyticsError::Database)?,
                updated_at: r.try_get("updated_at").map_err(AnalyticsError::Database)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Persist or update a log file checkpoint.
    pub async fn save_log_checkpoint(
        &self,
        checkpoint: &LogCheckpoint,
    ) -> Result<(), AnalyticsError> {
        let query = r#"
        INSERT INTO log_tailer_checkpoints (file_path, site_id, inode, file_offset, last_processed_ts, updated_at)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(file_path) DO UPDATE SET
            site_id = excluded.site_id,
            inode = excluded.inode,
            file_offset = excluded.file_offset,
            last_processed_ts = excluded.last_processed_ts,
            updated_at = excluded.updated_at
        "#;

        sqlx::query(query)
            .bind(&checkpoint.file_path)
            .bind(checkpoint.site_id)
            .bind(checkpoint.inode as i64)
            .bind(checkpoint.file_offset as i64)
            .bind(checkpoint.last_processed_ts)
            .bind(checkpoint.updated_at)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok(())
    }

    /// List all log file checkpoints.
    pub async fn list_log_checkpoints(&self) -> Result<Vec<LogCheckpoint>, AnalyticsError> {
        let query = "SELECT file_path, site_id, inode, file_offset, last_processed_ts, updated_at FROM log_tailer_checkpoints ORDER BY file_path ASC";
        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut res = Vec::with_capacity(rows.len());
        for r in rows {
            let inode_i64: i64 = r.try_get("inode").map_err(AnalyticsError::Database)?;
            let offset_i64: i64 = r.try_get("file_offset").map_err(AnalyticsError::Database)?;
            res.push(LogCheckpoint {
                file_path: r.try_get("file_path").map_err(AnalyticsError::Database)?,
                site_id: r.try_get("site_id").map_err(AnalyticsError::Database)?,
                inode: inode_i64.max(0) as u64,
                file_offset: offset_i64.max(0) as u64,
                last_processed_ts: r
                    .try_get("last_processed_ts")
                    .map_err(AnalyticsError::Database)?,
                updated_at: r.try_get("updated_at").map_err(AnalyticsError::Database)?,
            });
        }
        Ok(res)
    }

    /// Delete a log checkpoint when a site is removed.
    pub async fn delete_log_checkpoint(&self, file_path: &str) -> Result<(), AnalyticsError> {
        let query = "DELETE FROM log_tailer_checkpoints WHERE file_path = ?";
        sqlx::query(query)
            .bind(file_path)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;
        Ok(())
    }

    /// Insert a batch of 1-minute site traffic metric points.
    pub async fn insert_site_metrics_1m(
        &self,
        points: &[SiteMetricsPoint],
    ) -> Result<(), AnalyticsError> {
        for pt in points {
            let top_paths_json =
                serde_json::to_string(&pt.top_paths).unwrap_or_else(|_| "[]".to_string());
            let top_referrers_json =
                serde_json::to_string(&pt.top_referrers).unwrap_or_else(|_| "[]".to_string());
            let top_countries_json =
                serde_json::to_string(&pt.top_countries).unwrap_or_else(|_| "[]".to_string());
            let top_browsers_json =
                serde_json::to_string(&pt.top_browsers).unwrap_or_else(|_| "[]".to_string());
            let top_os_json =
                serde_json::to_string(&pt.top_os).unwrap_or_else(|_| "[]".to_string());
            let top_devices_json =
                serde_json::to_string(&pt.top_devices).unwrap_or_else(|_| "[]".to_string());
            let top_slow_urls_json =
                serde_json::to_string(&pt.top_slow_urls).unwrap_or_else(|_| "[]".to_string());
            let top_error_urls_json =
                serde_json::to_string(&pt.top_error_urls).unwrap_or_else(|_| "[]".to_string());
            let latency_buckets_json =
                serde_json::to_string(&pt.latency_buckets).unwrap_or_else(|_| "[]".to_string());

            let query = r#"
            INSERT OR REPLACE INTO site_metrics_1m (
                site_id, bucket_ts, total_requests, total_bytes, status_2xx, status_3xx, status_4xx, status_5xx,
                cache_hit, cache_miss, cache_bypass, cache_other, latency_p50_ms, latency_p95_ms, latency_p99_ms,
                latency_avg_ms, upstream_time_avg_ms, unique_visitors, human_requests, bot_requests,
                http_v1, http_v2, http_v3, tls_v12, tls_v13, tls_other,
                top_paths_json, top_referrers_json, top_countries_json, top_browsers_json, top_os_json,
                top_devices_json, top_slow_urls_json, top_error_urls_json, latency_buckets_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#;

            sqlx::query(query)
                .bind(pt.site_id)
                .bind(pt.bucket_ts)
                .bind(pt.total_requests as i64)
                .bind(pt.total_bytes as i64)
                .bind(pt.status_2xx as i64)
                .bind(pt.status_3xx as i64)
                .bind(pt.status_4xx as i64)
                .bind(pt.status_5xx as i64)
                .bind(pt.cache_hit as i64)
                .bind(pt.cache_miss as i64)
                .bind(pt.cache_bypass as i64)
                .bind(pt.cache_other as i64)
                .bind(pt.latency_p50_ms)
                .bind(pt.latency_p95_ms)
                .bind(pt.latency_p99_ms)
                .bind(pt.latency_avg_ms)
                .bind(pt.upstream_time_avg_ms)
                .bind(pt.unique_visitors as i64)
                .bind(pt.human_requests as i64)
                .bind(pt.bot_requests as i64)
                .bind(pt.http_v1 as i64)
                .bind(pt.http_v2 as i64)
                .bind(pt.http_v3 as i64)
                .bind(pt.tls_v12 as i64)
                .bind(pt.tls_v13 as i64)
                .bind(pt.tls_other as i64)
                .bind(&top_paths_json)
                .bind(&top_referrers_json)
                .bind(&top_countries_json)
                .bind(&top_browsers_json)
                .bind(&top_os_json)
                .bind(&top_devices_json)
                .bind(&top_slow_urls_json)
                .bind(&top_error_urls_json)
                .bind(&latency_buckets_json)
                .execute(&self.write_pool)
                .await
                .map_err(AnalyticsError::Database)?;
        }
        Ok(())
    }

    /// Insert a batch of 1-hour site traffic rollup records.
    pub async fn insert_site_metrics_1h(
        &self,
        rollups: &[SiteMetricsRollup],
    ) -> Result<(), AnalyticsError> {
        self.insert_site_rollups_to_table("site_metrics_1h", rollups)
            .await
    }

    /// Insert a batch of 1-day site traffic rollup records.
    pub async fn insert_site_metrics_1d(
        &self,
        rollups: &[SiteMetricsRollup],
    ) -> Result<(), AnalyticsError> {
        self.insert_site_rollups_to_table("site_metrics_1d", rollups)
            .await
    }

    async fn insert_site_rollups_to_table(
        &self,
        table_name: &str,
        rollups: &[SiteMetricsRollup],
    ) -> Result<(), AnalyticsError> {
        for r in rollups {
            let top_paths_json =
                serde_json::to_string(&r.top_paths).unwrap_or_else(|_| "[]".to_string());
            let top_referrers_json =
                serde_json::to_string(&r.top_referrers).unwrap_or_else(|_| "[]".to_string());
            let top_countries_json =
                serde_json::to_string(&r.top_countries).unwrap_or_else(|_| "[]".to_string());
            let top_browsers_json =
                serde_json::to_string(&r.top_browsers).unwrap_or_else(|_| "[]".to_string());
            let top_os_json = serde_json::to_string(&r.top_os).unwrap_or_else(|_| "[]".to_string());
            let top_devices_json =
                serde_json::to_string(&r.top_devices).unwrap_or_else(|_| "[]".to_string());
            let top_slow_urls_json =
                serde_json::to_string(&r.top_slow_urls).unwrap_or_else(|_| "[]".to_string());
            let top_error_urls_json =
                serde_json::to_string(&r.top_error_urls).unwrap_or_else(|_| "[]".to_string());
            let latency_buckets_json =
                serde_json::to_string(&r.latency_buckets).unwrap_or_else(|_| "[]".to_string());

            let query = format!(
                r#"
                INSERT OR REPLACE INTO {} (
                    site_id, bucket_ts, total_requests, total_bytes, status_2xx, status_3xx, status_4xx, status_5xx,
                    cache_hit, cache_miss, cache_bypass, cache_other, latency_p50_ms, latency_p95_ms, latency_p99_ms,
                    latency_avg_ms, upstream_time_avg_ms, unique_visitors, human_requests, bot_requests,
                    http_v1, http_v2, http_v3, tls_v12, tls_v13, tls_other,
                    top_paths_json, top_referrers_json, top_countries_json, top_browsers_json, top_os_json,
                    top_devices_json, top_slow_urls_json, top_error_urls_json, latency_buckets_json, sample_count
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
                table_name
            );

            sqlx::query(&query)
                .bind(r.site_id)
                .bind(r.bucket_ts)
                .bind(r.total_requests as i64)
                .bind(r.total_bytes as i64)
                .bind(r.status_2xx as i64)
                .bind(r.status_3xx as i64)
                .bind(r.status_4xx as i64)
                .bind(r.status_5xx as i64)
                .bind(r.cache_hit as i64)
                .bind(r.cache_miss as i64)
                .bind(r.cache_bypass as i64)
                .bind(r.cache_other as i64)
                .bind(r.latency_p50_ms)
                .bind(r.latency_p95_ms)
                .bind(r.latency_p99_ms)
                .bind(r.latency_avg_ms)
                .bind(r.upstream_time_avg_ms)
                .bind(r.unique_visitors as i64)
                .bind(r.human_requests as i64)
                .bind(r.bot_requests as i64)
                .bind(r.http_v1 as i64)
                .bind(r.http_v2 as i64)
                .bind(r.http_v3 as i64)
                .bind(r.tls_v12 as i64)
                .bind(r.tls_v13 as i64)
                .bind(r.tls_other as i64)
                .bind(&top_paths_json)
                .bind(&top_referrers_json)
                .bind(&top_countries_json)
                .bind(&top_browsers_json)
                .bind(&top_os_json)
                .bind(&top_devices_json)
                .bind(&top_slow_urls_json)
                .bind(&top_error_urls_json)
                .bind(&latency_buckets_json)
                .bind(r.sample_count as i64)
                .execute(&self.write_pool)
                .await
                .map_err(AnalyticsError::Database)?;
        }
        Ok(())
    }

    /// Query 1-minute site traffic metric points for a site within `[start_ts, end_ts]`.
    pub async fn query_site_metrics_1m(
        &self,
        site_id: i64,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<Vec<SiteMetricsPoint>, AnalyticsError> {
        let query = r#"
        SELECT * FROM site_metrics_1m
        WHERE site_id = ? AND bucket_ts >= ? AND bucket_ts <= ?
        ORDER BY bucket_ts ASC
        "#;

        let rows = sqlx::query(query)
            .bind(site_id)
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut res = Vec::with_capacity(rows.len());
        for r in rows {
            let total_requests: i64 = r
                .try_get("total_requests")
                .map_err(AnalyticsError::Database)?;
            let total_bytes: i64 = r.try_get("total_bytes").map_err(AnalyticsError::Database)?;
            let status_2xx: i64 = r.try_get("status_2xx").map_err(AnalyticsError::Database)?;
            let status_3xx: i64 = r.try_get("status_3xx").map_err(AnalyticsError::Database)?;
            let status_4xx: i64 = r.try_get("status_4xx").map_err(AnalyticsError::Database)?;
            let status_5xx: i64 = r.try_get("status_5xx").map_err(AnalyticsError::Database)?;
            let cache_hit: i64 = r.try_get("cache_hit").map_err(AnalyticsError::Database)?;
            let cache_miss: i64 = r.try_get("cache_miss").map_err(AnalyticsError::Database)?;
            let cache_bypass: i64 = r
                .try_get("cache_bypass")
                .map_err(AnalyticsError::Database)?;
            let cache_other: i64 = r.try_get("cache_other").map_err(AnalyticsError::Database)?;
            let unique_visitors: i64 = r
                .try_get("unique_visitors")
                .map_err(AnalyticsError::Database)?;
            let human_requests: i64 = r
                .try_get("human_requests")
                .map_err(AnalyticsError::Database)?;
            let bot_requests: i64 = r
                .try_get("bot_requests")
                .map_err(AnalyticsError::Database)?;
            let http_v1: i64 = r.try_get("http_v1").map_err(AnalyticsError::Database)?;
            let http_v2: i64 = r.try_get("http_v2").map_err(AnalyticsError::Database)?;
            let http_v3: i64 = r.try_get("http_v3").map_err(AnalyticsError::Database)?;
            let tls_v12: i64 = r.try_get("tls_v12").map_err(AnalyticsError::Database)?;
            let tls_v13: i64 = r.try_get("tls_v13").map_err(AnalyticsError::Database)?;
            let tls_other: i64 = r.try_get("tls_other").map_err(AnalyticsError::Database)?;

            let top_paths_str: String = r
                .try_get("top_paths_json")
                .map_err(AnalyticsError::Database)?;
            let top_referrers_str: String = r
                .try_get("top_referrers_json")
                .map_err(AnalyticsError::Database)?;
            let top_countries_str: String = r
                .try_get("top_countries_json")
                .map_err(AnalyticsError::Database)?;
            let top_browsers_str: String = r
                .try_get("top_browsers_json")
                .map_err(AnalyticsError::Database)?;
            let top_os_str: String = r.try_get("top_os_json").map_err(AnalyticsError::Database)?;
            let top_devices_str: String = r
                .try_get("top_devices_json")
                .map_err(AnalyticsError::Database)?;
            let top_slow_urls_str: String = r
                .try_get("top_slow_urls_json")
                .map_err(AnalyticsError::Database)?;
            let top_error_urls_str: String = r
                .try_get("top_error_urls_json")
                .map_err(AnalyticsError::Database)?;
            let latency_buckets_str: String = r
                .try_get("latency_buckets_json")
                .map_err(AnalyticsError::Database)?;

            res.push(SiteMetricsPoint {
                site_id: r.try_get("site_id").map_err(AnalyticsError::Database)?,
                bucket_ts: r.try_get("bucket_ts").map_err(AnalyticsError::Database)?,
                total_requests: total_requests.max(0) as u64,
                total_bytes: total_bytes.max(0) as u64,
                status_2xx: status_2xx.max(0) as u64,
                status_3xx: status_3xx.max(0) as u64,
                status_4xx: status_4xx.max(0) as u64,
                status_5xx: status_5xx.max(0) as u64,
                cache_hit: cache_hit.max(0) as u64,
                cache_miss: cache_miss.max(0) as u64,
                cache_bypass: cache_bypass.max(0) as u64,
                cache_other: cache_other.max(0) as u64,
                latency_p50_ms: r
                    .try_get("latency_p50_ms")
                    .map_err(AnalyticsError::Database)?,
                latency_p95_ms: r
                    .try_get("latency_p95_ms")
                    .map_err(AnalyticsError::Database)?,
                latency_p99_ms: r
                    .try_get("latency_p99_ms")
                    .map_err(AnalyticsError::Database)?,
                latency_avg_ms: r
                    .try_get("latency_avg_ms")
                    .map_err(AnalyticsError::Database)?,
                upstream_time_avg_ms: r
                    .try_get("upstream_time_avg_ms")
                    .map_err(AnalyticsError::Database)?,
                unique_visitors: unique_visitors.max(0) as u64,
                human_requests: human_requests.max(0) as u64,
                bot_requests: bot_requests.max(0) as u64,
                http_v1: http_v1.max(0) as u64,
                http_v2: http_v2.max(0) as u64,
                http_v3: http_v3.max(0) as u64,
                tls_v12: tls_v12.max(0) as u64,
                tls_v13: tls_v13.max(0) as u64,
                tls_other: tls_other.max(0) as u64,
                top_paths: serde_json::from_str(&top_paths_str).unwrap_or_default(),
                top_referrers: serde_json::from_str(&top_referrers_str).unwrap_or_default(),
                top_countries: serde_json::from_str(&top_countries_str).unwrap_or_default(),
                top_browsers: serde_json::from_str(&top_browsers_str).unwrap_or_default(),
                top_os: serde_json::from_str(&top_os_str).unwrap_or_default(),
                top_devices: serde_json::from_str(&top_devices_str).unwrap_or_default(),
                top_slow_urls: serde_json::from_str(&top_slow_urls_str).unwrap_or_default(),
                top_error_urls: serde_json::from_str(&top_error_urls_str).unwrap_or_default(),
                latency_buckets: serde_json::from_str(&latency_buckets_str).unwrap_or_default(),
            });
        }
        Ok(res)
    }

    /// Query 1-hour or 1-day site traffic rollups for a site.
    pub async fn query_site_rollups(
        &self,
        site_id: i64,
        start_ts: i64,
        end_ts: i64,
        resolution: RollupResolution,
    ) -> Result<Vec<SiteMetricsRollup>, AnalyticsError> {
        if matches!(
            resolution,
            RollupResolution::OneMinute | RollupResolution::Raw
        ) {
            let points = self
                .query_site_metrics_1m(site_id, start_ts, end_ts)
                .await?;
            return Ok(points
                .into_iter()
                .map(|pt| SiteMetricsRollup {
                    site_id: pt.site_id,
                    bucket_ts: pt.bucket_ts,
                    total_requests: pt.total_requests,
                    total_bytes: pt.total_bytes,
                    status_2xx: pt.status_2xx,
                    status_3xx: pt.status_3xx,
                    status_4xx: pt.status_4xx,
                    status_5xx: pt.status_5xx,
                    cache_hit: pt.cache_hit,
                    cache_miss: pt.cache_miss,
                    cache_bypass: pt.cache_bypass,
                    cache_other: pt.cache_other,
                    latency_p50_ms: pt.latency_p50_ms,
                    latency_p95_ms: pt.latency_p95_ms,
                    latency_p99_ms: pt.latency_p99_ms,
                    latency_avg_ms: pt.latency_avg_ms,
                    upstream_time_avg_ms: pt.upstream_time_avg_ms,
                    unique_visitors: pt.unique_visitors,
                    human_requests: pt.human_requests,
                    bot_requests: pt.bot_requests,
                    http_v1: pt.http_v1,
                    http_v2: pt.http_v2,
                    http_v3: pt.http_v3,
                    tls_v12: pt.tls_v12,
                    tls_v13: pt.tls_v13,
                    tls_other: pt.tls_other,
                    top_paths: pt.top_paths,
                    top_referrers: pt.top_referrers,
                    top_countries: pt.top_countries,
                    top_browsers: pt.top_browsers,
                    top_os: pt.top_os,
                    top_devices: pt.top_devices,
                    top_slow_urls: pt.top_slow_urls,
                    top_error_urls: pt.top_error_urls,
                    latency_buckets: pt.latency_buckets,
                    sample_count: 1,
                })
                .collect());
        }

        let table = match resolution {
            RollupResolution::OneHour => "site_metrics_1h",
            RollupResolution::OneDay => "site_metrics_1d",
            _ => "site_metrics_1h",
        };

        let query = format!(
            r#"
            SELECT * FROM {}
            WHERE site_id = ? AND bucket_ts >= ? AND bucket_ts <= ?
            ORDER BY bucket_ts ASC
            "#,
            table
        );

        let rows = sqlx::query(&query)
            .bind(site_id)
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut res = Vec::with_capacity(rows.len());
        for r in rows {
            let total_requests: i64 = r
                .try_get("total_requests")
                .map_err(AnalyticsError::Database)?;
            let total_bytes: i64 = r.try_get("total_bytes").map_err(AnalyticsError::Database)?;
            let status_2xx: i64 = r.try_get("status_2xx").map_err(AnalyticsError::Database)?;
            let status_3xx: i64 = r.try_get("status_3xx").map_err(AnalyticsError::Database)?;
            let status_4xx: i64 = r.try_get("status_4xx").map_err(AnalyticsError::Database)?;
            let status_5xx: i64 = r.try_get("status_5xx").map_err(AnalyticsError::Database)?;
            let cache_hit: i64 = r.try_get("cache_hit").map_err(AnalyticsError::Database)?;
            let cache_miss: i64 = r.try_get("cache_miss").map_err(AnalyticsError::Database)?;
            let cache_bypass: i64 = r
                .try_get("cache_bypass")
                .map_err(AnalyticsError::Database)?;
            let cache_other: i64 = r.try_get("cache_other").map_err(AnalyticsError::Database)?;
            let unique_visitors: i64 = r
                .try_get("unique_visitors")
                .map_err(AnalyticsError::Database)?;
            let human_requests: i64 = r
                .try_get("human_requests")
                .map_err(AnalyticsError::Database)?;
            let bot_requests: i64 = r
                .try_get("bot_requests")
                .map_err(AnalyticsError::Database)?;
            let http_v1: i64 = r.try_get("http_v1").map_err(AnalyticsError::Database)?;
            let http_v2: i64 = r.try_get("http_v2").map_err(AnalyticsError::Database)?;
            let http_v3: i64 = r.try_get("http_v3").map_err(AnalyticsError::Database)?;
            let tls_v12: i64 = r.try_get("tls_v12").map_err(AnalyticsError::Database)?;
            let tls_v13: i64 = r.try_get("tls_v13").map_err(AnalyticsError::Database)?;
            let tls_other: i64 = r.try_get("tls_other").map_err(AnalyticsError::Database)?;
            let sample_count: i64 = r
                .try_get("sample_count")
                .map_err(AnalyticsError::Database)?;

            let top_paths_str: String = r
                .try_get("top_paths_json")
                .map_err(AnalyticsError::Database)?;
            let top_referrers_str: String = r
                .try_get("top_referrers_json")
                .map_err(AnalyticsError::Database)?;
            let top_countries_str: String = r
                .try_get("top_countries_json")
                .map_err(AnalyticsError::Database)?;
            let top_browsers_str: String = r
                .try_get("top_browsers_json")
                .map_err(AnalyticsError::Database)?;
            let top_os_str: String = r.try_get("top_os_json").map_err(AnalyticsError::Database)?;
            let top_devices_str: String = r
                .try_get("top_devices_json")
                .map_err(AnalyticsError::Database)?;
            let top_slow_urls_str: String = r
                .try_get("top_slow_urls_json")
                .map_err(AnalyticsError::Database)?;
            let top_error_urls_str: String = r
                .try_get("top_error_urls_json")
                .map_err(AnalyticsError::Database)?;
            let latency_buckets_str: String = r
                .try_get("latency_buckets_json")
                .map_err(AnalyticsError::Database)?;

            res.push(SiteMetricsRollup {
                site_id: r.try_get("site_id").map_err(AnalyticsError::Database)?,
                bucket_ts: r.try_get("bucket_ts").map_err(AnalyticsError::Database)?,
                total_requests: total_requests.max(0) as u64,
                total_bytes: total_bytes.max(0) as u64,
                status_2xx: status_2xx.max(0) as u64,
                status_3xx: status_3xx.max(0) as u64,
                status_4xx: status_4xx.max(0) as u64,
                status_5xx: status_5xx.max(0) as u64,
                cache_hit: cache_hit.max(0) as u64,
                cache_miss: cache_miss.max(0) as u64,
                cache_bypass: cache_bypass.max(0) as u64,
                cache_other: cache_other.max(0) as u64,
                latency_p50_ms: r
                    .try_get("latency_p50_ms")
                    .map_err(AnalyticsError::Database)?,
                latency_p95_ms: r
                    .try_get("latency_p95_ms")
                    .map_err(AnalyticsError::Database)?,
                latency_p99_ms: r
                    .try_get("latency_p99_ms")
                    .map_err(AnalyticsError::Database)?,
                latency_avg_ms: r
                    .try_get("latency_avg_ms")
                    .map_err(AnalyticsError::Database)?,
                upstream_time_avg_ms: r
                    .try_get("upstream_time_avg_ms")
                    .map_err(AnalyticsError::Database)?,
                unique_visitors: unique_visitors.max(0) as u64,
                human_requests: human_requests.max(0) as u64,
                bot_requests: bot_requests.max(0) as u64,
                http_v1: http_v1.max(0) as u64,
                http_v2: http_v2.max(0) as u64,
                http_v3: http_v3.max(0) as u64,
                tls_v12: tls_v12.max(0) as u64,
                tls_v13: tls_v13.max(0) as u64,
                tls_other: tls_other.max(0) as u64,
                top_paths: serde_json::from_str(&top_paths_str).unwrap_or_default(),
                top_referrers: serde_json::from_str(&top_referrers_str).unwrap_or_default(),
                top_countries: serde_json::from_str(&top_countries_str).unwrap_or_default(),
                top_browsers: serde_json::from_str(&top_browsers_str).unwrap_or_default(),
                top_os: serde_json::from_str(&top_os_str).unwrap_or_default(),
                top_devices: serde_json::from_str(&top_devices_str).unwrap_or_default(),
                top_slow_urls: serde_json::from_str(&top_slow_urls_str).unwrap_or_default(),
                top_error_urls: serde_json::from_str(&top_error_urls_str).unwrap_or_default(),
                latency_buckets: serde_json::from_str(&latency_buckets_str).unwrap_or_default(),
                sample_count: sample_count.max(0) as u32,
            });
        }
        Ok(res)
    }

    /// Execute site rollups (1m -> 1h, and 1h -> 1d) and retention pruning.
    pub async fn execute_site_rollups(
        &self,
        current_ts: i64,
    ) -> Result<SiteRollupReport, AnalyticsError> {
        let hour_floor = (current_ts / 3600) * 3600;
        let day_floor = (current_ts / 86400) * 86400;

        // 1. Rollup 1m to 1h for completed hours
        let h_rows = sqlx::query(
            "SELECT DISTINCT site_id, (bucket_ts / 3600) * 3600 AS hour_ts FROM site_metrics_1m WHERE bucket_ts < ?"
        )
        .bind(hour_floor)
        .fetch_all(&self.pool)
        .await
        .map_err(AnalyticsError::Database)?;

        let mut site_1h_created = 0;
        for r in h_rows {
            let s_id: i64 = r.try_get("site_id").map_err(AnalyticsError::Database)?;
            let h_ts: i64 = r.try_get("hour_ts").map_err(AnalyticsError::Database)?;

            let points = self.query_site_metrics_1m(s_id, h_ts, h_ts + 3599).await?;
            if let Some(rollup) = SiteMetricsRollupProcessor::rollup_1m_to_1h(s_id, h_ts, &points) {
                self.insert_site_metrics_1h(&[rollup]).await?;
                site_1h_created += 1;
            }
        }

        // 2. Rollup 1h to 1d for completed days
        let d_rows = sqlx::query(
            "SELECT DISTINCT site_id, (bucket_ts / 86400) * 86400 AS day_ts FROM site_metrics_1h WHERE bucket_ts < ?"
        )
        .bind(day_floor)
        .fetch_all(&self.pool)
        .await
        .map_err(AnalyticsError::Database)?;

        let mut site_1d_created = 0;
        for r in d_rows {
            let s_id: i64 = r.try_get("site_id").map_err(AnalyticsError::Database)?;
            let d_ts: i64 = r.try_get("day_ts").map_err(AnalyticsError::Database)?;

            let rollups_1h = self
                .query_site_rollups(s_id, d_ts, d_ts + 86399, RollupResolution::OneHour)
                .await?;
            if let Some(rollup_1d) =
                SiteMetricsRollupProcessor::rollup_1h_to_1d(s_id, d_ts, &rollups_1h)
            {
                self.insert_site_metrics_1d(&[rollup_1d]).await?;
                site_1d_created += 1;
            }
        }

        // 3. Prune expired records adhering to retention config (default: 1m -> 14 days, 1h -> 400 days)
        let retention = RetentionConfig::default();
        let (pruned_1m, pruned_1h) = self
            .prune_expired_site_metrics(current_ts, &retention)
            .await?;

        Ok(SiteRollupReport {
            timestamp: current_ts,
            site_1h_created,
            site_1d_created,
            pruned_1m_records: pruned_1m,
            pruned_1h_records: pruned_1h,
        })
    }

    /// Prune expired site metrics according to `RetentionConfig`.
    pub async fn prune_expired_site_metrics(
        &self,
        current_ts: i64,
        config: &RetentionConfig,
    ) -> Result<(usize, usize), AnalyticsError> {
        let cutoff_1m = current_ts - config.one_minute_retention_secs;
        let res_1m = sqlx::query("DELETE FROM site_metrics_1m WHERE bucket_ts < ?")
            .bind(cutoff_1m)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let cutoff_1h = current_ts - config.one_hour_retention_secs;
        let res_1h = sqlx::query("DELETE FROM site_metrics_1h WHERE bucket_ts < ?")
            .bind(cutoff_1h)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok((
            res_1m.rows_affected() as usize,
            res_1h.rows_affected() as usize,
        ))
    }

    // ==========================================
    // PHP-FPM Pool Metrics (P9-T4)
    // ==========================================

    /// Insert a single PHP-FPM pool metric raw point.
    pub async fn insert_php_fpm_point(
        &self,
        pt: &PhpFpmPoolMetricPoint,
    ) -> Result<(), AnalyticsError> {
        self.insert_php_fpm_points(std::slice::from_ref(pt)).await
    }

    /// Insert batch of PHP-FPM pool metric raw points.
    pub async fn insert_php_fpm_points(
        &self,
        pts: &[PhpFpmPoolMetricPoint],
    ) -> Result<(), AnalyticsError> {
        let mut tx = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AnalyticsError::Database)?;
        let query = r#"
        INSERT OR REPLACE INTO metrics_php_fpm_raw (
            timestamp, pool_name, user, php_version,
            active_processes, idle_processes, total_processes,
            max_active_processes, max_children_reached,
            listen_queue, max_listen_queue, listen_queue_len,
            slow_requests, accepted_conn
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        for pt in pts {
            sqlx::query(query)
                .bind(pt.timestamp)
                .bind(&pt.pool_name)
                .bind(&pt.user)
                .bind(&pt.php_version)
                .bind(pt.active_processes as i64)
                .bind(pt.idle_processes as i64)
                .bind(pt.total_processes as i64)
                .bind(pt.max_active_processes as i64)
                .bind(pt.max_children_reached as i64)
                .bind(pt.listen_queue as i64)
                .bind(pt.max_listen_queue as i64)
                .bind(pt.listen_queue_len as i64)
                .bind(pt.slow_requests as i64)
                .bind(pt.accepted_conn as i64)
                .execute(&mut *tx)
                .await
                .map_err(AnalyticsError::Database)?;
        }

        tx.commit().await.map_err(AnalyticsError::Database)?;
        Ok(())
    }

    /// Rollup raw PHP-FPM pool metrics into 1-minute buckets up to `until_ts`.
    pub async fn rollup_php_fpm_1m(&self, until_ts: i64) -> Result<usize, AnalyticsError> {
        let query = r#"
        INSERT OR REPLACE INTO metrics_php_fpm_1m (
            bucket_ts, pool_name,
            active_avg, active_max,
            idle_avg,
            total_avg, total_max,
            listen_queue_avg, listen_queue_max,
            max_children_reached_max,
            slow_requests_delta, accepted_conn_delta,
            sample_count
        )
        SELECT
            (timestamp / 60) * 60 AS bucket_ts,
            pool_name,
            AVG(active_processes) AS active_avg,
            MAX(active_processes) AS active_max,
            AVG(idle_processes) AS idle_avg,
            AVG(total_processes) AS total_avg,
            MAX(total_processes) AS total_max,
            AVG(listen_queue) AS listen_queue_avg,
            MAX(listen_queue) AS listen_queue_max,
            MAX(max_children_reached) AS max_children_reached_max,
            MAX(slow_requests) - MIN(slow_requests) AS slow_requests_delta,
            MAX(accepted_conn) - MIN(accepted_conn) AS accepted_conn_delta,
            COUNT(*) AS sample_count
        FROM metrics_php_fpm_raw
        WHERE timestamp <= ?
        GROUP BY pool_name, (timestamp / 60) * 60
        "#;

        let res = sqlx::query(query)
            .bind(until_ts)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok(res.rows_affected() as usize)
    }

    /// Query raw PHP-FPM metrics for a specific pool in time range.
    pub async fn query_php_fpm_raw(
        &self,
        pool_name: &str,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<Vec<PhpFpmPoolMetricPoint>, AnalyticsError> {
        let query = r#"
        SELECT
            timestamp, pool_name, user, php_version,
            active_processes, idle_processes, total_processes,
            max_active_processes, max_children_reached,
            listen_queue, max_listen_queue, listen_queue_len,
            slow_requests, accepted_conn
        FROM metrics_php_fpm_raw
        WHERE pool_name = ? AND timestamp >= ? AND timestamp <= ?
        ORDER BY timestamp ASC
        "#;

        let rows = sqlx::query(query)
            .bind(pool_name)
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut pts = Vec::with_capacity(rows.len());
        for row in rows {
            let active: i64 = row
                .try_get("active_processes")
                .map_err(AnalyticsError::Database)?;
            let idle: i64 = row
                .try_get("idle_processes")
                .map_err(AnalyticsError::Database)?;
            let total: i64 = row
                .try_get("total_processes")
                .map_err(AnalyticsError::Database)?;
            let max_active: i64 = row
                .try_get("max_active_processes")
                .map_err(AnalyticsError::Database)?;
            let max_children: i64 = row
                .try_get("max_children_reached")
                .map_err(AnalyticsError::Database)?;
            let listen_q: i64 = row
                .try_get("listen_queue")
                .map_err(AnalyticsError::Database)?;
            let max_listen_q: i64 = row
                .try_get("max_listen_queue")
                .map_err(AnalyticsError::Database)?;
            let listen_q_len: i64 = row
                .try_get("listen_queue_len")
                .map_err(AnalyticsError::Database)?;
            let slow_reqs: i64 = row
                .try_get("slow_requests")
                .map_err(AnalyticsError::Database)?;
            let accepted: i64 = row
                .try_get("accepted_conn")
                .map_err(AnalyticsError::Database)?;

            pts.push(PhpFpmPoolMetricPoint {
                timestamp: row.try_get("timestamp").map_err(AnalyticsError::Database)?,
                pool_name: row.try_get("pool_name").map_err(AnalyticsError::Database)?,
                user: row.try_get("user").map_err(AnalyticsError::Database)?,
                php_version: row
                    .try_get("php_version")
                    .map_err(AnalyticsError::Database)?,
                active_processes: active.max(0) as u32,
                idle_processes: idle.max(0) as u32,
                total_processes: total.max(0) as u32,
                max_active_processes: max_active.max(0) as u32,
                max_children_reached: max_children.max(0) as u32,
                listen_queue: listen_q.max(0) as u32,
                max_listen_queue: max_listen_q.max(0) as u32,
                listen_queue_len: listen_q_len.max(0) as u32,
                slow_requests: slow_reqs.max(0) as u64,
                accepted_conn: accepted.max(0) as u64,
            });
        }

        Ok(pts)
    }

    /// Query 1-minute rollups for a PHP-FPM pool in time range.
    pub async fn query_php_fpm_1m(
        &self,
        pool_name: &str,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<Vec<PhpFpmPoolMetricRollup>, AnalyticsError> {
        let query = r#"
        SELECT
            bucket_ts, pool_name,
            active_avg, active_max,
            idle_avg,
            total_avg, total_max,
            listen_queue_avg, listen_queue_max,
            max_children_reached_max,
            slow_requests_delta, accepted_conn_delta,
            sample_count
        FROM metrics_php_fpm_1m
        WHERE pool_name = ? AND bucket_ts >= ? AND bucket_ts <= ?
        ORDER BY bucket_ts ASC
        "#;

        let rows = sqlx::query(query)
            .bind(pool_name)
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut rollups = Vec::with_capacity(rows.len());
        for row in rows {
            let active_max: i64 = row
                .try_get("active_max")
                .map_err(AnalyticsError::Database)?;
            let total_max: i64 = row.try_get("total_max").map_err(AnalyticsError::Database)?;
            let listen_q_max: i64 = row
                .try_get("listen_queue_max")
                .map_err(AnalyticsError::Database)?;
            let max_ch_max: i64 = row
                .try_get("max_children_reached_max")
                .map_err(AnalyticsError::Database)?;
            let slow_delta: i64 = row
                .try_get("slow_requests_delta")
                .map_err(AnalyticsError::Database)?;
            let acc_delta: i64 = row
                .try_get("accepted_conn_delta")
                .map_err(AnalyticsError::Database)?;
            let samples: i64 = row
                .try_get("sample_count")
                .map_err(AnalyticsError::Database)?;

            rollups.push(PhpFpmPoolMetricRollup {
                pool_name: row.try_get("pool_name").map_err(AnalyticsError::Database)?,
                bucket_ts: row.try_get("bucket_ts").map_err(AnalyticsError::Database)?,
                active_avg: row
                    .try_get("active_avg")
                    .map_err(AnalyticsError::Database)?,
                active_max: active_max.max(0) as u32,
                idle_avg: row.try_get("idle_avg").map_err(AnalyticsError::Database)?,
                total_avg: row.try_get("total_avg").map_err(AnalyticsError::Database)?,
                total_max: total_max.max(0) as u32,
                listen_queue_avg: row
                    .try_get("listen_queue_avg")
                    .map_err(AnalyticsError::Database)?,
                listen_queue_max: listen_q_max.max(0) as u32,
                max_children_reached_max: max_ch_max.max(0) as u32,
                slow_requests_delta: slow_delta.max(0) as u64,
                accepted_conn_delta: acc_delta.max(0) as u64,
                sample_count: samples.max(0) as u32,
            });
        }

        Ok(rollups)
    }

    // ==========================================
    // Slowlog Events (P9-T4)
    // ==========================================

    /// Insert a single PHP slowlog execution event.
    pub async fn insert_slowlog_entry(
        &self,
        entry: &PhpSlowlogEntry,
    ) -> Result<i64, AnalyticsError> {
        let frames_json = serde_json::to_string(&entry.frames)
            .map_err(|e| AnalyticsError::Internal(format!("Failed to serialize frames: {e}")))?;

        let query = r#"
        INSERT INTO php_slowlog_events (
            timestamp, pool, pid, script_filename,
            top_function, frames_json, duration_secs,
            site_id, tenant_user
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        let res = sqlx::query(query)
            .bind(entry.timestamp)
            .bind(&entry.pool)
            .bind(entry.pid as i64)
            .bind(&entry.script_filename)
            .bind(&entry.top_function)
            .bind(frames_json)
            .bind(entry.duration_secs)
            .bind(entry.site_id)
            .bind(&entry.tenant_user)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok(res.last_insert_rowid())
    }

    /// Insert batch of PHP slowlog execution events.
    pub async fn insert_slowlog_entries(
        &self,
        entries: &[PhpSlowlogEntry],
    ) -> Result<usize, AnalyticsError> {
        let mut count = 0;
        for entry in entries {
            self.insert_slowlog_entry(entry).await?;
            count += 1;
        }
        Ok(count)
    }

    /// Query slowlog events matching optional pool or site_id filter in time range.
    pub async fn query_slowlog_events(
        &self,
        pool: Option<&str>,
        site_id: Option<i64>,
        start_ts: i64,
        end_ts: i64,
        limit: u32,
    ) -> Result<Vec<PhpSlowlogEntry>, AnalyticsError> {
        let query = r#"
        SELECT
            id, timestamp, pool, pid, script_filename,
            top_function, frames_json, duration_secs,
            site_id, tenant_user
        FROM php_slowlog_events
        WHERE timestamp >= ? AND timestamp <= ?
          AND (? IS NULL OR pool = ?)
          AND (? IS NULL OR site_id = ?)
        ORDER BY timestamp DESC
        LIMIT ?
        "#;

        let rows = sqlx::query(query)
            .bind(start_ts)
            .bind(end_ts)
            .bind(pool)
            .bind(pool)
            .bind(site_id)
            .bind(site_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let pid_val: i64 = row.try_get("pid").map_err(AnalyticsError::Database)?;
            let frames_str: String = row
                .try_get("frames_json")
                .map_err(AnalyticsError::Database)?;
            let frames: Vec<PhpSlowlogFrame> =
                serde_json::from_str(&frames_str).unwrap_or_default();

            entries.push(PhpSlowlogEntry {
                id: Some(row.try_get("id").map_err(AnalyticsError::Database)?),
                timestamp: row.try_get("timestamp").map_err(AnalyticsError::Database)?,
                pool: row.try_get("pool").map_err(AnalyticsError::Database)?,
                pid: pid_val.max(0) as u32,
                script_filename: row
                    .try_get("script_filename")
                    .map_err(AnalyticsError::Database)?,
                top_function: row
                    .try_get("top_function")
                    .map_err(AnalyticsError::Database)?,
                frames,
                duration_secs: row
                    .try_get("duration_secs")
                    .map_err(AnalyticsError::Database)?,
                site_id: row.try_get("site_id").map_err(AnalyticsError::Database)?,
                tenant_user: row
                    .try_get("tenant_user")
                    .map_err(AnalyticsError::Database)?,
            });
        }

        Ok(entries)
    }

    // ==========================================
    // Application Process Metrics: Node / Python (P9-T4)
    // ==========================================

    /// Insert single application process raw metric point.
    pub async fn insert_app_process_point(
        &self,
        pt: &AppProcessMetricPoint,
    ) -> Result<(), AnalyticsError> {
        self.insert_app_process_points(std::slice::from_ref(pt))
            .await
    }

    /// Insert batch of application process raw metric points.
    pub async fn insert_app_process_points(
        &self,
        pts: &[AppProcessMetricPoint],
    ) -> Result<(), AnalyticsError> {
        let mut tx = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AnalyticsError::Database)?;
        let query = r#"
        INSERT OR REPLACE INTO metrics_app_process_raw (
            timestamp, site_id, app_type, service_name,
            cpu_usage_usec, cpu_percent,
            memory_current_bytes, memory_peak_bytes, memory_swap_bytes,
            pids_current, io_read_bytes_sec, io_write_bytes_sec
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        for pt in pts {
            sqlx::query(query)
                .bind(pt.timestamp)
                .bind(pt.site_id)
                .bind(pt.app_type.as_str())
                .bind(&pt.service_name)
                .bind(pt.cpu_usage_usec as i64)
                .bind(pt.cpu_percent)
                .bind(pt.memory_current_bytes as i64)
                .bind(pt.memory_peak_bytes.map(|v| v as i64))
                .bind(pt.memory_swap_bytes.map(|v| v as i64))
                .bind(pt.pids_current as i64)
                .bind(pt.io_read_bytes_sec)
                .bind(pt.io_write_bytes_sec)
                .execute(&mut *tx)
                .await
                .map_err(AnalyticsError::Database)?;
        }

        tx.commit().await.map_err(AnalyticsError::Database)?;
        Ok(())
    }

    /// Rollup application process metrics into 1-minute buckets up to `until_ts`.
    pub async fn rollup_app_process_1m(&self, until_ts: i64) -> Result<usize, AnalyticsError> {
        let query = r#"
        INSERT OR REPLACE INTO metrics_app_process_1m (
            bucket_ts, site_id, app_type,
            cpu_avg, cpu_max,
            memory_current_avg, memory_current_max, memory_peak_max,
            pids_max, io_read_avg, io_write_avg,
            sample_count
        )
        SELECT
            (timestamp / 60) * 60 AS bucket_ts,
            site_id,
            app_type,
            AVG(cpu_percent) AS cpu_avg,
            MAX(cpu_percent) AS cpu_max,
            AVG(memory_current_bytes) AS memory_current_avg,
            MAX(memory_current_bytes) AS memory_current_max,
            MAX(memory_peak_bytes) AS memory_peak_max,
            MAX(pids_current) AS pids_max,
            AVG(io_read_bytes_sec) AS io_read_avg,
            AVG(io_write_bytes_sec) AS io_write_avg,
            COUNT(*) AS sample_count
        FROM metrics_app_process_raw
        WHERE timestamp <= ?
        GROUP BY site_id, app_type, (timestamp / 60) * 60
        "#;

        let res = sqlx::query(query)
            .bind(until_ts)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok(res.rows_affected() as usize)
    }

    /// Query raw application process metrics for site_id and app_type in time range.
    pub async fn query_app_process_raw(
        &self,
        site_id: i64,
        app_type: AppType,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<Vec<AppProcessMetricPoint>, AnalyticsError> {
        let query = r#"
        SELECT
            timestamp, site_id, app_type, service_name,
            cpu_usage_usec, cpu_percent,
            memory_current_bytes, memory_peak_bytes, memory_swap_bytes,
            pids_current, io_read_bytes_sec, io_write_bytes_sec
        FROM metrics_app_process_raw
        WHERE site_id = ? AND app_type = ? AND timestamp >= ? AND timestamp <= ?
        ORDER BY timestamp ASC
        "#;

        let rows = sqlx::query(query)
            .bind(site_id)
            .bind(app_type.as_str())
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut pts = Vec::with_capacity(rows.len());
        for row in rows {
            let cpu_usec: i64 = row
                .try_get("cpu_usage_usec")
                .map_err(AnalyticsError::Database)?;
            let mem_curr: i64 = row
                .try_get("memory_current_bytes")
                .map_err(AnalyticsError::Database)?;
            let mem_peak: Option<i64> = row
                .try_get("memory_peak_bytes")
                .map_err(AnalyticsError::Database)?;
            let mem_swap: Option<i64> = row
                .try_get("memory_swap_bytes")
                .map_err(AnalyticsError::Database)?;
            let pids: i64 = row
                .try_get("pids_current")
                .map_err(AnalyticsError::Database)?;

            pts.push(AppProcessMetricPoint {
                timestamp: row.try_get("timestamp").map_err(AnalyticsError::Database)?,
                site_id: row.try_get("site_id").map_err(AnalyticsError::Database)?,
                app_type,
                service_name: row
                    .try_get("service_name")
                    .map_err(AnalyticsError::Database)?,
                cpu_usage_usec: cpu_usec.max(0) as u64,
                cpu_percent: row
                    .try_get("cpu_percent")
                    .map_err(AnalyticsError::Database)?,
                memory_current_bytes: mem_curr.max(0) as u64,
                memory_peak_bytes: mem_peak.map(|v| v.max(0) as u64),
                memory_swap_bytes: mem_swap.map(|v| v.max(0) as u64),
                pids_current: pids.max(0) as u64,
                io_read_bytes_sec: row
                    .try_get("io_read_bytes_sec")
                    .map_err(AnalyticsError::Database)?,
                io_write_bytes_sec: row
                    .try_get("io_write_bytes_sec")
                    .map_err(AnalyticsError::Database)?,
            });
        }

        Ok(pts)
    }

    /// Query 1-minute rollups for an application service in time range.
    pub async fn query_app_process_1m(
        &self,
        site_id: i64,
        app_type: AppType,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<Vec<AppProcessMetricRollup>, AnalyticsError> {
        let query = r#"
        SELECT
            bucket_ts, site_id, app_type,
            cpu_avg, cpu_max,
            memory_current_avg, memory_current_max, memory_peak_max,
            pids_max, io_read_avg, io_write_avg,
            sample_count
        FROM metrics_app_process_1m
        WHERE site_id = ? AND app_type = ? AND bucket_ts >= ? AND bucket_ts <= ?
        ORDER BY bucket_ts ASC
        "#;

        let rows = sqlx::query(query)
            .bind(site_id)
            .bind(app_type.as_str())
            .bind(start_ts)
            .bind(end_ts)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut rollups = Vec::with_capacity(rows.len());
        for row in rows {
            let mem_max: i64 = row
                .try_get("memory_current_max")
                .map_err(AnalyticsError::Database)?;
            let mem_peak: Option<i64> = row
                .try_get("memory_peak_max")
                .map_err(AnalyticsError::Database)?;
            let pids_max: i64 = row.try_get("pids_max").map_err(AnalyticsError::Database)?;
            let sample_cnt: i64 = row
                .try_get("sample_count")
                .map_err(AnalyticsError::Database)?;

            rollups.push(AppProcessMetricRollup {
                site_id: row.try_get("site_id").map_err(AnalyticsError::Database)?,
                app_type,
                bucket_ts: row.try_get("bucket_ts").map_err(AnalyticsError::Database)?,
                cpu_avg: row.try_get("cpu_avg").map_err(AnalyticsError::Database)?,
                cpu_max: row.try_get("cpu_max").map_err(AnalyticsError::Database)?,
                memory_current_avg: row
                    .try_get("memory_current_avg")
                    .map_err(AnalyticsError::Database)?,
                memory_current_max: mem_max.max(0) as u64,
                memory_peak_max: mem_peak.map(|v| v.max(0) as u64),
                pids_max: pids_max.max(0) as u64,
                io_read_avg: row
                    .try_get("io_read_avg")
                    .map_err(AnalyticsError::Database)?,
                io_write_avg: row
                    .try_get("io_write_avg")
                    .map_err(AnalyticsError::Database)?,
                sample_count: sample_cnt.max(0) as u32,
            });
        }

        Ok(rollups)
    }

    /// Insert a single security event.
    pub async fn insert_security_event(&self, ev: &SecurityEvent) -> Result<i64, AnalyticsError> {
        let query = r#"
        INSERT INTO site_security_events (
            site_id, timestamp, client_ip, method, uri, status, user_agent, country, event_type, threat_score
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        let res = sqlx::query(query)
            .bind(ev.site_id)
            .bind(ev.timestamp)
            .bind(&ev.client_ip)
            .bind(&ev.method)
            .bind(&ev.uri)
            .bind(ev.status as i64)
            .bind(&ev.user_agent)
            .bind(&ev.country)
            .bind(ev.event_type.as_str())
            .bind(ev.threat_score as i64)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok(res.last_insert_rowid())
    }

    /// Insert a batch of security events inside a transaction.
    pub async fn insert_security_events(
        &self,
        events: &[SecurityEvent],
    ) -> Result<usize, AnalyticsError> {
        if events.is_empty() {
            return Ok(0);
        }

        let mut tx = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(AnalyticsError::Database)?;
        let query = r#"
        INSERT INTO site_security_events (
            site_id, timestamp, client_ip, method, uri, status, user_agent, country, event_type, threat_score
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        for ev in events {
            sqlx::query(query)
                .bind(ev.site_id)
                .bind(ev.timestamp)
                .bind(&ev.client_ip)
                .bind(&ev.method)
                .bind(&ev.uri)
                .bind(ev.status as i64)
                .bind(&ev.user_agent)
                .bind(&ev.country)
                .bind(ev.event_type.as_str())
                .bind(ev.threat_score as i64)
                .execute(&mut *tx)
                .await
                .map_err(AnalyticsError::Database)?;
        }

        tx.commit().await.map_err(AnalyticsError::Database)?;
        Ok(events.len())
    }

    /// Query comprehensive security overview for a given site within time window.
    pub async fn query_security_overview(
        &self,
        site_id: i64,
        from_ts: i64,
        to_ts: i64,
        limit: usize,
    ) -> Result<SecurityOverviewSummary, AnalyticsError> {
        let limit = limit.clamp(1, 100);

        // 1. High-level aggregates
        let agg_query = r#"
        SELECT
            COALESCE(SUM(CASE WHEN event_type = 'probing' THEN 1 ELSE 0 END), 0) AS total_probing,
            COALESCE(SUM(CASE WHEN event_type = 'brute_force' THEN 1 ELSE 0 END), 0) AS total_brute_force,
            COUNT(DISTINCT client_ip) AS unique_ips
        FROM site_security_events
        WHERE site_id = ? AND timestamp >= ? AND timestamp <= ?
        "#;

        let agg_row = sqlx::query(agg_query)
            .bind(site_id)
            .bind(from_ts)
            .bind(to_ts)
            .fetch_one(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let total_probing: i64 = agg_row.try_get("total_probing").unwrap_or(0);
        let total_brute_force: i64 = agg_row.try_get("total_brute_force").unwrap_or(0);
        let unique_suspicious_ips: i64 = agg_row.try_get("unique_ips").unwrap_or(0);

        // 2. Top Client IPs
        let top_ips_query = r#"
        SELECT
            client_ip,
            COUNT(*) AS total_reqs,
            COALESCE(SUM(CASE WHEN status >= 400 AND status < 500 THEN 1 ELSE 0 END), 0) AS error_4xx,
            COALESCE(SUM(CASE WHEN status >= 500 AND status < 600 THEN 1 ELSE 0 END), 0) AS error_5xx,
            COALESCE(SUM(CASE WHEN event_type = 'probing' THEN 1 ELSE 0 END), 0) AS probing_cnt,
            COALESCE(SUM(CASE WHEN event_type = 'brute_force' THEN 1 ELSE 0 END), 0) AS auth_failures,
            MAX(country) AS country,
            MAX(uri) AS top_path,
            MAX(timestamp) AS last_seen
        FROM site_security_events
        WHERE site_id = ? AND timestamp >= ? AND timestamp <= ?
        GROUP BY client_ip
        ORDER BY (COALESCE(SUM(CASE WHEN event_type = 'probing' THEN 1 ELSE 0 END), 0) * 5 +
                  COALESCE(SUM(CASE WHEN event_type = 'brute_force' THEN 1 ELSE 0 END), 0) * 3 +
                  COUNT(*)) DESC
        LIMIT ?
        "#;

        let ip_rows = sqlx::query(top_ips_query)
            .bind(site_id)
            .bind(from_ts)
            .bind(to_ts)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut top_ips = Vec::with_capacity(ip_rows.len());
        for row in ip_rows {
            let total_reqs: i64 = row
                .try_get("total_reqs")
                .map_err(AnalyticsError::Database)?;
            let error_4xx: i64 = row.try_get("error_4xx").unwrap_or(0);
            let error_5xx: i64 = row.try_get("error_5xx").unwrap_or(0);
            let probing_cnt: i64 = row.try_get("probing_cnt").unwrap_or(0);
            let auth_failures: i64 = row.try_get("auth_failures").unwrap_or(0);
            let client_ip: String = row.try_get("client_ip").map_err(AnalyticsError::Database)?;
            let country: Option<String> = row.try_get("country").ok();
            let top_path: Option<String> = row.try_get("top_path").ok();
            let last_seen: i64 = row.try_get("last_seen").map_err(AnalyticsError::Database)?;

            let risk_level = calculate_ip_risk_level(
                total_reqs.max(0) as u64,
                error_4xx.max(0) as u64,
                probing_cnt.max(0) as u64,
                auth_failures.max(0) as u64,
            );

            top_ips.push(TopSecurityIp {
                ip: client_ip,
                total_requests: total_reqs.max(0) as u64,
                error_4xx: error_4xx.max(0) as u64,
                error_5xx: error_5xx.max(0) as u64,
                probing_count: probing_cnt.max(0) as u64,
                auth_failures: auth_failures.max(0) as u64,
                country,
                top_path,
                risk_level,
                last_seen,
            });
        }

        // 3. Probing paths statistics
        let probing_query = r#"
        SELECT
            uri,
            COUNT(*) AS cnt,
            COUNT(DISTINCT client_ip) AS unique_ips
        FROM site_security_events
        WHERE site_id = ? AND timestamp >= ? AND timestamp <= ? AND event_type = 'probing'
        GROUP BY uri
        ORDER BY cnt DESC
        LIMIT ?
        "#;

        let probing_rows = sqlx::query(probing_query)
            .bind(site_id)
            .bind(from_ts)
            .bind(to_ts)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut probing_paths = Vec::with_capacity(probing_rows.len());
        for row in probing_rows {
            let uri: String = row.try_get("uri").map_err(AnalyticsError::Database)?;
            let cnt: i64 = row.try_get("cnt").map_err(AnalyticsError::Database)?;
            let u_ips: i64 = row
                .try_get("unique_ips")
                .map_err(AnalyticsError::Database)?;
            let category = classify_probing_path(&uri)
                .map(|(c, _)| c.to_string())
                .unwrap_or_else(|| "unknown_probe".to_string());

            probing_paths.push(ProbingPathStat {
                path: uri,
                category,
                count: cnt.max(0) as u64,
                unique_ips: u_ips.max(0) as u64,
            });
        }

        // 4. Brute force incidents
        let bf_query = r#"
        SELECT
            client_ip,
            uri,
            COUNT(*) AS failed_attempts,
            MIN(timestamp) AS first_attempt_ts,
            MAX(timestamp) AS last_attempt_ts,
            MAX(country) AS country,
            MAX(threat_score) AS max_threat
        FROM site_security_events
        WHERE site_id = ? AND timestamp >= ? AND timestamp <= ? AND event_type = 'brute_force'
        GROUP BY client_ip, uri
        ORDER BY failed_attempts DESC
        LIMIT ?
        "#;

        let bf_rows = sqlx::query(bf_query)
            .bind(site_id)
            .bind(from_ts)
            .bind(to_ts)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(AnalyticsError::Database)?;

        let mut brute_force_incidents = Vec::with_capacity(bf_rows.len());
        for row in bf_rows {
            let ip: String = row.try_get("client_ip").map_err(AnalyticsError::Database)?;
            let uri: String = row.try_get("uri").map_err(AnalyticsError::Database)?;
            let attempts: i64 = row
                .try_get("failed_attempts")
                .map_err(AnalyticsError::Database)?;
            let first_ts: i64 = row
                .try_get("first_attempt_ts")
                .map_err(AnalyticsError::Database)?;
            let last_ts: i64 = row
                .try_get("last_attempt_ts")
                .map_err(AnalyticsError::Database)?;
            let country: Option<String> = row.try_get("country").ok();
            let max_threat: i64 = row.try_get("max_threat").unwrap_or(70);

            brute_force_incidents.push(BruteForceIncident {
                ip,
                target_path: uri,
                failed_attempts: attempts.max(0) as u64,
                first_attempt_ts: first_ts,
                last_attempt_ts: last_ts,
                country,
                risk_score: max_threat.max(0) as u32,
            });
        }

        Ok(SecurityOverviewSummary {
            total_probing_requests: total_probing.max(0) as u64,
            total_brute_force_attempts: total_brute_force.max(0) as u64,
            unique_suspicious_ips: unique_suspicious_ips.max(0) as u64,
            top_ips,
            probing_paths,
            brute_force_incidents,
        })
    }

    /// Prune old security events older than `cutoff_ts`.
    pub async fn prune_security_events(&self, cutoff_ts: i64) -> Result<u64, AnalyticsError> {
        let res = sqlx::query("DELETE FROM site_security_events WHERE timestamp < ?")
            .bind(cutoff_ts)
            .execute(&self.write_pool)
            .await
            .map_err(AnalyticsError::Database)?;

        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::SecurityEventType;

    #[tokio::test]
    async fn test_storage_lifecycle_and_rollups() {
        let storage = AnalyticsStorage::connect_in_memory()
            .await
            .expect("in-memory db");

        // Insert 6 raw points across 1 minute (every 10 seconds)
        let base_ts = 1770000000;
        for i in 0..6 {
            let ts = base_ts + (i * 10);
            storage
                .insert_server_point(&ServerMetricsPoint {
                    timestamp: ts,
                    cpu_percent: 10.0 + (i as f64 * 2.0),
                    cpu_cores: 4,
                    memory_used_bytes: 1000 + i as u64 * 100,
                    memory_total_bytes: 8000,
                    swap_used_bytes: 0,
                    swap_total_bytes: 2000,
                    load1: 0.5,
                    load5: 0.4,
                    load15: 0.3,
                    disk_used_bytes: 5000,
                    disk_total_bytes: 20000,
                    inodes_used: 100,
                    inodes_total: 1000,
                    net_rx_bytes_sec: 1024.0,
                    net_tx_bytes_sec: 2048.0,
                    io_read_bytes_sec: 512.0,
                    io_write_bytes_sec: 256.0,
                })
                .await
                .expect("insert server");

            storage
                .insert_tenant_points(&[TenantMetricsPoint {
                    timestamp: ts,
                    uid: 10001,
                    cpu_usage_usec: 100000 * (i as u64 + 1),
                    cpu_percent: 5.0 + i as f64,
                    memory_current_bytes: 500000 + i as u64 * 10000,
                    memory_peak_bytes: Some(600000),
                    memory_swap_bytes: None,
                    pids_current: 5 + i as u64,
                    io_read_bytes_sec: 100.0,
                    io_write_bytes_sec: 200.0,
                }])
                .await
                .expect("insert tenant");

            storage
                .insert_service_points(&[ServiceMetricsPoint {
                    timestamp: ts,
                    service_name: "nginx".to_string(),
                    cpu_usage_usec: 50000 * (i as u64 + 1),
                    cpu_percent: 2.0 + i as f64,
                    memory_current_bytes: 200000,
                    pids_current: 4,
                    io_read_bytes_sec: 50.0,
                    io_write_bytes_sec: 100.0,
                }])
                .await
                .expect("insert service");
        }

        // Run 1m rollup for timestamp base_ts + 60
        let (s_1m, t_1m, svc_1m) = storage.rollup_1m(base_ts + 60).await.expect("rollup 1m");
        assert_eq!(s_1m, 1);
        assert_eq!(t_1m, 1);
        assert_eq!(svc_1m, 1);

        // Query server rollups
        let server_rollups = storage
            .query_server_rollups(base_ts - 10, base_ts + 100, RollupResolution::OneMinute)
            .await
            .expect("query server rollups");
        assert_eq!(server_rollups.len(), 1);
        assert_eq!(server_rollups[0].sample_count, 6);
        assert_eq!(server_rollups[0].cpu_min, 10.0);
        assert_eq!(server_rollups[0].cpu_max, 20.0);
        assert_eq!(server_rollups[0].cpu_avg, 15.0);

        // Query tenant rollups
        let tenant_rollups = storage
            .query_tenant_rollups(
                10001,
                base_ts - 10,
                base_ts + 100,
                RollupResolution::OneMinute,
            )
            .await
            .expect("query tenant rollups");
        assert_eq!(tenant_rollups.len(), 1);
        assert_eq!(tenant_rollups[0].sample_count, 6);
        assert_eq!(tenant_rollups[0].uid, 10001);

        // Query service rollups
        let svc_rollups = storage
            .query_service_rollups(
                "nginx",
                base_ts - 10,
                base_ts + 100,
                RollupResolution::OneMinute,
            )
            .await
            .expect("query service rollups");
        assert_eq!(svc_rollups.len(), 1);
        assert_eq!(svc_rollups[0].service_name, "nginx");

        // Run 1h rollup
        let (s_1h, t_1h, svc_1h) = storage.rollup_1h(base_ts + 3600).await.expect("rollup 1h");
        assert_eq!(s_1h, 1);
        assert_eq!(t_1h, 1);
        assert_eq!(svc_1h, 1);

        // Prune test: with cutoff right after base_ts + 100
        let retention = RetentionConfig {
            raw_retention_secs: 10,
            one_minute_retention_secs: 10,
            one_hour_retention_secs: 10,
            one_day_retention_secs: None,
            ..Default::default()
        };
        let deleted = storage
            .prune_expired(base_ts + 200, &retention)
            .await
            .expect("prune");
        // 6 server raw + 6 tenant raw + 6 service raw + 1 server 1m + 1 tenant 1m + 1 svc 1m + 1 server 1h + 1 tenant 1h + 1 svc 1h = 24
        assert!(deleted >= 18);
    }

    #[tokio::test]
    async fn test_security_events_storage_and_overview() {
        let storage = AnalyticsStorage::connect_in_memory()
            .await
            .expect("in-memory db");

        let site_id = 42;
        let now = 1770001000;

        let events = vec![
            SecurityEvent {
                id: None,
                site_id,
                timestamp: now - 300,
                client_ip: "198.51.100.1".to_string(),
                method: "GET".to_string(),
                uri: "/.env".to_string(),
                status: 404,
                user_agent: Some("curl/7.81".to_string()),
                country: Some("ID".to_string()),
                event_type: SecurityEventType::Probing,
                threat_score: 85,
            },
            SecurityEvent {
                id: None,
                site_id,
                timestamp: now - 200,
                client_ip: "198.51.100.1".to_string(),
                method: "GET".to_string(),
                uri: "/.git/config".to_string(),
                status: 404,
                user_agent: Some("curl/7.81".to_string()),
                country: Some("ID".to_string()),
                event_type: SecurityEventType::Probing,
                threat_score: 80,
            },
            SecurityEvent {
                id: None,
                site_id,
                timestamp: now - 100,
                client_ip: "203.0.113.50".to_string(),
                method: "POST".to_string(),
                uri: "/wp-login.php".to_string(),
                status: 401,
                user_agent: Some("python-requests/2.28".to_string()),
                country: Some("RU".to_string()),
                event_type: SecurityEventType::BruteForce,
                threat_score: 70,
            },
            SecurityEvent {
                id: None,
                site_id,
                timestamp: now - 50,
                client_ip: "203.0.113.50".to_string(),
                method: "POST".to_string(),
                uri: "/wp-login.php".to_string(),
                status: 401,
                user_agent: Some("python-requests/2.28".to_string()),
                country: Some("RU".to_string()),
                event_type: SecurityEventType::BruteForce,
                threat_score: 70,
            },
        ];

        let inserted = storage
            .insert_security_events(&events)
            .await
            .expect("insert security events");
        assert_eq!(inserted, 4);

        let overview = storage
            .query_security_overview(site_id, now - 600, now, 10)
            .await
            .expect("query security overview");

        assert_eq!(overview.total_probing_requests, 2);
        assert_eq!(overview.total_brute_force_attempts, 2);
        assert_eq!(overview.unique_suspicious_ips, 2);

        // Top IPs
        assert_eq!(overview.top_ips.len(), 2);
        let top_ip_1 = overview
            .top_ips
            .iter()
            .find(|i| i.ip == "198.51.100.1")
            .expect("found ip 1");
        assert_eq!(top_ip_1.probing_count, 2);
        assert_eq!(top_ip_1.country.as_deref(), Some("ID"));

        let top_ip_2 = overview
            .top_ips
            .iter()
            .find(|i| i.ip == "203.0.113.50")
            .expect("found ip 2");
        assert_eq!(top_ip_2.auth_failures, 2);
        assert_eq!(top_ip_2.country.as_deref(), Some("RU"));

        // Probing paths
        assert_eq!(overview.probing_paths.len(), 2);
        assert!(
            overview
                .probing_paths
                .iter()
                .any(|p| p.path == "/.env" && p.category == "env_credentials")
        );
        assert!(
            overview
                .probing_paths
                .iter()
                .any(|p| p.path == "/.git/config" && p.category == "vcs_leak")
        );

        // Brute force incidents
        assert_eq!(overview.brute_force_incidents.len(), 1);
        assert_eq!(overview.brute_force_incidents[0].ip, "203.0.113.50");
        assert_eq!(
            overview.brute_force_incidents[0].target_path,
            "/wp-login.php"
        );
        assert_eq!(overview.brute_force_incidents[0].failed_attempts, 2);

        // Pruning
        let pruned = storage
            .prune_security_events(now - 150)
            .await
            .expect("prune");
        assert_eq!(pruned, 2); // 2 events older than now - 150 removed

        let after_prune = storage
            .query_security_overview(site_id, now - 600, now, 10)
            .await
            .expect("query security overview after prune");
        assert_eq!(after_prune.total_probing_requests, 0);
        assert_eq!(after_prune.total_brute_force_attempts, 2);
    }

    #[tokio::test]
    async fn test_storage_stats_and_integrity() {
        let storage = AnalyticsStorage::connect_in_memory()
            .await
            .expect("in-memory db");

        let integrity = storage.check_integrity().await.expect("check integrity");
        assert!(integrity.is_healthy);
        assert_eq!(integrity.status, "ok");

        let stats = storage.get_storage_stats().await.expect("storage stats");
        assert!(stats.is_healthy);
        assert_eq!(stats.integrity_message, "ok");
        assert_eq!(stats.table_stats.len(), 23);
        assert!(
            stats
                .table_stats
                .iter()
                .any(|t| t.table_name == "metrics_server_raw")
        );
        assert!(
            stats
                .table_stats
                .iter()
                .any(|t| t.table_name == "site_metrics_1m")
        );
        assert!(
            stats
                .table_stats
                .iter()
                .any(|t| t.table_name == "site_security_events")
        );
    }

    #[tokio::test]
    async fn test_run_maintenance_comprehensive() {
        let storage = AnalyticsStorage::connect_in_memory()
            .await
            .expect("in-memory db");

        let now = 1_700_000_000i64;

        // Insert raw server point
        let s_pt = ServerMetricsPoint {
            timestamp: now - 10000,
            cpu_percent: 15.0,
            cpu_cores: 4,
            memory_used_bytes: 1024,
            memory_total_bytes: 4096,
            swap_used_bytes: 0,
            swap_total_bytes: 1024,
            load1: 0.5,
            load5: 0.5,
            load15: 0.5,
            disk_used_bytes: 5000,
            disk_total_bytes: 20000,
            inodes_used: 100,
            inodes_total: 1000,
            net_rx_bytes_sec: 100.0,
            net_tx_bytes_sec: 100.0,
            io_read_bytes_sec: 50.0,
            io_write_bytes_sec: 50.0,
        };
        storage
            .insert_server_point(&s_pt)
            .await
            .expect("insert server raw");

        // Insert security event
        let sec_ev = SecurityEvent {
            id: None,
            site_id: 1,
            timestamp: now - 800_000, // 9+ days old
            client_ip: "192.0.2.1".to_string(),
            method: "GET".to_string(),
            uri: "/.env".to_string(),
            status: 404,
            user_agent: None,
            country: None,
            event_type: crate::security::SecurityEventType::Probing,
            threat_score: 80,
        };
        storage
            .insert_security_events(&[sec_ev])
            .await
            .expect("insert security event");

        // Insert checkpoint
        let cp = LogCheckpoint {
            file_path: "/var/log/old.log".to_string(),
            site_id: 1,
            inode: 1234,
            file_offset: 500,
            last_processed_ts: now - 35 * 86400,
            updated_at: now - 35 * 86400,
        };
        storage
            .save_log_checkpoint(&cp)
            .await
            .expect("save checkpoint");

        let config = RetentionConfig {
            raw_retention_secs: 7200,
            one_minute_retention_secs: 86400,
            one_hour_retention_secs: 864000,
            one_day_retention_secs: Some(8640000),
            security_events_retention_secs: 7 * 86400,
            max_db_size_bytes: Some(1024 * 1024 * 1024),
            auto_vacuum: true,
        };

        let report = storage
            .run_maintenance(now, &config, true)
            .await
            .expect("run maintenance");

        assert!(report.raw_records_pruned >= 1);
        assert_eq!(report.security_events_pruned, 1);
        assert_eq!(report.checkpoints_pruned, 1);
        assert!(report.total_records_pruned >= 3);
        assert!(report.wal_checkpoint_executed);
        assert!(report.vacuum_executed);
        assert!(!report.emergency_pruning_triggered);
    }

    #[tokio::test]
    async fn test_recover_corrupted_database() {
        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join(format!(
            "test_analytics_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db_path_str = db_file.to_string_lossy().to_string();

        // Write corrupt garbage to the db file
        tokio::fs::write(&db_file, b"NOT_A_VALID_SQLITE_DATABASE_HEADER_CORRUPTED")
            .await
            .expect("write corrupt");

        let report = AnalyticsStorage::recover_corrupted_database(&db_path_str)
            .await
            .expect("recover");
        assert!(report.success);
        assert!(report.backup_path.is_some());

        // Connect to newly created database
        let storage = AnalyticsStorage::connect(&db_file)
            .await
            .expect("connect recovered db");
        let integrity = storage
            .check_integrity()
            .await
            .expect("check integrity of fresh db");
        assert!(integrity.is_healthy);

        // Cleanup
        let _ = tokio::fs::remove_file(&db_file).await;
        if let Some(backup) = report.backup_path {
            let _ = tokio::fs::remove_file(backup).await;
        }
    }
}
