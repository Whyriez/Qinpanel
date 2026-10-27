use super::*;
use crate::system::{MockAction, MockSystemRunner};
use panel_core::capacity::{CapacityError, MIB, PackageDemand, PackageUse, recommend_package};

fn runner() -> MockSystemRunner {
    let runner = MockSystemRunner::new();
    for (path, content) in [
        (
            "/proc/self/mountinfo",
            "29 23 0:26 / /sys/fs/cgroup rw,nosuid,nodev,noexec - cgroup2 cgroup rw\n",
        ),
        (
            "/proc/meminfo",
            "MemTotal: 8388608 kB\nMemAvailable: 6291456 kB\nSwapTotal: 16777216 kB\n",
        ),
        ("/sys/devices/system/cpu/online", "0-7\n"),
        ("/sys/fs/cgroup/cpuset.cpus.effective", "0-7\n"),
        ("/sys/fs/cgroup/panel.slice/cgroup.type", "domain\n"),
        ("/sys/fs/cgroup/panel.slice/cpuset.cpus.effective", "2-5\n"),
        ("/sys/fs/cgroup/panel.slice/cpu.max", "250000 100000\n"),
        ("/sys/fs/cgroup/panel.slice/memory.max", "4294967296\n"),
        ("/sys/fs/cgroup/panel.slice/memory.current", "1073741824\n"),
        ("/sys/fs/cgroup/panel.slice/memory.high", "max\n"),
    ] {
        runner.set_file_content(path, content.as_bytes());
    }
    runner.set_response(
        "df",
        0,
        b"1B-blocks Avail Inodes IFree\n104857600000 52428800000 1000000 800000\n",
        b"",
    );
    runner
}

fn demand() -> PackageDemand {
    PackageDemand {
        usage: PackageUse::Personal,
        projects_per_tenant: 1,
        tenant_slots: 1,
    }
}

#[test]
fn uses_verified_tenant_parent_capacity_not_agent_service_limits() {
    let runner = runner();
    runner.set_file_content(
        "/sys/fs/cgroup/system.slice/panel-agent.service/memory.max",
        b"67108864",
    );
    let snapshot = collect_capacity(&runner, 1000);
    assert_eq!(snapshot.scope, "panel.slice");
    assert_eq!(snapshot.cpu_quota_percent.value, Some(250));
    assert_eq!(snapshot.memory_limit_bytes.value, Some(4096 * MIB));
    assert_eq!(snapshot.memory_available_bytes.value, Some(3072 * MIB));
    assert_eq!(snapshot.disk_available_bytes.value, Some(50_000 * MIB));
    assert!(
        snapshot
            .memory_limit_bytes
            .sources
            .iter()
            .all(|s| !s.contains("panel-agent.service"))
    );
    let preset = recommend_package(&snapshot, demand(), 1001).expect("valid measured capacity");
    assert_eq!(preset.cpu_quota_percent, 225);
    assert!(preset.memory_limit_bytes < 3072 * MIB);
}

#[test]
fn host_and_parent_limits_compose_and_current_usage_is_not_double_subtracted() {
    let runner = runner();
    runner.set_file_content("/sys/fs/cgroup/cpu.max", b"150000 100000");
    runner.set_file_content("/sys/fs/cgroup/memory.max", b"3221225472");
    runner.set_file_content("/sys/fs/cgroup/memory.current", b"2147483648");
    let snapshot = collect_capacity(&runner, 1000);
    assert_eq!(snapshot.cpu_quota_percent.value, Some(150));
    assert_eq!(snapshot.memory_limit_bytes.value, Some(3072 * MIB));
    assert_eq!(snapshot.memory_available_bytes.value, Some(1024 * MIB));
}

#[test]
fn memory_high_pressure_limits_available_budget_without_inventing_free_ram() {
    let runner = runner();
    runner.set_file_content("/sys/fs/cgroup/panel.slice/memory.high", b"1610612736");
    let snapshot = collect_capacity(&runner, 1000);
    assert_eq!(snapshot.memory_limit_bytes.value, Some(4096 * MIB));
    assert_eq!(snapshot.memory_available_bytes.value, Some(512 * MIB));
    assert_eq!(
        recommend_package(&snapshot, demand(), 1001),
        Err(CapacityError::Insufficient)
    );
}

#[test]
fn tiny_host_bounds_memory_despite_large_parent_and_swap() {
    let runner = runner();
    runner.set_file_content(
        "/proc/meminfo",
        b"MemTotal: 524288 kB\nMemAvailable: 262144 kB\nSwapTotal: 16777216 kB\n",
    );
    let snapshot = collect_capacity(&runner, 1000);
    assert_eq!(snapshot.memory_limit_bytes.value, Some(512 * MIB));
    assert_eq!(snapshot.memory_available_bytes.value, Some(256 * MIB));
    assert_eq!(
        recommend_package(&snapshot, demand(), 1001),
        Err(CapacityError::Insufficient)
    );
}

#[test]
fn missing_hardware_never_becomes_legacy_magic_defaults() {
    let snapshot = collect_capacity(&MockSystemRunner::new(), 1000);
    assert_eq!(snapshot.cpu_quota_percent.value, None);
    assert_eq!(snapshot.memory_limit_bytes.value, None);
    assert_eq!(snapshot.disk_available_bytes.value, None);
    assert!(snapshot.cpu_quota_percent.unavailable_reason.is_some());
    assert_eq!(
        recommend_package(&snapshot, demand(), 1001),
        Err(CapacityError::Unavailable)
    );
}

#[test]
fn inactive_parent_is_unknown_not_unlimited() {
    let runner = runner();
    runner
        .files
        .lock()
        .expect("fixtures")
        .remove(Path::new("/sys/fs/cgroup/panel.slice/cgroup.type"));
    let snapshot = collect_capacity(&runner, 1000);
    assert_eq!(snapshot.cpu_quota_percent.value, None);
    assert_eq!(snapshot.memory_limit_bytes.value, None);
    assert!(snapshot.disk_total_bytes.value.is_some());
}

#[test]
fn known_containers_and_subtree_mounts_are_not_misreported_as_host_capacity() {
    for path in [
        "/.dockerenv",
        "/run/.containerenv",
        "/.containerenv",
        "/run/systemd/container",
    ] {
        let runner = runner();
        runner.set_file_content(path, b"fixture");
        assert_eq!(
            collect_capacity(&runner, 1000)
                .cpu_quota_percent
                .unavailable_reason
                .as_deref(),
            Some("container_scope_not_supported")
        );
    }
    let runner = runner();
    runner.set_file_content(
        "/proc/self/mountinfo",
        b"29 23 0:26 /hidden/subtree /sys/fs/cgroup rw - cgroup2 cgroup rw\n",
    );
    let snapshot = collect_capacity(&runner, 1000);
    assert_eq!(
        snapshot.memory_limit_bytes.unavailable_reason.as_deref(),
        Some("cgroup_mount_unverified")
    );
}

#[test]
fn malformed_or_threaded_cgroups_disable_planning() {
    for (path, content) in [
        ("/sys/fs/cgroup/panel.slice/cgroup.type", "threaded"),
        ("/sys/fs/cgroup/panel.slice/cpu.max", "max 0"),
        ("/sys/fs/cgroup/panel.slice/cpuset.cpus.effective", "9"),
        ("/sys/fs/cgroup/cpuset.cpus.effective", "0-3,3"),
        ("/sys/devices/system/cpu/online", ""),
    ] {
        let runner = runner();
        runner.set_file_content(path, content.as_bytes());
        assert!(
            collect_capacity(&runner, 1000)
                .cpu_quota_percent
                .value
                .is_none(),
            "{path}"
        );
    }
}

#[test]
fn malformed_missing_and_non_utf8_meminfo_are_unavailable() {
    for content in [
        b"MemTotal: 8388608 kB\nMemFree: 100 kB".as_slice(),
        &[255],
        b"MemTotal: 10 kB\nMemAvailable: 20 kB",
    ] {
        let runner = runner();
        runner.set_file_content("/proc/meminfo", content);
        assert_eq!(
            collect_capacity(&runner, 1000).memory_available_bytes.value,
            None
        );
    }
    let runner = runner();
    runner.set_file_content("/proc/meminfo", &vec![b'0'; MAX_READ + 1]);
    assert_eq!(
        collect_capacity(&runner, 1000)
            .memory_limit_bytes
            .unavailable_reason
            .as_deref(),
        Some("source_too_large")
    );
}

#[test]
fn full_filesystem_is_a_real_zero_but_unsupported_inode_counts_are_unknown() {
    let runner = runner();
    runner.set_response("df", 0, b"header\n104857600000 0 1000000 0\n", b"");
    let snapshot = collect_capacity(&runner, 1000);
    assert_eq!(snapshot.disk_available_bytes.value, Some(0));
    assert_eq!(snapshot.inodes_available.value, Some(0));
    assert_eq!(
        recommend_package(&snapshot, demand(), 1001),
        Err(CapacityError::Insufficient)
    );
    runner.set_response("df", 0, b"header\n104857600000 52428800000 - -\n", b"");
    assert_eq!(
        collect_capacity(&runner, 1000).disk_available_bytes.value,
        None
    );
}

#[test]
fn failed_contradictory_or_ambiguous_df_results_do_not_produce_a_budget() {
    for output in [
        "header\n100 200 10 9\n",
        "header\n100 50 10 11\n",
        "header\n100 50 10 9\n100 50 10 9\n",
        "",
        "header\n0 0 0 0\n",
    ] {
        let runner = runner();
        runner.set_response("df", 0, output.as_bytes(), b"");
        assert_eq!(collect_capacity(&runner, 1000).disk_total_bytes.value, None);
    }
    let runner = runner();
    runner.set_response("df", 1, b"", b"fixture failure");
    assert_eq!(collect_capacity(&runner, 1000).disk_total_bytes.value, None);
}

#[test]
fn collector_performs_only_the_fixed_read_only_command() {
    let runner = runner();
    let _ = collect_capacity(&runner, 1000);
    let actions = runner.actions.lock().expect("mock action log");
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        MockAction::Command { binary, args } => {
            assert_eq!(binary, "df");
            let expected: Vec<String> = [
                "--block-size=1",
                "--output=size,avail,itotal,iavail",
                "--",
                "/home",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            assert_eq!(args, &expected);
        }
        action => panic!("unexpected mutation: {action:?}"),
    }
}

#[test]
fn optimizer_ipc_remains_compatible_with_payloads_without_capacity() {
    let runner = std::sync::Arc::new(runner());
    let manager = crate::optimizer::OptimizerManager::new(runner);
    let status = manager.get_status().expect("optimizer status");
    assert_eq!(
        status
            .capacity
            .as_ref()
            .expect("capacity")
            .cpu_quota_percent
            .value,
        Some(250)
    );
    let mut legacy = serde_json::to_value(&status).expect("status JSON");
    legacy
        .as_object_mut()
        .expect("status object")
        .remove("capacity");
    let decoded: panel_proto::OptimizerStatusResponse =
        serde_json::from_value(legacy.clone()).expect("legacy IPC");
    assert!(decoded.capacity.is_none());
    assert_eq!(serde_json::to_value(decoded).expect("round trip"), legacy);
}
