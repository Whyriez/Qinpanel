#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_analytics::{AnalyticsStorage, LogCheckpoint};

#[tokio::test]
async fn concurrent_ingestion_and_reads_use_separate_wal_connections() {
    let temp = tempfile::tempdir().unwrap();
    let storage = AnalyticsStorage::connect(temp.path().join("analytics.db"))
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..64 {
        let storage = storage.clone();
        tasks.spawn(async move {
            let path = format!("/log/site-{i}.log");
            storage
                .save_log_checkpoint(&LogCheckpoint {
                    file_path: path.clone(),
                    site_id: i,
                    inode: i as u64,
                    file_offset: 100,
                    last_processed_ts: 1,
                    updated_at: 1,
                })
                .await
                .unwrap();
            let checkpoint = storage.get_log_checkpoint(&path).await.unwrap().unwrap();
            assert_eq!(checkpoint.file_offset, 100);
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM log_tailer_checkpoints")
        .fetch_one(storage.write_pool())
        .await
        .unwrap();
    assert_eq!(count, 64);
}

#[tokio::test]
async fn maintenance_writes_are_visible_to_query_only_readers() {
    let temp = tempfile::tempdir().unwrap();
    let storage = AnalyticsStorage::connect(temp.path().join("analytics.db"))
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO log_tailer_checkpoints
         (file_path, site_id, inode, file_offset, last_processed_ts, updated_at)
         VALUES ('/log/maintenance.log', 1, 2, 300, 4, 5)",
    )
    .execute(storage.write_pool())
    .await
    .unwrap();

    let checkpoint = storage
        .get_log_checkpoint("/log/maintenance.log")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.site_id, 1);
    assert_eq!(checkpoint.file_offset, 300);
}
