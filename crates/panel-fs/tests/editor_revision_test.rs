use panel_core::file_revision::content_etag;
use panel_fs::{
    ConfinedFs, FsError, FsWorkerPool, FsWorkerPoolConfig, FsWorkerRequest, FsWorkerResponse,
    SymlinkPolicy, TenantWorkerKey,
};

#[test]
fn checked_write_rejects_same_length_edits_and_deleted_files_without_temp_leaks() {
    let temp = tempfile::tempdir().unwrap();
    let fs = ConfinedFs::new(temp.path(), SymlinkPolicy::DenyAll).unwrap();
    fs.write_atomic("config", b"old-value", Some(0o600))
        .unwrap();
    let revision = content_etag(b"old-value");
    fs.write_atomic("config", b"new-value", None).unwrap();
    assert!(matches!(
        fs.write_atomic_checked("config", b"overwrite", None, Some(&revision)),
        Err(FsError::ContentConflict { .. })
    ));
    assert_eq!(
        std::fs::read(temp.path().join("config")).unwrap(),
        b"new-value"
    );
    std::fs::remove_file(temp.path().join("config")).unwrap();
    assert!(
        fs.write_atomic_checked("config", b"resurrect", None, Some(&revision))
            .is_err()
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn rewriting_private_files_keeps_permissions_and_strips_special_bits() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let fs = ConfinedFs::new(temp.path(), SymlinkPolicy::DenyAll).unwrap();
    fs.write_atomic(".env", b"secret", Some(0o600)).unwrap();
    fs.write_atomic(".env", b"changed", None).unwrap();
    assert_eq!(
        std::fs::metadata(temp.path().join(".env"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
    fs.write_atomic("script", b"script", Some(0o6755)).unwrap();
    assert_eq!(
        std::fs::metadata(temp.path().join("script"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
}

#[tokio::test]
async fn concurrent_editor_saves_with_one_revision_cannot_both_commit() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("file"), b"initial").unwrap();
    let pool = FsWorkerPool::in_process_for_tests(FsWorkerPoolConfig::default());
    let tenant = TenantWorkerKey::new(10001, 10001, temp.path());
    let make_request = |value: &[u8]| FsWorkerRequest::WriteAtomic {
        path: "file".into(),
        data: value.to_vec(),
        mode: None,
        expected_etag: Some(content_etag(b"initial")),
    };
    let (first, second) = tokio::join!(
        pool.execute(&tenant, make_request(b"first")),
        pool.execute(&tenant, make_request(b"second"))
    );
    let responses = [first.unwrap(), second.unwrap()];
    assert_eq!(
        responses
            .iter()
            .filter(|response| matches!(response, FsWorkerResponse::WriteAtomic(Ok(_))))
            .count(),
        1
    );
    assert_eq!(
        responses
            .iter()
            .filter(|response| matches!(
                response,
                FsWorkerResponse::WriteAtomic(Err(FsError::ContentConflict { .. }))
            ))
            .count(),
        1
    );
    pool.shutdown_all().await;
}
