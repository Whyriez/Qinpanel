#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_db::{
    CreateSite, CreateSiteDeployment, CreateUser, SqliteGitDeployRepository, SqliteSiteRepository,
    SqliteUserRepository, UpdateSiteDeployment, UpsertSiteGitConfig, init_pool, run_migrations,
};

async fn setup_test_db() -> panel_db::DbPool {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.keep().join("test_panel_git_deploy.db");
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
async fn test_git_deploy_config_and_deployments_lifecycle() {
    let pool = setup_test_db().await;
    let user = create_test_user(&pool, "gitdev", 10002).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let git_repo = SqliteGitDeployRepository::new(pool.clone());

    let site = site_repo
        .create(&CreateSite {
            user_id: user.id,
            kind: "php".to_string(),
            docroot: "/var/www/vhosts/gitdev/app.test/current/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.2".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    // 1. Initial config should be None
    let initial_config = git_repo.get_config(site.id).await.expect("get config");
    assert!(initial_config.is_none());

    // 2. Upsert config
    let config = git_repo
        .upsert_config(&UpsertSiteGitConfig {
            site_id: site.id,
            repo_url: "git@github.com:nusa/example.git".to_string(),
            branch: "main".to_string(),
            deploy_key_pub: Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...".to_string()),
            deploy_key_priv_path: Some("/etc/panel/keys/site_1_deploy_key".to_string()),
            build_command: Some("composer install --no-dev".to_string()),
            post_deploy_hook: Some("php artisan migrate --force".to_string()),
            docroot_subpath: Some("public".to_string()),
            releases_to_keep: Some(5),
            webhook_secret: Some("whsec_secret_123456".to_string()),
            auto_deploy_enabled: Some(true),
            current_release_id: None,
        })
        .await
        .expect("upsert config");

    assert_eq!(config.site_id, site.id);
    assert_eq!(config.repo_url, "git@github.com:nusa/example.git");
    assert_eq!(config.branch, "main");
    assert_eq!(config.docroot_subpath, "public");
    assert!(config.auto_deploy_enabled);

    // 3. Create a pending deployment
    let now = 1775836800;
    let dep1 = git_repo
        .create_deployment(&CreateSiteDeployment {
            site_id: site.id,
            release_id: "20261010_100000_c0ffee1".to_string(),
            commit_hash: Some("c0ffee1122334455".to_string()),
            commit_message: Some("feat: initial commit".to_string()),
            commit_author: Some("Developer <dev@nusapanel.id>".to_string()),
            branch: "main".to_string(),
            trigger_type: "manual".to_string(),
            status: "running".to_string(),
            log_output: Some("Cloning repository...\n".to_string()),
            duration_ms: None,
            created_at: now,
            finished_at: None,
        })
        .await
        .expect("create deployment");

    assert_eq!(dep1.status, "running");
    assert_eq!(dep1.release_id, "20261010_100000_c0ffee1");

    // 4. Update deployment on success
    git_repo
        .update_deployment(
            dep1.id,
            &UpdateSiteDeployment {
                status: "success".to_string(),
                commit_hash: Some("c0ffee1122334455".to_string()),
                commit_message: Some("feat: initial commit".to_string()),
                commit_author: Some("Developer <dev@nusapanel.id>".to_string()),
                log_output: Some(
                    "Cloning repository...\nBuild completed successfully.\nSymlink swapped."
                        .to_string(),
                ),
                duration_ms: Some(4500),
                finished_at: Some(now + 5),
            },
        )
        .await
        .expect("update deployment");

    git_repo
        .update_current_release(site.id, "20261010_100000_c0ffee1")
        .await
        .expect("update current release");

    let updated_config = git_repo
        .get_config(site.id)
        .await
        .expect("get config")
        .expect("some");
    assert_eq!(
        updated_config.current_release_id.as_deref(),
        Some("20261010_100000_c0ffee1")
    );

    // 5. Create a second deployment (webhook triggered)
    let dep2 = git_repo
        .create_deployment(&CreateSiteDeployment {
            site_id: site.id,
            release_id: "20261010_120000_c0ffee2".to_string(),
            commit_hash: Some("c0ffee2233445566".to_string()),
            commit_message: Some("fix: patch issue".to_string()),
            commit_author: Some("GitHub Webhook".to_string()),
            branch: "main".to_string(),
            trigger_type: "webhook_github".to_string(),
            status: "success".to_string(),
            log_output: Some("Deployed via GitHub webhook.".to_string()),
            duration_ms: Some(3200),
            created_at: now + 7200,
            finished_at: Some(now + 7205),
        })
        .await
        .expect("create second deployment");

    // 6. List deployments
    let list = git_repo
        .list_deployments(site.id, 10, 0)
        .await
        .expect("list deployments");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, dep2.id); // newest first
    assert_eq!(list[1].id, dep1.id);

    // 7. List markers for analytics timeline
    let markers = git_repo
        .list_deploy_markers(site.id, now - 100, now + 10000)
        .await
        .expect("list markers");
    assert_eq!(markers.len(), 2);
    assert_eq!(markers[0].release_id, "20261010_100000_c0ffee1");
    assert_eq!(markers[1].release_id, "20261010_120000_c0ffee2");
}
