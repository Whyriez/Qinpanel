//! Cross-Tenant Isolation Test Suite for Qpanel (P2-T7).
//!
//! This suite validates all isolation boundaries defined in PRD Section 5:
//! 1. Cross-tenant filesystem read/write/list rejection (`0700` home directory permissions).
//! 2. Symlink escape prevention (path confinement and boundary validation).
//! 3. `/proc` process isolation (`hidepid=2` / invisible).
//! 4. Cgroup v2 slice memory confinement & OOM isolation (`MemoryMax`, `MemoryHigh`).
//! 5. Cgroup v2 task limit confinement & fork bomb protection (`TasksMax`).
//! 6. Tenant lifecycle suspension (process termination, freezing, login lockout).

pub mod cgroup_confinement;
pub mod cross_fs;
pub mod php_isolation;
pub mod proc_hidepid;
pub mod sample_apps_isolation;
pub mod suspend_isolation;
pub mod symlink_escape;
