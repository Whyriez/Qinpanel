//! Domain types and validators for Linux tenant identity and lifecycle.
//!
//! Rule 4.4: "1 tenant = 1 user Linux p_<slug> (UID 10000–59999), tanpa password login, shell sesuai kebijakan"

use crate::CoreError;
use serde::{Deserialize, Serialize};

pub const MIN_TENANT_UID: u32 = 10000;
pub const MAX_TENANT_UID: u32 = 59999;

/// Reserved system usernames that must never collide with tenant accounts.
pub const RESERVED_SYSTEM_NAMES: &[&str] = &[
    "root",
    "daemon",
    "bin",
    "sys",
    "sync",
    "games",
    "man",
    "lp",
    "mail",
    "news",
    "uucp",
    "proxy",
    "majordom",
    "postgres",
    "mysql",
    "mariadb",
    "www",
    "www-data",
    "panel",
    "nobody",
    "nogroup",
    "systemd",
    "systemd-journal",
    "systemd-network",
    "systemd-resolve",
    "systemd-timesync",
    "messagebus",
    "sshd",
    "nginx",
    "redis",
    "adm",
    "tty",
    "disk",
    "sudo",
    "audio",
    "dip",
    "backup",
    "operator",
    "list",
    "irc",
    "src",
    "gnats",
    "shadow",
    "utmp",
    "video",
    "sasl",
    "plugdev",
    "staff",
    "users",
    "admin",
    "administrator",
];

/// Validates whether a given slug or username is a reserved system name.
pub fn is_reserved_username(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    let stripped = normalized.strip_prefix("p_").unwrap_or(&normalized);
    RESERVED_SYSTEM_NAMES.contains(&stripped)
        || RESERVED_SYSTEM_NAMES.contains(&normalized.as_str())
}

/// Linux UID allocated for a tenant in range 10000–59999.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantUid(u32);

impl TenantUid {
    pub fn parse(uid: u32) -> Result<Self, CoreError> {
        if !(MIN_TENANT_UID..=MAX_TENANT_UID).contains(&uid) {
            return Err(CoreError::Validation(format!(
                "Tenant UID must be between {} and {}, got {}",
                MIN_TENANT_UID, MAX_TENANT_UID, uid
            )));
        }
        Ok(Self(uid))
    }

    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for TenantUid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Tenant shell access policy (PRD 5.2 & 6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ShellPolicy {
    #[default]
    None,
    Sftp,
    Full,
}

impl ShellPolicy {
    pub fn as_shell_path(&self) -> &'static str {
        match self {
            ShellPolicy::None => "/usr/sbin/nologin",
            ShellPolicy::Sftp => "/usr/lib/openssh/sftp-server",
            ShellPolicy::Full => "/bin/bash",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "nologin" | "/usr/sbin/nologin" | "/bin/false" => Ok(ShellPolicy::None),
            "sftp" | "/usr/lib/openssh/sftp-server" => Ok(ShellPolicy::Sftp),
            "full" | "bash" | "/bin/bash" | "/bin/sh" => Ok(ShellPolicy::Full),
            other => Err(CoreError::Validation(format!(
                "Unknown shell policy '{}'. Allowed: 'none', 'sftp', 'full'",
                other
            ))),
        }
    }
}

impl std::fmt::Display for ShellPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellPolicy::None => write!(f, "none"),
            ShellPolicy::Sftp => write!(f, "sftp"),
            ShellPolicy::Full => write!(f, "full"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_uid_valid() {
        let uid = TenantUid::parse(10000).expect("min valid uid");
        assert_eq!(uid.as_u32(), 10000);
        let uid2 = TenantUid::parse(59999).expect("max valid uid");
        assert_eq!(uid2.as_u32(), 59999);
    }

    #[test]
    fn test_tenant_uid_out_of_bounds() {
        assert!(TenantUid::parse(9999).is_err());
        assert!(TenantUid::parse(60000).is_err());
        assert!(TenantUid::parse(0).is_err());
    }

    #[test]
    fn test_reserved_usernames() {
        assert!(is_reserved_username("root"));
        assert!(is_reserved_username("p_root"));
        assert!(is_reserved_username("p_nginx"));
        assert!(is_reserved_username("p_www-data"));
        assert!(!is_reserved_username("p_client123"));
        assert!(!is_reserved_username("p_myshop"));
    }

    #[test]
    fn test_shell_policy() {
        assert_eq!(ShellPolicy::default().as_shell_path(), "/usr/sbin/nologin");
        assert_eq!(ShellPolicy::parse("none").unwrap(), ShellPolicy::None);
        assert_eq!(ShellPolicy::parse("sftp").unwrap(), ShellPolicy::Sftp);
        assert_eq!(ShellPolicy::parse("full").unwrap(), ShellPolicy::Full);
        assert!(ShellPolicy::parse("invalid").is_err());
    }
}
