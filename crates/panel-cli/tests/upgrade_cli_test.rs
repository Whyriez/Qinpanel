//! Integration tests for `panelctl upgrade` and `panelctl uninstall` commands.

use panel_cli::{UninstallCliOptions, UpgradeCliOptions, run_uninstall, run_upgrade};
use panel_db::pool::{init_pool, run_migrations};
use std::path::Path;
use tempfile::tempdir;

#[tokio::test]
async fn test_upgrade_dry_run_success() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("panel_test.db");
    let db_url = format!("sqlite://{}", db_path.to_str().unwrap());

    let pool = init_pool(&db_url).await.expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let opts = UpgradeCliOptions {
        binary_dir: None,
        skip_backup: false,
        restart_services: false,
        target_version: Some("1.1.0".to_string()),
        dry_run: true,
        json: true,
    };

    let res = run_upgrade(&opts, &pool, db_path.to_str().unwrap())
        .await
        .expect("run upgrade dry run");

    assert!(res.success);
    assert_eq!(res.to_version, "1.1.0");
    assert!(res.database_healthy);
    assert!(!res.rollback_performed);
    assert!(res.backup_created.is_none());
}

#[tokio::test]
async fn test_upgrade_with_safety_backup_success() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("panel_test.db");
    let db_url = format!("sqlite://{}", db_path.to_str().unwrap());

    let pool = init_pool(&db_url).await.expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let opts = UpgradeCliOptions {
        binary_dir: None,
        skip_backup: false,
        restart_services: false,
        target_version: Some("1.2.0".to_string()),
        dry_run: false,
        json: true,
    };

    let res = run_upgrade(&opts, &pool, db_path.to_str().unwrap())
        .await
        .expect("run upgrade");

    assert!(res.success);
    assert_eq!(res.to_version, "1.2.0");
    assert!(res.migrations_applied);
    assert!(res.database_healthy);
    assert!(!res.rollback_performed);
    assert!(res.backup_created.is_some());

    let backup_path = res.backup_created.unwrap();
    assert!(Path::new(&backup_path).exists());
}

#[tokio::test]
async fn test_uninstall_execution_with_keep_data() {
    let opts = UninstallCliOptions {
        purge: false,
        yes: true,
        json: true,
    };

    let res = run_uninstall(&opts).await.expect("run uninstall");

    assert!(res.success);
    assert!(!res.data_purged);
}
