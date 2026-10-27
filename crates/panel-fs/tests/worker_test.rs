//! Integration test suite for `fs-worker` protocol, pool lifecycle, and isolation.
//!
//! Complies with PRD Section 5.3 and Task P5-T2:
//! - Worker request/response protocol execution
//! - Pool multi-tenant routing and instance reuse
//! - Idle timeout worker reaping
//! - Automatic worker recovery after termination
//! - Cross-tenant filesystem isolation

use panel_fs::{
    FsWorkerPool, FsWorkerPoolConfig, FsWorkerRequest, FsWorkerResponse, ListOptions,
    SymlinkPolicy, TenantWorkerKey,
};
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn test_worker_ping_and_crud() {
    let jail = tempdir().expect("tempdir");
    let pool = FsWorkerPool::in_process_for_tests(FsWorkerPoolConfig::default());
    let tenant = TenantWorkerKey::new(10001, 10001, jail.path());

    // Ping
    let pong = pool
        .execute(&tenant, FsWorkerRequest::Ping)
        .await
        .expect("ping");
    assert!(matches!(pong, FsWorkerResponse::Pong));

    // Create dir
    let mkdir_res = pool
        .execute(
            &tenant,
            FsWorkerRequest::CreateDir {
                path: "public/assets".into(),
                recursive: true,
            },
        )
        .await
        .expect("mkdir");
    assert!(matches!(mkdir_res, FsWorkerResponse::CreateDir(Ok(()))));

    // Write file
    let write_res = pool
        .execute(
            &tenant,
            FsWorkerRequest::WriteAtomic {
                path: "public/index.html".into(),
                data: b"<h1>Nusa Panel</h1>".to_vec(),
                mode: Some(0o644),
                expected_etag: None,
            },
        )
        .await
        .expect("write");
    assert!(matches!(write_res, FsWorkerResponse::WriteAtomic(Ok(19))));

    // Stat
    let stat_res = pool
        .execute(
            &tenant,
            FsWorkerRequest::Stat {
                path: "public/index.html".into(),
            },
        )
        .await
        .expect("stat");
    match stat_res {
        FsWorkerResponse::Stat(Ok(entry)) => {
            assert_eq!(entry.name, "index.html");
            assert_eq!(entry.size_bytes, 19);
        }
        other => panic!("Expected Stat Ok, got {:?}", other),
    }

    // Read file
    let read_res = pool
        .execute(
            &tenant,
            FsWorkerRequest::ReadFile {
                path: "public/index.html".into(),
                max_bytes: None,
            },
        )
        .await
        .expect("read");
    match read_res {
        FsWorkerResponse::ReadFile(Ok(data)) => {
            assert_eq!(data, b"<h1>Nusa Panel</h1>");
        }
        other => panic!("Expected ReadFile Ok, got {:?}", other),
    }

    // List dir
    let list_res = pool
        .execute(
            &tenant,
            FsWorkerRequest::ListDir {
                path: "public".into(),
                options: ListOptions::default(),
            },
        )
        .await
        .expect("list");
    match list_res {
        FsWorkerResponse::ListDir(Ok(list)) => {
            assert_eq!(list.total_count, 2); // assets/ and index.html
        }
        other => panic!("Expected ListDir Ok, got {:?}", other),
    }
}

#[tokio::test]
async fn test_worker_pool_multi_tenant_isolation() {
    let alice_jail = tempdir().expect("alice tempdir");
    let bob_jail = tempdir().expect("bob tempdir");

    let pool = FsWorkerPool::in_process_for_tests(FsWorkerPoolConfig::default());
    let alice = TenantWorkerKey::new(10001, 10001, alice_jail.path());
    let bob = TenantWorkerKey::new(10002, 10002, bob_jail.path());

    // Alice writes a secret file
    pool.execute(
        &alice,
        FsWorkerRequest::WriteAtomic {
            path: "secret.txt".into(),
            data: b"Alice secret key".to_vec(),
            mode: None,
            expected_etag: None,
        },
    )
    .await
    .expect("alice write");

    // Bob writes his own file
    pool.execute(
        &bob,
        FsWorkerRequest::WriteAtomic {
            path: "bob_file.txt".into(),
            data: b"Bob public note".to_vec(),
            mode: None,
            expected_etag: None,
        },
    )
    .await
    .expect("bob write");

    assert_eq!(pool.active_worker_count(), 2);

    // Bob tries to read Alice's secret file within his jail -> NotFound
    let bob_read = pool
        .execute(
            &bob,
            FsWorkerRequest::ReadFile {
                path: "secret.txt".into(),
                max_bytes: None,
            },
        )
        .await
        .expect("bob read");
    match bob_read {
        FsWorkerResponse::ReadFile(Err(err)) => {
            assert!(matches!(err, panel_fs::FsError::NotFound { .. }));
        }
        other => panic!(
            "Expected NotFound for Bob reading Alice's file, got {:?}",
            other
        ),
    }

    // Alice reads her secret file -> Ok
    let alice_read = pool
        .execute(
            &alice,
            FsWorkerRequest::ReadFile {
                path: "secret.txt".into(),
                max_bytes: None,
            },
        )
        .await
        .expect("alice read");
    match alice_read {
        FsWorkerResponse::ReadFile(Ok(data)) => {
            assert_eq!(data, b"Alice secret key");
        }
        other => panic!("Expected Alice read ok, got {:?}", other),
    }
}

#[tokio::test]
async fn test_worker_idle_timeout_reaping_and_recovery() {
    let jail = tempdir().expect("tempdir");

    // Configure aggressive idle timeout of 50ms for testing
    let config = FsWorkerPoolConfig {
        idle_timeout: Duration::from_millis(50),
        max_workers: 10,
        symlink_policy: SymlinkPolicy::DenyAll,
    };
    let pool = FsWorkerPool::in_process_for_tests(config);
    let tenant = TenantWorkerKey::new(10001, 10001, jail.path());

    // Execute first request -> spawns worker
    let resp = pool
        .execute(&tenant, FsWorkerRequest::Ping)
        .await
        .expect("ping");
    assert!(matches!(resp, FsWorkerResponse::Pong));
    assert_eq!(pool.active_worker_count(), 1);

    // Sleep past idle timeout
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Reap idle workers
    let reaped = pool.cleanup_idle();
    assert_eq!(reaped, 1);
    assert_eq!(pool.active_worker_count(), 0);

    // Subsequent request automatically spawns a fresh worker and succeeds!
    let resp2 = pool
        .execute(&tenant, FsWorkerRequest::Ping)
        .await
        .expect("ping 2");
    assert!(matches!(resp2, FsWorkerResponse::Pong));
    assert_eq!(pool.active_worker_count(), 1);

    // Shutdown all
    pool.shutdown_all().await;
    assert_eq!(pool.active_worker_count(), 0);
}
