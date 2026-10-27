//! Role-Based Access Control (RBAC) and centralized policy engine.

use crate::CoreError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// User roles in Qinpanel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
    Reseller,
}

impl Role {
    /// Returns the string slice representation of the role.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
            Self::Reseller => "reseller",
        }
    }

    /// Returns `true` if this role is administrator.
    pub fn is_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    /// Returns `true` if this role can manage other users.
    pub fn can_manage_users(&self) -> bool {
        matches!(self, Self::Admin | Self::Reseller)
    }

    /// Returns `true` if this role can manage system-wide settings.
    pub fn can_manage_settings(&self) -> bool {
        matches!(self, Self::Admin)
    }
}

impl FromStr for Role {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            "reseller" => Ok(Self::Reseller),
            _ => Err(CoreError::Validation(format!("Invalid role: '{}'", s))),
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// System and tenant actions that can be authorized against policies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Read system-wide metrics and status
    ViewSystemInfo,
    /// Read/update global panel configuration
    ManageSettings,
    /// List all tenants/users in the system
    ListUsers,
    /// Create new tenant account
    CreateUser,
    /// View tenant details
    ViewUser { target_user_id: i64 },
    /// Update tenant details / profile
    UpdateUser { target_user_id: i64 },
    /// Suspend / unsuspend tenant account
    SuspendUser { target_user_id: i64 },
    /// Delete tenant account
    DeleteUser { target_user_id: i64 },
    /// Impersonate a tenant ("login as user")
    ImpersonateUser { target_user_id: i64 },
    /// View audit logs (system-wide if None, or user-specific)
    ViewAuditLogs { target_user_id: Option<i64> },
    /// Manage hosting plan definitions
    ManagePlan,
    /// Manage website / domain
    ManageSite { owner_id: i64 },
    /// Manage database
    ManageDatabase { owner_id: i64 },
    /// Manage background job
    ManageJob { owner_id: i64 },
    /// Manage SSH keys
    ManageSshKey { owner_id: i64 },
    /// Web terminal access
    AccessTerminal { owner_id: i64 },
    /// File manager access
    AccessFileManager { owner_id: i64 },
    /// View site/tenant analytics
    ViewAnalytics { owner_id: Option<i64> },
}

/// Centralized policy authorization function.
/// Evaluates whether a subject with `role` and `subject_id` is permitted to perform `action`.
pub fn authorize(role: Role, subject_id: i64, action: &Action) -> bool {
    // 1. Admin has unrestricted access to all operations
    if role.is_admin() {
        return true;
    }

    // 2. Regular user / tenant permissions
    match action {
        // Global administrative operations are strictly denied for non-admins
        Action::ManageSettings | Action::ListUsers | Action::CreateUser | Action::ManagePlan => {
            false
        }
        Action::ImpersonateUser { .. } => false,
        Action::SuspendUser { .. } | Action::DeleteUser { .. } => false,

        // System info read is public/allowed
        Action::ViewSystemInfo => true,

        // User can view/update only their own user record
        Action::ViewUser { target_user_id } => *target_user_id == subject_id,
        Action::UpdateUser { target_user_id } => *target_user_id == subject_id,

        // User can only view their own audit logs
        Action::ViewAuditLogs { target_user_id } => match target_user_id {
            Some(id) => *id == subject_id,
            None => false, // Global audit logs require admin
        },

        // Resource ownership check (sites, databases, jobs, SSH keys, files, terminal)
        Action::ManageSite { owner_id } => *owner_id == subject_id,
        Action::ManageDatabase { owner_id } => *owner_id == subject_id,
        Action::ManageJob { owner_id } => *owner_id == subject_id,
        Action::ManageSshKey { owner_id } => *owner_id == subject_id,
        Action::AccessTerminal { owner_id } => *owner_id == subject_id,
        Action::AccessFileManager { owner_id } => *owner_id == subject_id,

        // Analytics
        Action::ViewAnalytics { owner_id } => match owner_id {
            Some(id) => *id == subject_id,
            None => false, // Server-wide analytics require admin
        },
    }
}

/// Policy evaluation returning a descriptive Result.
pub fn check_permission(role: Role, subject_id: i64, action: &Action) -> Result<(), CoreError> {
    if authorize(role, subject_id, action) {
        Ok(())
    } else {
        Err(CoreError::Forbidden(format!(
            "Role '{}' is not authorized to perform action '{:?}' on subject ID {}",
            role, action, subject_id
        )))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_role_parsing_and_methods() {
        assert_eq!(Role::from_str("admin").unwrap(), Role::Admin);
        assert_eq!(Role::from_str("ADMIN").unwrap(), Role::Admin);
        assert_eq!(Role::from_str("user").unwrap(), Role::User);
        assert_eq!(Role::from_str("reseller").unwrap(), Role::Reseller);
        assert!(Role::from_str("invalid").is_err());

        assert!(Role::Admin.is_admin());
        assert!(!Role::User.is_admin());

        assert!(Role::Admin.can_manage_settings());
        assert!(!Role::User.can_manage_settings());

        assert_eq!(Role::Admin.to_string(), "admin");
        assert_eq!(Role::User.to_string(), "user");
    }

    #[test]
    fn test_admin_policy_permissions() {
        let admin = Role::Admin;
        let admin_id = 1;

        assert!(authorize(admin, admin_id, &Action::ListUsers));
        assert!(authorize(admin, admin_id, &Action::CreateUser));
        assert!(authorize(admin, admin_id, &Action::ManageSettings));
        assert!(authorize(admin, admin_id, &Action::ManagePlan));
        assert!(authorize(
            admin,
            admin_id,
            &Action::ViewAuditLogs {
                target_user_id: None
            }
        ));
        assert!(authorize(
            admin,
            admin_id,
            &Action::ManageSite { owner_id: 999 }
        ));
        assert!(authorize(
            admin,
            admin_id,
            &Action::AccessFileManager { owner_id: 999 }
        ));
        assert!(authorize(
            admin,
            admin_id,
            &Action::DeleteUser {
                target_user_id: 999
            }
        ));
    }

    #[test]
    fn test_user_policy_permissions() {
        let user = Role::User;
        let user_id = 42;

        // Denied global administrative actions
        assert!(!authorize(user, user_id, &Action::ListUsers));
        assert!(!authorize(user, user_id, &Action::CreateUser));
        assert!(!authorize(user, user_id, &Action::ManageSettings));
        assert!(!authorize(user, user_id, &Action::ManagePlan));
        assert!(!authorize(
            user,
            user_id,
            &Action::ViewAuditLogs {
                target_user_id: None
            }
        ));
        assert!(!authorize(
            user,
            user_id,
            &Action::DeleteUser {
                target_user_id: user_id
            }
        ));

        // Allowed on own resources
        assert!(authorize(
            user,
            user_id,
            &Action::ViewUser {
                target_user_id: user_id
            }
        ));
        assert!(authorize(
            user,
            user_id,
            &Action::UpdateUser {
                target_user_id: user_id
            }
        ));
        assert!(authorize(
            user,
            user_id,
            &Action::ViewAuditLogs {
                target_user_id: Some(user_id)
            }
        ));
        assert!(authorize(
            user,
            user_id,
            &Action::ManageSite { owner_id: user_id }
        ));
        assert!(authorize(
            user,
            user_id,
            &Action::AccessFileManager { owner_id: user_id }
        ));
        assert!(authorize(
            user,
            user_id,
            &Action::AccessTerminal { owner_id: user_id }
        ));

        // Denied on another user's resources
        assert!(!authorize(
            user,
            user_id,
            &Action::ViewUser { target_user_id: 99 }
        ));
        assert!(!authorize(
            user,
            user_id,
            &Action::ManageSite { owner_id: 99 }
        ));
        assert!(!authorize(
            user,
            user_id,
            &Action::AccessFileManager { owner_id: 99 }
        ));
        assert!(!authorize(
            user,
            user_id,
            &Action::ViewAuditLogs {
                target_user_id: Some(99)
            }
        ));
    }
}
