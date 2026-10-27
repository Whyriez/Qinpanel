//! Read-only capacity collector. All paths and command arguments are fixed.
use crate::{AgentError, system::SystemRunner};
use panel_core::capacity::{
    CapacityMetric, CapacitySnapshot, intersect_cpu_count, parse_cpu_list, parse_cpu_max,
    parse_limit, parse_memory,
};
use std::path::Path;

const ROOT: &str = "/sys/fs/cgroup";
const MAX_READ: usize = 65_536;
type ReadResult<T> = Result<T, &'static str>;

fn read(runner: &dyn SystemRunner, path: &str) -> ReadResult<String> {
    let bytes = runner
        .read_file(Path::new(path))
        .map_err(|_| "source_unreadable")?;
    if bytes.len() > MAX_READ {
        return Err("source_too_large");
    }
    String::from_utf8(bytes).map_err(|_| "source_not_utf8")
}

fn optional(runner: &dyn SystemRunner, path: &str) -> ReadResult<Option<String>> {
    match runner.read_file(Path::new(path)) {
        Ok(bytes) if bytes.len() <= MAX_READ => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "source_not_utf8"),
        Ok(_) => Err("source_too_large"),
        Err(AgentError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("source_unreadable"),
    }
}

fn source_paths(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

fn metric(value: ReadResult<u64>, paths: &[&str]) -> CapacityMetric {
    match value {
        Ok(value) => CapacityMetric::measured(value, source_paths(paths)),
        Err(reason) => CapacityMetric::unavailable(reason, source_paths(paths)),
    }
}

fn verify_scope(runner: &dyn SystemRunner) -> ReadResult<()> {
    // Container ancestor limits can be hidden outside its cgroup namespace.
    // Do not pass a host-looking /proc value off as verified container capacity.
    if [
        "/run/systemd/container",
        "/.dockerenv",
        "/run/.containerenv",
        "/.containerenv",
    ]
    .iter()
    .any(|path| runner.path_exists(Path::new(path)))
    {
        return Err("container_scope_not_supported");
    }
    let mountinfo = read(runner, "/proc/self/mountinfo")?;
    let verified = mountinfo.lines().any(|line| {
        let Some((left, right)) = line.split_once(" - ") else {
            return false;
        };
        let fields: Vec<_> = left.split_whitespace().collect();
        fields.len() >= 6
            && fields[3] == "/"
            && fields[4] == ROOT
            && right.split_whitespace().next() == Some("cgroup2")
    });
    if !verified {
        return Err("cgroup_mount_unverified");
    }
    // panel-u<uid>.slice belongs to panel.slice by systemd slice naming rules.
    // An absent/inactive parent is not evidence of an unlimited parent.
    let Some(parent_type) = optional(runner, "/sys/fs/cgroup/panel.slice/cgroup.type")? else {
        return Err("tenant_parent_inactive");
    };
    if parent_type.trim() != "domain" {
        return Err("tenant_parent_unverified");
    }
    Ok(())
}

fn cpu_capacity(runner: &dyn SystemRunner) -> ReadResult<u64> {
    verify_scope(runner)?;
    let online = parse_cpu_list(&read(runner, "/sys/devices/system/cpu/online")?)
        .map_err(|_| "invalid_cpu_list")?;
    let root_set = parse_cpu_list(&read(runner, "/sys/fs/cgroup/cpuset.cpus.effective")?)
        .map_err(|_| "invalid_cpu_list")?;
    let parent_set = optional(runner, "/sys/fs/cgroup/panel.slice/cpuset.cpus.effective")?;
    let allowed = match parent_set {
        Some(value) => {
            let parent = parse_cpu_list(&value).map_err(|_| "invalid_cpu_list")?;
            // An impossible parent set indicates racing/contradictory readings.
            if intersect_cpu_count(&parent, &root_set) != intersect_cpu_count(&parent, &parent) {
                return Err("inconsistent_cpu_set");
            }
            parent
        }
        None => root_set,
    };
    let mut quota = intersect_cpu_count(&online, &allowed)
        .checked_mul(100)
        .ok_or("invalid_cpu_count")?;
    for (index, path) in [
        "/sys/fs/cgroup/cpu.max",
        "/sys/fs/cgroup/panel.slice/cpu.max",
    ]
    .iter()
    .enumerate()
    {
        let value = if index == 0 {
            optional(runner, path)?
        } else {
            Some(read(runner, path)?)
        };
        if let Some(value) = value
            && let Some(limit) = parse_cpu_max(&value).map_err(|_| "invalid_cpu_quota")?
        {
            quota = quota.min(limit);
        }
    }
    Ok(quota)
}

fn memory_capacity(runner: &dyn SystemRunner) -> ReadResult<(u64, u64)> {
    verify_scope(runner)?;
    let (mut limit, mut available) =
        parse_memory(&read(runner, "/proc/meminfo")?).map_err(|_| "invalid_meminfo")?;
    for (index, (max_path, current_path, high_path)) in [
        (
            "/sys/fs/cgroup/memory.max",
            "/sys/fs/cgroup/memory.current",
            "/sys/fs/cgroup/memory.high",
        ),
        (
            "/sys/fs/cgroup/panel.slice/memory.max",
            "/sys/fs/cgroup/panel.slice/memory.current",
            "/sys/fs/cgroup/panel.slice/memory.high",
        ),
    ]
    .iter()
    .enumerate()
    {
        let max = if index == 0 {
            optional(runner, max_path)?
        } else {
            Some(read(runner, max_path)?)
        };
        let max = max
            .map(|value| parse_limit(&value))
            .transpose()
            .map_err(|_| "invalid_memory_limit")?
            .flatten();
        let high = optional(runner, high_path)?
            .map(|value| parse_limit(&value))
            .transpose()
            .map_err(|_| "invalid_memory_high")?
            .flatten();
        if max.is_some() || high.is_some() {
            let current = read(runner, current_path)?
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid_memory_usage")?;
            if let Some(max) = max {
                limit = limit.min(max);
                available = available.min(max.saturating_sub(current));
            }
            if let Some(high) = high {
                available = available.min(high.saturating_sub(current));
            }
        }
    }
    Ok((limit, available.min(limit)))
}

fn disk_capacity(runner: &dyn SystemRunner) -> ReadResult<[u64; 4]> {
    let output = runner
        .run_command(
            "df",
            &[
                "--block-size=1",
                "--output=size,avail,itotal,iavail",
                "--",
                "/home",
            ],
        )
        .map_err(|_| "filesystem_unreadable")?;
    if !output.status.success() || output.stdout.len() > MAX_READ {
        return Err("filesystem_unreadable");
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "invalid_filesystem_capacity")?;
    let lines: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() != 2 {
        return Err("invalid_filesystem_capacity");
    }
    let values: Vec<_> = lines[1]
        .split_whitespace()
        .map(str::parse::<u64>)
        .collect::<Result<_, _>>()
        .map_err(|_| "invalid_filesystem_capacity")?;
    let [total, available, inodes, free_inodes] = values.as_slice() else {
        return Err("invalid_filesystem_capacity");
    };
    if *total == 0 || *inodes == 0 || available > total || free_inodes > inodes {
        return Err("invalid_filesystem_capacity");
    }
    Ok([*total, *available, *inodes, *free_inodes])
}

pub fn collect_capacity(runner: &dyn SystemRunner, observed_at: u64) -> CapacitySnapshot {
    let cpu = cpu_capacity(runner);
    let memory = memory_capacity(runner);
    let disk = disk_capacity(runner);
    let cpu_sources = [
        "/proc/self/mountinfo",
        "/sys/devices/system/cpu/online",
        "/sys/fs/cgroup/cpuset.cpus.effective",
        "/sys/fs/cgroup/cpu.max",
        "/sys/fs/cgroup/panel.slice/cgroup.type",
        "/sys/fs/cgroup/panel.slice/cpuset.cpus.effective",
        "/sys/fs/cgroup/panel.slice/cpu.max",
    ];
    let memory_sources = [
        "/proc/self/mountinfo",
        "/proc/meminfo",
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory.high",
        "/sys/fs/cgroup/memory.current",
        "/sys/fs/cgroup/panel.slice/cgroup.type",
        "/sys/fs/cgroup/panel.slice/memory.max",
        "/sys/fs/cgroup/panel.slice/memory.high",
        "/sys/fs/cgroup/panel.slice/memory.current",
    ];
    let disk_sources = ["df:--block-size=1:--output=size,avail,itotal,iavail:--:/home"];
    CapacitySnapshot {
        schema_version: 1,
        observed_at,
        scope: "panel.slice".into(),
        cpu_quota_percent: metric(cpu, &cpu_sources),
        memory_limit_bytes: metric(memory.map(|value| value.0), &memory_sources),
        memory_available_bytes: metric(memory.map(|value| value.1), &memory_sources),
        disk_total_bytes: metric(disk.map(|value| value[0]), &disk_sources),
        disk_available_bytes: metric(disk.map(|value| value[1]), &disk_sources),
        inodes_total: metric(disk.map(|value| value[2]), &disk_sources),
        inodes_available: metric(disk.map(|value| value[3]), &disk_sources),
        notices: vec![
            "instantaneous_measurement_no_history".into(),
            "fixed_home_volume".into(),
            "requires_active_verified_panel_slice".into(),
        ],
    }
}

#[cfg(test)]
mod tests;
