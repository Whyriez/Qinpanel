#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_db::{CreateAuditLog, CreateJob, DbPool, SqliteAuditLogRepository, SqliteJobRepository};
use panel_db::{init_pool, operations, run_migrations, snapshot};
use std::time::Duration;

async fn core(temp: &tempfile::TempDir) -> DbPool {
    let url = format!("sqlite://{}", temp.path().join("panel.db").display());
    let pool = init_pool(&url).await.unwrap();
    run_migrations(&pool).await.unwrap();
    pool
}

fn job(id: impl Into<String>) -> CreateJob {
    CreateJob {
        id: id.into(),
        user_id: None,
        name: "test.sleep".to_owned(),
        payload_json: "{}".to_owned(),
        max_retries: 3,
    }
}

fn audit() -> CreateAuditLog {
    CreateAuditLog {
        user_id: None,
        actor_username: "admin".to_owned(),
        action: "test.partition".to_owned(),
        target_resource: "test".to_owned(),
        details_json: None,
        ip_address: None,
        status: "success".to_owned(),
    }
}

#[tokio::test]
async fn import_preserves_ids_and_does_not_replay_stale_data_on_restart() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp).await;
    SqliteJobRepository::new(core.clone())
        .create(&job("legacy-job"))
        .await
        .unwrap();
    let old_log = SqliteAuditLogRepository::new(core.clone())
        .create(&audit())
        .await
        .unwrap();
    let (writer, reader) = operations::open(&core).await.unwrap();
    let jobs = SqliteJobRepository::new(writer.clone());
    assert!(jobs.find_by_id("legacy-job").await.unwrap().is_some());
    assert!(
        SqliteAuditLogRepository::new(reader.clone())
            .find_by_id(old_log.id)
            .await
            .unwrap()
            .is_some()
    );
    jobs.update_status("legacy-job", "succeeded", None, None)
        .await
        .unwrap();
    let new_log = SqliteAuditLogRepository::new(writer.clone())
        .create(&audit())
        .await
        .unwrap();
    assert!(new_log.id > old_log.id);
    // WAL readers cannot accidentally bypass the sole writer.
    assert!(
        sqlx::query("DELETE FROM jobs")
            .execute(&reader)
            .await
            .is_err()
    );
    writer.close().await;
    reader.close().await;
    let (writer, reader) = operations::open(&core).await.unwrap();
    let migrated = SqliteJobRepository::new(reader.clone())
        .find_by_id("legacy-job")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(migrated.status, "succeeded");
    assert_eq!(migrated.progress, 100);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audit_logs")
            .fetch_one(&reader)
            .await
            .unwrap(),
        2
    );
    // Source remains intact for upgrade recovery, but no new writes go there.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audit_logs")
            .fetch_one(&core)
            .await
            .unwrap(),
        1
    );
    writer.close().await;
    reader.close().await;
    core.close().await;
}

#[tokio::test]
async fn core_writer_does_not_block_job_and_audit_writes_or_wal_reads() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp).await;
    let (writer, reader) = operations::open(&core).await.unwrap();
    let mut core_lock = core.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("INSERT INTO settings VALUES ('held', '1', 0)")
        .execute(&mut *core_lock)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..64 {
            let writer = writer.clone();
            tasks.spawn(async move {
                SqliteJobRepository::new(writer.clone())
                    .create(&job(format!("job-{i}")))
                    .await
                    .unwrap();
                SqliteAuditLogRepository::new(writer)
                    .create(&audit())
                    .await
                    .unwrap();
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("operational writes must not wait for the core lock");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs")
            .fetch_one(&reader)
            .await
            .unwrap(),
        64
    );
    let mut write_lock = writer.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("UPDATE jobs SET progress = 10")
        .execute(&mut *write_lock)
        .await
        .unwrap();
    let visible: i64 = tokio::time::timeout(
        Duration::from_secs(1),
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE progress = 0").fetch_one(&reader),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(visible, 64);
    write_lock.rollback().await.unwrap();
    core_lock.rollback().await.unwrap();
    writer.close().await;
    reader.close().await;
    core.close().await;
}

#[tokio::test]
async fn concurrent_claims_are_unique_and_snapshot_contains_uncheckpointed_writes() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp).await;
    let (writer, reader) = operations::open(&core).await.unwrap();
    let repo = SqliteJobRepository::new(writer.clone());
    for i in 0..32 {
        repo.create(&job(format!("job-{i}"))).await.unwrap();
    }
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let repo = repo.clone();
        tasks.spawn(async move { repo.claim_next_queued().await.unwrap().unwrap().id });
    }
    let mut ids = std::collections::HashSet::new();
    while let Some(result) = tasks.join_next().await {
        assert!(ids.insert(result.unwrap()));
    }
    assert_eq!(ids.len(), 32);
    let destination = temp.path().join("snapshot.db");
    snapshot::from_file(&temp.path().join("panel.operations.db"), &destination)
        .await
        .unwrap();
    let copy = init_pool(&format!("sqlite://{}", destination.display()))
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE status='running'")
            .fetch_one(&copy)
            .await
            .unwrap(),
        32
    );
    copy.close().await;
    writer.close().await;
    reader.close().await;
    core.close().await;
}

#[tokio::test]
async fn restored_rollback_journal_snapshot_reopens_with_operational_pools() {
    use sqlx::Connection;

    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp).await;
    let path = temp.path().join("panel.operations.db");
    let (writer, reader) = operations::open(&core).await.unwrap();
    SqliteJobRepository::new(writer.clone())
        .create(&job("saved-job"))
        .await
        .unwrap();
    let snapshot = temp.path().join("operations.snapshot.db");
    snapshot::from_file(&path, &snapshot).await.unwrap();
    // VACUUM INTO produces a standalone snapshot without the source WAL files.
    let mut copy = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(&snapshot),
    )
    .await
    .unwrap();
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&mut copy)
        .await
        .unwrap();
    assert_eq!(mode, "delete");
    copy.close().await.unwrap();

    SqliteJobRepository::new(writer.clone())
        .update_status("saved-job", "failed", None, Some("after snapshot"))
        .await
        .unwrap();
    reader.close().await;
    writer.close().await;
    std::fs::copy(&snapshot, &path).unwrap();

    let (writer, reader) = operations::open(&core).await.unwrap();
    for pool in [&writer, &reader] {
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(integrity, "ok");
    }
    let jobs = SqliteJobRepository::new(reader.clone());
    assert_eq!(
        jobs.find_by_id("saved-job").await.unwrap().unwrap().status,
        "queued"
    );
    SqliteJobRepository::new(writer.clone())
        .update_status("saved-job", "running", None, None)
        .await
        .unwrap();
    assert_eq!(
        jobs.find_by_id("saved-job").await.unwrap().unwrap().status,
        "running"
    );
    assert!(
        sqlx::query("DELETE FROM jobs")
            .execute(&reader)
            .await
            .is_err()
    );
    reader.close().await;
    writer.close().await;
    core.close().await;
}

#[tokio::test]
async fn failed_import_rolls_back_both_tables_and_can_be_retried() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp).await;
    SqliteJobRepository::new(core.clone())
        .create(&job("legacy-job"))
        .await
        .unwrap();
    sqlx::query("ALTER TABLE audit_logs RENAME TO audit_logs_missing")
        .execute(&core)
        .await
        .unwrap();
    assert!(operations::open(&core).await.is_err());
    let path = temp.path().join("panel.operations.db");
    let failed = init_pool(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs")
            .fetch_one(&failed)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM legacy_import")
            .fetch_one(&failed)
            .await
            .unwrap(),
        0
    );
    sqlx::query("ALTER TABLE audit_logs_missing RENAME TO audit_logs")
        .execute(&core)
        .await
        .unwrap();
    let (writer, reader) = operations::open(&core).await.unwrap();
    assert!(
        SqliteJobRepository::new(reader.clone())
            .find_by_id("legacy-job")
            .await
            .unwrap()
            .is_some()
    );
    writer.close().await;
    reader.close().await;
    failed.close().await;
    core.close().await;
}
