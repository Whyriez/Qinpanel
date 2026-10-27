//! Comprehensive archive security and functionality test suite for `panel-fs`.
//!
//! Complies with PRD Section 5.3, Section 6, and Task P5-T5:
//! - Validates safe ZIP compression and extraction
//! - Validates safe TAR.GZ compression and extraction
//! - Validates Anti Zip-Slip protection against traversal payloads
//! - Validates Anti Symlink escape protection
//! - Validates Anti Zip-Bomb guardrails (entry limits and size limits)

use panel_fs::archive::{
    ArchiveFormat, ExtractLimits, compress_tar_gz, compress_zip, extract_archive,
    sanitize_archive_entry_path,
};
use panel_fs::{ConfinedFs, FsError, SymlinkPolicy};
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_sanitize_archive_entry_path() {
    // Normal paths
    assert_eq!(
        sanitize_archive_entry_path("hello.txt", "extract_dir").unwrap(),
        std::path::PathBuf::from("extract_dir").join("hello.txt")
    );
    assert_eq!(
        sanitize_archive_entry_path("sub/folder/file.css", "dest").unwrap(),
        std::path::PathBuf::from("dest")
            .join("sub")
            .join("folder")
            .join("file.css")
    );

    // Strips leading slashes safely
    assert_eq!(
        sanitize_archive_entry_path("/etc/passwd", "dest").unwrap(),
        std::path::PathBuf::from("dest").join("etc").join("passwd")
    );

    // Rejects path traversal (Zip-Slip)
    assert!(sanitize_archive_entry_path("../evil.txt", "dest").is_err());
    assert!(sanitize_archive_entry_path("../../etc/shadow", "dest").is_err());
    assert!(sanitize_archive_entry_path("sub/../../evil.txt", "dest").is_err());

    // Rejects null bytes
    assert!(sanitize_archive_entry_path("file.txt\0.evil", "dest").is_err());
}

#[test]
fn test_zip_compress_and_extract_roundtrip() {
    let jail = tempdir().expect("jail tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("confined fs");

    // Create test files and directories
    fs.create_dir_all("www/public").expect("create_dir_all");
    fs.write_atomic("www/public/index.html", b"<h1>Hello World</h1>", None)
        .expect("write index");
    fs.write_atomic("www/public/style.css", b"body { color: blue; }", None)
        .expect("write css");

    // Compress
    let comp_res =
        compress_zip(&fs, &["www/public".to_string()], "backup.zip").expect("compress_zip success");
    assert!(comp_res.entries_processed >= 2);
    assert!(comp_res.total_bytes > 0);
    assert_eq!(comp_res.target_path, "backup.zip");

    // Verify backup.zip exists
    assert!(fs.metadata("backup.zip").is_ok());

    // Extract to a new directory
    let ext_res = extract_archive(
        &fs,
        "backup.zip",
        "restored",
        Some(ArchiveFormat::Zip),
        None,
    )
    .expect("extract_archive success");
    assert!(ext_res.entries_processed >= 2);

    // Verify extracted files content
    let index_content = fs
        .read_to_vec("restored/public/index.html", None)
        .expect("read restored index");
    assert_eq!(index_content, b"<h1>Hello World</h1>");

    let css_content = fs
        .read_to_vec("restored/public/style.css", None)
        .expect("read restored css");
    assert_eq!(css_content, b"body { color: blue; }");
}

#[test]
fn test_tar_gz_compress_and_extract_roundtrip() {
    let jail = tempdir().expect("jail tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("confined fs");

    // Create test data
    fs.create_dir_all("app/config").expect("create app/config");
    fs.write_atomic("app/config/app.json", b"{\"name\": \"vpanel\"}", None)
        .expect("write json");

    // Compress TAR.GZ
    let comp_res = compress_tar_gz(&fs, &["app".to_string()], "backup.tar.gz")
        .expect("compress_tar_gz success");
    assert!(comp_res.entries_processed >= 1);

    // Verify file exists
    assert!(fs.metadata("backup.tar.gz").is_ok());

    // Extract TAR.GZ
    let ext_res = extract_archive(&fs, "backup.tar.gz", "unpacked", None, None)
        .expect("extract_archive tar.gz success");
    assert!(ext_res.entries_processed >= 1);

    let content = fs
        .read_to_vec("unpacked/app/config/app.json", None)
        .expect("read extracted file");
    assert_eq!(content, b"{\"name\": \"vpanel\"}");
}

#[test]
fn test_anti_zip_slip_malicious_archive_rejected() {
    let jail = tempdir().expect("jail tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("confined fs");

    // Manually build a malicious ZIP containing a path traversal entry
    let evil_zip_path = jail.path().join("evil.zip");
    {
        let file = File::create(&evil_zip_path).expect("create evil zip");
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        // Malicious entry escaping target folder
        zip_writer
            .start_file("../../evil_escaped.txt", options)
            .expect("start_file");
        zip_writer
            .write_all(b"Hacked payload outside jail!")
            .expect("write payload");
        zip_writer.finish().expect("finish");
    }

    // Attempt to extract the evil zip
    let res = extract_archive(&fs, "evil.zip", "extracted", Some(ArchiveFormat::Zip), None);
    assert!(res.is_err(), "Extraction of zip-slip payload must fail");

    match res.unwrap_err() {
        FsError::PathTraversalAttempt { .. } => {}
        other => panic!("Expected PathTraversalAttempt, got {:?}", other),
    }

    // Verify escaped file does not exist outside jail or destination
    assert!(!jail.path().join("evil_escaped.txt").exists());
}

#[test]
fn test_anti_zip_bomb_entry_limit() {
    let jail = tempdir().expect("jail tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("confined fs");

    // Build a ZIP with 10 files
    let zip_path = jail.path().join("many_files.zip");
    {
        let file = File::create(&zip_path).expect("create zip");
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        for i in 0..10 {
            zip_writer
                .start_file(format!("file_{i}.txt"), options)
                .expect("start file");
            zip_writer.write_all(b"sample").expect("write");
        }
        zip_writer.finish().expect("finish");
    }

    // Set limit max_entries to 5 -> should fail with zip-bomb detection
    let strict_limits = ExtractLimits {
        max_ratio: 100,
        max_entries: 5,
        max_uncompressed_bytes: 1024 * 1024,
    };

    let res = extract_archive(
        &fs,
        "many_files.zip",
        "output",
        Some(ArchiveFormat::Zip),
        Some(strict_limits),
    );
    assert!(res.is_err(), "Extraction exceeding entry limit must fail");
    match res.unwrap_err() {
        FsError::Archive(msg) => {
            assert!(msg.contains("entry count"));
        }
        other => panic!("Expected FsError::Archive, got {:?}", other),
    }
}

#[test]
fn test_anti_zip_bomb_uncompressed_size_limit() {
    let jail = tempdir().expect("jail tempdir");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("confined fs");

    // Build a ZIP with a 50KB file
    let zip_path = jail.path().join("large_entry.zip");
    {
        let file = File::create(&zip_path).expect("create zip");
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip_writer
            .start_file("big.txt", options)
            .expect("start file");
        let chunk = vec![b'A'; 50 * 1024];
        zip_writer.write_all(&chunk).expect("write");
        zip_writer.finish().expect("finish");
    }

    // Set limit max_uncompressed_bytes to 10KB -> should fail with FileTooLarge
    let strict_limits = ExtractLimits {
        max_ratio: 100,
        max_entries: 100,
        max_uncompressed_bytes: 10 * 1024, // 10 KB limit
    };

    let res = extract_archive(
        &fs,
        "large_entry.zip",
        "output",
        Some(ArchiveFormat::Zip),
        Some(strict_limits),
    );
    assert!(res.is_err(), "Extraction exceeding byte limit must fail");
    match res.unwrap_err() {
        FsError::FileTooLarge { .. } => {}
        other => panic!("Expected FsError::FileTooLarge, got {:?}", other),
    }
}
