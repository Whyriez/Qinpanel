#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_cli::{SetupStatus, create_admin, get_setup_status, list_admins, reset_password};
use panel_core::auth::verify_password;
use panel_db::pool::{init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_admin_create_and_verify() {
    let tmp = NamedTempFile::new().expect("create temp file");
    let db_path = tmp.path().to_str().expect("to_str");
    let pool = init_pool(&format!("sqlite://{}", db_path.replace('\\', "/")))
        .await
        .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let user_id = create_admin(
        &pool,
        "superadmin",
        "Password123!",
        Some("admin@example.com".to_string()),
    )
    .await
    .expect("create admin");
    assert!(user_id > 0);

    let user_repo = SqliteUserRepository::new(pool.clone());
    let user = user_repo
        .find_by_username("superadmin")
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(user.username, "superadmin");
    assert_eq!(user.role, "admin");
    assert_eq!(user.email.as_deref(), Some("admin@example.com"));

    let valid = verify_password("Password123!", &user.password_hash).expect("verify");
    assert!(valid);
}

#[tokio::test]
async fn test_admin_duplicate_username_fails() {
    let tmp = NamedTempFile::new().expect("create temp file");
    let db_path = tmp.path().to_str().expect("to_str");
    let pool = init_pool(&format!("sqlite://{}", db_path.replace('\\', "/")))
        .await
        .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    create_admin(&pool, "admin1", "Password123!", None)
        .await
        .expect("create admin");

    let err = create_admin(&pool, "admin1", "AnotherPassword123!", None).await;
    assert!(err.is_err());
}

#[tokio::test]
async fn test_admin_short_password_fails() {
    let tmp = NamedTempFile::new().expect("create temp file");
    let db_path = tmp.path().to_str().expect("to_str");
    let pool = init_pool(&format!("sqlite://{}", db_path.replace('\\', "/")))
        .await
        .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let err = create_admin(&pool, "admin2", "short", None).await;
    assert!(err.is_err());
}

#[tokio::test]
async fn test_admin_invalid_username_fails() {
    let tmp = NamedTempFile::new().expect("create temp file");
    let db_path = tmp.path().to_str().expect("to_str");
    let pool = init_pool(&format!("sqlite://{}", db_path.replace('\\', "/")))
        .await
        .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    // Invalid characters (spaces, special chars)
    let err1 = create_admin(&pool, "admin user", "Password123!", None).await;
    assert!(err1.is_err());

    // Too short (< 3 chars)
    let err2 = create_admin(&pool, "ad", "Password123!", None).await;
    assert!(err2.is_err());
}

#[tokio::test]
async fn test_admin_reset_password() {
    let tmp = NamedTempFile::new().expect("create temp file");
    let db_path = tmp.path().to_str().expect("to_str");
    let pool = init_pool(&format!("sqlite://{}", db_path.replace('\\', "/")))
        .await
        .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    create_admin(&pool, "admin_reset", "OldPassword123!", None)
        .await
        .expect("create admin");

    reset_password(&pool, "admin_reset", "NewPassword456!")
        .await
        .expect("reset password");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let user = user_repo
        .find_by_username("admin_reset")
        .await
        .expect("find")
        .expect("exists");

    assert!(!verify_password("OldPassword123!", &user.password_hash).expect("verify"));
    assert!(verify_password("NewPassword456!", &user.password_hash).expect("verify"));
}

#[tokio::test]
async fn test_admin_list() {
    let tmp = NamedTempFile::new().expect("create temp file");
    let db_path = tmp.path().to_str().expect("to_str");
    let pool = init_pool(&format!("sqlite://{}", db_path.replace('\\', "/")))
        .await
        .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    create_admin(&pool, "admin_one", "Password123!", None)
        .await
        .expect("create 1");
    create_admin(&pool, "admin_two", "Password123!", None)
        .await
        .expect("create 2");

    let admins = list_admins(&pool).await.expect("list admins");
    assert_eq!(admins.len(), 2);
}

#[tokio::test]
async fn test_creating_admin_clears_setup_token() {
    let tmp = NamedTempFile::new().expect("create temp file");
    let db_path = tmp.path().to_str().expect("to_str");
    let pool = init_pool(&format!("sqlite://{}", db_path.replace('\\', "/")))
        .await
        .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    // Simulate an unfinished legacy installation before the tokenless upgrade.
    sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES ('setup.first_run_token','legacy-token',0)")
        .execute(&pool).await.unwrap();
    // Create admin
    create_admin(&pool, "admin_final", "Password123!", None)
        .await
        .expect("create admin");

    // Status should now be Completed
    let status_after = get_setup_status(&pool).await.expect("status after");
    assert_eq!(status_after, SetupStatus::Completed);
}

#[tokio::test]
async fn test_admin_reset_2fa() {
    use panel_cli::reset_2fa;
    use panel_db::repositories::SqliteRecoveryCodeRepository;

    let tmp = NamedTempFile::new().expect("create temp file");
    let db_path = tmp.path().to_str().expect("to_str");
    let pool = init_pool(&format!("sqlite://{}", db_path.replace('\\', "/")))
        .await
        .expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let user_id = create_admin(&pool, "admin_2fa", "Password123!", None)
        .await
        .expect("create admin");

    let user_repo = SqliteUserRepository::new(pool.clone());
    let recovery_repo = SqliteRecoveryCodeRepository::new(pool.clone());

    // Enable 2FA manually
    user_repo
        .update_totp(user_id, Some("JBSWY3DPEHPK3PXP"), true)
        .await
        .expect("update totp");

    let codes = vec!["hash1".to_string(), "hash2".to_string()];
    recovery_repo
        .replace_codes(user_id, &codes)
        .await
        .expect("replace codes");

    let unused_before = recovery_repo.count_unused(user_id).await.expect("count");
    assert_eq!(unused_before, 2);

    // Call CLI reset_2fa
    reset_2fa(&pool, "admin_2fa").await.expect("reset 2fa");

    // Verify 2FA is now disabled
    let user_after = user_repo
        .find_by_username("admin_2fa")
        .await
        .expect("find")
        .expect("exists");
    assert!(!user_after.totp_enabled);
    assert_eq!(user_after.totp_secret, None);

    let unused_after = recovery_repo.count_unused(user_id).await.expect("count");
    assert_eq!(unused_after, 0);
}
