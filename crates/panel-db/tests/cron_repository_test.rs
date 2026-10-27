#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_db::{
    CreateCronJob, CreateCronRun, CreateSite, CreateUser, SqliteCronRepository,
    SqliteSiteRepository, SqliteUserRepository, UpdateCronJob, init_pool, run_migrations,
};

async fn setup_test_db() -> panel_db::DbPool {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.keep().join("test_panel_cron.db");
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
async fn test_cron_job_lifecycle_and_runs() {
    let pool = setup_test_db().await;
    let user = create_test_user(&pool, "alice", 10001).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let cron_repo = SqliteCronRepository::new(pool.clone());

    let site = site_repo
        .create(&CreateSite {
            user_id: user.id,
            kind: "php".to_string(),
            docroot: "/home/p_alice/sites/laravel.test/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    // 1. Create a cron job
    let job = cron_repo
        .create_job(&CreateCronJob {
            site_id: site.id,
            name: "Artisan Schedule".to_string(),
            command: "php artisan schedule:run".to_string(),
            schedule: "* * * * *".to_string(),
            systemd_calendar: "*-*-* *:*:00".to_string(),
            description: Some("Run scheduled Laravel tasks".to_string()),
            work_dir: Some("/home/p_alice/sites/laravel.test".to_string()),
            timeout_secs: Some(300),
            enabled: Some(true),
        })
        .await
        .expect("create cron job");

    assert_eq!(job.name, "Artisan Schedule");
    assert_eq!(job.site_id, site.id);
    assert_eq!(job.status, "idle");
    assert!(job.enabled);

    // 2. Count jobs for user (quota check)
    let user_job_count = cron_repo
        .count_jobs_for_user(user.id)
        .await
        .expect("count jobs");
    assert_eq!(user_job_count, 1);

    // 3. List jobs for site
    let jobs = cron_repo
        .list_jobs_for_site(site.id)
        .await
        .expect("list jobs");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, job.id);

    // 4. Update cron job
    let updated = cron_repo
        .update_job(
            job.id,
            &UpdateCronJob {
                name: Some("Artisan Schedule Updated".to_string()),
                schedule: Some("*/5 * * * *".to_string()),
                systemd_calendar: Some("*-*-* *:00/5:00".to_string()),
                status: Some("success".to_string()),
                last_run_at: Some(1728000100),
                last_duration_ms: Some(1250),
                last_exit_code: Some(0),
                ..Default::default()
            },
        )
        .await
        .expect("update cron job");

    assert_eq!(updated.name, "Artisan Schedule Updated");
    assert_eq!(updated.schedule, "*/5 * * * *");
    assert_eq!(updated.status, "success");
    assert_eq!(updated.last_exit_code, Some(0));

    // 5. Create execution run history
    let run = cron_repo
        .create_run(&CreateCronRun {
            cron_job_id: job.id,
            status: "success".to_string(),
            exit_code: Some(0),
            stdout: Some("Scheduled commands run successfully".to_string()),
            stderr: None,
            duration_ms: Some(1250),
            started_at: 1728000100,
            finished_at: Some(1728000101),
        })
        .await
        .expect("create run");

    assert_eq!(run.cron_job_id, job.id);
    assert_eq!(run.status, "success");

    let runs = cron_repo
        .list_runs_for_job(job.id, 10)
        .await
        .expect("list runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, run.id);

    // 6. Delete job
    let deleted = cron_repo.delete_job(job.id).await.expect("delete job");
    assert!(deleted);

    let empty_jobs = cron_repo
        .list_jobs_for_site(site.id)
        .await
        .expect("list jobs after delete");
    assert!(empty_jobs.is_empty());
}
