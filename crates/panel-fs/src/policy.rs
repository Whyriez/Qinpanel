//! Policy configuration for symlink resolution in `panel-fs`.

use serde::{Deserialize, Serialize};

/// Symlink resolution policy for path confinement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SymlinkPolicy {
    /// Strictly forbid all symbolic links during path resolution.
    ///
    /// Checks directory entries during resolution. Any encounter with a symlink
    /// returns `FsError::SymlinkForbidden`.
    #[default]
    DenyAll,

    /// Allow symbolic links only if their targets resolve strictly within the confined root.
    ///
    /// Checks containment during resolution. Any symlink whose resolved target
    /// escapes the jail boundary returns `FsError::SymlinkEscape`.
    ResolveBeneath,
}

impl SymlinkPolicy {
    pub fn is_deny_all(&self) -> bool {
        matches!(self, Self::DenyAll)
    }

    pub fn is_resolve_beneath(&self) -> bool {
        matches!(self, Self::ResolveBeneath)
    }
}
