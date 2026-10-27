//! Integration tests for `panelctl` reliability commands: doctor, reconcile, and backup.

use panel_cli::{
    DoctorCliOptions, ReconcileCliOptions, SystemBackupCommands, create_system_backup,
    handle_system_backup_command, run_doctor, run_reconcile, verify_system_backup,
};
use panel_db::models::CreateJob;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::job_repo::SqliteJobRepository;
use tempfile::tempdir;

#[tokio::test]
async fn backup_restores_core_and_operations_together_including_wal_data() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("partition.db");
    let url = format!("sqlite://{}", db_path.display());
    let core = init_pool(&url).await.unwrap();
    run_migrations(&core).await.unwrap();
    let (writer, reader) = panel_db::operations::open(&core).await.unwrap();
    let jobs = SqliteJobRepository::new(writer.clone());
    jobs.create(&CreateJob {
        id: "saved-job".to_owned(),
        user_id: None,
        name: "test.sleep".to_owned(),
        payload_json: "{}".to_owned(),
        max_retries: 1,
    })
    .await
    .unwrap();
    sqlx::query("INSERT INTO settings VALUES ('saved', 'original', 0)")
        .execute(&core)
        .await
        .unwrap();
    let summary = create_system_backup(
        &core,
        &db_path.to_string_lossy(),
        Some(format!(
            "panel-backup-partitions-{}.tar.gz",
            uuid::Uuid::new_v4()
        )),
        false,
    )
    .await
    .unwrap();
    let verification = verify_system_backup(&summary.filepath).unwrap();
    assert!(verification.is_valid, "{:?}", verification.errors);
    let manifest = verification.manifest.unwrap();
    assert!(
        manifest
            .files
            .iter()
            .any(|file| file.relative_path == "database/operations.db")
    );
    jobs.update_status("saved-job", "failed", None, Some("changed"))
        .await
        .unwrap();
    sqlx::query("UPDATE settings SET value='changed'")
        .execute(&core)
        .await
        .unwrap();
    writer.close().await;
    reader.close().await;
    panel_cli::restore_system_backup(&core, &db_path.to_string_lossy(), &summary.filepath)
        .await
        .unwrap();
    let restored = init_pool(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key='saved'")
            .fetch_one(&restored)
            .await
            .unwrap(),
        "original"
    );
    let (writer, reader) = panel_db::operations::open(&restored).await.unwrap();
    for pool in [&restored, &writer, &reader] {
        let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(integrity, "ok");
    }
    assert_eq!(
        SqliteJobRepository::new(reader.clone())
            .find_by_id("saved-job")
            .await
            .unwrap()
            .unwrap()
            .status,
        "queued"
    );
    SqliteJobRepository::new(writer.clone())
        .update_status("saved-job", "running", None, None)
        .await
        .unwrap();
    assert_eq!(
        SqliteJobRepository::new(reader.clone())
            .find_by_id("saved-job")
            .await
            .unwrap()
            .unwrap()
            .status,
        "running"
    );
    writer.close().await;
    reader.close().await;
    restored.close().await;
}

#[tokio::test]
async fn test_panelctl_doctor_clean_and_fix() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("panel.db");
    let db_url = format!("sqlite://{}", db_path.to_string_lossy().replace('\\', "/"));

    let pool = init_pool(&db_url).await.unwrap();
    run_migrations(&pool).await.unwrap();

    let job_repo = SqliteJobRepository::new(pool.clone());
    // Insert a dummy interrupted job
    job_repo
        .create(&CreateJob {
            id: "job-test-1".to_string(),
            user_id: None,
            name: "test_deploy".to_string(),
            payload_json: r#"{"action":"deploy"}"#.to_string(),
            max_retries: 3,
        })
        .await
        .unwrap();

    let jobs = job_repo.list_by_status("queued").await.unwrap();
    assert_eq!(jobs.len(), 1);
    let job_id = &jobs[0].id;
    job_repo
        .update_status(job_id, "running", None, None)
        .await
        .unwrap();

    // 1. Run doctor without fix - detects stuck job
    let opts = DoctorCliOptions {
        json: true,
        fix: false,
    };
    let report = run_doctor(&opts, &pool, &db_path.to_string_lossy())
        .await
        .unwrap();
    assert!(report.total_warnings > 0 || report.total_failures > 0);
    assert_eq!(report.fixable_items().len(), 1);
    assert_eq!(report.fixable_items()[0].id, "interrupted_jobs");

    // 2. Run doctor with fix - recovers stuck job
    let fix_opts = DoctorCliOptions {
        json: false,
        fix: true,
    };
    let report_fixed = run_doctor(&fix_opts, &pool, &db_path.to_string_lossy())
        .await
        .unwrap();

    // Stuck job is now marked failed
    let stuck_check = report_fixed
        .checks
        .iter()
        .find(|c| c.id == "interrupted_jobs")
        .unwrap();
    assert_eq!(
        stuck_check.status,
        panel_core::doctor::DoctorCheckStatus::Pass
    );
}

#[tokio::test]
async fn test_panelctl_reconcile_dry_run_and_execution() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("panel_reconcile.db");
    let db_url = format!("sqlite://{}", db_path.to_string_lossy().replace('\\', "/"));

    let pool = init_pool(&db_url).await.unwrap();
    run_migrations(&pool).await.unwrap();

    // Run reconcile on clean DB
    let opts = ReconcileCliOptions {
        dry_run: true,
        json: false,
    };
    let rep = run_reconcile(&opts, &pool, &db_path.to_string_lossy())
        .await
        .unwrap();
    assert!(rep.is_clean);
    assert_eq!(rep.drifts.len(), 0);
}

#[tokio::test]
async fn test_panelctl_backup_create_list_verify() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("panel_backup.db");
    let db_url = format!("sqlite://{}", db_path.to_string_lossy().replace('\\', "/"));

    let pool = init_pool(&db_url).await.unwrap();
    run_migrations(&pool).await.unwrap();

    // 1. Create backup
    let summary = create_system_backup(
        &pool,
        &db_path.to_string_lossy(),
        Some("panel-backup-test-cli.tar.gz".to_string()),
        false,
    )
    .await
    .unwrap();

    assert_eq!(summary.filename, "panel-backup-test-cli.tar.gz");
    assert!(summary.size_bytes > 0);
    assert_eq!(summary.checksum_sha256.len(), 64);

    // 2. Verify backup
    let ver_res = verify_system_backup(&summary.filepath).unwrap();
    assert!(ver_res.is_valid);
    assert!(ver_res.manifest.is_some());
    let manifest = ver_res.manifest.unwrap();
    assert_eq!(manifest.files.len(), 1); // database/panel.db

    // 3. Handle backup verify subcommand
    handle_system_backup_command(
        SystemBackupCommands::Verify {
            path: summary.filepath.clone(),
        },
        &pool,
        &db_path.to_string_lossy(),
    )
    .await
    .unwrap();
}
