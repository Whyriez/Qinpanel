#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_db::{
    CreateBackupInput, CreateSite, CreateUser, SqliteBackupRepository, SqliteSiteRepository,
    SqliteUserRepository, UpsertBackupScheduleInput, init_pool, run_migrations,
};

async fn setup_test_db() -> panel_db::DbPool {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.keep().join("test_panel_backups.db");
    let db_url = format!("sqlite://{}", db_path.to_str().expect("path to str"));

    let pool = init_pool(&db_url).await.expect("init pool");
    run_migrations(&pool).await.expect("run migrations");
    pool
}

async fn create_test_user(pool: &panel_db::DbPool, username: &str, uid: i64) -> panel_db::User {
    let user_repo = SqliteUserRepository::new(pool.clone());
    let input = CreateUser {
        username: username.to_string(),
        password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$test".to_string(),
        role: "user".to_string(),
        email: Some(format!("{}@example.com", username)),
        linux_username: Some(format!("p_{}", username)),
        linux_uid: Some(uid),
        plan_id: Some(1),
    };
    user_repo.create(&input).await.expect("create test user")
}

#[tokio::test]
async fn test_backup_and_schedule_lifecycle() {
    let pool = setup_test_db().await;
    let user = create_test_user(&pool, "backuptenant", 10006).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let backup_repo = SqliteBackupRepository::new(pool.clone());

    let site = site_repo
        .create(&CreateSite {
            user_id: user.id,
            kind: "php".to_string(),
            docroot: "/home/p_backuptenant/sites/backup.test/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    // 1. Initially no backups
    let backups = backup_repo
        .list_backups_by_site(site.id)
        .await
        .expect("list backups");
    assert!(backups.is_empty());
    assert_eq!(
        backup_repo
            .get_total_backup_bytes_by_user(user.id)
            .await
            .expect("total user bytes"),
        0
    );

    // 2. Create backup
    let created = backup_repo
        .create_backup(&CreateBackupInput {
            site_id: site.id,
            user_id: user.id,
            filename: "backup_backup_test_1700000000.tar.zst".to_string(),
            filepath: "/home/p_backuptenant/backups/backup_backup_test_1700000000.tar.zst"
                .to_string(),
            size_bytes: 1048576,
            label: Some("Manual snapshot".to_string()),
            includes_files: true,
            includes_databases: true,
            status: "pending".to_string(),
            storage_type: "local".to_string(),
            s3_key: None,
            checksum_sha256: None,
        })
        .await
        .expect("create backup");

    assert_eq!(created.site_id, site.id);
    assert_eq!(created.status, "pending");

    // 3. Update status to completed
    backup_repo
        .update_backup_status(
            created.id,
            "completed",
            Some(2097152),
            Some("sha256hash12345"),
            None,
        )
        .await
        .expect("update status");

    let found = backup_repo
        .find_backup_by_id(created.id)
        .await
        .expect("find backup")
        .unwrap();
    assert_eq!(found.status, "completed");
    assert_eq!(found.size_bytes, 2097152);
    assert_eq!(found.checksum_sha256.as_deref(), Some("sha256hash12345"));

    // 4. Check quota calculations
    let total_user = backup_repo
        .get_total_backup_bytes_by_user(user.id)
        .await
        .expect("total user bytes");
    assert_eq!(total_user, 2097152);

    let total_site = backup_repo
        .get_total_backup_bytes_by_site(site.id)
        .await
        .expect("total site bytes");
    assert_eq!(total_site, 2097152);

    // 5. Upsert automated backup schedule
    let sched = backup_repo
        .upsert_schedule(&UpsertBackupScheduleInput {
            site_id: site.id,
            is_enabled: true,
            frequency: "daily".to_string(),
            time_hour: 3,
            time_minute: 15,
            retention_count: 5,
            include_files: true,
            include_databases: true,
            storage_destination: "local".to_string(),
            s3_endpoint: None,
            s3_bucket: None,
            s3_region: None,
            s3_access_key: None,
            s3_secret_key_enc: None,
            s3_prefix: None,
        })
        .await
        .expect("upsert schedule");

    assert!(sched.is_enabled);
    assert_eq!(sched.frequency, "daily");
    assert_eq!(sched.time_hour, 3);
    assert_eq!(sched.retention_count, 5);

    // 6. Prune test
    // Create 2 more backups to test retention
    for i in 1..=3 {
        let b = backup_repo
            .create_backup(&CreateBackupInput {
                site_id: site.id,
                user_id: user.id,
                filename: format!("backup_{}.tar.zst", i),
                filepath: format!("/path/backup_{}.tar.zst", i),
                size_bytes: 1000,
                label: None,
                includes_files: true,
                includes_databases: false,
                status: "completed".to_string(),
                storage_type: "local".to_string(),
                s3_key: None,
                checksum_sha256: None,
            })
            .await
            .expect("create extra backup");
        let _ = b;
    }

    // Now we have 4 completed backups. Prune with retention = 2
    let pruned = backup_repo
        .prune_old_backups(site.id, 2)
        .await
        .expect("prune backups");
    assert_eq!(pruned.len(), 2);

    let remaining = backup_repo
        .list_backups_by_site(site.id)
        .await
        .expect("remaining backups");
    assert_eq!(remaining.len(), 2);

    // 7. Delete backup
    let del = backup_repo
        .delete_backup(remaining[0].id)
        .await
        .expect("delete");
    assert!(del);
}
