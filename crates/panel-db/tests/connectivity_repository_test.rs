#![allow(clippy::unwrap_used, clippy::expect_used)]
use panel_core::{DatabaseEngine, connectivity::*};
use panel_db::{
    CreateSite, CreateUser, DbError, DbPool, SqliteConnectivityRepository, SqliteSiteRepository,
    SqliteUserRepository, init_pool, run_migrations,
};
use std::num::NonZeroU16;

async fn fixture() -> (DbPool, i64, i64, i64, i64, i64) {
    fixture_before(None).await
}
async fn fixture_before(limit: Option<i64>) -> (DbPool, i64, i64, i64, i64, i64) {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    let mut migrator = sqlx::migrate!("./migrations");
    if let Some(limit) = limit {
        migrator.migrations = std::borrow::Cow::Owned(
            migrator
                .iter()
                .filter(|m| m.version <= limit)
                .cloned()
                .collect(),
        );
    }
    migrator.run(&pool).await.unwrap();
    let users = SqliteUserRepository::new(pool.clone());
    let mut ids = Vec::new();
    for (name, role) in [("admin", "admin"), ("alice", "user"), ("bob", "user")] {
        ids.push(
            users
                .create(&CreateUser {
                    username: name.into(),
                    password_hash: "fixture".into(),
                    role: role.into(),
                    email: None,
                    linux_username: None,
                    linux_uid: None,
                    plan_id: None,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let sites = SqliteSiteRepository::new(pool.clone());
    let mut site_ids = Vec::new();
    for id in &ids[1..] {
        site_ids.push(
            sites
                .create(&CreateSite {
                    user_id: *id,
                    kind: "static".into(),
                    docroot: "/fixture/unused".into(),
                    runtime: None,
                    version: None,
                    status: None,
                    settings_json: None,
                })
                .await
                .unwrap()
                .id,
        );
    }
    (pool, ids[0], ids[1], ids[2], site_ids[0], site_ids[1])
}

fn web(site_id: i64) -> EndpointConfig {
    EndpointConfig {
        service: EndpointService::Website,
        scope: EndpointScope::Project { site_id },
        transport: EndpointTransport::Direct,
        host: EndpointHost::parse("www.example.test").unwrap(),
        port: NonZeroU16::new(8443).unwrap(),
        tls_mode: EndpointTlsMode::Tls,
        certificate_id: None,
        enabled: true,
        primary: true,
    }
}

fn ssh() -> EndpointConfig {
    EndpointConfig {
        service: EndpointService::Ssh,
        scope: EndpointScope::Host,
        transport: EndpointTransport::Direct,
        host: EndpointHost::parse("2001:db8::10").unwrap(),
        port: NonZeroU16::new(2222).unwrap(),
        tls_mode: EndpointTlsMode::Ssh,
        certificate_id: None,
        enabled: true,
        primary: true,
    }
}

fn report(revision: i64) -> EndpointEvidence {
    let now = chrono::Utc::now().timestamp();
    EndpointEvidence {
        endpoint_revision: revision,
        kind: EvidenceKind::ExternalReachability,
        observation_point: ObservationPoint::UserClient,
        observer_id: "client-a".into(),
        address_family: Some(AddressFamily::Ipv4),
        observed_at: now - 10,
        expires_at: now + 600,
        outcome: EvidenceOutcome::Pass,
        reason: EvidenceReason::Verified,
    }
}

#[tokio::test]
async fn upgrade_is_additive_repeatable_and_preserves_legacy_sources_without_inference() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    let mut legacy = sqlx::migrate!("./migrations");
    legacy.migrations = std::borrow::Cow::Owned(
        legacy
            .iter()
            .filter(|m| m.version < 20261020000001)
            .cloned()
            .collect(),
    );
    legacy.run(&pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO users(id,username,password_hash,role,created_at,updated_at) VALUES (10,'legacy','legacy-hash','user',1,1);
        INSERT INTO sites(id,user_id,kind,docroot,settings_json,created_at,updated_at) VALUES (10,10,'static','/legacy/public','{\"operator\":true}',1,1);
        INSERT INTO domains(site_id,name,unicode_name,is_primary,created_at,updated_at) VALUES (10,'legacy.example','legacy.example',1,1,1);")
        .execute(&pool).await.unwrap();
    for (key, value) in [
        ("network.public_ipv4", "203.0.113.7"),
        ("network.public_ipv6", "2001:db8::7"),
        ("operator.custom", "preserve-me"),
    ] {
        sqlx::query("INSERT INTO settings(key,value,updated_at) VALUES (?,?,123)")
            .bind(key)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
    }
    let before: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT key,value,updated_at FROM settings ORDER BY key")
            .fetch_all(&pool)
            .await
            .unwrap();
    let engines: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT engine,is_installed,is_running,port FROM database_servers ORDER BY engine",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    run_migrations(&pool).await.unwrap();
    run_migrations(&pool).await.unwrap();
    for _ in 0..2 {
        sqlx::raw_sql(include_str!(
            "../migrations/20261020000001_connectivity.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    let after: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT key,value,updated_at FROM settings ORDER BY key")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let after_engines: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT engine,is_installed,is_running,port FROM database_servers ORDER BY engine",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(engines, after_engines);
    let preserved: (String, String, String, String) = sqlx::query_as("SELECT u.password_hash,s.docroot,s.settings_json,d.name FROM users u JOIN sites s ON s.user_id=u.id JOIN domains d ON d.site_id=s.id WHERE s.id=10")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        preserved,
        (
            "legacy-hash".into(),
            "/legacy/public".into(),
            "{\"operator\":true}".into(),
            "legacy.example".into()
        )
    );
    for table in ["service_endpoints", "endpoint_evidence"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
}

#[tokio::test]
async fn services_have_independent_endpoints_with_ipv6_port_and_derived_client_requirements() {
    let (pool, admin, _, _, a, _) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool);
    let web = repo.create(admin, &web(a)).await.unwrap();
    let ssh = repo.create(admin, &ssh()).await.unwrap();
    assert_eq!(
        ssh.config.host.authority(ssh.config.port),
        "[2001:db8::10]:2222"
    );
    assert_eq!(ssh.client_requirement, ClientRequirement::None);
    assert!(repo.evidence(admin, &ssh.id).await.unwrap().is_empty());
    let mut access = ssh.config.clone();
    access.transport = EndpointTransport::CloudflareAccess;
    access.host = EndpointHost::parse("ssh.example.test").unwrap();
    let changed = repo.update(admin, &ssh.id, 1, &access).await.unwrap();
    assert_eq!(changed.client_requirement, ClientRequirement::Cloudflared);
    assert_eq!(repo.find(admin, &web.id).await.unwrap().unwrap(), web);
    let mut db = ssh.config.clone();
    db.service = EndpointService::Database;
    db.scope = EndpointScope::Engine {
        engine: DatabaseEngine::PostgreSql,
    };
    db.tls_mode = EndpointTlsMode::Tls;
    db.port = NonZeroU16::new(5432).unwrap();
    db.transport = EndpointTransport::PrivateNetwork;
    let database = repo.create(admin, &db).await.unwrap();
    assert_eq!(
        database.client_requirement,
        ClientRequirement::CloudflareOne
    );
    assert!(repo.evidence(admin, &database.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn tenant_reads_are_owned_and_current_and_tenants_cannot_mutate() {
    let (pool, admin, alice, bob, a, b) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool.clone());
    let owned = repo.create(admin, &web(a)).await.unwrap();
    repo.create(admin, &web(b)).await.unwrap();
    repo.create(admin, &ssh()).await.unwrap();
    repo.record_evidence(admin, &owned.id, &report(1))
        .await
        .unwrap();
    assert_eq!(repo.list(alice).await.unwrap(), vec![owned.clone()]);
    assert_eq!(repo.list(bob).await.unwrap().len(), 1);
    assert!(repo.find(bob, &owned.id).await.unwrap().is_none());
    assert!(repo.evidence(bob, &owned.id).await.unwrap().is_empty());
    assert!(repo.create(alice, &web(b)).await.is_err());
    assert!(repo.update(alice, &owned.id, 1, &web(a)).await.is_err());
    assert!(repo.delete(alice, &owned.id, 1).await.is_err());
    assert!(
        repo.record_evidence(alice, &owned.id, &report(1))
            .await
            .is_err()
    );
    assert!(repo.list(-1).await.unwrap().is_empty());
    sqlx::query("UPDATE users SET status='suspended' WHERE id=?")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.list(alice).await.unwrap().is_empty());
    assert!(repo.evidence(alice, &owned.id).await.unwrap().is_empty());
    assert!(repo.find(admin, &owned.id).await.unwrap().is_some());
    sqlx::query("UPDATE users SET status='active' WHERE id=?")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE sites SET user_id=? WHERE id=?")
        .bind(bob)
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.find(alice, &owned.id).await.unwrap().is_none());
    assert_eq!(repo.evidence(bob, &owned.id).await.unwrap().len(), 1);
    sqlx::query("UPDATE sites SET status='suspended' WHERE id=?")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.find(bob, &owned.id).await.unwrap().is_none());
}

#[tokio::test]
async fn unique_primary_is_per_service_scope_and_failed_writes_preserve_previous_revision() {
    let (pool, admin, _, _, a, b) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool);
    let first = repo.create(admin, &web(a)).await.unwrap();
    repo.create(admin, &web(b)).await.unwrap();
    assert!(matches!(
        repo.create(admin, &web(a)).await,
        Err(DbError::Conflict(_))
    ));
    let mut alternative = web(a);
    alternative.primary = false;
    alternative.host = EndpointHost::parse("alternate.example.test").unwrap();
    let second = repo.create(admin, &alternative).await.unwrap();
    alternative.primary = true;
    assert!(matches!(
        repo.update(admin, &second.id, 1, &alternative).await,
        Err(DbError::Conflict(_))
    ));
    assert_eq!(repo.find(admin, &second.id).await.unwrap().unwrap(), second);
    let mut moved = web(b);
    moved.primary = false;
    assert!(repo.update(admin, &first.id, 1, &moved).await.is_err());
    assert_eq!(repo.find(admin, &first.id).await.unwrap().unwrap(), first);
}

#[tokio::test]
async fn concurrent_updates_and_stale_deletes_cannot_overwrite_metadata() {
    let (pool, admin, _, _, a, _) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool);
    let endpoint = repo.create(admin, &web(a)).await.unwrap();
    let mut left = endpoint.config.clone();
    left.port = NonZeroU16::new(9443).unwrap();
    let mut right = left.clone();
    right.port = NonZeroU16::new(10443).unwrap();
    let (one, two) = tokio::join!(
        repo.update(admin, &endpoint.id, 1, &left),
        repo.update(admin, &endpoint.id, 1, &right)
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    assert!(matches!(
        one.as_ref().err().or(two.as_ref().err()),
        Some(DbError::Conflict(_))
    ));
    assert!(matches!(
        repo.delete(admin, &endpoint.id, 1).await,
        Err(DbError::Conflict(_))
    ));
    assert_eq!(
        repo.find(admin, &endpoint.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    repo.delete(admin, &endpoint.id, 2).await.unwrap();
    assert!(repo.find(admin, &endpoint.id).await.unwrap().is_none());
}

async fn certificate(pool: &DbPool, site: i64) -> i64 {
    sqlx::query("INSERT INTO certificates(site_id,primary_domain,issuer,cert_path,key_path,fullchain_pem,privkey_pem,not_before,not_after,created_at,updated_at) VALUES (?,'www.example.test','custom','fixture-cert','fixture-key','fixture-pem','fixture-private-key',1,9999999999,1,1)")
        .bind(site).execute(pool).await.unwrap().last_insert_rowid()
}

#[tokio::test]
async fn nonexistent_projects_and_cross_project_certificates_are_rejected() {
    let (pool, admin, _, _, a, b) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool.clone());
    assert!(repo.create(admin, &web(99999)).await.is_err());
    let mut cfg = web(a);
    cfg.certificate_id = Some(certificate(&pool, b).await);
    assert!(repo.create(admin, &cfg).await.is_err());
    cfg.certificate_id = Some(certificate(&pool, a).await);
    let valid = repo.create(admin, &cfg).await.unwrap();
    cfg.certificate_id = Some(99999);
    assert!(repo.update(admin, &valid.id, 1, &cfg).await.is_err());
    assert_eq!(repo.find(admin, &valid.id).await.unwrap().unwrap(), valid);
    sqlx::query("DELETE FROM sites WHERE id=?")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.find(admin, &valid.id).await.unwrap().is_none());
}

#[tokio::test]
async fn private_bindings_and_certificate_keys_never_enter_public_json_or_debug() {
    let (pool, admin, alice, _, a, _) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool.clone());
    let mut cfg = web(a);
    cfg.certificate_id = Some(certificate(&pool, a).await);
    let endpoint = repo.create(admin, &cfg).await.unwrap();
    // Only a future trusted binding integration writes these reserved columns.
    sqlx::query("UPDATE service_endpoints SET secret_ref='private-vault-ref',route_id='private-route',connection_id='private-connection' WHERE id=?")
        .bind(&endpoint.id).execute(&pool).await.unwrap();
    for actor in [admin, alice] {
        let value = repo.find(actor, &endpoint.id).await.unwrap().unwrap();
        let json = serde_json::to_string(&value).unwrap();
        let debug = format!("{value:?}");
        let list = serde_json::to_string(&repo.list(actor).await.unwrap()).unwrap();
        for output in [&json, &debug, &list] {
            for secret in [
                "private-vault-ref",
                "private-route",
                "private-connection",
                "fixture-private-key",
                "secret_ref",
                "privkey_pem",
            ] {
                assert!(!output.contains(secret));
            }
        }
    }
}

#[tokio::test]
async fn evidence_keeps_ip_families_and_observers_separate_and_obsoletes_old_revisions() {
    let (pool, admin, alice, _, a, _) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool);
    let endpoint = repo.create(admin, &web(a)).await.unwrap();
    let ipv4 = report(1);
    repo.record_evidence(admin, &endpoint.id, &ipv4)
        .await
        .unwrap();
    let mut ipv6 = ipv4.clone();
    ipv6.address_family = Some(AddressFamily::Ipv6);
    ipv6.outcome = EvidenceOutcome::Fail;
    ipv6.reason = EvidenceReason::PortBlocked;
    repo.record_evidence(admin, &endpoint.id, &ipv6)
        .await
        .unwrap();
    let mut timeout = ipv4.clone();
    timeout.observed_at += 1;
    timeout.outcome = EvidenceOutcome::Unknown;
    timeout.reason = EvidenceReason::Timeout;
    repo.record_evidence(admin, &endpoint.id, &timeout)
        .await
        .unwrap();
    let mut other_observer = ipv4.clone();
    other_observer.observer_id = "probe-b".into();
    other_observer.observation_point = ObservationPoint::ExternalProbe;
    repo.record_evidence(admin, &endpoint.id, &other_observer)
        .await
        .unwrap();
    let rows = repo.evidence(alice, &endpoint.id).await.unwrap();
    assert_eq!(rows.iter().filter(|r| r.current).count(), 3);
    assert!(
        rows.iter()
            .any(|r| r.current && r.evidence.observer_id == "probe-b")
    );
    assert!(rows.iter().any(|r| r.current
        && r.evidence.address_family == Some(AddressFamily::Ipv6)
        && r.evidence.outcome == EvidenceOutcome::Fail));
    assert!(rows.iter().any(|r| r.current
        && r.evidence.address_family == Some(AddressFamily::Ipv4)
        && r.evidence.outcome == EvidenceOutcome::Unknown));
    assert!(
        rows.iter()
            .any(|r| !r.current && r.evidence.outcome == EvidenceOutcome::Pass)
    );
    let mut updated = endpoint.config.clone();
    updated.port = NonZeroU16::new(9443).unwrap();
    repo.update(admin, &endpoint.id, 1, &updated).await.unwrap();
    assert!(matches!(
        repo.record_evidence(admin, &endpoint.id, &ipv4).await,
        Err(DbError::Conflict(_))
    ));
    let stale = repo.evidence(admin, &endpoint.id).await.unwrap();
    assert!(stale.iter().all(|r| !r.current && !r.revision_matches));
    assert_eq!(stale.len(), 4);
    repo.delete(admin, &endpoint.id, 2).await.unwrap();
    assert!(repo.evidence(admin, &endpoint.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn expiry_disabled_endpoints_and_local_checks_cannot_imply_external_success() {
    let (pool, admin, _, _, a, _) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool);
    let endpoint = repo.create(admin, &web(a)).await.unwrap();
    let mut expired = report(1);
    expired.observed_at -= 1000;
    expired.expires_at = chrono::Utc::now().timestamp() - 1;
    repo.record_evidence(admin, &endpoint.id, &expired)
        .await
        .unwrap();
    let rows = repo.evidence(admin, &endpoint.id).await.unwrap();
    assert!(rows[0].expired && !rows[0].current && rows[0].revision_matches);
    let mut local = report(1);
    local.observation_point = ObservationPoint::LocalServer;
    assert!(
        repo.record_evidence(admin, &endpoint.id, &local)
            .await
            .is_err()
    );
    local.kind = EvidenceKind::Service;
    local.address_family = None;
    repo.record_evidence(admin, &endpoint.id, &local)
        .await
        .unwrap();
    assert!(
        repo.evidence(admin, &endpoint.id)
            .await
            .unwrap()
            .iter()
            .filter(|r| r.current)
            .all(|r| r.evidence.kind == EvidenceKind::Service)
    );
    let mut disabled = endpoint.config.clone();
    disabled.enabled = false;
    disabled.primary = false;
    repo.update(admin, &endpoint.id, 1, &disabled)
        .await
        .unwrap();
    local.endpoint_revision = 2;
    assert!(
        repo.record_evidence(admin, &endpoint.id, &local)
            .await
            .is_err()
    );
    assert!(
        repo.evidence(admin, &endpoint.id)
            .await
            .unwrap()
            .iter()
            .all(|r| !r.current)
    );
}

#[tokio::test]
async fn literal_ip_reports_reject_wrong_family_and_suspended_admin_cannot_write() {
    let (pool, admin, _, _, _, _) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool.clone());
    let endpoint = repo.create(admin, &ssh()).await.unwrap();
    let mut evidence = report(1);
    assert!(
        repo.record_evidence(admin, &endpoint.id, &evidence)
            .await
            .is_err()
    );
    evidence.address_family = Some(AddressFamily::Ipv6);
    repo.record_evidence(admin, &endpoint.id, &evidence)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='suspended' WHERE id=?")
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.list(admin).await.unwrap().is_empty());
    assert!(
        repo.update(admin, &endpoint.id, 1, &endpoint.config)
            .await
            .is_err()
    );
    assert!(repo.delete(admin, &endpoint.id, 1).await.is_err());
    assert!(
        repo.record_evidence(admin, &endpoint.id, &evidence)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn diagnosis_lease_is_atomic_bounded_durable_and_revision_checked() {
    let (pool, admin, alice, _, site, _) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool.clone());
    let endpoint = repo.create(admin, &web(site)).await.unwrap();
    assert!(
        repo.claim_diagnosis(alice, &endpoint.id, 1, "tenant")
            .await
            .is_err()
    );
    assert!(
        repo.claim_diagnosis(admin, &endpoint.id, 2, "stale")
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        repo.claim_diagnosis(admin, &endpoint.id, 1, "one"),
        repo.claim_diagnosis(admin, &endpoint.id, 1, "two")
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let token = if a.is_ok() { "one" } else { "two" };
    let mut evidence = report(1);
    evidence.observation_point = ObservationPoint::LocalServer;
    evidence.kind = EvidenceKind::Dns;
    evidence.observer_id = "server-resolver".into();
    assert!(
        repo.finish_diagnosis(admin, &endpoint.id, 1, "wrong", &[evidence.clone()])
            .await
            .is_err()
    );
    repo.finish_diagnosis(admin, &endpoint.id, 1, token, &[evidence.clone()])
        .await
        .unwrap();
    let repo = SqliteConnectivityRepository::new(pool.clone());
    assert!(
        repo.claim_diagnosis(admin, &endpoint.id, 1, "retry")
            .await
            .is_err()
    );
    assert!(
        repo.finish_diagnosis(admin, &endpoint.id, 1, token, &[evidence])
            .await
            .is_err()
    );
    let history = repo.evidence(admin, &endpoint.id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].current);
    // Expire cooldown, then edit during an active run: reject the whole batch.
    sqlx::query("UPDATE connectivity_diagnosis_leases SET cooldown_until=0 WHERE endpoint_id=?")
        .bind(&endpoint.id)
        .execute(&pool)
        .await
        .unwrap();
    repo.claim_diagnosis(admin, &endpoint.id, 1, "edit-race")
        .await
        .unwrap();
    let mut cfg = web(site);
    cfg.port = NonZeroU16::new(9443).unwrap();
    repo.update(admin, &endpoint.id, 1, &cfg).await.unwrap();
    let mut evidence = report(1);
    evidence.kind = EvidenceKind::Dns;
    evidence.observation_point = ObservationPoint::LocalServer;
    assert!(
        repo.finish_diagnosis(admin, &endpoint.id, 1, "edit-race", &[evidence])
            .await
            .is_err()
    );
    assert_eq!(repo.evidence(admin, &endpoint.id).await.unwrap().len(), 1);
    assert!(!repo.evidence(admin, &endpoint.id).await.unwrap()[0].current);
}
#[tokio::test]
async fn diagnosis_global_limit_and_revocation_prevent_network_work_or_partial_results() {
    let (pool, admin, _, _, site, _) = fixture().await;
    let repo = SqliteConnectivityRepository::new(pool.clone());
    let mut ids = vec![];
    for port in [8443, 9443, 10443] {
        let mut cfg = web(site);
        cfg.primary = false;
        cfg.port = NonZeroU16::new(port).unwrap();
        ids.push(repo.create(admin, &cfg).await.unwrap().id);
    }
    repo.claim_diagnosis(admin, &ids[0], 1, "one")
        .await
        .unwrap();
    repo.claim_diagnosis(admin, &ids[1], 1, "two")
        .await
        .unwrap();
    assert!(
        repo.claim_diagnosis(admin, &ids[2], 1, "three")
            .await
            .is_err()
    );
    sqlx::query("UPDATE connectivity_diagnosis_leases SET lease_until=0 WHERE endpoint_id=?")
        .bind(&ids[0])
        .execute(&pool)
        .await
        .unwrap();
    repo.claim_diagnosis(admin, &ids[2], 1, "three")
        .await
        .unwrap();
    let mut evidence = report(1);
    evidence.kind = EvidenceKind::Dns;
    evidence.observation_point = ObservationPoint::LocalServer;
    let bad = report(1);
    assert!(
        repo.finish_diagnosis(admin, &ids[2], 1, "three", &[evidence.clone(), bad])
            .await
            .is_err()
    );
    assert!(repo.evidence(admin, &ids[2]).await.unwrap().is_empty());
    sqlx::query("UPDATE users SET status='suspended' WHERE id=?")
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        repo.finish_diagnosis(admin, &ids[2], 1, "three", &[evidence])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn diagnosis_migration_preserves_existing_evidence_sequence_and_foreign_keys() {
    let (pool, admin, _, _, site, _) = fixture_before(Some(20261020000002)).await;
    let repo = SqliteConnectivityRepository::new(pool.clone());
    let endpoint = repo.create(admin, &web(site)).await.unwrap();
    let first = repo
        .record_evidence(admin, &endpoint.id, &report(1))
        .await
        .unwrap();
    let deleted = repo
        .record_evidence(admin, &endpoint.id, &report(1))
        .await
        .unwrap();
    sqlx::query("DELETE FROM endpoint_evidence WHERE id=?")
        .bind(deleted)
        .execute(&pool)
        .await
        .unwrap();
    let before = repo.evidence(admin, &endpoint.id).await.unwrap();
    run_migrations(&pool).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let after = repo.evidence(admin, &endpoint.id).await.unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, first);
    assert_eq!(after[0].evidence, before[0].evidence);
    let mut evidence = report(1);
    evidence.kind = EvidenceKind::Firewall;
    evidence.observation_point = ObservationPoint::LocalServer;
    evidence.outcome = EvidenceOutcome::Unknown;
    evidence.reason = EvidenceReason::LocalGuardOnly;
    let next = repo
        .record_evidence(admin, &endpoint.id, &evidence)
        .await
        .unwrap();
    assert!(next > deleted);
    let foreign_keys = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(foreign_keys.is_empty());
    repo.claim_diagnosis(admin, &endpoint.id, 1, "delete")
        .await
        .unwrap();
    repo.delete(admin, &endpoint.id, 1).await.unwrap();
    for table in ["endpoint_evidence", "connectivity_diagnosis_leases"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
