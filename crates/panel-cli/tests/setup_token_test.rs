#![allow(clippy::unwrap_used, clippy::expect_used)]
use panel_cli::{SetupStatus, generate_setup_token, get_setup_status, verify_setup_token};
use panel_db::pool::{init_pool, run_migrations};
#[tokio::test]
async fn legacy_tokens_cannot_authorize_or_restart_public_setup() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO settings (key,value,updated_at) VALUES ('setup.first_run_token','legacy',0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        get_setup_status(&pool).await.unwrap(),
        SetupStatus::Uninitialized
    );
    assert!(generate_setup_token(&pool).await.is_err());
    assert!(!verify_setup_token(&pool, "legacy").await.unwrap());
}
