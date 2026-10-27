//! Root-only acceptance, explicitly run in disposable Linux CI, never on the developer host.
#![cfg(target_os = "linux")]
use panel_fs::{
    FsError, FsWorkerPool, FsWorkerPoolConfig, FsWorkerRequest, FsWorkerResponse, TenantWorkerKey,
};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::Arc;

fn owned_home(path: &std::path::Path, uid: u32) {
    std::fs::create_dir(path).expect("home");
    std::os::unix::fs::chown(path, Some(uid), Some(uid)).expect("chown fixture");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).expect("mode");
}

#[tokio::test]
#[ignore = "Requires root in disposable Linux CI; does not run API or install services"]
async fn native_worker_drops_identity_and_contains_path_races() {
    #[allow(
        unsafe_code,
        reason = "Read-only effective UID query in the disposable Linux acceptance test"
    )]
    // SAFETY: Read-only credential query in a dedicated acceptance test process.
    let effective_uid = unsafe { libc::geteuid() };
    assert_eq!(effective_uid, 0);
    let temp = tempfile::tempdir().expect("temp");
    let home = temp.path().join("tenant");
    owned_home(&home, 10001);
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).expect("outside dir");
    std::fs::write(outside.join("secret"), b"HOST SECRET MUST NEVER BE READ")
        .expect("outside file");
    std::fs::create_dir(home.join("private")).expect("private dir");
    std::fs::set_permissions(home.join("private"), std::fs::Permissions::from_mode(0o700))
        .expect("private mode");
    std::fs::write(home.join("private/secret"), b"ROOT ONLY").expect("private file");
    let pool = Arc::new(FsWorkerPool::with_executable(
        FsWorkerPoolConfig::default(),
        env!("CARGO_BIN_EXE_panel-agent").into(),
    ));
    let tenant = TenantWorkerKey::new(10001, 10001, &home);
    // A legacy root-created regular file remains readable/editable via its writable
    // tenant-owned parent, without broad chown of protected legacy directories.
    std::fs::write(home.join("legacy.txt"), b"legacy").expect("legacy file");
    std::fs::set_permissions(
        home.join("legacy.txt"),
        std::fs::Permissions::from_mode(0o644),
    )
    .expect("legacy mode");
    let legacy = pool
        .execute(
            &tenant,
            FsWorkerRequest::WriteAtomic {
                path: "legacy.txt".into(),
                data: b"edited".to_vec(),
                mode: None,
                expected_etag: Some(panel_core::file_revision::content_etag(b"legacy")),
            },
        )
        .await
        .expect("legacy write transport");
    assert!(matches!(legacy, FsWorkerResponse::WriteAtomic(Ok(6))));
    assert_eq!(
        std::fs::metadata(home.join("legacy.txt")).unwrap().uid(),
        10001
    );
    let response = pool
        .execute(
            &tenant,
            FsWorkerRequest::WriteAtomic {
                path: "created.txt".into(),
                data: b"safe".to_vec(),
                mode: Some(0o600),
                expected_etag: None,
            },
        )
        .await
        .expect("worker transport");
    assert!(matches!(response, FsWorkerResponse::WriteAtomic(Ok(4))));
    let metadata = std::fs::metadata(home.join("created.txt")).expect("metadata");
    assert_eq!((metadata.uid(), metadata.gid()), (10001, 10001));
    for path in ["private/secret", "/etc/passwd", "../../outside/secret"] {
        let response = pool
            .execute(
                &tenant,
                FsWorkerRequest::ReadFile {
                    path: path.into(),
                    max_bytes: Some(1024),
                },
            )
            .await
            .expect("read transport");
        assert!(
            matches!(response, FsWorkerResponse::ReadFile(Err(_))),
            "{path}"
        );
    }
    // An admin flag in an IPC operation cannot restore root after the worker's drop.
    for (uid, gid, caller_uid, is_admin) in [
        (Some(0), Some(0), 0, true),
        (Some(10002), None, 10001, true),
        (None, Some(0), 10001, true),
        (Some(0), None, 10001, false),
    ] {
        let response = pool
            .execute(
                &tenant,
                FsWorkerRequest::Chown {
                    path: "created.txt".into(),
                    uid,
                    gid,
                    caller_uid,
                    is_admin,
                },
            )
            .await
            .expect("chown transport");
        assert!(
            matches!(
                &response,
                FsWorkerResponse::Chown(Err(FsError::PermissionDenied { .. }))
            ),
            "ownership denial must be returned through IPC: {response:?}"
        );
        let saved = std::fs::metadata(home.join("created.txt")).expect("file owner after denial");
        assert_eq!((saved.uid(), saved.gid()), (10001, 10001));
        assert_eq!(saved.mode() & 0o7777, 0o600);
        assert_eq!(std::fs::read(home.join("created.txt")).unwrap(), b"safe");
    }

    let race = home.join("race");
    let parked = home.join("parked");
    std::fs::create_dir(&race).expect("race dir");
    std::os::unix::fs::chown(&race, Some(10001), Some(10001)).expect("race owner");
    std::fs::write(race.join("secret"), b"tenant content").expect("tenant content");
    let attacker = std::thread::spawn(move || {
        for _ in 0..2000 {
            if std::fs::rename(&race, &parked).is_ok() {
                let _ = std::os::unix::fs::symlink(&outside, &race);
                std::thread::yield_now();
                let _ = std::fs::remove_file(&race);
                let _ = std::fs::rename(&parked, &race);
            }
        }
    });
    for _ in 0..100 {
        if let FsWorkerResponse::ReadFile(Ok(bytes)) = pool
            .execute(
                &tenant,
                FsWorkerRequest::ReadFile {
                    path: "race/secret".into(),
                    max_bytes: Some(1024),
                },
            )
            .await
            .expect("race transport")
        {
            assert_eq!(bytes, b"tenant content");
        }
    }
    attacker.join().expect("attacker");
    pool.shutdown_all().await;
}

#[tokio::test]
#[ignore = "Requires root in disposable Linux CI"]
async fn native_worker_fails_closed_for_invalid_identity_or_home_owner() {
    let temp = tempfile::tempdir().expect("temp");
    let home = temp.path().join("tenant");
    owned_home(&home, 10001);
    let pool = FsWorkerPool::with_executable(
        FsWorkerPoolConfig::default(),
        env!("CARGO_BIN_EXE_panel-agent").into(),
    );
    for (uid, gid) in [(0, 0), (10002, 10002)] {
        assert!(
            pool.execute(
                &TenantWorkerKey::new(uid, gid, &home),
                FsWorkerRequest::Ping
            )
            .await
            .is_err()
        );
    }
    pool.shutdown_all().await;
}
