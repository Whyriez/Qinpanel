use crate::error::AnalyticsError;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Raw snapshot of CPU ticks from `/proc/stat`.
#[derive(Debug, Clone, Copy, Default)]
struct CpuTicks {
    total: u64,
    idle: u64,
    cores: u32,
}

/// Raw snapshot of network bytes from `/proc/net/dev`.
#[derive(Debug, Clone, Copy, Default)]
struct NetBytes {
    rx_bytes: u64,
    tx_bytes: u64,
}

/// Raw snapshot of disk I/O bytes from `/proc/diskstats`.
#[derive(Debug, Clone, Copy, Default)]
struct DiskIoBytes {
    read_bytes: u64,
    write_bytes: u64,
}

#[derive(Debug, Default)]
struct ProcTrackerState {
    last_cpu: Option<(CpuTicks, Instant)>,
    last_net: Option<(NetBytes, Instant)>,
    last_disk: Option<(DiskIoBytes, Instant)>,
}

/// Reader for host-level metrics from `/proc`.
pub struct ProcReader {
    proc_root: PathBuf,
    state: Mutex<ProcTrackerState>,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    pub mem_total_bytes: u64,
    pub mem_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LoadStats {
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub running_tasks: u32,
    pub total_tasks: u32,
}

#[derive(Debug, Clone, Default)]
pub struct CpuStats {
    pub cpu_percent: f64,
    pub cpu_cores: u32,
}

#[derive(Debug, Clone, Default)]
pub struct NetStats {
    pub rx_bytes_sec: f64,
    pub tx_bytes_sec: f64,
}

#[derive(Debug, Clone, Default)]
pub struct DiskIoStats {
    pub read_bytes_sec: f64,
    pub write_bytes_sec: f64,
}

impl ProcReader {
    pub fn new(proc_root: impl AsRef<Path>) -> Self {
        Self {
            proc_root: proc_root.as_ref().to_path_buf(),
            state: Mutex::new(ProcTrackerState::default()),
        }
    }

    pub fn default_system() -> Self {
        Self::new("/proc")
    }

    /// Read CPU utilization and core count from `/proc/stat`.
    pub fn read_cpu_stats(&self) -> Result<CpuStats, AnalyticsError> {
        let stat_path = self.proc_root.join("stat");
        let content = fs::read_to_string(&stat_path).map_err(AnalyticsError::Io)?;
        let current_ticks = parse_proc_stat(&content)?;
        let now = Instant::now();

        let mut lock = self.state.lock().map_err(|e| {
            AnalyticsError::System(format!("Mutex lock error on proc tracker: {e}"))
        })?;

        let cpu_percent = if let Some((prev_ticks, prev_time)) = lock.last_cpu {
            let total_delta = current_ticks.total.saturating_sub(prev_ticks.total);
            let idle_delta = current_ticks.idle.saturating_sub(prev_ticks.idle);
            let _elapsed = now.duration_since(prev_time);

            if total_delta > 0 {
                let usage = 100.0 * (1.0 - (idle_delta as f64 / total_delta as f64));
                usage.clamp(0.0, 100.0)
            } else {
                0.0
            }
        } else {
            0.0
        };

        lock.last_cpu = Some((current_ticks, now));

        Ok(CpuStats {
            cpu_percent,
            cpu_cores: current_ticks.cores,
        })
    }

    /// Read memory and swap consumption from `/proc/meminfo`.
    pub fn read_memory_stats(&self) -> Result<MemoryStats, AnalyticsError> {
        let meminfo_path = self.proc_root.join("meminfo");
        let content = fs::read_to_string(&meminfo_path).map_err(AnalyticsError::Io)?;
        parse_proc_meminfo(&content)
    }

    /// Read 1m, 5m, 15m load averages from `/proc/loadavg`.
    pub fn read_load_stats(&self) -> Result<LoadStats, AnalyticsError> {
        let loadavg_path = self.proc_root.join("loadavg");
        let content = fs::read_to_string(&loadavg_path).map_err(AnalyticsError::Io)?;
        parse_proc_loadavg(&content)
    }

    /// Read network interface I/O rates from `/proc/net/dev`.
    pub fn read_net_stats(&self) -> Result<NetStats, AnalyticsError> {
        let net_path = self.proc_root.join("net").join("dev");
        let content = fs::read_to_string(&net_path).map_err(AnalyticsError::Io)?;
        let current_net = parse_proc_net_dev(&content)?;
        let now = Instant::now();

        let mut lock = self.state.lock().map_err(|e| {
            AnalyticsError::System(format!("Mutex lock error on proc tracker: {e}"))
        })?;

        let (rx_bytes_sec, tx_bytes_sec) = if let Some((prev_net, prev_time)) = lock.last_net {
            let elapsed_secs = now.duration_since(prev_time).as_secs_f64();
            if elapsed_secs > 0.001 {
                let rx_delta = current_net.rx_bytes.saturating_sub(prev_net.rx_bytes);
                let tx_delta = current_net.tx_bytes.saturating_sub(prev_net.tx_bytes);
                (
                    rx_delta as f64 / elapsed_secs,
                    tx_delta as f64 / elapsed_secs,
                )
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        lock.last_net = Some((current_net, now));

        Ok(NetStats {
            rx_bytes_sec,
            tx_bytes_sec,
        })
    }

    /// Read disk read/write throughput rates from `/proc/diskstats`.
    pub fn read_disk_io_stats(&self) -> Result<DiskIoStats, AnalyticsError> {
        let disk_path = self.proc_root.join("diskstats");
        let content = fs::read_to_string(&disk_path).map_err(AnalyticsError::Io)?;
        let current_disk = parse_proc_diskstats(&content)?;
        let now = Instant::now();

        let mut lock = self.state.lock().map_err(|e| {
            AnalyticsError::System(format!("Mutex lock error on proc tracker: {e}"))
        })?;

        let (read_bytes_sec, write_bytes_sec) = if let Some((prev_disk, prev_time)) = lock.last_disk
        {
            let elapsed_secs = now.duration_since(prev_time).as_secs_f64();
            if elapsed_secs > 0.001 {
                let r_delta = current_disk.read_bytes.saturating_sub(prev_disk.read_bytes);
                let w_delta = current_disk
                    .write_bytes
                    .saturating_sub(prev_disk.write_bytes);
                (r_delta as f64 / elapsed_secs, w_delta as f64 / elapsed_secs)
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        lock.last_disk = Some((current_disk, now));

        Ok(DiskIoStats {
            read_bytes_sec,
            write_bytes_sec,
        })
    }
}

fn parse_proc_stat(content: &str) -> Result<CpuTicks, AnalyticsError> {
    let mut total_cpu = None;
    let mut cores = 0u32;

    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let Some(tag) = parts.next() else { continue };

        if tag == "cpu" {
            // Aggregate cpu line: user nice system idle iowait irq softirq steal guest guest_nice
            let nums: Vec<u64> = parts.filter_map(|s| s.parse::<u64>().ok()).collect();
            if nums.len() >= 4 {
                let user = nums[0];
                let nice = nums[1];
                let system = nums[2];
                let idle = nums[3];
                let iowait = nums.get(4).copied().unwrap_or(0);
                let irq = nums.get(5).copied().unwrap_or(0);
                let softirq = nums.get(6).copied().unwrap_or(0);
                let steal = nums.get(7).copied().unwrap_or(0);

                let total = user + nice + system + idle + iowait + irq + softirq + steal;
                let idle_all = idle + iowait;
                total_cpu = Some((total, idle_all));
            }
        } else if tag.starts_with("cpu") && tag[3..].chars().all(|c| c.is_ascii_digit()) {
            cores += 1;
        }
    }

    let (total, idle) = total_cpu
        .ok_or_else(|| AnalyticsError::Parse("Missing aggregate 'cpu' line in stat".to_string()))?;

    Ok(CpuTicks {
        total,
        idle,
        cores: if cores > 0 { cores } else { 1 },
    })
}

fn parse_proc_meminfo(content: &str) -> Result<MemoryStats, AnalyticsError> {
    let mut mem_total_kb = 0u64;
    let mut mem_free_kb = 0u64;
    let mut mem_avail_kb: Option<u64> = None;
    let mut buffers_kb = 0u64;
    let mut cached_kb = 0u64;
    let mut swap_total_kb = 0u64;
    let mut swap_free_kb = 0u64;

    for line in content.lines() {
        let mut parts = line.split(':');
        let Some(key) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(val_part) = parts.next().map(str::trim) else {
            continue;
        };
        let num: u64 = val_part
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        match key {
            "MemTotal" => mem_total_kb = num,
            "MemFree" => mem_free_kb = num,
            "MemAvailable" => mem_avail_kb = Some(num),
            "Buffers" => buffers_kb = num,
            "Cached" => cached_kb = num,
            "SwapTotal" => swap_total_kb = num,
            "SwapFree" => swap_free_kb = num,
            _ => {}
        }
    }

    let mem_total_bytes = mem_total_kb * 1024;
    let mem_used_bytes = match mem_avail_kb {
        Some(avail) => mem_total_kb.saturating_sub(avail) * 1024,
        None => mem_total_kb.saturating_sub(mem_free_kb + buffers_kb + cached_kb) * 1024,
    };

    let swap_total_bytes = swap_total_kb * 1024;
    let swap_used_bytes = swap_total_kb.saturating_sub(swap_free_kb) * 1024;

    Ok(MemoryStats {
        mem_total_bytes,
        mem_used_bytes,
        swap_total_bytes,
        swap_used_bytes,
    })
}

fn parse_proc_loadavg(content: &str) -> Result<LoadStats, AnalyticsError> {
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(AnalyticsError::Parse(
            "Invalid loadavg line format".to_string(),
        ));
    }

    let load1 = parts[0].parse::<f64>().unwrap_or(0.0);
    let load5 = parts[1].parse::<f64>().unwrap_or(0.0);
    let load15 = parts[2].parse::<f64>().unwrap_or(0.0);

    let (running_tasks, total_tasks) = if parts.len() >= 4 {
        let task_parts: Vec<&str> = parts[3].split('/').collect();
        if task_parts.len() == 2 {
            (
                task_parts[0].parse().unwrap_or(0),
                task_parts[1].parse().unwrap_or(0),
            )
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    Ok(LoadStats {
        load1,
        load5,
        load15,
        running_tasks,
        total_tasks,
    })
}

fn parse_proc_net_dev(content: &str) -> Result<NetBytes, AnalyticsError> {
    let mut total_rx = 0u64;
    let mut total_tx = 0u64;

    for line in content.lines().skip(2) {
        let mut colon_parts = line.split(':');
        let Some(iface) = colon_parts.next().map(str::trim) else {
            continue;
        };
        let Some(stats) = colon_parts.next() else {
            continue;
        };

        // Skip loopback interface
        if iface == "lo" {
            continue;
        }

        let fields: Vec<u64> = stats
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();

        if fields.len() >= 9 {
            total_rx += fields[0]; // bytes received
            total_tx += fields[8]; // bytes transmitted
        }
    }

    Ok(NetBytes {
        rx_bytes: total_rx,
        tx_bytes: total_tx,
    })
}

fn parse_proc_diskstats(content: &str) -> Result<DiskIoBytes, AnalyticsError> {
    let mut total_read_bytes = 0u64;
    let mut total_write_bytes = 0u64;

    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 10 {
            continue;
        }

        let dev_name = fields[2];
        // Exclude loop devices, ram disks, and typical virtual block devices
        if dev_name.starts_with("loop")
            || dev_name.starts_with("ram")
            || dev_name.starts_with("dm-")
        {
            continue;
        }

        // Field 5: sectors read (index 5)
        // Field 9: sectors written (index 9)
        if let (Ok(r_sec), Ok(w_sec)) = (fields[5].parse::<u64>(), fields[9].parse::<u64>()) {
            total_read_bytes += r_sec * 512;
            total_write_bytes += w_sec * 512;
        }
    }

    Ok(DiskIoBytes {
        read_bytes: total_read_bytes,
        write_bytes: total_write_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_proc_stat() {
        let stat_text = "cpu  1000 200 300 5000 100 50 20 10 0 0\n\
                         cpu0 500 100 150 2500 50 25 10 5 0 0\n\
                         cpu1 500 100 150 2500 50 25 10 5 0 0\n\
                         intr 1234567\n";

        let ticks = parse_proc_stat(stat_text).expect("parse stat");
        assert_eq!(ticks.cores, 2);
        assert_eq!(ticks.idle, 5100); // idle(5000) + iowait(100)
        assert_eq!(ticks.total, 6680);
    }

    #[test]
    fn test_parse_proc_meminfo() {
        let mem_text = "MemTotal:        8192000 kB\n\
                        MemFree:         2048000 kB\n\
                        MemAvailable:    4096000 kB\n\
                        Buffers:          100000 kB\n\
                        Cached:           900000 kB\n\
                        SwapTotal:       2048000 kB\n\
                        SwapFree:        1024000 kB\n";

        let mem = parse_proc_meminfo(mem_text).expect("parse meminfo");
        assert_eq!(mem.mem_total_bytes, 8192000 * 1024);
        assert_eq!(mem.mem_used_bytes, (8192000 - 4096000) * 1024);
        assert_eq!(mem.swap_total_bytes, 2048000 * 1024);
        assert_eq!(mem.swap_used_bytes, 1024000 * 1024);
    }

    #[test]
    fn test_parse_proc_loadavg() {
        let load_text = "0.75 1.25 0.90 3/350 49210\n";
        let load = parse_proc_loadavg(load_text).expect("parse loadavg");
        assert_eq!(load.load1, 0.75);
        assert_eq!(load.load5, 1.25);
        assert_eq!(load.load15, 0.90);
        assert_eq!(load.running_tasks, 3);
        assert_eq!(load.total_tasks, 350);
    }

    #[test]
    fn test_parse_proc_net_dev() {
        let net_text = "Inter-|   Receive                                                |  Transmit\n\
                        face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n\
                        lo: 1000       10    0    0    0     0          0         0     1000       10    0    0    0     0       0          0\n\
                        eth0: 5000000   1000    0    0    0     0          0         0  2000000    800    0    0    0     0       0          0\n\
                        eth1: 1000000    200    0    0    0     0          0         0   500000    100    0    0    0     0       0          0\n";

        let net = parse_proc_net_dev(net_text).expect("parse net dev");
        // eth0 + eth1 (lo excluded)
        assert_eq!(net.rx_bytes, 6000000);
        assert_eq!(net.tx_bytes, 2500000);
    }

    #[test]
    fn test_parse_proc_diskstats() {
        let disk_text = "   8       0 sda 100 0 2000 50 300 0 6000 90 0 100 140\n\
                            8       1 sda1 50 0 1000 25 150 0 3000 45 0 50 70\n\
                            7       0 loop0 10 0 20 5 0 0 0 0 0 5 5\n";

        let disk = parse_proc_diskstats(disk_text).expect("parse diskstats");
        // sda and sda1 (loop0 excluded)
        // 2000 + 1000 = 3000 sectors * 512 = 1536000 bytes
        // 6000 + 3000 = 9000 sectors * 512 = 4608000 bytes
        assert_eq!(disk.read_bytes, 3000 * 512);
        assert_eq!(disk.write_bytes, 9000 * 512);
    }

    #[test]
    fn test_proc_reader_full_flow() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        fs::write(
            root.join("stat"),
            "cpu  1000 0 200 8800 0 0 0 0 0 0\ncpu0 1000 0 200 8800 0 0 0 0 0 0\n",
        )
        .expect("write stat");
        fs::write(
            root.join("meminfo"),
            "MemTotal: 4000000 kB\nMemFree: 1000000 kB\nMemAvailable: 2000000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
        )
        .expect("write meminfo");
        fs::write(root.join("loadavg"), "0.10 0.20 0.30 1/100 1234\n").expect("write loadavg");
        fs::create_dir_all(root.join("net")).expect("mkdir net");
        fs::write(
            root.join("net").join("dev"),
            "hdr\nhdr2\neth0: 1000 1 0 0 0 0 0 0 2000 1 0 0 0 0 0 0\n",
        )
        .expect("write net");
        fs::write(
            root.join("diskstats"),
            "8 0 sda 10 0 100 0 20 0 200 0 0 0 0\n",
        )
        .expect("write diskstats");

        let reader = ProcReader::new(root);

        let cpu = reader.read_cpu_stats().expect("cpu");
        assert_eq!(cpu.cpu_cores, 1);
        assert_eq!(cpu.cpu_percent, 0.0); // First sample has 0.0%

        let mem = reader.read_memory_stats().expect("mem");
        assert_eq!(mem.mem_total_bytes, 4000000 * 1024);
        assert_eq!(mem.mem_used_bytes, 2000000 * 1024);

        let load = reader.read_load_stats().expect("load");
        assert_eq!(load.load1, 0.10);

        let net = reader.read_net_stats().expect("net");
        assert_eq!(net.rx_bytes_sec, 0.0);

        let disk = reader.read_disk_io_stats().expect("disk");
        assert_eq!(disk.read_bytes_sec, 0.0);

        // Update stat for second sample
        fs::write(
            root.join("stat"),
            "cpu  2000 0 400 16600 0 0 0 0 0 0\ncpu0 2000 0 400 16600 0 0 0 0 0 0\n",
        )
        .expect("write stat 2");
        let cpu2 = reader.read_cpu_stats().expect("cpu2");
        // delta total: (2000+400+16600) - (1000+200+8800) = 19000 - 10000 = 9000
        // delta idle: 16600 - 8800 = 7800
        // usage: 100 * (1 - 7800 / 9000) = 100 * (1200 / 9000) = 13.33%
        assert!((cpu2.cpu_percent - 13.33).abs() < 0.1);
    }
}
