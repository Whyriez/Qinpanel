#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_db::{
    AuditLogFilter, CreateApiToken, CreateAuditLog, CreateJob, CreatePlan, CreateSession,
    CreateUser, SqliteApiTokenRepository, SqliteAuditLogRepository, SqliteJobRepository,
    SqlitePlanRepository, SqliteSessionRepository, SqliteSettingRepository, SqliteUserRepository,
    UpdatePlan, init_pool, run_migrations,
};

async fn setup_test_db() -> panel_db::DbPool {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.keep().join("test_panel.db");
    let db_url = format!("sqlite://{}", db_path.to_str().expect("path to str"));

    let pool = init_pool(&db_url).await.expect("init pool");
    run_migrations(&pool).await.expect("run migrations");
    pool
}

#[tokio::test]
async fn test_user_repository_crud() {
    let pool = setup_test_db().await;
    let repo = SqliteUserRepository::new(pool);

    // 1. Create User
    let input = CreateUser {
        username: "admin".to_string(),
        password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$test".to_string(),
        role: "admin".to_string(),
        email: Some("admin@example.com".to_string()),
        linux_username: None,
        linux_uid: None,
        plan_id: None,
    };
    let user = repo.create(&input).await.expect("create user");
    assert_eq!(user.username, "admin");
    assert_eq!(user.role, "admin");
    assert_eq!(user.status, "active");

    // 2. Conflict on duplicate username
    let conflict_res = repo.create(&input).await;
    assert!(conflict_res.is_err());

    // 3. Find by ID and username
    let by_id = repo
        .find_by_id(user.id)
        .await
        .expect("find by id")
        .expect("user exists");
    assert_eq!(by_id.id, user.id);

    let by_username = repo
        .find_by_username("admin")
        .await
        .expect("find by username")
        .expect("user exists");
    assert_eq!(by_username.id, user.id);

    // 4. Update password
    repo.update_password(user.id, "new_hash")
        .await
        .expect("update password");
    let updated_user = repo
        .find_by_id(user.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(updated_user.password_hash, "new_hash");

    // 5. Update TOTP
    repo.update_totp(user.id, Some("JBSWY3DPEHPK3PXP"), true)
        .await
        .expect("update totp");
    let totp_user = repo
        .find_by_id(user.id)
        .await
        .expect("find")
        .expect("exists");
    assert!(totp_user.totp_enabled);
    assert_eq!(totp_user.totp_secret, Some("JBSWY3DPEHPK3PXP".to_string()));

    // 6. Update Status
    repo.update_status(user.id, "suspended")
        .await
        .expect("update status");
    let suspended = repo
        .find_by_id(user.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(suspended.status, "suspended");

    // 7. List all
    let list = repo.list_all().await.expect("list all");
    assert_eq!(list.len(), 1);

    // 8. Allocate next UID
    let uid_initial = repo.allocate_next_uid().await.expect("allocate uid");
    assert_eq!(uid_initial, 10000); // No user with linux_uid yet

    let tenant_user = repo
        .create(&CreateUser {
            username: "tenant1".to_string(),
            password_hash: "hash".to_string(),
            role: "user".to_string(),
            email: None,
            linux_username: Some("p_tenant1".to_string()),
            linux_uid: Some(10000),
            plan_id: None,
        })
        .await
        .expect("create tenant");

    let uid_next = repo.allocate_next_uid().await.expect("allocate next uid");
    assert_eq!(uid_next, 10001); // Incremented from 10000

    repo.delete(tenant_user.id).await.expect("delete tenant");

    // 9. Delete admin
    repo.delete(user.id).await.expect("delete");
    let after_delete = repo.find_by_id(user.id).await.expect("find");
    assert!(after_delete.is_none());
}

#[tokio::test]
async fn test_session_repository_crud() {
    let pool = setup_test_db().await;
    let user_repo = SqliteUserRepository::new(pool.clone());
    let session_repo = SqliteSessionRepository::new(pool);

    let user = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: "hash".to_string(),
            role: "user".to_string(),
            email: None,
            linux_username: Some("p_alice".to_string()),
            linux_uid: Some(10001),
            plan_id: None,
        })
        .await
        .expect("create user");

    let now = 1000000;
    let session_input = CreateSession {
        id: "sess_123".to_string(),
        user_id: user.id,
        token_hash: "token_hash_abc".to_string(),
        ip_address: Some("127.0.0.1".to_string()),
        user_agent: Some("Mozilla/5.0".to_string()),
        expires_at: now + 3600,
    };

    // 1. Create
    let session = session_repo
        .create(&session_input)
        .await
        .expect("create session");
    assert_eq!(session.id, "sess_123");

    // 2. Find by token hash
    let found = session_repo
        .find_by_token_hash("token_hash_abc")
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(found.user_id, user.id);

    // 3. List by user id
    let user_sessions = session_repo.list_by_user_id(user.id).await.expect("list");
    assert_eq!(user_sessions.len(), 1);

    // 4. Delete expired
    let expired_count = session_repo
        .delete_expired(now + 4000)
        .await
        .expect("delete expired");
    assert_eq!(expired_count, 1);

    let after_expire = session_repo
        .find_by_token_hash("token_hash_abc")
        .await
        .expect("find");
    assert!(after_expire.is_none());
}

#[tokio::test]
async fn test_audit_log_repository() {
    let pool = setup_test_db().await;
    let repo = SqliteAuditLogRepository::new(pool.clone());

    let user_repo = SqliteUserRepository::new(pool.clone());
    let u1 = user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: "hash1".to_string(),
            role: "admin".to_string(),
            email: None,
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create user 1");

    let u2 = user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: "hash2".to_string(),
            role: "user".to_string(),
            email: None,
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create user 2");

    let log_input1 = CreateAuditLog {
        user_id: Some(u1.id),
        actor_username: "admin".to_string(),
        action: "site.create".to_string(),
        target_resource: "example.com".to_string(),
        details_json: Some(r#"{"kind":"php"}"#.to_string()),
        ip_address: Some("192.168.1.1".to_string()),
        status: "success".to_string(),
    };

    let log_input2 = CreateAuditLog {
        user_id: Some(u2.id),
        actor_username: "alice".to_string(),
        action: "auth.login".to_string(),
        target_resource: "auth".to_string(),
        details_json: None,
        ip_address: Some("10.0.0.5".to_string()),
        status: "failure".to_string(),
    };

    let log_input3 = CreateAuditLog {
        user_id: Some(u1.id),
        actor_username: "admin".to_string(),
        action: "user.suspend".to_string(),
        target_resource: format!("user:{}", u2.id),
        details_json: Some(r#"{"reason":"abuse"}"#.to_string()),
        ip_address: Some("192.168.1.1".to_string()),
        status: "success".to_string(),
    };

    let log1 = repo.create(&log_input1).await.expect("create log 1");
    let log2 = repo.create(&log_input2).await.expect("create log 2");
    let log3 = repo.create(&log_input3).await.expect("create log 3");

    // Test find_by_id
    let found = repo
        .find_by_id(log1.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(found.action, "site.create");
    assert_eq!(found.actor_username, "admin");

    let not_found = repo.find_by_id(99999).await.expect("find not found");
    assert!(not_found.is_none());

    // Test list without filter
    let list_all = repo.list(10, 0).await.expect("list all");
    assert_eq!(list_all.len(), 3);

    // Test count all
    let count_all = repo
        .count_filtered(&AuditLogFilter::default())
        .await
        .expect("count all");
    assert_eq!(count_all, 3);

    // Test filter by user_id
    let filter_user = AuditLogFilter {
        user_id: Some(u1.id),
        ..Default::default()
    };
    let list_user = repo
        .list_filtered(&filter_user, 10, 0)
        .await
        .expect("list user");
    assert_eq!(list_user.len(), 2);
    let count_user = repo.count_filtered(&filter_user).await.expect("count user");
    assert_eq!(count_user, 2);

    // Test filter by actor_username
    let filter_actor = AuditLogFilter {
        actor_username: Some("alice".to_string()),
        ..Default::default()
    };
    let list_actor = repo
        .list_filtered(&filter_actor, 10, 0)
        .await
        .expect("list actor");
    assert_eq!(list_actor.len(), 1);
    assert_eq!(list_actor[0].id, log2.id);

    // Test filter by action
    let filter_action = AuditLogFilter {
        action: Some("user.suspend".to_string()),
        ..Default::default()
    };
    let list_action = repo
        .list_filtered(&filter_action, 10, 0)
        .await
        .expect("list action");
    assert_eq!(list_action.len(), 1);
    assert_eq!(list_action[0].id, log3.id);

    // Test filter by status
    let filter_failed = AuditLogFilter {
        status: Some("failure".to_string()),
        ..Default::default()
    };
    let list_failed = repo
        .list_filtered(&filter_failed, 10, 0)
        .await
        .expect("list failed");
    assert_eq!(list_failed.len(), 1);
    assert_eq!(list_failed[0].status, "failure");

    // Test pagination
    let page1 = repo
        .list_filtered(&AuditLogFilter::default(), 2, 0)
        .await
        .expect("page 1");
    assert_eq!(page1.len(), 2);
    let page2 = repo
        .list_filtered(&AuditLogFilter::default(), 2, 2)
        .await
        .expect("page 2");
    assert_eq!(page2.len(), 1);
}

#[tokio::test]
async fn test_job_repository_and_recovery() {
    let pool = setup_test_db().await;
    let repo = SqliteJobRepository::new(pool);

    let job_input = CreateJob {
        id: "job_deploy_1".to_string(),
        user_id: None,
        name: "site.deploy".to_string(),
        payload_json: r#"{"domain":"example.com"}"#.to_string(),
        max_retries: 3,
    };

    // 1. Create
    let job = repo.create(&job_input).await.expect("create job");
    assert_eq!(job.status, "queued");
    assert_eq!(job.progress, 0);

    // 2. Update progress
    repo.update_progress(&job.id, 50).await.expect("progress");
    let updated = repo
        .find_by_id(&job.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(updated.progress, 50);

    // 3. Mark running
    repo.update_status(&job.id, "running", None, None)
        .await
        .expect("running");

    // 4. Test recovery: Server restarts unexpectedly while job is running
    let recovered = repo.recover_interrupted_jobs().await.expect("recover");
    assert_eq!(recovered, 1);

    let failed_job = repo
        .find_by_id(&job.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(failed_job.status, "failed");
    assert!(failed_job.error_message.unwrap().contains("restart"));
}

#[tokio::test]
async fn test_setting_repository_upsert() {
    let pool = setup_test_db().await;
    let repo = SqliteSettingRepository::new(pool);

    // 1. Set
    repo.set("panel.title", "Nusa Panel").await.expect("set");
    let val = repo.get("panel.title").await.expect("get");
    assert_eq!(val, Some("Nusa Panel".to_string()));

    // 2. Upsert
    repo.set("panel.title", "Nusa Control Panel")
        .await
        .expect("upsert");
    let updated_val = repo.get("panel.title").await.expect("get");
    assert_eq!(updated_val, Some("Nusa Control Panel".to_string()));

    // 3. List
    let list = repo.list().await.expect("list");
    assert_eq!(list.len(), 1);

    // 4. Delete
    repo.delete("panel.title").await.expect("delete");
    let after_del = repo.get("panel.title").await.expect("get");
    assert!(after_del.is_none());
}

#[tokio::test]
async fn test_api_token_repository() {
    let pool = setup_test_db().await;
    let user_repo = SqliteUserRepository::new(pool.clone());
    let token_repo = SqliteApiTokenRepository::new(pool);

    let user = user_repo
        .create(&CreateUser {
            username: "bob".to_string(),
            password_hash: "hash".to_string(),
            role: "user".to_string(),
            email: None,
            linux_username: Some("p_bob".to_string()),
            linux_uid: Some(10002),
            plan_id: None,
        })
        .await
        .expect("create user");

    let token_input = CreateApiToken {
        id: "tok_1".to_string(),
        user_id: user.id,
        name: "CI Deploy Token".to_string(),
        token_hash: "sha256_hash_value".to_string(),
        scopes_json: r#"["sites:read", "sites:deploy"]"#.to_string(),
        expires_at: Some(2000000000),
    };

    let token = token_repo.create(&token_input).await.expect("create token");
    assert_eq!(token.name, "CI Deploy Token");

    let found = token_repo
        .find_by_token_hash("sha256_hash_value")
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(found.id, "tok_1");

    token_repo
        .update_last_used("tok_1", 1700000000)
        .await
        .expect("update");
    let after_used = token_repo
        .find_by_token_hash("sha256_hash_value")
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(after_used.last_used_at, Some(1700000000));

    let user_tokens = token_repo.list_by_user_id(user.id).await.expect("list");
    assert_eq!(user_tokens.len(), 1);

    token_repo.delete("tok_1").await.expect("delete");
    let after_del = token_repo
        .find_by_token_hash("sha256_hash_value")
        .await
        .expect("find");
    assert!(after_del.is_none());
}

#[tokio::test]
async fn test_recovery_code_repository() {
    use panel_db::SqliteRecoveryCodeRepository;

    let pool = setup_test_db().await;
    let user_repo = SqliteUserRepository::new(pool.clone());
    let code_repo = SqliteRecoveryCodeRepository::new(pool.clone());

    let user = user_repo
        .create(&CreateUser {
            username: "totp_user".to_string(),
            password_hash: "hash".to_string(),
            role: "user".to_string(),
            email: None,
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create user");

    let codes = vec![
        "hash_code_1".to_string(),
        "hash_code_2".to_string(),
        "hash_code_3".to_string(),
    ];

    code_repo
        .replace_codes(user.id, &codes)
        .await
        .expect("replace codes");
    assert_eq!(code_repo.count_unused(user.id).await.expect("count"), 3);

    // Consume first code
    let consumed = code_repo
        .verify_and_consume(user.id, "hash_code_1")
        .await
        .expect("consume");
    assert!(consumed);
    assert_eq!(code_repo.count_unused(user.id).await.expect("count"), 2);

    // Cannot consume already used code
    let consumed_again = code_repo
        .verify_and_consume(user.id, "hash_code_1")
        .await
        .expect("consume again");
    assert!(!consumed_again);

    // Cannot consume nonexistent code
    let consumed_invalid = code_repo
        .verify_and_consume(user.id, "hash_code_invalid")
        .await
        .expect("consume invalid");
    assert!(!consumed_invalid);

    // Delete all codes
    code_repo.delete_by_user_id(user.id).await.expect("delete");
    assert_eq!(code_repo.count_unused(user.id).await.expect("count"), 0);
}

#[tokio::test]
async fn test_plan_repository_crud() {
    let pool = setup_test_db().await;
    let plan_repo = SqlitePlanRepository::new(pool.clone());
    let user_repo = SqliteUserRepository::new(pool);

    // Initial baseline plans created by migration
    let all = plan_repo.list_all().await.expect("list plans");
    assert!(all.len() >= 2);

    // Create new plan
    let new_plan = plan_repo
        .create(&CreatePlan {
            name: "Enterprise".to_string(),
            description: Some("Custom high-performance plan".to_string()),
            cpu_quota_percent: Some(400),
            memory_limit_bytes: Some(4294967296),
            memory_high_bytes: Some(3435973836),
            tasks_max: Some(500),
            io_weight: Some(500),
            disk_limit_bytes: Some(107374182400),
            inode_limit: Some(2000000),
            max_sites: Some(50),
            max_domains: Some(100),
            max_databases: Some(50),
            max_cron_jobs: Some(50),
            max_terminal_sessions: Some(5),
            max_upload_bytes: Some(524288000),
            shell_policy: Some("full".to_string()),
            allow_external_upstream: Some(true),
        })
        .await
        .expect("create plan");

    assert_eq!(new_plan.name, "Enterprise");
    assert_eq!(new_plan.cpu_quota_percent, 400);
    assert_eq!(new_plan.memory_limit_bytes, 4294967296);

    // Duplicate name rejected
    let dup_err = plan_repo
        .create(&CreatePlan {
            name: "Enterprise".to_string(),
            description: None,
            cpu_quota_percent: None,
            memory_limit_bytes: None,
            memory_high_bytes: None,
            tasks_max: None,
            io_weight: None,
            disk_limit_bytes: None,
            inode_limit: None,
            max_sites: None,
            max_domains: None,
            max_databases: None,
            max_cron_jobs: None,
            max_terminal_sessions: None,
            max_upload_bytes: None,
            shell_policy: None,
            allow_external_upstream: None,
        })
        .await;
    assert!(dup_err.is_err());

    // Update plan
    let updated = plan_repo
        .update(
            new_plan.id,
            &UpdatePlan {
                name: Some("Enterprise Plus".to_string()),
                cpu_quota_percent: Some(800),
                ..Default::default()
            },
        )
        .await
        .expect("update plan");
    assert_eq!(updated.name, "Enterprise Plus");
    assert_eq!(updated.cpu_quota_percent, 800);

    // Assign to a tenant user to test deletion prevention
    let tenant = user_repo
        .create(&CreateUser {
            username: "ent_user".to_string(),
            password_hash: "hash".to_string(),
            role: "user".to_string(),
            email: None,
            linux_username: Some("p_ent_user".to_string()),
            linux_uid: Some(10050),
            plan_id: Some(new_plan.id),
        })
        .await
        .expect("create user with plan");

    // Cannot delete plan in use
    let del_err = plan_repo.delete(new_plan.id).await;
    assert!(del_err.is_err());

    // Remove tenant, then delete should succeed
    user_repo.delete(tenant.id).await.expect("delete user");
    plan_repo.delete(new_plan.id).await.expect("delete plan");

    let not_found = plan_repo.find_by_id(new_plan.id).await.expect("find");
    assert!(not_found.is_none());
}
