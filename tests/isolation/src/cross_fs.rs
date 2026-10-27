use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
#[cfg(test)]
use tempfile::TempDir;

/// Represents simulated Linux user credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatedUser {
    pub uid: u32,
    pub gid: u32,
    pub username: &'static str,
}

pub const USER_ALICE: SimulatedUser = SimulatedUser {
    uid: 10001,
    gid: 10001,
    username: "p_alice",
};

pub const USER_BOB: SimulatedUser = SimulatedUser {
    uid: 10002,
    gid: 10002,
    username: "p_bob",
};

pub const USER_WWW_DATA: SimulatedUser = SimulatedUser {
    uid: 33,
    gid: 33,
    username: "www-data",
};

/// Result of evaluating file access permissions.
#[derive(Debug, PartialEq, Eq)]
pub enum AccessDecision {
    Allowed,
    PermissionDenied,
}

/// Evaluates standard POSIX DAC permissions (mode bits) for a simulated user.
pub fn evaluate_posix_access(
    file_mode: u32,
    file_owner_uid: u32,
    file_owner_gid: u32,
    accessor: SimulatedUser,
    want_read: bool,
    want_write: bool,
    want_execute: bool,
) -> AccessDecision {
    // Determine which mode bits apply (owner, group, other)
    let (r, w, x) = if accessor.uid == file_owner_uid {
        (
            file_mode & 0o400 != 0,
            file_mode & 0o200 != 0,
            file_mode & 0o100 != 0,
        )
    } else if accessor.gid == file_owner_gid {
        (
            file_mode & 0o040 != 0,
            file_mode & 0o020 != 0,
            file_mode & 0o010 != 0,
        )
    } else {
        (
            file_mode & 0o004 != 0,
            file_mode & 0o002 != 0,
            file_mode & 0o001 != 0,
        )
    };

    if (want_read && !r) || (want_write && !w) || (want_execute && !x) {
        AccessDecision::PermissionDenied
    } else {
        AccessDecision::Allowed
    }
}

/// Verifies that a tenant's home directory conforms to the strict `0700` isolation model.
pub fn verify_tenant_home_permissions(home_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let metadata = fs::metadata(home_path)?;
    #[cfg(unix)]
    {
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o700 {
            return Err(format!(
                "Home directory {:?} has mode 0o{:o}, expected 0o700",
                home_path, mode
            )
            .into());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_cross_tenant_read_write_list_is_rejected() {
        // Mode 0700: rwx for owner only, 000 for group and other
        let home_mode = 0o700;
        let bob_uid = USER_BOB.uid;
        let bob_gid = USER_BOB.gid;

        // Alice (UID 10001) attempts to read Bob's home (UID 10002)
        assert_eq!(
            evaluate_posix_access(home_mode, bob_uid, bob_gid, USER_ALICE, true, false, false),
            AccessDecision::PermissionDenied,
            "Alice must NOT be allowed to read Bob's home"
        );

        // Alice attempts to write into Bob's home
        assert_eq!(
            evaluate_posix_access(home_mode, bob_uid, bob_gid, USER_ALICE, false, true, false),
            AccessDecision::PermissionDenied,
            "Alice must NOT be allowed to write in Bob's home"
        );

        // Alice attempts to list (execute/traverse + read) Bob's home
        assert_eq!(
            evaluate_posix_access(home_mode, bob_uid, bob_gid, USER_ALICE, true, false, true),
            AccessDecision::PermissionDenied,
            "Alice must NOT be allowed to list Bob's home"
        );

        // Bob himself can read, write, and execute in his own home
        assert_eq!(
            evaluate_posix_access(home_mode, bob_uid, bob_gid, USER_BOB, true, true, true),
            AccessDecision::Allowed,
            "Bob must have full rwx in his own home"
        );
    }

    #[test]
    fn test_sites_directory_mode_750_isolation() {
        // /home/p_bob/sites has mode 0750 (owned by p_bob:panel-tenants or p_bob:bob)
        // Group panel-tenants GID is shared, but other tenants don't have traverse on /home/p_bob (mode 0700)
        let sites_mode = 0o750;
        let bob_uid = USER_BOB.uid;
        let tenant_group_gid = 9999; // panel-tenants

        // Even if Alice is in panel-tenants, she cannot reach /home/p_bob/sites because /home/p_bob is 0700
        let parent_home_access = evaluate_posix_access(
            0o700,
            bob_uid,
            tenant_group_gid,
            USER_ALICE,
            false,
            false,
            true,
        );
        assert_eq!(
            parent_home_access,
            AccessDecision::PermissionDenied,
            "Parent home 0700 blocks directory traversal to sites"
        );

        // And directly on 'other' for sites_mode 0750, others have 000
        let alice_other = SimulatedUser {
            uid: 10001,
            gid: 10001,
            username: "p_alice",
        };
        assert_eq!(
            evaluate_posix_access(
                sites_mode,
                bob_uid,
                bob_uid,
                alice_other,
                true,
                false,
                false
            ),
            AccessDecision::PermissionDenied
        );
    }

    #[test]
    fn test_filesystem_directory_creation_and_mode_verification() {
        let temp_dir = TempDir::new().unwrap();
        let home_bob = temp_dir.path().join("p_bob");
        fs::create_dir(&home_bob).unwrap();

        #[cfg(unix)]
        {
            fs::set_permissions(&home_bob, fs::Permissions::from_mode(0o700)).unwrap();
            let res = verify_tenant_home_permissions(&home_bob);
            assert!(res.is_ok(), "Expected 0700 permissions to pass validation");

            // Intentionally weaken permissions to 0755
            fs::set_permissions(&home_bob, fs::Permissions::from_mode(0o755)).unwrap();
            let res = verify_tenant_home_permissions(&home_bob);
            assert!(
                res.is_err(),
                "Expected non-0700 permissions to fail validation"
            );
        }
    }
}
