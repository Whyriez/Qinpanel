#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_core::DomainName;
use panel_db::{
    CreateDomain, CreateSite, CreateUser, DomainStatus, RedirectMode, SiteKind, SiteStatus,
    SqliteDomainRepository, SqliteSiteRepository, SqliteUserRepository, UpdateDomain, UpdateSite,
    init_pool, run_migrations,
};

async fn setup_test_db() -> panel_db::DbPool {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.keep().join("test_panel_site_domain.db");
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
async fn test_site_crud_and_status_lifecycle() {
    let pool = setup_test_db().await;
    let user = create_test_user(&pool, "alice", 10001).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());

    // 1. Create a static website
    let create_req = CreateSite {
        user_id: user.id,
        kind: "static".to_string(),
        docroot: "/home/p_alice/sites/my-static.com/public".to_string(),
        runtime: None,
        version: None,
        status: Some("active".to_string()),
        settings_json: Some(r#"{"spa_fallback":true}"#.to_string()),
    };
    let site = site_repo.create(&create_req).await.expect("create site");
    assert_eq!(site.user_id, user.id);
    assert_eq!(site.kind, "static");
    assert_eq!(site.parsed_kind(), Some(SiteKind::Static));
    assert_eq!(site.status, "active");
    assert_eq!(site.parsed_status(), Some(SiteStatus::Active));
    assert_eq!(site.docroot, "/home/p_alice/sites/my-static.com/public");

    // 2. Find by ID
    let found = site_repo
        .find_by_id(site.id)
        .await
        .expect("find site")
        .expect("site exists");
    assert_eq!(found.id, site.id);
    assert_eq!(found.settings_json, r#"{"spa_fallback":true}"#);

    // 3. Update site configuration (switch to PHP 8.3)
    let update_req = UpdateSite {
        kind: Some("php".to_string()),
        runtime: Some("php".to_string()),
        version: Some("8.3".to_string()),
        docroot: None,
        status: None,
        settings_json: Some(r#"{"php_preset":"wordpress"}"#.to_string()),
    };
    let updated = site_repo
        .update(site.id, &update_req)
        .await
        .expect("update site");
    assert_eq!(updated.kind, "php");
    assert_eq!(updated.parsed_kind(), Some(SiteKind::Php));
    assert_eq!(updated.runtime.as_deref(), Some("php"));
    assert_eq!(updated.version.as_deref(), Some("8.3"));

    // 4. Update status lifecycle (active -> disabled -> suspended)
    let disabled = site_repo
        .update_status(site.id, "disabled")
        .await
        .expect("disable site");
    assert_eq!(disabled.status, "disabled");
    assert_eq!(disabled.parsed_status(), Some(SiteStatus::Disabled));

    let suspended = site_repo
        .update_status(site.id, "suspended")
        .await
        .expect("suspend site");
    assert_eq!(suspended.status, "suspended");
    assert_eq!(suspended.parsed_status(), Some(SiteStatus::Suspended));

    // 5. User sites listing and count
    let sites_list = site_repo
        .find_by_user_id(user.id)
        .await
        .expect("find user sites");
    assert_eq!(sites_list.len(), 1);
    assert_eq!(sites_list[0].id, site.id);

    let site_count = site_repo
        .count_by_user_id(user.id)
        .await
        .expect("count user sites");
    assert_eq!(site_count, 1);

    // 6. Delete site
    let deleted = site_repo.delete(site.id).await.expect("delete site");
    assert!(deleted);
    let after_delete = site_repo.find_by_id(site.id).await.expect("query deleted");
    assert!(after_delete.is_none());
}

#[tokio::test]
async fn test_domain_validation_and_idn_punycode_storage() {
    let pool = setup_test_db().await;
    let user = create_test_user(&pool, "bob", 10002).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let domain_repo = SqliteDomainRepository::new(pool.clone());

    let site = site_repo
        .create(&CreateSite {
            user_id: user.id,
            kind: "static".to_string(),
            docroot: "/home/p_bob/sites/bob.com/public".to_string(),
            runtime: None,
            version: None,
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    // 1. Standard ASCII domain
    let parsed_ascii = DomainName::parse("BOB-STORE.COM").expect("parse domain");
    assert_eq!(parsed_ascii.as_ascii(), "bob-store.com");

    let dom1 = domain_repo
        .create(&CreateDomain {
            site_id: site.id,
            name: parsed_ascii.as_ascii().to_string(),
            unicode_name: parsed_ascii.as_unicode().to_string(),
            is_primary: true,
            redirect_mode: Some("none".to_string()),
            redirect_target: None,
            status: Some("active".to_string()),
        })
        .await
        .expect("create domain");

    assert_eq!(dom1.name, "bob-store.com");
    assert_eq!(dom1.unicode_name, "bob-store.com");
    assert!(dom1.is_primary);
    assert_eq!(dom1.parsed_redirect_mode(), Some(RedirectMode::None));
    assert_eq!(dom1.parsed_status(), Some(DomainStatus::Active));

    // 2. IDN (German Umlaut) Domain: "münchen-store.de"
    let parsed_idn = DomainName::parse("münchen-store.de").expect("parse idn");
    assert_eq!(parsed_idn.as_ascii(), "xn--mnchen-store-dlb.de");
    assert_eq!(parsed_idn.as_unicode(), "münchen-store.de");
    assert!(parsed_idn.is_idn());

    let dom_idn = domain_repo
        .create(&CreateDomain {
            site_id: site.id,
            name: parsed_idn.as_ascii().to_string(),
            unicode_name: parsed_idn.as_unicode().to_string(),
            is_primary: false,
            redirect_mode: Some("to_primary".to_string()),
            redirect_target: None,
            status: Some("pending_ssl".to_string()),
        })
        .await
        .expect("create idn domain");

    assert_eq!(dom_idn.name, "xn--mnchen-store-dlb.de");
    assert_eq!(dom_idn.unicode_name, "münchen-store.de");
    assert!(!dom_idn.is_primary);
    assert_eq!(
        dom_idn.parsed_redirect_mode(),
        Some(RedirectMode::ToPrimary)
    );
    assert_eq!(dom_idn.parsed_status(), Some(DomainStatus::PendingSsl));

    // 3. Find by ASCII name or Unicode name
    let by_ascii = domain_repo
        .find_by_name("xn--mnchen-store-dlb.de")
        .await
        .expect("find by ascii")
        .expect("found");
    assert_eq!(by_ascii.id, dom_idn.id);

    let by_unicode = domain_repo
        .find_by_name("münchen-store.de")
        .await
        .expect("find by unicode")
        .expect("found");
    assert_eq!(by_unicode.id, dom_idn.id);

    // 4. Update domain redirect and status
    let update_req = UpdateDomain {
        name: None,
        unicode_name: None,
        is_primary: None,
        redirect_mode: Some("to_www".to_string()),
        redirect_target: None,
        status: Some("active".to_string()),
    };
    let updated_dom = domain_repo
        .update(dom_idn.id, &update_req)
        .await
        .expect("update domain");
    assert_eq!(
        updated_dom.parsed_redirect_mode(),
        Some(RedirectMode::ToWww)
    );
    assert_eq!(updated_dom.parsed_status(), Some(DomainStatus::Active));
}

#[tokio::test]
async fn test_domain_global_uniqueness_enforcement() {
    let pool = setup_test_db().await;
    let alice = create_test_user(&pool, "alice_unique", 10003).await;
    let bob = create_test_user(&pool, "bob_unique", 10004).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let domain_repo = SqliteDomainRepository::new(pool.clone());

    let site_alice = site_repo
        .create(&CreateSite {
            user_id: alice.id,
            kind: "static".to_string(),
            docroot: "/home/p_alice_unique/sites/shared.com".to_string(),
            runtime: None,
            version: None,
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site alice");

    let site_bob = site_repo
        .create(&CreateSite {
            user_id: bob.id,
            kind: "static".to_string(),
            docroot: "/home/p_bob_unique/sites/shared.com".to_string(),
            runtime: None,
            version: None,
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site bob");

    // Alice claims "shared-domain.com"
    let available_before = domain_repo
        .is_domain_available("shared-domain.com")
        .await
        .expect("check availability");
    assert!(available_before, "Domain should be available initially");

    domain_repo
        .create(&CreateDomain {
            site_id: site_alice.id,
            name: "shared-domain.com".to_string(),
            unicode_name: "shared-domain.com".to_string(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: None,
        })
        .await
        .expect("claim domain for alice");

    let available_after = domain_repo
        .is_domain_available("shared-domain.com")
        .await
        .expect("check availability");
    assert!(!available_after, "Domain should no longer be available");

    // Bob attempts to claim the exact same domain
    let bob_claim = domain_repo
        .create(&CreateDomain {
            site_id: site_bob.id,
            name: "shared-domain.com".to_string(),
            unicode_name: "shared-domain.com".to_string(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: None,
        })
        .await;

    assert!(
        bob_claim.is_err(),
        "Claiming an existing domain must fail due to unique constraint"
    );
    match bob_claim {
        Err(panel_db::DbError::Conflict(msg)) => {
            assert!(msg.contains("already exists"));
        }
        other => panic!("Expected DbError::Conflict, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_primary_domain_promotion_and_cascading_deletion() {
    let pool = setup_test_db().await;
    let user = create_test_user(&pool, "charlie", 10005).await;
    let site_repo = SqliteSiteRepository::new(pool.clone());
    let domain_repo = SqliteDomainRepository::new(pool.clone());

    let site = site_repo
        .create(&CreateSite {
            user_id: user.id,
            kind: "php".to_string(),
            docroot: "/home/p_charlie/sites/charlie.com/public".to_string(),
            runtime: Some("php".to_string()),
            version: Some("8.3".to_string()),
            status: Some("active".to_string()),
            settings_json: None,
        })
        .await
        .expect("create site");

    // Add first domain as primary
    let dom1 = domain_repo
        .create(&CreateDomain {
            site_id: site.id,
            name: "primary-first.com".to_string(),
            unicode_name: "primary-first.com".to_string(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: None,
        })
        .await
        .expect("create dom1");
    assert!(dom1.is_primary);

    // Add second domain as non-primary
    let dom2 = domain_repo
        .create(&CreateDomain {
            site_id: site.id,
            name: "secondary.com".to_string(),
            unicode_name: "secondary.com".to_string(),
            is_primary: false,
            redirect_mode: Some("to_primary".to_string()),
            redirect_target: None,
            status: None,
        })
        .await
        .expect("create dom2");
    assert!(!dom2.is_primary);

    // Check primary is dom1
    let primary = domain_repo
        .find_primary_by_site_id(site.id)
        .await
        .expect("find primary")
        .expect("has primary");
    assert_eq!(primary.id, dom1.id);

    // Switch primary to dom2
    domain_repo
        .set_primary(site.id, dom2.id)
        .await
        .expect("set primary to dom2");

    let new_primary = domain_repo
        .find_primary_by_site_id(site.id)
        .await
        .expect("find primary")
        .expect("has primary");
    assert_eq!(new_primary.id, dom2.id);

    // Verify dom1 was demoted
    let dom1_updated = domain_repo
        .find_by_id(dom1.id)
        .await
        .expect("find dom1")
        .expect("exists");
    assert!(!dom1_updated.is_primary);

    // Verify count before deletion
    assert_eq!(
        domain_repo.count_by_site_id(site.id).await.expect("count"),
        2
    );
    assert_eq!(
        domain_repo.count_by_user_id(user.id).await.expect("count"),
        2
    );

    // Cascading deletion: deleting the site must remove all domains of that site
    site_repo.delete(site.id).await.expect("delete site");

    let domains_after = domain_repo
        .find_by_site_id(site.id)
        .await
        .expect("find domains after delete");
    assert_eq!(domains_after.len(), 0);

    let dom1_after = domain_repo.find_by_id(dom1.id).await.expect("find dom1");
    assert!(dom1_after.is_none());

    let dom2_after = domain_repo.find_by_id(dom2.id).await.expect("find dom2");
    assert!(dom2_after.is_none());
}
