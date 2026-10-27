#![allow(clippy::unwrap_used, clippy::expect_used)]
use panel_db::{
    models::{CreateJob, job::JobEvent},
    pool::{init_pool, run_migrations},
    repositories::SqliteJobRepository,
};

fn input(id: &str, engine: &str) -> CreateJob {
    CreateJob {
        id: id.into(),
        user_id: None,
        name: "database.server.install".into(),
        payload_json: serde_json::json!({"engine":engine}).to_string(),
        max_retries: 1,
    }
}

#[tokio::test]
async fn concurrent_install_requests_join_active_job_and_allow_retry_after_completion() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = SqliteJobRepository::new(pool);
    let first = input("one", "mariadb");
    let second = input("two", "mariadb");
    let (left, right) = tokio::join!(
        repo.create_or_find_active(&first),
        repo.create_or_find_active(&second)
    );
    let (left, left_duplicate) = left.unwrap();
    let (right, right_duplicate) = right.unwrap();
    assert_eq!(left.id, right.id);
    assert_ne!(left_duplicate, right_duplicate);
    let postgres = repo
        .create_or_find_active(&input("postgres", "postgresql"))
        .await
        .unwrap();
    assert!(!postgres.1);
    repo.update_status(&left.id, "succeeded", None, None)
        .await
        .unwrap();
    let retried = repo
        .create_or_find_active(&input("retry", "mariadb"))
        .await
        .unwrap();
    assert!(!retried.1);
    assert_ne!(retried.0.id, left.id);
}

#[tokio::test]
async fn history_keeps_equal_timestamp_lines_and_cancel_does_not_unlock_running_apt() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = SqliteJobRepository::new(pool);
    repo.create(&input("one", "mariadb")).await.unwrap();
    let job = repo.claim_next_queued().await.unwrap().unwrap();
    assert!(!repo.cancel(&job.id).await.unwrap());
    let event = JobEvent {
        sequence: 0,
        job_id: job.id.clone(),
        status: "running".into(),
        progress: 15,
        message: Some("Unpacking package...\nSetting up package...".into()),
        timestamp: 100,
    };
    let first = repo.append_event(&event).await.unwrap();
    let second = repo.append_event(&event).await.unwrap();
    assert!(second > first);
    let events = repo.events_after(&job.id, 0).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].message, event.message);
    assert_eq!(repo.events_after(&job.id, first).await.unwrap().len(), 1);
    repo.create(&input("queued", "postgresql")).await.unwrap();
    assert!(repo.cancel("queued").await.unwrap());
}
