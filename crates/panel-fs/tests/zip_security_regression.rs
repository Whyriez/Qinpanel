//! Regression coverage for P10-T6an and RUSTSEC-2026-0367/0368/0369.
//! Exercises the panel extractor and the actual Swagger build extractor without daemons.

#[path = "../../../vendor/utoipa-swagger-ui/extract.rs"]
mod swagger_extract;

use panel_fs::archive::{ArchiveFormat, extract_archive};
use panel_fs::{ConfinedFs, FsError, SymlinkPolicy};
use std::io::{Cursor, Write};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

fn symlink_zip(target: &str) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("swagger/dist/escape", target, SimpleFileOptions::default())
        .expect("write symlink metadata without creating a filesystem symlink");
    writer.finish().expect("finish zip").into_inner()
}

#[test]
fn panel_rejects_zip_symlinks_before_writing_or_changing_the_target() {
    let jail = tempdir().expect("jail");
    let outside = tempdir().expect("outside jail");
    let sentinel = outside.path().join("sentinel");
    std::fs::write(&sentinel, b"unchanged").expect("sentinel");
    let original_permissions = std::fs::metadata(&sentinel)
        .expect("metadata")
        .permissions();
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("confined fs");
    for target in [
        sentinel.to_string_lossy().as_ref(),
        "../../../sentinel",
        "../local",
    ] {
        std::fs::write(jail.path().join("attack.zip"), symlink_zip(target)).expect("zip fixture");
        let result = extract_archive(&fs, "attack.zip", "out", Some(ArchiveFormat::Zip), None);
        assert!(matches!(result, Err(FsError::SymlinkForbidden { .. })));
        assert!(std::fs::symlink_metadata(jail.path().join("out/swagger/dist/escape")).is_err());
        assert_eq!(
            std::fs::read(&sentinel).expect("sentinel retained"),
            b"unchanged"
        );
        assert_eq!(
            std::fs::metadata(&sentinel)
                .expect("metadata")
                .permissions(),
            original_permissions
        );
    }
}

#[test]
fn swagger_rejects_zip_symlinks_before_writing_or_changing_the_target() {
    let destination = tempdir().expect("assets");
    let outside = tempdir().expect("outside assets");
    let sentinel = outside.path().join("sentinel");
    std::fs::write(&sentinel, b"unchanged").expect("sentinel");
    let original_permissions = std::fs::metadata(&sentinel)
        .expect("metadata")
        .permissions();
    for target in [
        sentinel.to_string_lossy().as_ref(),
        "../../../sentinel",
        "../local",
    ] {
        let mut archive = zip::ZipArchive::new(Cursor::new(symlink_zip(target))).expect("zip");
        let error = swagger_extract::extract_dist(&mut archive, destination.path())
            .expect_err("reject symlink");
        assert!(matches!(
            error,
            zip::result::ZipError::UnsupportedArchive(_)
        ));
        assert!(std::fs::symlink_metadata(destination.path().join("swagger/dist/escape")).is_err());
        assert_eq!(
            std::fs::read(&sentinel).expect("sentinel retained"),
            b"unchanged"
        );
        assert_eq!(
            std::fs::metadata(&sentinel)
                .expect("metadata")
                .permissions(),
            original_permissions
        );
    }
}

#[test]
fn swagger_rejects_traversal_in_asset_paths() {
    let destination = tempdir().expect("assets");
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("swagger/dist/../../../escape", SimpleFileOptions::default())
        .expect("entry");
    writer.write_all(b"escape").expect("payload");
    let mut archive = zip::ZipArchive::new(writer.finish().expect("zip")).expect("read zip");
    assert!(swagger_extract::extract_dist(&mut archive, destination.path()).is_err());
    assert_eq!(
        std::fs::read_dir(destination.path())
            .expect("directory")
            .count(),
        0
    );
}

#[test]
fn swagger_extracts_distribution_assets_without_repository_files() {
    let destination = tempdir().expect("assets");
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in [
        ("swagger/README.md", "ignored"),
        ("swagger/dist/index.html", "<html>Swagger UI</html>"),
    ] {
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("entry");
        writer.write_all(content.as_bytes()).expect("payload");
    }
    let mut archive = zip::ZipArchive::new(writer.finish().expect("zip")).expect("read zip");
    assert_eq!(
        swagger_extract::extract_dist(&mut archive, destination.path()).expect("extract"),
        "swagger"
    );
    assert_eq!(
        std::fs::read(destination.path().join("swagger/dist/index.html")).expect("asset"),
        b"<html>Swagger UI</html>"
    );
    assert!(!destination.path().join("swagger/README.md").exists());
}

fn privileged_permissions_zip() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_directory("swagger/dist/", SimpleFileOptions::default())
        .expect("directory");
    writer
        .start_file("swagger/dist/index.html", SimpleFileOptions::default())
        .expect("file");
    writer.write_all(b"asset").expect("payload");
    let mut bytes = writer.finish().expect("zip").into_inner();
    // The writer correctly masks privileged bits. Inject hostile raw metadata instead.
    for offset in 0..bytes.len().saturating_sub(46) {
        if bytes[offset..offset + 4] == *b"PK\x01\x02" {
            bytes[offset + 5] = 3; // Unix origin.
            let attrs = u32::from_le_bytes(
                bytes[offset + 38..offset + 42]
                    .try_into()
                    .expect("attributes"),
            );
            bytes[offset + 38..offset + 42]
                .copy_from_slice(&(attrs | (0o7000 << 16)).to_le_bytes());
        }
    }
    bytes
}

#[test]
fn zip_reader_masks_privileged_bits_from_hostile_raw_metadata() {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(privileged_permissions_zip())).expect("read metadata");
    assert_eq!(archive.len(), 2, "fixture contains a directory and a file");
    for index in 0..archive.len() {
        let entry = archive.by_index(index).expect("entry");
        // external_attributes() is raw ZIP metadata; unix_mode() is sanitized by ZIP 9.
        let raw_mode = entry.external_attributes() >> 16;
        assert_eq!(raw_mode & 0o7000, 0o7000, "fixture has hostile raw bits");
        let sanitized_mode = entry.unix_mode().expect("sanitized mode");
        assert_eq!(sanitized_mode & 0o7000, 0, "reader removes privileged bits");
        assert_eq!(
            sanitized_mode,
            raw_mode & !0o7000,
            "ordinary mode bits remain"
        );
    }
}

#[cfg(unix)]
#[test]
fn extractors_never_restore_privileged_unix_bits() {
    use std::os::unix::fs::PermissionsExt;
    let bytes = privileged_permissions_zip();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).expect("read metadata");
    let jail = tempdir().expect("jail");
    std::fs::write(jail.path().join("attack.zip"), &bytes).expect("zip fixture");
    let fs = ConfinedFs::new(jail.path(), SymlinkPolicy::DenyAll).expect("confined fs");
    extract_archive(&fs, "attack.zip", "panel", Some(ArchiveFormat::Zip), None)
        .expect("panel extraction");
    swagger_extract::extract_dist(&mut archive, &jail.path().join("build"))
        .expect("Swagger extraction");
    for prefix in ["panel", "build"] {
        for entry in ["swagger/dist", "swagger/dist/index.html"] {
            let mode = std::fs::metadata(jail.path().join(prefix).join(entry))
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o7000,
                0,
                "{prefix}/{entry} retained privileged bits"
            );
        }
    }
}
