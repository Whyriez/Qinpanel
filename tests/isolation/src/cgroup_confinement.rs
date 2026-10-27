use panel_proto::SliceApplyRequest;

/// Evaluates whether a process spawn attempt within a slice is allowed or rejected
/// by cgroup v2 `pids.max` (TasksMax).
#[derive(Debug, PartialEq, Eq)]
pub enum SpawnDecision {
    Allowed,
    ResourceTemporarilyUnavailable, // EAGAIN
}

/// Evaluates whether an unprivileged fork/clone call will succeed given current tasks count and limit.
pub fn evaluate_fork_attempt(current_tasks: u32, tasks_max: u32) -> SpawnDecision {
    if tasks_max > 0 && current_tasks >= tasks_max {
        SpawnDecision::ResourceTemporarilyUnavailable
    } else {
        SpawnDecision::Allowed
    }
}

/// Result of evaluating memory consumption against cgroup v2 `memory.max`.
#[derive(Debug, PartialEq, Eq)]
pub enum MemoryStatus {
    WithinLimits,
    Throttled, // Exceeded memory.high (reclaim pressure)
    OomKilled, // Exceeded memory.max
}

/// Evaluates memory allocation against slice limits.
pub fn evaluate_memory_usage(
    usage_bytes: u64,
    memory_high_bytes: Option<u64>,
    memory_max_bytes: u64,
) -> MemoryStatus {
    if memory_max_bytes > 0 && usage_bytes >= memory_max_bytes {
        MemoryStatus::OomKilled
    } else if let Some(high) = memory_high_bytes {
        if high > 0 && usage_bytes >= high {
            MemoryStatus::Throttled
        } else {
            MemoryStatus::WithinLimits
        }
    } else {
        MemoryStatus::WithinLimits
    }
}

/// Generates systemd slice unit drop-in contents for a tenant.
pub fn generate_slice_dropin(req: &SliceApplyRequest) -> String {
    let mut conf = String::new();
    conf.push_str("[Slice]\n");

    if let Some(ref max) = req.memory_max {
        conf.push_str(&format!("MemoryMax={}\n", max));
    }
    if let Some(ref high) = req.memory_high {
        conf.push_str(&format!("MemoryHigh={}\n", high));
    }
    if let Some(ref cpu) = req.cpu_quota {
        conf.push_str(&format!("CPUQuota={}\n", cpu));
    }
    if let Some(tasks) = req.tasks_max {
        conf.push_str(&format!("TasksMax={}\n", tasks));
    }
    if let Some(io) = req.io_weight {
        conf.push_str(&format!("IOWeight={}\n", io));
    }

    conf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fork_bomb_stopped_by_tasks_max() {
        let tasks_max = 50;

        // Up to 49 tasks: spawning allowed
        assert_eq!(evaluate_fork_attempt(0, tasks_max), SpawnDecision::Allowed);
        assert_eq!(evaluate_fork_attempt(49, tasks_max), SpawnDecision::Allowed);

        // At 50 tasks (at limit): fork returns EAGAIN
        assert_eq!(
            evaluate_fork_attempt(50, tasks_max),
            SpawnDecision::ResourceTemporarilyUnavailable
        );

        // At 100 tasks: still rejected
        assert_eq!(
            evaluate_fork_attempt(100, tasks_max),
            SpawnDecision::ResourceTemporarilyUnavailable
        );

        // Another tenant Bob with independent tasks_max is unaffected
        let bob_tasks_max = 100;
        let bob_current = 2;
        assert_eq!(
            evaluate_fork_attempt(bob_current, bob_tasks_max),
            SpawnDecision::Allowed
        );
    }

    #[test]
    fn test_memory_confinement_and_oom_killer() {
        let memory_max = 512 * 1024 * 1024; // 512 MB
        let memory_high = Some(400 * 1024 * 1024); // 400 MB

        // Safe usage (100 MB)
        assert_eq!(
            evaluate_memory_usage(100 * 1024 * 1024, memory_high, memory_max),
            MemoryStatus::WithinLimits
        );

        // Throttled usage (450 MB > 400 MB)
        assert_eq!(
            evaluate_memory_usage(450 * 1024 * 1024, memory_high, memory_max),
            MemoryStatus::Throttled
        );

        // OOM usage (513 MB > 512 MB) -> OOM killed specifically inside this slice
        assert_eq!(
            evaluate_memory_usage(513 * 1024 * 1024, memory_high, memory_max),
            MemoryStatus::OomKilled
        );
    }

    #[test]
    fn test_slice_dropin_generation_contains_all_directives() {
        let req = SliceApplyRequest {
            uid: 10001,
            memory_max: Some("1073741824".to_string()),
            memory_high: Some("838860800".to_string()),
            cpu_quota: Some("200%".to_string()),
            tasks_max: Some(150),
            io_weight: Some(200),
        };

        let dropin = generate_slice_dropin(&req);
        assert!(dropin.contains("[Slice]"));
        assert!(dropin.contains("MemoryMax=1073741824"));
        assert!(dropin.contains("MemoryHigh=838860800"));
        assert!(dropin.contains("CPUQuota=200%"));
        assert!(dropin.contains("TasksMax=150"));
        assert!(dropin.contains("IOWeight=200"));
    }
}
