#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_fs::fs::ConfinedFs;
use panel_fs::policy::SymlinkPolicy;
use panel_fs::search::{SearchOptions, search_confined};
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_search_by_filename_and_extension_filter() {
    let tmp = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(tmp.path(), SymlinkPolicy::DenyAll).expect("fs");

    // Setup files
    fs.create_dir_all("src").expect("create src");
    fs.create_dir_all("docs").expect("create docs");
    fs.write_atomic("src/main.rs", b"fn main() {}", None)
        .expect("write main.rs");
    fs.write_atomic("src/lib.rs", b"pub mod test;", None)
        .expect("write lib.rs");
    fs.write_atomic("docs/guide.md", b"# Guide", None)
        .expect("write guide.md");
    fs.write_atomic("src/main.js", b"console.log('hello')", None)
        .expect("write main.js");

    // 1. Search for "main" without extension filter
    let opts = SearchOptions {
        query: "main".to_string(),
        search_content: false,
        case_sensitive: false,
        extension: None,
        max_results: 50,
        timeout: Duration::from_secs(5),
        max_file_size_for_content: 1024 * 1024,
    };
    let report = search_confined(&fs, Path::new(""), &opts).expect("search");
    assert_eq!(report.items.len(), 2);
    let paths: Vec<&str> = report.items.iter().map(|i| i.path.as_str()).collect();
    assert!(paths.contains(&"src/main.rs"));
    assert!(paths.contains(&"src/main.js"));
    assert!(!report.limit_reached);
    assert!(!report.timed_out);

    // 2. Search for "main" with extension "rs"
    let opts_rs = SearchOptions {
        query: "main".to_string(),
        search_content: false,
        case_sensitive: false,
        extension: Some("rs".to_string()),
        max_results: 50,
        timeout: Duration::from_secs(5),
        max_file_size_for_content: 1024 * 1024,
    };
    let report_rs = search_confined(&fs, Path::new(""), &opts_rs).expect("search rs");
    assert_eq!(report_rs.items.len(), 1);
    assert_eq!(report_rs.items[0].path, "src/main.rs");
}

#[test]
fn test_search_content_matches_with_snippets_and_binary_skipping() {
    let tmp = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(tmp.path(), SymlinkPolicy::DenyAll).expect("fs");

    fs.create_dir_all("app").expect("create app");
    let php_code = "<?php\n// Secret key configuration\n$api_key = 'PANEL_SECRET_TOKEN_123';\necho 'Connected';\n";
    fs.write_atomic("app/config.php", php_code.as_bytes(), None)
        .expect("write config.php");

    // Binary file containing query string but with a null byte
    let mut bin_data = b"PANEL_SECRET_TOKEN_123 in binary".to_vec();
    bin_data.insert(0, 0x00); // Null byte flags file as binary
    fs.write_atomic("app/data.bin", &bin_data, None)
        .expect("write data.bin");

    let opts = SearchOptions {
        query: "PANEL_SECRET_TOKEN".to_string(),
        search_content: true,
        case_sensitive: true,
        extension: None,
        max_results: 50,
        timeout: Duration::from_secs(5),
        max_file_size_for_content: 1024 * 1024,
    };

    let report = search_confined(&fs, Path::new("app"), &opts).expect("search content");
    assert_eq!(report.items.len(), 1);
    let item = &report.items[0];
    assert_eq!(item.path, "app/config.php");
    assert_eq!(item.matches.len(), 1);
    assert_eq!(item.matches[0].line_number, 3);
    assert!(
        item.matches[0]
            .line_text
            .contains("$api_key = 'PANEL_SECRET_TOKEN_123';")
    );
}

#[test]
fn test_search_max_results_cap() {
    let tmp = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(tmp.path(), SymlinkPolicy::DenyAll).expect("fs");

    // Create 15 files with prefix "item_"
    fs.create_dir_all("items").expect("create items");
    for i in 1..=15 {
        fs.write_atomic(format!("items/item_{:02}.txt", i), b"content", None)
            .expect("write");
    }

    let opts = SearchOptions {
        query: "item_".to_string(),
        search_content: false,
        case_sensitive: false,
        extension: None,
        max_results: 5,
        timeout: Duration::from_secs(5),
        max_file_size_for_content: 1024 * 1024,
    };

    let report = search_confined(&fs, Path::new("items"), &opts).expect("search");
    assert_eq!(report.items.len(), 5);
    assert!(report.limit_reached);
    assert!(!report.timed_out);
}

#[test]
fn test_search_skips_trash_and_upload_tmp() {
    let tmp = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(tmp.path(), SymlinkPolicy::DenyAll).expect("fs");

    // Create regular file
    fs.create_dir_all("site").expect("create site");
    fs.write_atomic("site/target.txt", b"find me", None)
        .expect("write site target");

    // Create trash directory and file
    fs.create_dir_all(".panel-trash/sub").expect("create trash");
    let trash_file = tmp.path().join(".panel-trash/target.txt");
    fs::write(trash_file, b"deleted target").expect("write trash file");

    // Create upload tmp directory and file
    fs.create_dir_all(".panel-upload-tmp")
        .expect("create upload tmp");
    let upload_file = tmp.path().join(".panel-upload-tmp/target.txt");
    fs::write(upload_file, b"upload target").expect("write upload file");

    let opts = SearchOptions {
        query: "target.txt".to_string(),
        search_content: false,
        case_sensitive: false,
        extension: None,
        max_results: 50,
        timeout: Duration::from_secs(5),
        max_file_size_for_content: 1024 * 1024,
    };

    let report = search_confined(&fs, Path::new(""), &opts).expect("search");
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].path, "site/target.txt");
}

#[test]
fn test_search_confinement_violation_rejected() {
    let tmp = tempdir().expect("tempdir");
    let fs = ConfinedFs::new(tmp.path(), SymlinkPolicy::DenyAll).expect("fs");

    let opts = SearchOptions {
        query: "passwd".to_string(),
        search_content: false,
        case_sensitive: false,
        extension: None,
        max_results: 10,
        timeout: Duration::from_secs(5),
        max_file_size_for_content: 1024 * 1024,
    };

    // Attempting to search starting from path traversal .. must error
    let res = search_confined(&fs, Path::new("../../etc"), &opts);
    assert!(res.is_err());
}
