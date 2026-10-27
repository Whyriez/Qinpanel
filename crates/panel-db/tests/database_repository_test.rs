#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_db::{
    CreateDatabaseInput, CreateSite, CreateUser, DatabaseServer, SqliteDatabaseRepository,
    SqliteSiteRepository, SqliteUserRepository, UpdateDatabaseInput, init_pool, run_migrations,
};

async fn setup_test_db() -> panel_db::DbPool {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.keep().join("test_panel_databases.db");
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
async fn test_database_servers_and_database_lifecycle() {
    let pool = setup_test_db().await;
    let user = create_test_user(&pool, "dbtenant", 10005).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let db_repo = SqliteDatabaseRepository::new(pool.clone());

    let site = site_repo
        .create(&CreateSite {
            user_id: user.id,
            kind: "php".to_string(),
            docroot: "/home/p_dbtenant/sites/db.test/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    // 1. Initial server states (seeded by migration)
    let servers = db_repo.list_servers().await.expect("list servers");
    assert_eq!(servers.len(), 2);
    let mariadb = servers
        .iter()
        .find(|s| s.engine == "mariadb")
        .expect("mariadb server");
    assert!(!mariadb.is_installed);
    assert_eq!(mariadb.port, 3306);

    // 2. Upsert server state (simulate installed & running)
    db_repo
        .upsert_server(&DatabaseServer {
            engine: "mariadb".to_string(),
            is_installed: true,
            is_running: true,
            version: Some("11.4.2-MariaDB".to_string()),
            port: 3306,
            socket_path: Some("/run/mysqld/mysqld.sock".to_string()),
            updated_at: 1728560000,
        })
        .await
        .expect("upsert server");

    let updated_mariadb = db_repo
        .get_server("mariadb")
        .await
        .expect("get server")
        .expect("some");
    assert!(updated_mariadb.is_installed);
    assert!(updated_mariadb.is_running);
    assert_eq!(updated_mariadb.version.as_deref(), Some("11.4.2-MariaDB"));

    // 3. Initial database count for user
    let count0 = db_repo.count_by_user(user.id).await.expect("count");
    assert_eq!(count0, 0);

    // 4. Create database
    let db1 = db_repo
        .create(&CreateDatabaseInput {
            user_id: user.id,
            site_id: Some(site.id),
            engine: "mariadb".to_string(),
            name: "p_dbtenant_wordpress".to_string(),
            db_user: "p_dbtenant_wpuser".to_string(),
            remote_access: false,
            allowed_hosts: "localhost".to_string(),
            collation: "utf8mb4_unicode_ci".to_string(),
            character_set: "utf8mb4".to_string(),
        })
        .await
        .expect("create db");

    assert_eq!(db1.name, "p_dbtenant_wordpress");
    assert_eq!(db1.site_id, Some(site.id));
    assert_eq!(db1.user_id, user.id);
    assert!(!db1.remote_access);
    assert_eq!(db1.size_bytes, 0);

    // Count is now 1
    let count1 = db_repo.count_by_user(user.id).await.expect("count");
    assert_eq!(count1, 1);

    // List by user and by site
    let user_dbs = db_repo.list_by_user(user.id).await.expect("list by user");
    assert_eq!(user_dbs.len(), 1);
    assert_eq!(user_dbs[0].id, db1.id);

    let site_dbs = db_repo.list_by_site(site.id).await.expect("list by site");
    assert_eq!(site_dbs.len(), 1);

    // 5. Update mutable fields (toggle remote access)
    let updated_db = db_repo
        .update(
            db1.id,
            &UpdateDatabaseInput {
                site_id: None,
                remote_access: Some(true),
                allowed_hosts: Some("192.168.1.50, 10.0.0.1".to_string()),
                collation: None,
                character_set: None,
            },
        )
        .await
        .expect("update db");
    assert!(updated_db.remote_access);
    assert_eq!(updated_db.allowed_hosts, "192.168.1.50, 10.0.0.1");

    // 6. Update disk size
    db_repo
        .update_size(db1.id, 10485760)
        .await
        .expect("update size");
    let fetched = db_repo
        .find_by_id(db1.id)
        .await
        .expect("find by id")
        .expect("some");
    assert_eq!(fetched.size_bytes, 10485760);

    // 7. Delete database
    let deleted = db_repo.delete(db1.id).await.expect("delete");
    assert!(deleted);
    let count_after = db_repo.count_by_user(user.id).await.expect("count after");
    assert_eq!(count_after, 0);
}
