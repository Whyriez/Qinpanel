#![allow(clippy::unwrap_used, clippy::expect_used)]
//! SQLite-only retry state regression; no API or agent daemon.

use panel_db::{
    models::CreateJob,
    pool::{init_pool, run_migrations},
    repositories::SqliteJobRepository,
};

#[tokio::test]
async fn claiming_retry_clears_previous_error_but_final_failure_keeps_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .join("jobs.db")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = init_pool(&format!("sqlite://{path}")).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = SqliteJobRepository::new(pool);
    repo.create(&CreateJob {
        id: "node-install-retry".into(),
        user_id: None,
        name: "node.install".into(),
        payload_json: "{}".into(),
        max_retries: 2,
    })
    .await
    .unwrap();
    let first = repo.claim_next_queued().await.unwrap().unwrap();
    assert_eq!(first.status, "running");
    repo.update_progress(&first.id, 15).await.unwrap();
    let queued = repo
        .increment_retry_or_fail(&first.id, "First extraction failed")
        .await
        .unwrap();
    assert_eq!(queued.status, "queued");
    assert_eq!(
        queued.error_message.as_deref(),
        Some("First extraction failed")
    );

    let retry = repo.claim_next_queued().await.unwrap().unwrap();
    assert_eq!(retry.status, "running");
    assert_eq!(retry.retry_count, 1);
    assert!(
        retry.error_message.is_none(),
        "a running retry must not expose the previous attempt as its current error"
    );
    assert!(
        repo.find_by_id(&retry.id)
            .await
            .unwrap()
            .unwrap()
            .error_message
            .is_none()
    );

    let failed = repo
        .increment_retry_or_fail(&retry.id, "Final extraction failed")
        .await
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.retry_count, 2);
    assert_eq!(
        failed.error_message.as_deref(),
        Some("Final extraction failed")
    );
    assert!(repo.claim_next_queued().await.unwrap().is_none());
}
