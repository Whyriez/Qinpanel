#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_fs::{
    ConfinedFs, FsError, PermissionGrid, SymlinkPolicy, check_mode_security_warning,
    format_octal_mode, parse_octal_mode,
};
use tempfile::tempdir;

#[test]
fn test_octal_parsing_and_formatting() {
    assert_eq!(parse_octal_mode("0755").unwrap(), 0o755);
    assert_eq!(parse_octal_mode("755").unwrap(), 0o755);
    assert_eq!(parse_octal_mode("0644").unwrap(), 0o644);
    assert_eq!(parse_octal_mode("644").unwrap(), 0o644);
    assert_eq!(parse_octal_mode("0777").unwrap(), 0o777);
    assert_eq!(format_octal_mode(0o755), "0755");
    assert_eq!(format_octal_mode(0o644), "0644");
    assert_eq!(format_octal_mode(0o777), "0777");

    assert!(parse_octal_mode("").is_err());
    assert!(parse_octal_mode("888").is_err());
    assert!(parse_octal_mode("0999").is_err());
    assert!(parse_octal_mode("177777").is_err());
}

#[test]
fn test_permission_grid_logic() {
    let grid_755 = PermissionGrid::from_mode(0o755);
    assert!(grid_755.owner_read && grid_755.owner_write && grid_755.owner_execute);
    assert!(grid_755.group_read && !grid_755.group_write && grid_755.group_execute);
    assert!(grid_755.others_read && !grid_755.others_write && grid_755.others_execute);
    assert_eq!(grid_755.to_mode(), 0o755);
    assert_eq!(grid_755.to_octal_string(), "0755");

    let grid_644 = PermissionGrid::from_mode(0o644);
    assert!(grid_644.owner_read && grid_644.owner_write && !grid_644.owner_execute);
    assert!(grid_644.group_read && !grid_644.group_write && !grid_644.group_execute);
    assert!(grid_644.others_read && !grid_644.others_write && !grid_644.others_execute);
    assert_eq!(grid_644.to_mode(), 0o644);
}

#[test]
fn test_security_warning_on_777() {
    let warn_777 = check_mode_security_warning(0o777);
    assert!(warn_777.is_some());
    assert!(warn_777.unwrap().contains("Peringatan keamanan"));

    // World writable mode 0666 should also warn
    let warn_666 = check_mode_security_warning(0o666);
    assert!(warn_666.is_some());

    // Safe presets should not warn
    assert!(check_mode_security_warning(0o755).is_none());
    assert!(check_mode_security_warning(0o644).is_none());
    assert!(check_mode_security_warning(0o700).is_none());
    assert!(check_mode_security_warning(0o600).is_none());
}

#[test]
fn test_confined_fs_chown_boundary_enforcement() {
    let dir = tempdir().unwrap();
    let fs = ConfinedFs::new(dir.path(), SymlinkPolicy::DenyAll).unwrap();

    fs.write_atomic("test.txt", b"secret", None).unwrap();

    // 1. Tenant (non-admin) mencoba chown ke user lain (UID 1002 vs caller 1001)
    let err = fs.chown("test.txt", Some(1002), Some(1002), 1001, false);
    assert!(err.is_err());
    match err.unwrap_err() {
        FsError::PermissionDenied { path, reason } => {
            assert!(path.contains("test.txt"));
            assert!(reason.contains("Penyewa tidak diizinkan"));
        }
        other => panic!("Expected PermissionDenied, got {:?}", other),
    }
}

#[cfg(unix)]
#[test]
fn test_chown_to_actual_fixture_owner_succeeds() {
    use std::os::unix::fs::MetadataExt;

    let dir = tempdir().unwrap();
    let fs = ConfinedFs::new(dir.path(), SymlinkPolicy::DenyAll).unwrap();
    fs.write_atomic("test.txt", b"secret", None).unwrap();
    let path = dir.path().join("test.txt");
    let owner = std::fs::metadata(&path).unwrap();
    fs.chown(
        "test.txt",
        Some(owner.uid()),
        Some(owner.gid()),
        owner.uid(),
        false,
    )
    .unwrap();
    let saved = std::fs::metadata(&path).unwrap();
    assert_eq!((saved.uid(), saved.gid()), (owner.uid(), owner.gid()));
    assert_eq!(std::fs::read(path).unwrap(), b"secret");
}

#[cfg(unix)]
#[test]
fn test_admin_chown_reports_actual_os_outcome_instead_of_false_success() {
    use std::os::unix::fs::MetadataExt;

    let dir = tempdir().unwrap();
    let fs = ConfinedFs::new(dir.path(), SymlinkPolicy::DenyAll).unwrap();
    fs.write_atomic("test.txt", b"secret", None).unwrap();
    let path = dir.path().join("test.txt");
    let owner = std::fs::metadata(&path).unwrap().uid();
    let target = if owner == 0 { 10001 } else { 0 };
    let result = fs.chown("test.txt", Some(target), None, owner, true);
    let saved = std::fs::metadata(&path).unwrap().uid();
    match result {
        Ok(()) => assert_eq!(saved, target, "success must reflect the real file owner"),
        Err(FsError::PermissionDenied { .. }) => assert_eq!(saved, owner),
        Err(error) => panic!("unexpected ownership error: {error}"),
    }
    assert_eq!(std::fs::read(path).unwrap(), b"secret");
}

#[cfg(not(unix))]
#[test]
fn test_chown_is_not_reported_as_success_on_unsupported_platforms() {
    let dir = tempdir().unwrap();
    let fs = ConfinedFs::new(dir.path(), SymlinkPolicy::DenyAll).unwrap();
    fs.write_atomic("test.txt", b"secret", None).unwrap();
    assert!(matches!(
        fs.chown("test.txt", Some(10001), None, 10001, true),
        Err(FsError::Io { .. })
    ));
}

#[test]
fn test_confined_fs_chmod_recursive_file_vs_dir() {
    let dir = tempdir().unwrap();
    let fs = ConfinedFs::new(dir.path(), SymlinkPolicy::DenyAll).unwrap();

    fs.create_dir_all("folder/sub").unwrap();
    fs.write_atomic("folder/file1.txt", b"content", None)
        .unwrap();
    fs.write_atomic("folder/sub/file2.txt", b"content", None)
        .unwrap();

    let affected = fs.chmod_recursive("folder", 0o644, 0o755).unwrap();
    // "folder", "folder/sub", "folder/file1.txt", "folder/sub/file2.txt" = 4
    assert_eq!(affected, 4);
}
