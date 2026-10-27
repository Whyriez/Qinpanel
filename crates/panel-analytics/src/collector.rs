use crate::cgroup::CgroupMetricsCollector;
use crate::disk::DiskUsageReader;
use crate::error::AnalyticsError;
use crate::models::{
    MetricsCollectionReport, RetentionConfig, RollupReport, ServerMetricsPoint,
    ServiceMetricsPoint, TenantMetricsPoint,
};
use crate::proc::ProcReader;
use crate::storage::AnalyticsStorage;
use std::sync::Arc;

/// Comprehensive system metrics collector covering server (/proc), tenant cgroups, and service cgroups.
pub struct SystemMetricsCollector {
    proc_reader: Arc<ProcReader>,
    cgroup_collector: Arc<CgroupMetricsCollector>,
    disk_reader: Arc<DiskUsageReader>,
    storage: AnalyticsStorage,
    retention_config: RetentionConfig,
}

impl SystemMetricsCollector {
    pub fn new(
        proc_reader: Arc<ProcReader>,
        cgroup_collector: Arc<CgroupMetricsCollector>,
        disk_reader: Arc<DiskUsageReader>,
        storage: AnalyticsStorage,
        retention_config: RetentionConfig,
    ) -> Self {
        Self {
            proc_reader,
            cgroup_collector,
            disk_reader,
            storage,
            retention_config,
        }
    }

    pub fn storage(&self) -> &AnalyticsStorage {
        &self.storage
    }

    pub fn retention_config(&self) -> &RetentionConfig {
        &self.retention_config
    }

    /// Collect single snapshot of host/server resource metrics.
    pub async fn collect_server_point(
        &self,
        now_ts: i64,
    ) -> Result<ServerMetricsPoint, AnalyticsError> {
        let cpu = self.proc_reader.read_cpu_stats()?;
        let mem = self.proc_reader.read_memory_stats()?;
        let load = self.proc_reader.read_load_stats()?;
        let net = self.proc_reader.read_net_stats()?;
        let disk_io = self.proc_reader.read_disk_io_stats()?;
        let disk = self.disk_reader.read_disk_stats()?;

        Ok(ServerMetricsPoint {
            timestamp: now_ts,
            cpu_percent: cpu.cpu_percent,
            cpu_cores: cpu.cpu_cores,
            memory_used_bytes: mem.mem_used_bytes,
            memory_total_bytes: mem.mem_total_bytes,
            swap_used_bytes: mem.swap_used_bytes,
            swap_total_bytes: mem.swap_total_bytes,
            load1: load.load1,
            load5: load.load5,
            load15: load.load15,
            disk_used_bytes: disk.disk_used_bytes,
            disk_total_bytes: disk.disk_total_bytes,
            inodes_used: disk.inodes_used,
            inodes_total: disk.inodes_total,
            net_rx_bytes_sec: net.rx_bytes_sec,
            net_tx_bytes_sec: net.tx_bytes_sec,
            io_read_bytes_sec: disk_io.read_bytes_sec,
            io_write_bytes_sec: disk_io.write_bytes_sec,
        })
    }

    /// Collect tenant metrics for all active tenant UIDs.
    pub async fn collect_tenant_points(
        &self,
        active_uids: &[u32],
        now_ts: i64,
    ) -> Result<Vec<TenantMetricsPoint>, AnalyticsError> {
        let mut results = Vec::with_capacity(active_uids.len());
        for &uid in active_uids {
            let pt = self.cgroup_collector.collect_tenant_metrics(uid, now_ts)?;
            results.push(pt);
        }
        Ok(results)
    }

    /// Collect service metrics for monitored system services.
    pub async fn collect_service_points(
        &self,
        monitored_services: &[&str],
        now_ts: i64,
    ) -> Result<Vec<ServiceMetricsPoint>, AnalyticsError> {
        let mut results = Vec::with_capacity(monitored_services.len());
        for &svc in monitored_services {
            let pt = self.cgroup_collector.collect_service_metrics(svc, now_ts)?;
            results.push(pt);
        }
        Ok(results)
    }

    /// Collect 10-second metrics across all 3 scopes and insert into raw SQLite tables.
    pub async fn collect_and_store(
        &self,
        active_uids: &[u32],
        monitored_services: &[&str],
        now_ts: i64,
    ) -> Result<MetricsCollectionReport, AnalyticsError> {
        let server_pt = self.collect_server_point(now_ts).await?;
        self.storage.insert_server_point(&server_pt).await?;

        let tenant_pts = self.collect_tenant_points(active_uids, now_ts).await?;
        self.storage.insert_tenant_points(&tenant_pts).await?;

        let service_pts = self
            .collect_service_points(monitored_services, now_ts)
            .await?;
        self.storage.insert_service_points(&service_pts).await?;

        Ok(MetricsCollectionReport {
            timestamp: now_ts,
            server_collected: true,
            tenants_collected: tenant_pts.len(),
            services_collected: service_pts.len(),
        })
    }

    /// Run rollups (1m, 1h, 1d) and prune expired raw and aggregated metrics.
    pub async fn execute_rollups_and_retention(
        &self,
        now_ts: i64,
    ) -> Result<RollupReport, AnalyticsError> {
        let (s_1m, t_1m, svc_1m) = self.storage.rollup_1m(now_ts).await?;
        let (s_1h, t_1h, svc_1h) = self.storage.rollup_1h(now_ts).await?;
        let (s_1d, t_1d, svc_1d) = self.storage.rollup_1d(now_ts).await?;

        let pruned_records = self
            .storage
            .prune_expired(now_ts, &self.retention_config)
            .await?;

        Ok(RollupReport {
            timestamp: now_ts,
            server_1m_created: s_1m,
            server_1h_created: s_1h,
            server_1d_created: s_1d,
            tenant_1m_created: t_1m,
            tenant_1h_created: t_1h,
            tenant_1d_created: t_1d,
            service_1m_created: svc_1m,
            service_1h_created: svc_1h,
            service_1d_created: svc_1d,
            pruned_records,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::DiskStats;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_system_metrics_collector_end_to_end() {
        let tmp = tempdir().expect("tempdir");
        let proc_dir = tmp.path().join("proc");
        let cgroup_dir = tmp.path().join("cgroup");
        std::fs::create_dir_all(&proc_dir).expect("proc_dir");
        std::fs::create_dir_all(&cgroup_dir).expect("cgroup_dir");

        // Prepare mock /proc
        std::fs::write(
            proc_dir.join("stat"),
            "cpu  500 100 200 4200 0 0 0 0 0 0\ncpu0 500 100 200 4200 0 0 0 0 0 0\n",
        )
        .expect("write stat");
        std::fs::write(
            proc_dir.join("meminfo"),
            "MemTotal: 8192000 kB\nMemFree: 2048000 kB\nMemAvailable: 4096000 kB\nSwapTotal: 1024000 kB\nSwapFree: 512000 kB\n",
        )
        .expect("write meminfo");
        std::fs::write(proc_dir.join("loadavg"), "0.50 0.80 0.90 2/200 9999\n")
            .expect("write loadavg");
        std::fs::create_dir_all(proc_dir.join("net")).expect("mkdir net");
        std::fs::write(
            proc_dir.join("net").join("dev"),
            "hdr\nhdr2\neth0: 1048576 100 0 0 0 0 0 0 524288 50 0 0 0 0 0 0\n",
        )
        .expect("write net");
        std::fs::write(
            proc_dir.join("diskstats"),
            "8 0 sda 10 0 2048 0 20 0 4096 0 0 0 0\n",
        )
        .expect("write diskstats");

        // Prepare mock tenant cgroup
        let tenant_dir = cgroup_dir.join("panel-u10001.slice");
        std::fs::create_dir_all(&tenant_dir).expect("mkdir tenant");
        std::fs::write(tenant_dir.join("cpu.stat"), "usage_usec 200000\n").expect("write cpu.stat");
        std::fs::write(tenant_dir.join("memory.current"), "67108864\n")
            .expect("write memory.current");
        std::fs::write(tenant_dir.join("pids.current"), "8\n").expect("write pids.current");
        std::fs::write(
            tenant_dir.join("io.stat"),
            "8:0 rbytes=100000 wbytes=200000\n",
        )
        .expect("write io.stat");

        // Prepare mock service cgroup
        let svc_dir = cgroup_dir.join("system.slice").join("nginx.service");
        std::fs::create_dir_all(&svc_dir).expect("mkdir svc");
        std::fs::write(svc_dir.join("cpu.stat"), "usage_usec 150000\n")
            .expect("write svc cpu.stat");
        std::fs::write(svc_dir.join("memory.current"), "33554432\n")
            .expect("write svc memory.current");
        std::fs::write(svc_dir.join("pids.current"), "2\n").expect("write svc pids.current");

        let proc_reader = Arc::new(ProcReader::new(&proc_dir));
        let cgroup_collector = Arc::new(CgroupMetricsCollector::new(&cgroup_dir));
        let disk_reader = Arc::new(DiskUsageReader::default_root());
        disk_reader.set_mock_stats(DiskStats {
            disk_total_bytes: 100 * 1024 * 1024 * 1024,
            disk_used_bytes: 30 * 1024 * 1024 * 1024,
            inodes_total: 10_000_000,
            inodes_used: 1_500_000,
        });

        let storage = AnalyticsStorage::connect_in_memory()
            .await
            .expect("storage");
        let collector = SystemMetricsCollector::new(
            proc_reader,
            cgroup_collector,
            disk_reader,
            storage,
            RetentionConfig::default(),
        );

        let now_ts = 1770000000;
        let report = collector
            .collect_and_store(&[10001], &["nginx"], now_ts)
            .await
            .expect("collect and store");

        assert!(report.server_collected);
        assert_eq!(report.tenants_collected, 1);
        assert_eq!(report.services_collected, 1);

        // Run rollup
        let rollup_rep = collector
            .execute_rollups_and_retention(now_ts + 70)
            .await
            .expect("rollup and prune");
        assert_eq!(rollup_rep.server_1m_created, 1);
        assert_eq!(rollup_rep.tenant_1m_created, 1);
        assert_eq!(rollup_rep.service_1m_created, 1);
    }
}
