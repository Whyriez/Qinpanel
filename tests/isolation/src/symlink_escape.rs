use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum EscapeError {
    #[error("Path traverses outside tenant home root: {0}")]
    PathEscapesRoot(String),

    #[error("Symlink target points outside tenant home directory: {0} -> {1}")]
    SymlinkEscapesRoot(String, String),

    #[error("Invalid path component or null byte detected")]
    InvalidPath,

    #[error("Target does not exist: {0}")]
    NotFound(String),
}

/// Safely resolves and validates a tenant-provided relative or absolute path against
/// the tenant's root home directory, preventing path traversal (`..`) and symlink escapes.
pub fn resolve_confined_path(
    tenant_home: &Path,
    target_path: &Path,
) -> Result<PathBuf, EscapeError> {
    // 1. Check for null bytes
    if target_path.to_string_lossy().contains('\0') {
        return Err(EscapeError::InvalidPath);
    }

    // 2. If target path is absolute or has a leading root separator
    if target_path.is_absolute() || target_path.has_root() {
        if let Ok(rel) = target_path.strip_prefix(tenant_home) {
            let mut normalized = PathBuf::from(tenant_home);
            for component in rel.components() {
                match component {
                    Component::Normal(c) => normalized.push(c),
                    Component::CurDir => {}
                    Component::ParentDir => {
                        if normalized == tenant_home {
                            return Err(EscapeError::PathEscapesRoot(
                                target_path.to_string_lossy().to_string(),
                            ));
                        }
                        normalized.pop();
                    }
                    _ => {}
                }
            }
            return Ok(normalized);
        } else {
            // Absolute path outside tenant home (e.g. /home/p_bob or /etc/shadow)
            return Err(EscapeError::PathEscapesRoot(
                target_path.to_string_lossy().to_string(),
            ));
        }
    }

    // 3. Relative path: resolve safely within tenant_home
    let mut normalized = PathBuf::from(tenant_home);

    for component in target_path.components() {
        match component {
            Component::Normal(c) => {
                normalized.push(c);
            }
            Component::CurDir => {
                // Ignore '.'
            }
            Component::ParentDir => {
                // Cannot pop above tenant_home
                if normalized == tenant_home {
                    return Err(EscapeError::PathEscapesRoot(
                        target_path.to_string_lossy().to_string(),
                    ));
                }
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(EscapeError::InvalidPath);
            }
        }
    }

    if !normalized.starts_with(tenant_home) {
        return Err(EscapeError::PathEscapesRoot(
            target_path.to_string_lossy().to_string(),
        ));
    }

    Ok(normalized)
}

/// Validates whether a symlink target safely resides within the tenant's home boundary.
pub fn validate_symlink_target(
    tenant_home: &Path,
    symlink_target: &Path,
) -> Result<(), EscapeError> {
    if symlink_target.is_absolute() || symlink_target.has_root() {
        if !symlink_target.starts_with(tenant_home) {
            return Err(EscapeError::SymlinkEscapesRoot(
                "symlink".to_string(),
                symlink_target.to_string_lossy().to_string(),
            ));
        }
    } else {
        // Resolve relative symlink against tenant_home
        let resolved = resolve_confined_path(tenant_home, symlink_target)?;
        if !resolved.starts_with(tenant_home) {
            return Err(EscapeError::SymlinkEscapesRoot(
                "symlink".to_string(),
                symlink_target.to_string_lossy().to_string(),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_path_traversal_dot_dot_is_blocked() {
        let home = Path::new("/home/p_alice");

        // Attempting to traverse out to bob's directory
        let bad_path_1 = Path::new("../p_bob/sites/secret.txt");
        let res1 = resolve_confined_path(home, bad_path_1);
        assert!(matches!(res1, Err(EscapeError::PathEscapesRoot(_))));

        // Attempting to reach /etc/shadow
        let bad_path_2 = Path::new("../../../etc/shadow");
        let res2 = resolve_confined_path(home, bad_path_2);
        assert!(matches!(res2, Err(EscapeError::PathEscapesRoot(_))));

        // Legitimate subfolder path
        let good_path = Path::new("sites/example.com/public/index.html");
        let res3 = resolve_confined_path(home, good_path);
        assert_eq!(
            res3.unwrap(),
            PathBuf::from("/home/p_alice/sites/example.com/public/index.html")
        );
    }

    #[test]
    fn test_symlink_escape_to_other_tenant_is_blocked() {
        let alice_home = Path::new("/home/p_alice");

        // Symlink pointing to Bob's home
        let evil_link_target = Path::new("/home/p_bob/sites/confidential.txt");
        let res = validate_symlink_target(alice_home, evil_link_target);
        assert!(
            matches!(res, Err(EscapeError::SymlinkEscapesRoot(_, _))),
            "Symlink pointing to another tenant's path must be flagged as an escape"
        );

        // Symlink pointing to /etc/shadow
        let system_link = Path::new("/etc/shadow");
        assert!(matches!(
            validate_symlink_target(alice_home, system_link),
            Err(EscapeError::SymlinkEscapesRoot(_, _))
        ));

        // Symlink pointing inside Alice's own home
        let valid_link = Path::new("/home/p_alice/sites/common/shared.js");
        assert!(validate_symlink_target(alice_home, valid_link).is_ok());
    }

    #[test]
    fn test_null_byte_path_injection_rejected() {
        let alice_home = Path::new("/home/p_alice");
        let evil_null = Path::new("sites/evil\0file.txt");
        assert_eq!(
            resolve_confined_path(alice_home, evil_null),
            Err(EscapeError::InvalidPath)
        );
    }
}
