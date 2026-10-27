#![allow(clippy::unwrap_used, clippy::expect_used)]
use panel_cli::{bootstrap_admin, create_admin, reset_password};
use panel_core::auth::verify_password;
use panel_db::{
    pool::{DbPool, init_pool, run_migrations},
    repositories::SqliteUserRepository,
};
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

async fn fixture() -> (DbPool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("panel.db")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = init_pool(&format!("sqlite://{path}")).await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES ('setup.first_run_token','legacy-secret',0)")
        .execute(&pool).await.unwrap();
    (pool, dir)
}

#[tokio::test]
async fn concurrent_bootstrap_returns_exactly_one_credential_pair_and_no_plaintext_storage() {
    let (pool, _dir) = fixture().await;
    let (a, b) = tokio::join!(bootstrap_admin(&pool), bootstrap_admin(&pool));
    let mut results = [a.unwrap(), b.unwrap()];
    assert_eq!(results.iter().filter(|v| v.is_some()).count(), 1);
    let creds = results.iter_mut().find_map(Option::take).unwrap();
    let user = SqliteUserRepository::new(pool.clone())
        .find_by_username(&creds.username)
        .await
        .unwrap()
        .unwrap();
    assert!(user.password_hash.starts_with("$argon2id$"));
    assert!(verify_password(&creds.password, &user.password_hash).unwrap());
    assert!(user.linux_username.is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'setup.first_run%'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM settings WHERE value=?")
            .bind(&creds.password)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(bootstrap_admin(&pool).await.unwrap().is_none());
    assert_eq!(
        SqliteUserRepository::new(pool.clone())
            .find_by_id(user.id)
            .await
            .unwrap()
            .unwrap()
            .password_hash,
        user.password_hash
    );
}

#[tokio::test]
async fn bootstrap_preserves_existing_admin_even_when_suspended_and_removes_legacy_token() {
    let (pool, _dir) = fixture().await;
    let id = create_admin(&pool, "existing_admin", "KnownPassword123!", None)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='suspended' WHERE id=?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO settings (key,value,updated_at) VALUES ('setup.first_run_token','legacy',0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(bootstrap_admin(&pool).await.unwrap().is_none());
    let user = SqliteUserRepository::new(pool.clone())
        .find_by_id(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.status, "suspended");
    assert!(verify_password("KnownPassword123!", &user.password_hash).unwrap());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM settings WHERE key='setup.first_run_token'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn failed_bootstrap_is_atomic_and_can_be_retried() {
    for trigger in [
        "CREATE TRIGGER injected BEFORE INSERT ON users BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TRIGGER injected BEFORE DELETE ON settings BEGIN SELECT RAISE(ABORT,'injected'); END",
    ] {
        let (pool, _dir) = fixture().await;
        sqlx::query(trigger).execute(&pool).await.unwrap();
        assert!(bootstrap_admin(&pool).await.is_err());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT value FROM settings WHERE key='setup.first_run_token'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "legacy-secret"
        );
        sqlx::query("DROP TRIGGER injected")
            .execute(&pool)
            .await
            .unwrap();
        assert!(bootstrap_admin(&pool).await.unwrap().is_some());
    }
}

#[tokio::test]
async fn reset_rolls_back_on_revocation_failure_then_revokes_sessions_and_api_tokens() {
    let (pool, _dir) = fixture().await;
    let id = create_admin(&pool, "admin_reset", "KnownPassword123!", None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions (id,user_id,token_hash,expires_at,created_at) VALUES ('old',?,'old-token',9999999999,0)").bind(id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO api_tokens (user_id,name,token_hash,scopes_json,created_at) VALUES (?,'test','api-token','[]',0)").bind(id).execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER injected BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT,'injected'); END").execute(&pool).await.unwrap();
    assert!(
        reset_password(&pool, "admin_reset", "NewPassword456!")
            .await
            .is_err()
    );
    let user = SqliteUserRepository::new(pool.clone())
        .find_by_id(id)
        .await
        .unwrap()
        .unwrap();
    assert!(verify_password("KnownPassword123!", &user.password_hash).unwrap());
    sqlx::query("DROP TRIGGER injected")
        .execute(&pool)
        .await
        .unwrap();
    reset_password(&pool, "admin_reset", "NewPassword456!")
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM api_tokens")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[test]
fn cli_help_is_discoverable_without_opening_a_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("should-not-exist.db");
    for args in [
        vec!["--help"],
        vec!["admin", "--help"],
        vec!["admin", "bootstrap", "--help"],
        vec!["admin", "reset-password", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_panelctl"))
            .arg("--db-path")
            .arg(&db)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("--help"));
        assert!(!text.contains("P10-T"));
        assert!(!db.exists());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_panelctl"))
        .args(["admin", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("bootstrap") && text.contains("reset-password") && text.contains("reset2fa")
    );
}

#[test]
fn cli_json_bootstrap_is_idempotent_and_stdin_reset_has_no_secret_output() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("panel.db");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_panelctl"))
            .arg("--db-path")
            .arg(&db)
            .args(["admin", "bootstrap", "--json"])
            .output()
            .unwrap()
    };
    let first = run();
    assert!(first.status.success());
    let body: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(body["created"], true);
    let username = body["credentials"]["username"].as_str().unwrap();
    let second = run();
    assert!(second.status.success());
    let second: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(
        second,
        serde_json::json!({"created":false,"credentials":null})
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_panelctl"))
        .arg("--db-path")
        .arg(&db)
        .args([
            "admin",
            "reset-password",
            "--username",
            username,
            "--password-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"OperatorPassword123!\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("OperatorPassword123!")
    );
}
