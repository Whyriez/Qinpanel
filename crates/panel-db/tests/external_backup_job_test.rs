#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Observed root work must survive API restart without replay or cancellation.
use panel_db::{
    models::{CreateJob, ExternalJobSnapshot},
    pool::{init_pool, run_migrations},
    repositories::SqliteJobRepository,
};

#[tokio::test]
async fn external_backup_state_is_monotonic_and_never_executed_by_the_queue() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = SqliteJobRepository::new(pool);
    let mut observed = ExternalJobSnapshot {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: None,
        name: "server.backup.create".into(),
        status: "queued".into(),
        progress: 0,
        payload_json: r#"{"external":true,"operation":"create"}"#.into(),
        result_json: r#"{"message":"Queued","progress_known":true}"#.into(),
        error_message: None,
        created_at: 100,
        updated_at: 100,
    };
    assert!(
        repo.observe_server_backup(&observed)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        repo.observe_server_backup(&observed)
            .await
            .unwrap()
            .is_none()
    );
    assert!(repo.claim_next_queued().await.unwrap().is_none());
    assert!(!repo.cancel(&observed.id).await.unwrap());
    observed.status = "running".into();
    observed.progress = 40;
    observed.updated_at = 110;
    repo.observe_server_backup(&observed).await.unwrap();
    assert_eq!(repo.recover_interrupted_jobs().await.unwrap(), 0);
    assert!(!repo.cancel(&observed.id).await.unwrap());
    let mut stale = observed.clone();
    stale.status = "queued".into();
    stale.updated_at = 120;
    assert!(repo.observe_server_backup(&stale).await.unwrap().is_none());
    stale.status = "running".into();
    stale.progress = 10;
    assert!(repo.observe_server_backup(&stale).await.unwrap().is_none());
    observed.status = "succeeded".into();
    observed.progress = 100;
    observed.updated_at = 90;
    repo.observe_server_backup(&observed).await.unwrap();
    assert!(repo.observe_server_backup(&stale).await.unwrap().is_none());
    assert!(repo.active_server_backups().await.unwrap().is_empty());
    let saved = repo.find_by_id(&observed.id).await.unwrap().unwrap();
    assert_eq!(saved.status, "succeeded");
    assert_eq!(saved.progress, 100);
    assert_eq!(saved.updated_at, 110);
    observed.name = "server.backup.restore".into();
    assert!(repo.observe_server_backup(&observed).await.is_err());
    repo.create(&CreateJob {
        id: "ordinary".into(),
        user_id: None,
        name: "system.echo".into(),
        payload_json: "{}".into(),
        max_retries: 0,
    })
    .await
    .unwrap();
    assert_eq!(
        repo.claim_next_queued().await.unwrap().unwrap().id,
        "ordinary"
    );
    assert_eq!(repo.recover_interrupted_jobs().await.unwrap(), 1);
}

#[tokio::test]
async fn read_only_observations_filter_before_pagination_without_mutating_frozen_sqlite() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = SqliteJobRepository::new(pool);
    let mut value = ExternalJobSnapshot {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: None,
        name: "server.backup.create".into(),
        status: "queued".into(),
        progress: 0,
        payload_json: "{}".into(),
        result_json: "{}".into(),
        error_message: None,
        created_at: 100,
        updated_at: 100,
    };
    repo.observe_server_backup(&value).await.unwrap();
    value.status = "running".into();
    value.progress = 40;
    assert_eq!(
        repo.count_filtered_observing(None, Some("running"), Some(&value))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repo.count_filtered_observing(None, Some("queued"), Some(&value))
            .await
            .unwrap(),
        0
    );
    let rows = repo
        .list_paginated_observing(None, Some("running"), 1, 0, Some(&value))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, value.id);
    assert!(
        repo.list_paginated_observing(None, Some("running"), 1, 1, Some(&value))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.count_filtered_observing(Some(999), Some("running"), Some(&value))
            .await
            .unwrap(),
        0
    );
    let saved = repo.find_by_id(&value.id).await.unwrap().unwrap();
    assert_eq!(saved.status, "queued");
    assert_eq!(saved.progress, 0);
    value.name = "server.backup.restore".into();
    assert_eq!(
        repo.count_filtered_observing(None, Some("running"), Some(&value))
            .await
            .unwrap(),
        0
    );
}
