#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_db::{
    CreateNotificationInput, CreateSite, CreateUser, NotificationFilter,
    SqliteNotificationRepository, SqliteSiteRepository, SqliteUserRepository,
    UpdateNotificationSettingsInput, init_pool, run_migrations,
};

async fn setup_test_db() -> panel_db::DbPool {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.keep().join("test_panel_notifications.db");
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
async fn historical_brand_migration_updates_defaults_and_preserves_customer_settings() {
    let pool = setup_test_db().await;
    let repo = SqliteNotificationRepository::new(pool.clone());
    assert_eq!(repo.get_settings().await.unwrap().from_name, "Qinpanel");
    for (before, after) in [
        ("Nusa Panel", "Qpanel"),
        ("Nusa Panel Alerts", "Qpanel Alerts"),
        ("Customer Hosting", "Customer Hosting"),
    ] {
        sqlx::query("UPDATE notification_settings SET from_name = ?, smtp_host = 'smtp.customer.test', smtp_enabled = 1 WHERE id = 1")
            .bind(before).execute(&pool).await.unwrap();
        sqlx::raw_sql(include_str!(
            "../migrations/20261016000001_qpanel_branding.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let saved = repo.get_settings().await.unwrap();
        assert_eq!(saved.from_name, after);
        assert_eq!(saved.smtp_host.as_deref(), Some("smtp.customer.test"));
        assert!(saved.smtp_enabled);
    }
}

#[tokio::test]
async fn test_notification_crud_and_filtering() {
    let pool = setup_test_db().await;
    let user = create_test_user(&pool, "notifuser", 10007).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let notif_repo = SqliteNotificationRepository::new(pool.clone());

    let site = site_repo
        .create(&CreateSite {
            user_id: user.id,
            kind: "php".to_string(),
            docroot: "/home/p_notifuser/sites/notif.test/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    // 1. Initially no notifications
    let (items, total) = notif_repo
        .list(Some(user.id), &NotificationFilter::default())
        .await
        .expect("list");
    assert_eq!(total, 0);
    assert!(items.is_empty());
    assert_eq!(
        notif_repo
            .unread_count(Some(user.id))
            .await
            .expect("unread"),
        0
    );

    // 2. Create notification 1: SSL expiring
    let notif1 = notif_repo
        .create(&CreateNotificationInput {
            id: None,
            user_id: Some(user.id),
            site_id: Some(site.id),
            category: "ssl_expiring".to_string(),
            severity: "warning".to_string(),
            title: "Sertifikat SSL hampir kedaluwarsa".to_string(),
            message: "Sertifikat untuk notif.test tersisa 5 hari".to_string(),
            metadata_json: Some(r#"{"days_left": 5}"#.to_string()),
            action_url: Some(format!("/sites/{}?tab=ssl", site.id)),
        })
        .await
        .expect("create notif1");
    assert!(!notif1.id.is_empty());
    assert!(!notif1.is_read);

    // 3. Create notification 2: Quota warning
    let notif2 = notif_repo
        .create(&CreateNotificationInput {
            id: None,
            user_id: Some(user.id),
            site_id: None,
            category: "quota_warning".to_string(),
            severity: "error".to_string(),
            title: "Penggunaan kuota disk melebihi 90%".to_string(),
            message: "Anda telah menggunakan 9.2 GB dari batas 10 GB".to_string(),
            metadata_json: Some(r#"{"used_pct": 92}"#.to_string()),
            action_url: Some("/settings/quota".to_string()),
        })
        .await
        .expect("create notif2");

    // 4. Create notification 3: Global System broadcast (user_id = None)
    let notif3 = notif_repo
        .create(&CreateNotificationInput {
            id: None,
            user_id: None,
            site_id: None,
            category: "system".to_string(),
            severity: "info".to_string(),
            title: "Pemeliharaan terjadwal server".to_string(),
            message: "Server akan menjalani pembaruan rutin malam ini pukul 02:00 WIB".to_string(),
            metadata_json: None,
            action_url: None,
        })
        .await
        .expect("create notif3");

    // 5. User sees their own notifications AND global notifications
    let (user_items, user_total) = notif_repo
        .list(Some(user.id), &NotificationFilter::default())
        .await
        .expect("list for user");
    assert_eq!(user_total, 3);
    assert_eq!(user_items.len(), 3);
    assert_eq!(
        notif_repo
            .unread_count(Some(user.id))
            .await
            .expect("unread"),
        3
    );

    // 6. Test filtering: category = quota_warning
    let filter_quota = NotificationFilter {
        category: Some("quota_warning".to_string()),
        ..Default::default()
    };
    let (quota_items, quota_total) = notif_repo
        .list(Some(user.id), &filter_quota)
        .await
        .expect("list filter quota");
    assert_eq!(quota_total, 1);
    assert_eq!(quota_items[0].id, notif2.id);

    // 7. Mark notif1 as read
    let marked = notif_repo
        .mark_as_read(&notif1.id, Some(user.id))
        .await
        .expect("mark read");
    assert!(marked);

    let found1 = notif_repo
        .find_by_id(&notif1.id)
        .await
        .expect("find by id")
        .expect("must exist");
    assert!(found1.is_read);
    assert!(found1.read_at.is_some());

    assert_eq!(
        notif_repo
            .unread_count(Some(user.id))
            .await
            .expect("unread"),
        2
    );

    // 8. Filter unread only
    let filter_unread = NotificationFilter {
        unread_only: true,
        ..Default::default()
    };
    let (unread_items, unread_total) = notif_repo
        .list(Some(user.id), &filter_unread)
        .await
        .expect("list unread");
    assert_eq!(unread_total, 2);
    assert_eq!(unread_items.len(), 2);

    // 9. Mark all read
    let marked_all = notif_repo
        .mark_all_read(Some(user.id))
        .await
        .expect("mark all read");
    assert_eq!(marked_all, 2);
    assert_eq!(
        notif_repo
            .unread_count(Some(user.id))
            .await
            .expect("unread"),
        0
    );

    // 10. Delete single notification
    let deleted = notif_repo
        .delete(&notif3.id, Some(user.id))
        .await
        .expect("delete");
    assert!(deleted);
    let (remaining, rem_total) = notif_repo
        .list(Some(user.id), &NotificationFilter::default())
        .await
        .expect("list remaining");
    assert_eq!(rem_total, 2);
    assert_eq!(remaining.len(), 2);

    // 11. Clear all read notifications
    let cleared = notif_repo
        .clear_all(Some(user.id), true)
        .await
        .expect("clear all read");
    assert_eq!(cleared, 2);

    let (empty_items, empty_total) = notif_repo
        .list(Some(user.id), &NotificationFilter::default())
        .await
        .expect("list empty");
    assert_eq!(empty_total, 0);
    assert!(empty_items.is_empty());
}

#[tokio::test]
async fn test_notification_settings() {
    let pool = setup_test_db().await;
    let notif_repo = SqliteNotificationRepository::new(pool.clone());

    // 1. Initial settings
    let initial = notif_repo
        .get_settings()
        .await
        .expect("get initial settings");
    assert_eq!(initial.id, 1);
    assert!(!initial.smtp_enabled);
    assert_eq!(initial.smtp_port, 587);
    assert_eq!(initial.from_address, "noreply@panel.local");
    assert_eq!(initial.ssl_warning_days, 30);
    assert_eq!(initial.quota_warning_percent, 90);

    // 2. Update settings
    let updated = notif_repo
        .update_settings(&UpdateNotificationSettingsInput {
            smtp_enabled: Some(true),
            smtp_host: Some("smtp.example.com".to_string()),
            smtp_port: Some(465),
            smtp_username: Some("panel_mailer".to_string()),
            smtp_password_enc: Some("secret123".to_string()),
            smtp_encryption: Some("tls".to_string()),
            from_address: Some("alerts@myserver.com".to_string()),
            from_name: Some("Production Panel".to_string()),
            admin_email: Some("ops@company.org".to_string()),
            ssl_warning_days: Some(14),
            quota_warning_percent: Some(85),
            notify_ssl_expiring: Some(true),
            notify_quota_warning: Some(true),
            notify_job_failed: Some(true),
            notify_deploy_failed: Some(true),
        })
        .await
        .expect("update settings");

    assert!(updated.smtp_enabled);
    assert_eq!(updated.smtp_host, Some("smtp.example.com".to_string()));
    assert_eq!(updated.smtp_port, 465);
    assert_eq!(updated.smtp_username, Some("panel_mailer".to_string()));
    assert_eq!(updated.from_address, "alerts@myserver.com");
    assert_eq!(updated.ssl_warning_days, 14);
    assert_eq!(updated.quota_warning_percent, 85);

    // 3. Verify persistence
    let fetched = notif_repo.get_settings().await.expect("re-fetch settings");
    assert_eq!(fetched.smtp_host, Some("smtp.example.com".to_string()));
    assert_eq!(fetched.admin_email, Some("ops@company.org".to_string()));
}

#[tokio::test]
async fn qinpanel_branding_migrates_defaults_idempotently_and_preserves_custom_data() {
    let pool = setup_test_db().await;
    let repo = SqliteNotificationRepository::new(pool.clone());
    for (before, after) in [
        ("QonPanel", "Qinpanel"),
        ("Qonpanel", "Qinpanel"),
        ("Qpanel", "Qinpanel"),
        ("Nusa Panel", "Qinpanel"),
        ("Vpanel", "Qinpanel"),
        ("Qinpanel", "Qinpanel"),
        ("QonPanel Alerts", "Qinpanel Alerts"),
        ("Qonpanel Alerts", "Qinpanel Alerts"),
        ("Qpanel Alerts", "Qinpanel Alerts"),
        ("Nusa Panel Alerts", "Qinpanel Alerts"),
        ("Vpanel Alerts", "Qinpanel Alerts"),
        ("Qinpanel Alerts", "Qinpanel Alerts"),
        ("Customer Hosting", "Customer Hosting"),
    ] {
        sqlx::query("UPDATE notification_settings SET from_name = ?, smtp_host = 'smtp.customer.test', smtp_password_enc = 'existing-encrypted-secret', smtp_enabled = 1 WHERE id = 1")
            .bind(before).execute(&pool).await.unwrap();
        for _ in 0..2 {
            sqlx::raw_sql(include_str!(
                "../migrations/20261026000003_qinpanel_branding.sql"
            ))
            .execute(&pool)
            .await
            .unwrap();
            let saved = repo.get_settings().await.unwrap();
            assert_eq!(saved.from_name, after);
            assert_eq!(saved.smtp_host.as_deref(), Some("smtp.customer.test"));
            assert_eq!(
                saved.smtp_password_enc.as_deref(),
                Some("existing-encrypted-secret")
            );
            assert!(saved.smtp_enabled);
        }
    }
    for (before, after) in [
        ("QonPanel", "Qinpanel"),
        ("Qonpanel", "Qinpanel"),
        ("Qpanel", "Qinpanel"),
        ("Nusa Panel", "Qinpanel"),
        ("Vpanel", "Qinpanel"),
        ("Qinpanel", "Qinpanel"),
        ("Customer console", "Customer console"),
    ] {
        sqlx::query(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('panel.title', ?, 1)",
        )
        .bind(before)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('custom.title', 'Qpanel', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../migrations/20261026000003_qinpanel_branding.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let title: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'panel.title'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(title, after);
        let custom: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'custom.title'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(custom, "Qpanel");
    }
}

#[tokio::test]
async fn qinpanel_upgrade_from_previous_schema_preserves_existing_tenant_and_authentication() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    let mut legacy = sqlx::migrate!("./migrations");
    legacy.migrations = std::borrow::Cow::Owned(
        legacy
            .iter()
            .filter(|m| m.version < 20261024000001)
            .cloned()
            .collect(),
    );
    legacy.run(&pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO users(id,username,password_hash,role,totp_secret,created_at,updated_at) VALUES (10,'legacy','existing-password-hash','user','existing-totp-secret',1,1);
        INSERT INTO sites(id,user_id,kind,docroot,settings_json,created_at,updated_at) VALUES (10,10,'static','/home/p_legacy/sites/existing.test/public','{\"customer\":true}',1,1);
        INSERT INTO domains(site_id,name,unicode_name,is_primary,created_at,updated_at) VALUES (10,'existing.test','existing.test',1,1,1);
        UPDATE notification_settings SET from_name='Qpanel', smtp_password_enc='existing-smtp-secret' WHERE id=1;").execute(&pool).await.unwrap();
    for _ in 0..2 {
        run_migrations(&pool).await.unwrap();
    }
    let auth: (String, String, String) =
        sqlx::query_as("SELECT username, password_hash, totp_secret FROM users WHERE id=10")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        auth,
        (
            "legacy".into(),
            "existing-password-hash".into(),
            "existing-totp-secret".into()
        )
    );
    let site: (i64, String, String) =
        sqlx::query_as("SELECT user_id, docroot, settings_json FROM sites WHERE id=10")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        site,
        (
            10,
            "/home/p_legacy/sites/existing.test/public".into(),
            "{\"customer\":true}".into()
        )
    );
    let domain: String = sqlx::query_scalar("SELECT name FROM domains WHERE site_id=10")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(domain, "existing.test");
    let settings = SqliteNotificationRepository::new(pool)
        .get_settings()
        .await
        .unwrap();
    assert_eq!(settings.from_name, "Qinpanel");
    assert_eq!(
        settings.smtp_password_enc.as_deref(),
        Some("existing-smtp-secret")
    );
}
