//! Terminal and pseudo-terminal (PTY) session management subsystem.
//!
//! Complies with PRD Section 5.3 and Phase 6 Task P6-T1:
//! - Spawns login shell as tenant UID (`p_<slug>`) inside tenant cgroup slice (`panel-u<uid>.slice`).
//! - Confines working directory (`cwd`) strictly within tenant's home root using `panel-fs` safety rules.
//! - Enforces hardening: `no_new_privs`, rlimits, privilege drop (`setresgid`/`setresuid`).
//! - Manages PTY lifecycle: input/output streaming, window resize (`TIOCSWINSZ`), process signals.
//! - Enforces concurrent session quotas per tenant and idle timeout reaping.

pub mod backend;
pub mod manager;
pub mod session;

#[cfg(test)]
mod tests;

#[cfg(unix)]
pub use backend::UnixPtyBackend;
pub use backend::{MockPtyBackend, PtyBackend, PtyHandle, PtySpawnConfig};
pub use manager::{
    DEFAULT_HOME_BASE, DEFAULT_IDLE_TIMEOUT_SECS, DEFAULT_MAX_SESSIONS_PER_TENANT, DEFAULT_SHELL,
    DEFAULT_TERM, PtyManagerConfig, PtySessionManager,
};
pub use session::PtySession;
