#![allow(clippy::unwrap_used, clippy::expect_used)]
use panel_core::audit::AuditCategory;
use panel_db::{
    AuditLogFilter, CreateAuditLog, SqliteAuditLogRepository, init_pool, run_migrations,
};

#[tokio::test]
async fn existing_audit_rows_are_partitioned_without_migration_or_data_changes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("legacy.db");
    let pool = init_pool(&format!(
        "sqlite://{}",
        path.to_string_lossy().replace('\\', "/")
    ))
    .await
    .expect("pool");
    run_migrations(&pool).await.expect("migrations");
    let repo = SqliteAuditLogRepository::new(pool.clone());
    let actions = [
        "auth.login",
        "tenant.create",
        "site.create",
        "domain.create",
        "site.ssl.issue",
        "file.upload",
        "database.create",
        "php.install",
        "ssh_key.create",
        "terminal.session_end",
        "cloudflare.install",
        "panel.tls.issue",
        "email.create",
        "server.backup.job",
        "panel.update.apply",
        "future.action",
        "sshXkey.create",
        "AUTH.login",
    ];
    // Existing writer shape: no category column or backfill is required.
    for (index, action) in actions.into_iter().enumerate() {
        repo.create(&CreateAuditLog {
            user_id: None,
            actor_username: "legacy-admin".into(),
            action: action.into(),
            target_resource: if index == 2 {
                "demo%_site.test".into()
            } else {
                "demo.test".into()
            },
            details_json: Some("{legacy-invalid-json".into()),
            ip_address: Some("203.0.113.5".into()),
            status: if index == 2 {
                "failure".into()
            } else {
                "success".into()
            },
        })
        .await
        .expect("legacy row");
    }
    let before = repo.list(100, 0).await.expect("before");
    let mut partitioned_ids = Vec::new();
    for category in AuditCategory::ALL {
        let filter = AuditLogFilter {
            category: Some(category),
            ..Default::default()
        };
        let count = repo.count_filtered(&filter).await.expect("count");
        let logs = repo.list_filtered(&filter, 100, 0).await.expect("list");
        assert_eq!(count as usize, logs.len());
        for log in logs {
            assert_eq!(AuditCategory::for_action(&log.action), category);
            partitioned_ids.push(log.id);
        }
    }
    partitioned_ids.sort_unstable();
    let mut expected_ids: Vec<_> = before.iter().map(|row| row.id).collect();
    expected_ids.sort_unstable();
    assert_eq!(
        partitioned_ids, expected_ids,
        "each historical row occurs in exactly one category"
    );
    let filter = AuditLogFilter {
        category: Some(AuditCategory::Websites),
        ..Default::default()
    };
    assert_eq!(repo.count_filtered(&filter).await.expect("count"), 3);
    let page1 = repo.list_filtered(&filter, 2, 0).await.expect("page1");
    let page2 = repo.list_filtered(&filter, 2, 2).await.expect("page2");
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 1);
    assert!(page1.iter().all(|a| page2.iter().all(|b| a.id != b.id)));

    for q in [
        "%_",
        "DEMO%_SITE",
        "legacy-admin",
        "203.0.113.5",
        "site.create",
    ] {
        let filter = AuditLogFilter {
            category: Some(AuditCategory::Websites),
            q: Some(q.into()),
            status: Some("failure".into()),
            ..Default::default()
        };
        assert_eq!(
            repo.count_filtered(&filter).await.expect("combined count"),
            1
        );
        let rows = repo
            .list_filtered(&filter, 10, 0)
            .await
            .expect("combined list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "site.create");
    }
    let conflict = AuditLogFilter {
        category: Some(AuditCategory::Authentication),
        action: Some("site.create".into()),
        ..Default::default()
    };
    assert_eq!(repo.count_filtered(&conflict).await.expect("conflict"), 0);
    let injection = AuditLogFilter {
        q: Some("' OR 1=1 --".into()),
        ..Default::default()
    };
    assert_eq!(
        repo.count_filtered(&injection)
            .await
            .expect("literal search"),
        0
    );
    let after = repo.list(100, 0).await.expect("after");
    assert_eq!(
        serde_json::to_value(before).expect("json"),
        serde_json::to_value(after).expect("json")
    );
}
