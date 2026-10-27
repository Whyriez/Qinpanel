//! Adversarial security test suite for `panel-fs`.
//!
//! Complies with PRD Section 5.3 and Task P5-T1:
//! - Tests directory traversal (`..`) attempts
//! - Tests symlink escapes (relative, absolute, chained, cyclic)
//! - Tests null byte injections
//! - Tests unicode normalization and fullwidth slash attacks
//! - Tests excessive path lengths (> 4096 bytes)
//! - Tests strange and malicious filenames (`-rf`, spaces, `$VAR`, `;`, UTF-8)
//! - Tests root jail deletion protections
//! - Tests atomic file writes and range reads
//! - Tests directory listing pagination, sorting, and filtering

use panel_fs::{ConfinedFs, FsError, ListOptions, SortBy, SortOrder, SymlinkPolicy};
use tempfile::tempdir;

#[test]
fn test_traversal_dot_dot_simple() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    let cases = [
        "../",
        "../../",
        "../../../etc/passwd",
        "sites/../../..",
        "sites/example.com/../../../../etc/shadow",
        "..",
        "./..",
        "a/b/../../../c",
    ];

    for path in cases {
        let res = fs.resolve(path);
        assert!(
            res.is_err(),
            "Path '{}' should have failed traversal check",
            path
        );
        match res.unwrap_err() {
            FsError::PathTraversalAttempt { .. } => {}
            other => panic!(
                "Expected PathTraversalAttempt for '{}', got {:?}",
                path, other
            ),
        }
    }
}

#[test]
fn test_null_byte_injection() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    let cases = [
        "index.html\0.php",
        "\0/etc/passwd",
        "sites/\0/test",
        "evil.txt\0",
    ];

    for path in cases {
        let res = fs.resolve(path);
        assert!(res.is_err(), "Null byte path '{}' must be rejected", path);
        match res.unwrap_err() {
            FsError::InvalidPath(msg) => {
                assert!(msg.contains("null byte"));
            }
            other => panic!("Expected InvalidPath with null byte, got {:?}", other),
        }
    }
}

#[test]
fn test_unicode_fullwidth_slashes_and_controls() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    // U+FF0F (fullwidth solidus '／') and U+FF3C (fullwidth reverse solidus '＼')
    let unicode_cases = [
        "sites\u{FF0F}evil",
        "test\u{FF3C}escape",
        "..\u{FF0F}..\u{FF0F}etc",
    ];

    for path in unicode_cases {
        let res = fs.resolve(path);
        assert!(res.is_err(), "Fullwidth slash path must be rejected");
        match res.unwrap_err() {
            FsError::InvalidPath(msg) => {
                assert!(msg.contains("fullwidth slash"));
            }
            other => panic!("Expected InvalidPath, got {:?}", other),
        }
    }

    // Control characters (bell \x07, escape \x1B, tab \t, newline \n)
    let control_cases = [
        "file\x07name.txt",
        "escape\x1Bcode.php",
        "tab\tindented.log",
        "newline\nname.txt",
    ];
    for path in control_cases {
        let res = fs.resolve(path);
        assert!(
            res.is_err(),
            "Control characters must be rejected: {:?}",
            path
        );
        match res.unwrap_err() {
            FsError::InvalidPath(msg) => {
                assert!(msg.contains("control characters"));
            }
            other => panic!("Expected InvalidPath, got {:?}", other),
        }
    }
}

#[test]
fn test_excessive_path_length() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    let long_path = "a/".repeat(2500); // 5000 characters
    let res = fs.resolve(&long_path);
    assert!(res.is_err(), "Path over 4096 chars must be rejected");
    match res.unwrap_err() {
        FsError::InvalidPath(msg) => {
            assert!(msg.contains("maximum length"));
        }
        other => panic!("Expected InvalidPath, got {:?}", other),
    }
}

#[test]
fn test_weird_and_malicious_filenames() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    // Challenging filenames that must work safely without shell execution or escaping
    let weird_names = [
        "-rf",
        "--preserve-root",
        "file with spaces.txt",
        "semi;colon$VAR&more.json",
        "quote'single.txt",
        "indonesia_merdeka_🇮🇩.txt",
        "many.dots.in.name.tar.gz",
        "@custom-scope#file",
    ];

    for name in weird_names {
        // Write atomic
        let data = format!("content of {name}").into_bytes();
        let write_res = fs.write_atomic(name, &data, Some(0o644));
        assert!(write_res.is_ok(), "Failed to write weird file: {}", name);

        // Read back
        let read_res = fs.read_to_vec(name, None);
        assert!(read_res.is_ok(), "Failed to read back weird file: {}", name);
        assert_eq!(read_res.unwrap(), data);

        // Metadata check
        let meta_res = fs.metadata(name);
        assert!(meta_res.is_ok(), "Failed to get metadata for: {}", name);
        let entry = meta_res.unwrap();
        assert_eq!(entry.name, name);
        assert_eq!(entry.size_bytes, data.len() as u64);

        // Remove file
        let rm_res = fs.remove_file(name);
        assert!(rm_res.is_ok(), "Failed to remove weird file: {}", name);
    }
}

#[test]
fn test_root_protection_cannot_delete_jail() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    assert!(matches!(
        fs.remove_dir_all(""),
        Err(FsError::RootProtectionViolation { .. })
    ));
    assert!(matches!(
        fs.remove_dir_all("."),
        Err(FsError::RootProtectionViolation { .. })
    ));
    assert!(matches!(
        fs.remove_dir_all("/"),
        Err(FsError::RootProtectionViolation { .. })
    ));
    assert!(matches!(
        fs.remove_file(""),
        Err(FsError::RootProtectionViolation { .. })
    ));
}

#[test]
fn test_atomic_write_and_read_range() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    let data = b"Hello, Nusa Panel confined filesystem engine!";
    fs.write_atomic("hello.txt", data, Some(0o644))
        .expect("write");

    // Full read
    let read_all = fs.read_to_vec("hello.txt", None).expect("read");
    assert_eq!(read_all, data);

    // Bounded read limit exceeded
    let bounded_err = fs.read_to_vec("hello.txt", Some(10));
    assert!(matches!(bounded_err, Err(FsError::FileTooLarge { .. })));

    // Bounded read within limit
    let bounded_ok = fs.read_to_vec("hello.txt", Some(100)).expect("bounded ok");
    assert_eq!(bounded_ok, data);

    // Range read (HTTP Range simulation: start 7, len 10 -> "Nusa Panel")
    let range = fs.read_range("hello.txt", 7, 10).expect("read_range");
    assert_eq!(range, b"Nusa Panel");
}

#[test]
fn test_directory_operations_and_pagination() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    fs.create_dir_all("sites/example.com/public")
        .expect("mkdir all");
    fs.write_atomic(
        "sites/example.com/public/index.php",
        b"<?php phpinfo(); ?>",
        None,
    )
    .expect("write");
    fs.write_atomic(
        "sites/example.com/public/style.css",
        b"body { color: red; }",
        None,
    )
    .expect("write");
    fs.write_atomic(
        "sites/example.com/public/app.js",
        b"console.log('hi');",
        None,
    )
    .expect("write");
    fs.write_atomic("sites/example.com/public/.env", b"SECRET=123", None)
        .expect("write");

    // List without hidden files
    let list_no_hidden = fs
        .list_dir("sites/example.com/public", ListOptions::default())
        .expect("list");
    assert_eq!(list_no_hidden.total_count, 3);
    assert!(!list_no_hidden.entries.iter().any(|e| e.name == ".env"));

    // List with hidden files
    let opts_hidden = ListOptions {
        show_hidden: true,
        ..Default::default()
    };
    let list_hidden = fs
        .list_dir("sites/example.com/public", opts_hidden)
        .expect("list");
    assert_eq!(list_hidden.total_count, 4);
    assert!(list_hidden.entries.iter().any(|e| e.name == ".env"));

    // Filter by extension
    let opts_ext = ListOptions {
        extension: Some("php".to_string()),
        ..Default::default()
    };
    let list_ext = fs
        .list_dir("sites/example.com/public", opts_ext)
        .expect("list");
    assert_eq!(list_ext.total_count, 1);
    assert_eq!(list_ext.entries[0].name, "index.php");

    // Pagination: limit 2, offset 0
    let opts_page1 = ListOptions {
        limit: 2,
        offset: 0,
        sort_by: SortBy::Name,
        sort_order: SortOrder::Asc,
        ..Default::default()
    };
    let page1 = fs
        .list_dir("sites/example.com/public", opts_page1)
        .expect("page1");
    assert_eq!(page1.entries.len(), 2);
    assert!(page1.has_more);

    // Pagination: limit 2, offset 2
    let opts_page2 = ListOptions {
        limit: 2,
        offset: 2,
        sort_by: SortBy::Name,
        sort_order: SortOrder::Asc,
        ..Default::default()
    };
    let page2 = fs
        .list_dir("sites/example.com/public", opts_page2)
        .expect("page2");
    assert_eq!(page2.entries.len(), 1);
    assert!(!page2.has_more);
}

#[test]
fn test_rename_and_copy_confinement() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    fs.write_atomic("orig.txt", b"original data", None)
        .expect("write");

    // Copy within root
    fs.copy_file("orig.txt", "copied.txt").expect("copy");
    let read_copy = fs.read_to_vec("copied.txt", None).expect("read copy");
    assert_eq!(read_copy, b"original data");

    // Rename within root
    fs.rename("copied.txt", "renamed.txt").expect("rename");
    assert!(fs.read_to_vec("copied.txt", None).is_err());
    assert_eq!(
        fs.read_to_vec("renamed.txt", None).expect("read"),
        b"original data"
    );

    // Rename escaping root via destination '..'
    let esc_rename = fs.rename("renamed.txt", "../escaped.txt");
    assert!(matches!(
        esc_rename,
        Err(FsError::PathTraversalAttempt { .. })
    ));

    // Copy escaping root via destination '..'
    let esc_copy = fs.copy_file("renamed.txt", "../../escaped.txt");
    assert!(matches!(
        esc_copy,
        Err(FsError::PathTraversalAttempt { .. })
    ));
}

#[cfg(unix)]
#[test]
fn test_symlink_security_scenarios() {
    let outside = tempdir().expect("outside tempdir");
    let jail = tempdir().expect("jail tempdir");

    let outside_secret = outside.path().join("secret.txt");
    std::fs::write(&outside_secret, b"HOST_CONFIDENTIAL_KEY").expect("write secret");

    // Scenario 1: SymlinkPolicy::DenyAll rejects any symlink
    let fs_deny = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");
    let link_inside = jail.path().join("symlink_inside");
    std::os::unix::fs::symlink(jail.path().join("legit.txt"), &link_inside).expect("symlink");

    let res_deny = fs_deny.resolve("symlink_inside");
    assert!(matches!(res_deny, Err(FsError::SymlinkForbidden { .. })));

    // Scenario 2: SymlinkPolicy::ResolveBeneath rejects symlink escaping outside jail
    let fs_beneath = ConfinedFs::new(jail.path(), SymlinkPolicy::ResolveBeneath).expect("fs");
    let link_outside = jail.path().join("symlink_to_host");
    std::os::unix::fs::symlink(&outside_secret, &link_outside).expect("symlink outside");

    let res_outside = fs_beneath.resolve("symlink_to_host");
    assert!(matches!(res_outside, Err(FsError::SymlinkEscape { .. })));

    // Scenario 3: SymlinkPolicy::ResolveBeneath allows symlinks confined within root
    let legit_target = jail.path().join("target.txt");
    std::fs::write(&legit_target, b"INSIDE_JAIL_CONTENT").expect("write target");
    let link_valid = jail.path().join("link_valid.txt");
    std::os::unix::fs::symlink("target.txt", &link_valid).expect("symlink relative valid");

    let res_valid = fs_beneath.resolve("link_valid.txt");
    assert!(res_valid.is_ok(), "Valid internal symlink should resolve");
    let read_link_data = fs_beneath
        .read_to_vec("link_valid.txt", None)
        .expect("read");
    assert_eq!(read_link_data, b"INSIDE_JAIL_CONTENT");

    // Scenario 4: Symlink loop cycle
    let loop_a = jail.path().join("loop_a");
    let loop_b = jail.path().join("loop_b");
    std::os::unix::fs::symlink("loop_b", &loop_a).expect("symlink loop_a");
    std::os::unix::fs::symlink("loop_a", &loop_b).expect("symlink loop_b");

    let res_loop = fs_beneath.resolve("loop_a");
    assert!(matches!(res_loop, Err(FsError::SymlinkLoop { .. })));
}

#[test]
fn test_permissions_validation() {
    let jail = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("fs");

    fs.write_atomic("perm.txt", b"test", None).expect("write");

    // Valid mode
    assert!(fs.set_permissions("perm.txt", 0o644).is_ok());

    // Invalid mode with high bits outside 0o7777
    assert!(matches!(
        fs.set_permissions("perm.txt", 0o100000),
        Err(FsError::InvalidMode(_))
    ));
}
