/// Represents the lifecycle state of a tenant's system isolation resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantLifecycleState {
    pub username: String,
    pub uid: u32,
    pub is_locked: bool,
    pub is_cgroup_frozen: bool,
    pub active_processes_count: usize,
    pub shell: String,
}

impl TenantLifecycleState {
    pub fn new_active(username: &str, uid: u32, shell: &str) -> Self {
        Self {
            username: username.to_string(),
            uid,
            is_locked: false,
            is_cgroup_frozen: false,
            active_processes_count: 1, // e.g. active worker
            shell: shell.to_string(),
        }
    }

    /// Simulates agent suspension action on tenant:
    /// 1. Locks account (usermod -L)
    /// 2. Sets shell to /usr/sbin/nologin
    /// 3. Terminates running processes (SIGTERM)
    /// 4. Freezes cgroup slice (cgroup.freeze = 1)
    pub fn apply_suspend(&mut self) {
        self.is_locked = true;
        self.shell = "/usr/sbin/nologin".to_string();
        self.active_processes_count = 0;
        self.is_cgroup_frozen = true;
    }

    /// Simulates agent unsuspend action:
    /// 1. Unlocks account (usermod -U)
    /// 2. Restores shell policy
    /// 3. Unfreezes cgroup slice (cgroup.freeze = 0)
    pub fn apply_unsuspend(&mut self, restored_shell: &str) {
        self.is_locked = false;
        self.shell = restored_shell.to_string();
        self.is_cgroup_frozen = false;
    }

    /// Verifies if any new process or shell session can be started under the current state.
    pub fn can_spawn_new_process(&self) -> bool {
        !self.is_locked && !self.is_cgroup_frozen && self.shell != "/usr/sbin/nologin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suspend_halts_all_services_and_locks_tenant() {
        let mut tenant = TenantLifecycleState::new_active("p_charlie", 10003, "/bin/bash");

        // Initially active: can spawn processes
        assert!(tenant.can_spawn_new_process());
        assert_eq!(tenant.active_processes_count, 1);
        assert!(!tenant.is_cgroup_frozen);
        assert!(!tenant.is_locked);

        // Suspend tenant
        tenant.apply_suspend();

        // Must be locked, frozen, processes drained, shell disabled
        assert!(tenant.is_locked, "Account must be locked in shadow");
        assert!(tenant.is_cgroup_frozen, "cgroup.freeze must be active");
        assert_eq!(
            tenant.active_processes_count, 0,
            "All active processes must be terminated"
        );
        assert_eq!(
            tenant.shell, "/usr/sbin/nologin",
            "Shell must be /usr/sbin/nologin"
        );
        assert!(
            !tenant.can_spawn_new_process(),
            "Suspended tenant must NOT be able to spawn processes"
        );

        // Unsuspend tenant
        tenant.apply_unsuspend("/bin/bash");
        assert!(!tenant.is_locked);
        assert!(!tenant.is_cgroup_frozen);
        assert_eq!(tenant.shell, "/bin/bash");
        assert!(
            tenant.can_spawn_new_process(),
            "Tenant can spawn processes again once unsuspended"
        );
    }
}
