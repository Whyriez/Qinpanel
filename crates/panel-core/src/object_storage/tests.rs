use super::{capability::*, identity::*, network::*, quota::*, *};
use crate::{connectivity::EndpointHost, rbac::Role};
use std::{net::IpAddr, num::NonZeroU16};
use uuid::Uuid;

fn tenant(id: i64) -> TenantId {
    id.try_into().expect("tenant fixture")
}
fn project(id: i64) -> ProjectId {
    id.try_into().expect("project fixture")
}
fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("UUID fixture")
}
fn admin() -> StorageActor {
    StorageActor::from_authenticated(1, Role::Admin, true).expect("admin fixture")
}
fn actor(id: i64) -> StorageActor {
    StorageActor::from_authenticated(id, Role::User, true).expect("actor fixture")
}
fn endpoint(host: &str) -> StorageEndpoint {
    StorageEndpoint {
        host: EndpointHost::parse(host).expect("host fixture"),
        port: NonZeroU16::new(443).expect("port"),
    }
}
fn panel() -> StorageEndpoint {
    endpoint("panel.example.net")
}

fn connection(kind: ProviderKind) -> StorageConnection {
    let target = match kind {
        ProviderKind::SeaweedFs450 => StorageTarget::Managed {
            instance_id: uuid("44444444-4444-4444-8444-444444444444")
                .try_into()
                .expect("instance"),
            instance_revision: 5,
        },
        ProviderKind::AwsS3 => StorageTarget::External {
            provider: ExternalProvider::AwsS3,
        },
        ProviderKind::CloudflareR2 => StorageTarget::External {
            provider: ExternalProvider::CloudflareR2,
        },
        ProviderKind::S3Compatible => StorageTarget::External {
            provider: ExternalProvider::S3Compatible,
        },
    };
    let credential = if kind == ProviderKind::SeaweedFs450 {
        None
    } else {
        Some(CredentialRef {
            id: uuid("33333333-3333-4333-8333-333333333333")
                .try_into()
                .expect("credential"),
            owner: tenant(101),
            revision: 7,
        })
    };
    StorageConnection::new(
        admin(),
        uuid("11111111-1111-4111-8111-111111111111")
            .try_into()
            .expect("connection"),
        3,
        tenant(101),
        ConnectionConfig {
            target,
            endpoint: endpoint("s3.storage.example.net"),
            region: if kind == ProviderKind::CloudflareR2 {
                "auto"
            } else {
                "us-east-1"
            }
            .into(),
            addressing: AddressingStyle::Path,
            credential,
        },
    )
    .expect("connection fixture")
}
fn bucket(connection: &StorageConnection) -> StorageBucket {
    StorageBucket::new(
        actor(101),
        connection,
        uuid("22222222-2222-4222-8222-222222222222")
            .try_into()
            .expect("bucket"),
        "project-media".to_string().try_into().expect("name"),
        Some(ProjectIdentity {
            id: project(202),
            tenant_id: tenant(101),
        }),
    )
    .expect("bucket fixture")
}
fn report(kind: ProviderKind) -> AdapterProbeReport {
    let bytes: &[u8] = match kind {
        ProviderKind::SeaweedFs450 => include_bytes!("fixtures/managed.json"),
        ProviderKind::AwsS3 => include_bytes!("fixtures/aws.json"),
        ProviderKind::CloudflareR2 => include_bytes!("fixtures/r2.json"),
        ProviderKind::S3Compatible => include_bytes!("fixtures/generic.json"),
    };
    AdapterProbeReport::parse_fixture(bytes).expect("normalized fixture")
}
fn ip(value: &str) -> IpAddr {
    value.parse().expect("IP fixture")
}

#[test]
fn identifiers_are_validated_during_json_deserialization() {
    for invalid in [0, -1, i64::MIN] {
        assert!(TenantId::try_from(invalid).is_err());
        assert!(serde_json::from_value::<ProjectId>(serde_json::json!(invalid)).is_err());
    }
    assert!(ConnectionId::try_from(Uuid::nil()).is_err());
    assert!(serde_json::from_value::<BucketId>(serde_json::json!(Uuid::nil())).is_err());
    assert!(serde_json::from_value::<ConnectionId>(serde_json::json!("not-an-id")).is_err());
}

#[test]
fn portable_bucket_names_preserve_names_and_reject_reserved_or_unsafe_names() {
    for name in ["project-media", "abc", "images.example", "tenant-101-202"] {
        let parsed: BucketName = name.to_owned().try_into().expect("valid name");
        assert_eq!(parsed.as_str(), name);
    }
    for name in [
        "",
        "ab",
        "UPPER",
        "a_b",
        "-abc",
        "abc-",
        "a..b",
        "192.168.0.1",
        "001.002.003.004",
        "xn--bucket",
        "sthree-bucket",
        "amzn-s3-demo-media",
        "abc-s3alias",
        "abc--ol-s3",
        "abc.mrap",
        "abc--x-s3",
        "abc--table-s3",
        "abc-an",
        "../etc/passwd",
        "abc\n",
        "a/b",
    ] {
        assert!(BucketName::try_from(name.to_owned()).is_err(), "{name:?}");
    }
    assert!(BucketName::try_from("a".repeat(64)).is_err());
}

#[test]
fn object_keys_are_opaque_and_not_host_filesystem_paths() {
    for value in [
        "media/foto produk.jpg",
        "arsip/日本語.pdf",
        "%2e%2e%2fdata",
        "prefix//file",
        "/leading-key",
    ] {
        assert_eq!(
            ObjectKey::try_from(value.to_owned())
                .expect("portable key")
                .as_str(),
            value
        );
    }
    for value in ["", "../secret", "a/./b", "a/../b", "a\\b", "a\0b", "a\nb"] {
        assert!(ObjectKey::try_from(value.to_owned()).is_err());
    }
    assert!(ObjectKey::try_from("é".repeat(513)).is_err());
}

#[test]
fn connections_require_admin_and_credentials_belong_to_the_owner() {
    let base = connection(ProviderKind::AwsS3);
    assert_eq!(
        StorageConnection::new(actor(101), base.id, 3, base.owner, base.config.clone()),
        Err(StorageError::Forbidden)
    );
    let mut config = base.config;
    config.credential.as_mut().expect("credential").owner = tenant(102);
    assert_eq!(
        config.validate(tenant(101)),
        Err(StorageError::InvalidConnection)
    );
    assert!(StorageActor::from_authenticated(101, Role::User, false).is_err());
    assert!(StorageActor::from_authenticated(101, Role::Reseller, true).is_err());
}

#[test]
fn json_config_cannot_accept_raw_secrets_paths_or_insecure_flags() {
    let base = connection(ProviderKind::AwsS3);
    for (field, value) in [
        (
            "secret_access_key",
            serde_json::json!("fixture-not-a-secret"),
        ),
        ("data_path", serde_json::json!("/root")),
        ("allow_private", serde_json::json!(true)),
        ("insecure", serde_json::json!(true)),
    ] {
        let draft = ConnectionDraft {
            target: base.config.target,
            endpoint: base.config.endpoint.clone(),
            region: base.config.region.clone(),
            addressing: base.config.addressing,
            credential_id: base.config.credential.map(|c| c.id),
        };
        let mut raw = serde_json::to_value(draft).expect("JSON");
        raw[field] = value;
        assert!(serde_json::from_value::<ConnectionDraft>(raw).is_err());
    }
    let serialized = serde_json::to_string(&base).expect("JSON");
    assert!(!serialized.contains("secret_access_key"));
    assert!(!serialized.contains("access_key_id"));
}

#[test]
fn provider_modes_validate_configuration_without_assuming_capabilities() {
    let mut r2 = connection(ProviderKind::CloudflareR2).config;
    r2.region = "us-east-1".into();
    assert!(r2.validate(tenant(101)).is_err());
    let mut external = connection(ProviderKind::AwsS3).config;
    external.credential = None;
    assert!(external.validate(tenant(101)).is_err());
    let managed = connection(ProviderKind::SeaweedFs450);
    assert_eq!(
        managed.config.target.managed_engine(),
        Some(("seaweedfs", MANAGED_ENGINE_VERSION, MANAGED_ENGINE_COMMIT))
    );
    let mut config = managed.config;
    config.target = StorageTarget::Managed {
        instance_id: uuid("44444444-4444-4444-8444-444444444444")
            .try_into()
            .expect("instance"),
        instance_revision: 0,
    };
    assert!(config.validate(tenant(101)).is_err());
}

#[test]
fn project_and_bucket_identity_cannot_cross_tenants_even_for_admin() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = bucket(&connection);
    assert_eq!(bucket.visibility, BucketVisibility::Private);
    assert_eq!(
        bucket.require_project(ProjectIdentity {
            id: project(202),
            tenant_id: tenant(102)
        }),
        Err(StorageError::Forbidden)
    );
    assert_eq!(
        bucket.require_project(ProjectIdentity {
            id: project(203),
            tenant_id: tenant(101)
        }),
        Err(StorageError::Forbidden)
    );
    assert_eq!(
        StorageBucket::new(
            admin(),
            &connection,
            bucket.id,
            bucket.name.clone(),
            Some(ProjectIdentity {
                id: project(202),
                tenant_id: tenant(102)
            })
        ),
        Err(StorageError::Forbidden)
    );
    assert_eq!(
        StorageBucket::new(actor(102), &connection, bucket.id, bucket.name, None),
        Err(StorageError::Forbidden)
    );
}

#[test]
fn dotted_buckets_require_path_style_for_the_supported_tls_profile() {
    let mut connection = connection(ProviderKind::AwsS3);
    connection.config.addressing = AddressingStyle::VirtualHost;
    let bucket = bucket(&connection);
    assert!(
        StorageBucket::new(
            actor(101),
            &connection,
            bucket.id,
            "images.example".to_owned().try_into().expect("name"),
            None
        )
        .is_err()
    );
}

#[test]
fn four_provider_fixtures_bind_capabilities_to_the_exact_metadata() {
    for kind in [
        ProviderKind::SeaweedFs450,
        ProviderKind::AwsS3,
        ProviderKind::CloudflareR2,
        ProviderKind::S3Compatible,
    ] {
        let connection = connection(kind);
        let bucket = bucket(&connection);
        let report = report(kind);
        assert_eq!(
            configuration_fingerprint(&connection, &bucket).expect("fingerprint"),
            report.configuration_fingerprint
        );
        let intent = authorize_operation(
            actor(101),
            &connection,
            &bucket,
            &report,
            StorageAction::MultipartUpload,
            1001,
        )
        .expect("fixture-authorized intent");
        assert_eq!(intent.bucket_id(), bucket.id);
        assert_eq!(intent.action(), StorageAction::MultipartUpload);
        assert!(intent.require_current(&connection, &bucket, 1002).is_ok());
        assert_eq!(
            intent.require_current(&connection, &bucket, 999),
            Err(StorageError::StaleEvidence)
        );
        assert_eq!(
            intent.require_current(&connection, &bucket, 1300),
            Err(StorageError::StaleEvidence)
        );
    }
}

#[test]
fn declarations_and_unknown_or_missing_observations_cannot_grant_permissions() {
    let connection = connection(ProviderKind::S3Compatible);
    let bucket = bucket(&connection);
    let contract = ProviderContract::for_connection(&connection);
    assert_eq!(
        contract.declared_support(StorageAction::ReadObject),
        CapabilityState::Unknown
    );
    let mut report = report(ProviderKind::S3Compatible);
    report.checks.clear();
    assert_eq!(
        authorize_operation(
            actor(101),
            &connection,
            &bucket,
            &report,
            StorageAction::ReadObject,
            1001
        ),
        Err(StorageError::Unverified)
    );
    report.checks.push(CapabilityCheck {
        action: StorageAction::ReadObject,
        state: CapabilityState::Unknown,
    });
    assert_eq!(
        authorize_operation(
            actor(101),
            &connection,
            &bucket,
            &report,
            StorageAction::ReadObject,
            1001
        ),
        Err(StorageError::Unverified)
    );
    report.checks[0].state = CapabilityState::Unsupported;
    assert_eq!(
        authorize_operation(
            actor(101),
            &connection,
            &bucket,
            &report,
            StorageAction::ReadObject,
            1001
        ),
        Err(StorageError::Unsupported)
    );
}

#[test]
fn stale_or_foreign_reports_are_rejected_before_they_can_be_used() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = bucket(&connection);
    for field in [
        "connection_revision",
        "credential_revision",
        "bucket_revision",
        "provider",
        "fingerprint",
    ] {
        let mut report = report(ProviderKind::AwsS3);
        match field {
            "connection_revision" => report.connection_revision += 1,
            "credential_revision" => report.credential_revision += 1,
            "bucket_revision" => report.bucket_revision += 1,
            "provider" => report.provider = ProviderKind::S3Compatible,
            _ => report.configuration_fingerprint = "a".repeat(64),
        }
        assert_eq!(
            report.require_current(&connection, &bucket, 1001),
            Err(StorageError::StaleEvidence)
        );
    }
    let report = report(ProviderKind::AwsS3);
    for now in [999, 1300, u64::MAX] {
        assert_eq!(
            report.require_current(&connection, &bucket, now),
            Err(StorageError::StaleEvidence)
        );
    }
    assert_eq!(
        authorize_operation(
            actor(102),
            &connection,
            &bucket,
            &report,
            StorageAction::ReadObject,
            1001
        ),
        Err(StorageError::Forbidden)
    );
}

#[test]
fn configuration_changes_without_revision_bumps_still_invalidate_evidence_and_intents() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = bucket(&connection);
    let report = report(ProviderKind::AwsS3);
    let intent = authorize_operation(
        actor(101),
        &connection,
        &bucket,
        &report,
        StorageAction::ReadObject,
        1001,
    )
    .expect("intent");
    for field in [
        "host",
        "region",
        "credential",
        "bucket_name",
        "project",
        "visibility",
    ] {
        let mut connection = connection.clone();
        let mut bucket = bucket.clone();
        match field {
            "host" => connection.config.endpoint = endpoint("different.storage.example.net"),
            "region" => connection.config.region = "eu-west-1".into(),
            "credential" => {
                connection
                    .config
                    .credential
                    .as_mut()
                    .expect("credential")
                    .id = uuid("55555555-5555-4555-8555-555555555555")
                    .try_into()
                    .expect("new credential")
            }
            "bucket_name" => {
                bucket.name = "other-media".to_owned().try_into().expect("bucket name")
            }
            "project" => bucket.project_id = Some(project(203)),
            _ => bucket.visibility = BucketVisibility::PublicRead,
        }
        assert_eq!(
            report.require_current(&connection, &bucket, 1001),
            Err(StorageError::StaleEvidence)
        );
        assert_eq!(
            intent.require_current(&connection, &bucket, 1001),
            Err(StorageError::StaleEvidence)
        );
    }
}

#[test]
fn malformed_duplicate_or_unpinned_reports_are_not_accepted() {
    let base = report(ProviderKind::SeaweedFs450);
    for field in [
        "schema",
        "expiry",
        "commit",
        "duplicate",
        "unknown",
        "fingerprint",
    ] {
        let mut raw = serde_json::to_value(&base).expect("JSON");
        match field {
            "schema" => raw["schema_version"] = serde_json::json!(99),
            "expiry" => raw["expires_at"] = serde_json::json!(1301),
            "commit" => raw["engine_commit"] = serde_json::json!("latest"),
            "duplicate" => {
                let first = raw["checks"][0].clone();
                raw["checks"].as_array_mut().expect("checks").push(first);
            }
            "unknown" => raw["secret_access_key"] = serde_json::json!("not-a-secret"),
            _ => raw["configuration_fingerprint"] = serde_json::json!("invalid"),
        }
        assert!(
            AdapterProbeReport::parse_fixture(&serde_json::to_vec(&raw).expect("JSON bytes"))
                .is_err()
        );
    }
    assert!(AdapterProbeReport::parse_fixture(&vec![b' '; 65_537]).is_err());
}

#[test]
fn s3_credentials_do_not_imply_key_management_or_publication_permission() {
    let connection = connection(ProviderKind::SeaweedFs450);
    let bucket = bucket(&connection);
    let mut report = report(ProviderKind::SeaweedFs450);
    for action in [StorageAction::IssueKey, StorageAction::PublicRead] {
        report.checks.push(CapabilityCheck {
            action,
            state: CapabilityState::Supported,
        });
        assert_eq!(
            authorize_operation(admin(), &connection, &bucket, &report, action, 1001),
            Err(StorageError::Unsupported)
        );
    }
}

fn key_grant(connection: &StorageConnection, bucket: &StorageBucket) -> BucketKeyGrant {
    BucketKeyGrant {
        credential: CredentialRef {
            id: uuid("55555555-5555-4555-8555-555555555555")
                .try_into()
                .expect("credential"),
            owner: bucket.owner,
            revision: 2,
        },
        connection_id: connection.id,
        bucket_id: bucket.id,
        project_id: bucket.project_id,
        permissions: KeyPermissions {
            read: true,
            write: false,
            delete: false,
        },
        expires_at: Some(1200),
        revoked: false,
    }
}

#[test]
fn bucket_key_scope_is_project_specific_and_never_grants_control_plane_actions() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = bucket(&connection);
    let grant = key_grant(&connection, &bucket);
    let active_credential = grant.credential;
    let project_context = ProjectIdentity {
        id: project(202),
        tenant_id: tenant(101),
    };
    assert!(
        grant
            .authorize_metadata(
                &connection,
                &bucket,
                &active_credential,
                Some(project_context),
                StorageAction::ReadObject,
                1001
            )
            .is_ok()
    );
    for action in [
        StorageAction::WriteObject,
        StorageAction::DeleteObject,
        StorageAction::CreateBucket,
        StorageAction::IssueKey,
    ] {
        assert_eq!(
            grant.authorize_metadata(
                &connection,
                &bucket,
                &active_credential,
                Some(project_context),
                action,
                1001
            ),
            Err(StorageError::Forbidden)
        );
    }
    assert!(
        grant
            .authorize_metadata(
                &connection,
                &bucket,
                &active_credential,
                None,
                StorageAction::ReadObject,
                1001
            )
            .is_err()
    );
    assert!(
        grant
            .authorize_metadata(
                &connection,
                &bucket,
                &active_credential,
                Some(ProjectIdentity {
                    id: project(203),
                    tenant_id: tenant(101)
                }),
                StorageAction::ReadObject,
                1001
            )
            .is_err()
    );
    assert!(
        grant
            .authorize_metadata(
                &connection,
                &bucket,
                &active_credential,
                Some(ProjectIdentity {
                    id: project_context.id,
                    tenant_id: tenant(102)
                }),
                StorageAction::ReadObject,
                1001
            )
            .is_err()
    );
}

#[test]
fn revoked_expired_or_rebound_key_metadata_cannot_authorize_an_operation() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = bucket(&connection);
    let project = ProjectIdentity {
        id: project(202),
        tenant_id: tenant(101),
    };
    let active_credential = key_grant(&connection, &bucket).credential;
    for field in [
        "revoked",
        "expiry",
        "owner",
        "credential_revision",
        "rotated",
        "bucket",
        "project",
    ] {
        let mut grant = key_grant(&connection, &bucket);
        match field {
            "revoked" => grant.revoked = true,
            "expiry" => grant.expires_at = Some(1001),
            "owner" => grant.credential.owner = tenant(102),
            "credential_revision" => grant.credential.revision = 0,
            "rotated" => grant.credential.revision += 1,
            "bucket" => {
                grant.bucket_id = uuid("55555555-5555-4555-8555-555555555555")
                    .try_into()
                    .expect("other bucket")
            }
            _ => grant.project_id = None,
        }
        assert_eq!(
            grant.authorize_metadata(
                &connection,
                &bucket,
                &active_credential,
                Some(project),
                StorageAction::ReadObject,
                1001
            ),
            Err(StorageError::Forbidden)
        );
    }
}

#[test]
fn provider_periodic_and_external_limits_cannot_be_promoted_to_hard_quota() {
    for kind in [
        ProviderKind::SeaweedFs450,
        ProviderKind::AwsS3,
        ProviderKind::CloudflareR2,
        ProviderKind::S3Compatible,
    ] {
        let connection = connection(kind);
        let bucket = bucket(&connection);
        let provider = ProviderContract::for_connection(&connection);
        assert_eq!(
            StorageQuota::RequireProviderHard { limit_bytes: 1000 }.validate(&provider),
            Err(StorageError::HardQuotaUnavailable)
        );
        assert_eq!(
            StorageQuota::Observe.check_panel_upload(&connection, &bucket, None, u64::MAX, 1001),
            Ok(UploadQuotaResult::NotEnforced)
        );
    }
    assert_eq!(
        ProviderContract::for_connection(&connection(ProviderKind::SeaweedFs450))
            .quota_enforcement(),
        QuotaEnforcement::ProviderPeriodic
    );
}

#[test]
fn quota_admission_counts_reserved_parts_and_checks_addition_without_overflow() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = bucket(&connection);
    let quota = StorageQuota::PanelUploads { limit_bytes: 1000 };
    let usage =
        UsageSample::from_measurement(&connection, &bucket, 500, 200, 1000).expect("usage fixture");
    assert_eq!(
        quota.check_panel_upload(&connection, &bucket, Some(usage.clone()), 300, 1001),
        Ok(UploadQuotaResult::PanelCheckOnly { remaining_bytes: 0 })
    );
    for incoming in [301, u64::MAX] {
        assert_eq!(
            quota.check_panel_upload(&connection, &bucket, Some(usage.clone()), incoming, 1001),
            Err(StorageError::QuotaExceeded)
        );
    }
    let overflow = UsageSample::from_measurement(&connection, &bucket, u64::MAX, 1, 1000)
        .expect("usage fixture");
    assert_eq!(
        quota.check_panel_upload(&connection, &bucket, Some(overflow), 0, 1001),
        Err(StorageError::QuotaExceeded)
    );
}

#[test]
fn unknown_or_stale_usage_does_not_turn_into_zero_usage() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = bucket(&connection);
    let quota = StorageQuota::PanelUploads { limit_bytes: 1000 };
    assert_eq!(
        quota.check_panel_upload(&connection, &bucket, None, 1, 1001),
        Err(StorageError::UnknownUsage)
    );
    assert_eq!(
        UsageSample::from_measurement(&connection, &bucket, 0, 0, 0),
        Err(StorageError::UnknownUsage)
    );
    for time in [900, 1002] {
        let usage =
            UsageSample::from_measurement(&connection, &bucket, 0, 0, time).expect("usage fixture");
        assert_eq!(
            quota.check_panel_upload(&connection, &bucket, Some(usage), 1, 1001),
            Err(StorageError::UnknownUsage)
        );
    }
    assert!(
        StorageQuota::PanelUploads { limit_bytes: 0 }
            .validate(&ProviderContract::for_connection(&connection))
            .is_err()
    );
}

#[test]
fn usage_is_bound_to_the_exact_bucket_configuration_and_credential_epoch() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = bucket(&connection);
    let quota = StorageQuota::PanelUploads { limit_bytes: 1000 };
    let usage =
        UsageSample::from_measurement(&connection, &bucket, 0, 0, 1000).expect("usage fixture");
    let mut other_bucket = bucket.clone();
    other_bucket.id = uuid("66666666-6666-4666-8666-666666666666")
        .try_into()
        .expect("bucket ID");
    assert_eq!(
        quota.check_panel_upload(&connection, &other_bucket, Some(usage.clone()), 1, 1001),
        Err(StorageError::UnknownUsage)
    );
    other_bucket = bucket.clone();
    other_bucket.revision += 1;
    assert_eq!(
        quota.check_panel_upload(&connection, &other_bucket, Some(usage.clone()), 1, 1001),
        Err(StorageError::UnknownUsage)
    );
    let mut rotated = connection.clone();
    rotated
        .config
        .credential
        .as_mut()
        .expect("credential fixture")
        .revision += 1;
    assert_eq!(
        quota.check_panel_upload(&rotated, &bucket, Some(usage.clone()), 1, 1001),
        Err(StorageError::UnknownUsage)
    );
    let mut foreign = bucket.clone();
    foreign.owner = tenant(102);
    assert_eq!(
        UsageSample::from_measurement(&connection, &foreign, 0, 0, 1000),
        Err(StorageError::Forbidden)
    );
    assert_eq!(
        quota.check_panel_upload(&connection, &foreign, Some(usage), 1, 1001),
        Err(StorageError::Forbidden)
    );
}

#[test]
fn public_dns_targets_are_pinned_to_a_short_lived_origin_bound_result() {
    let connection = connection(ProviderKind::AwsS3);
    let targets = validate_dial_targets(
        &connection,
        &panel(),
        &[ip("8.8.8.8"), ip("2606:4700:4700::1111")],
        None,
        1000,
    )
    .expect("public fixture targets");
    assert_eq!(targets.addresses().len(), 2);
    assert_eq!(
        targets.endpoint().origin(),
        "https://s3.storage.example.net:443"
    );
    assert_eq!(targets.expires_at(), 1060);
    assert!(targets.require_current(&connection, 1001).is_ok());
    assert_eq!(
        targets.require_current(&connection, 999),
        Err(StorageError::StaleEvidence)
    );
    assert_eq!(
        targets.require_current(&connection, 1060),
        Err(StorageError::StaleEvidence)
    );
    let mut changed = connection;
    changed
        .config
        .credential
        .as_mut()
        .expect("credential")
        .revision += 1;
    assert_eq!(
        targets.require_current(&changed, 1001),
        Err(StorageError::StaleEvidence)
    );
}

#[test]
fn metadata_loopback_and_nonpublic_aliases_cannot_pass_as_public_endpoints() {
    let connection = connection(ProviderKind::AwsS3);
    for address in [
        "127.0.0.1",
        "169.254.169.254",
        "100.100.100.200",
        "168.63.129.16",
        "0.0.0.0",
        "224.0.0.1",
        "240.0.0.1",
        "192.0.2.1",
        "::1",
        "::",
        "fe80::1",
        "ff02::1",
        "::ffff:127.0.0.1",
        "::ffff:169.254.169.254",
        "2001:db8::1",
        "2002:7f00:1::1",
        "64:ff9b::7f00:1",
        "10.0.0.1",
        "fd00::1",
        "fd00:ec2::254",
        "fd20:ce::254",
    ] {
        assert!(
            validate_dial_targets(&connection, &panel(), &[ip(address)], None, 1000).is_err(),
            "{address}"
        );
    }
    assert!(
        validate_dial_targets(
            &connection,
            &panel(),
            &[ip("8.8.8.8"), ip("10.0.0.1")],
            None,
            1000
        )
        .is_err()
    );
    assert!(validate_dial_targets(&connection, &panel(), &[], None, 1000).is_err());
}

#[test]
fn private_endpoint_approval_requires_admin_and_exact_addresses_revision_and_origin() {
    let connection = connection(ProviderKind::S3Compatible);
    assert!(
        PrivateEndpointApproval::approve(actor(101), &connection, vec![ip("10.0.0.1")], 1000, 1200)
            .is_err()
    );
    let approval =
        PrivateEndpointApproval::approve(admin(), &connection, vec![ip("10.0.0.1")], 1000, 1200)
            .expect("scoped private fixture approval");
    assert!(
        validate_dial_targets(
            &connection,
            &panel(),
            &[ip("10.0.0.1")],
            Some(&approval),
            1001
        )
        .is_ok()
    );
    assert!(
        validate_dial_targets(
            &connection,
            &panel(),
            &[ip("10.0.0.2")],
            Some(&approval),
            1001
        )
        .is_err()
    );
    assert!(
        validate_dial_targets(
            &connection,
            &panel(),
            &[ip("10.0.0.1")],
            Some(&approval),
            1200
        )
        .is_err()
    );
    let mut changed = connection.clone();
    changed.revision += 1;
    assert!(
        validate_dial_targets(&changed, &panel(), &[ip("10.0.0.1")], Some(&approval), 1001)
            .is_err()
    );
    changed = connection.clone();
    changed.config.endpoint = endpoint("changed.storage.example.net");
    assert!(
        validate_dial_targets(&changed, &panel(), &[ip("10.0.0.1")], Some(&approval), 1001)
            .is_err()
    );
    assert!(
        PrivateEndpointApproval::approve(
            admin(),
            &connection,
            vec![ip("169.254.169.254")],
            1000,
            1200
        )
        .is_err()
    );
}

#[test]
fn endpoints_do_not_accept_urls_ambiguous_ip_literals_or_the_panel_origin() {
    for host in [
        "https://example.net",
        "user@example.net",
        "example.net/path",
        "example.net?key=value",
        "example.net:443",
        " example.net",
    ] {
        assert!(EndpointHost::parse(host).is_err(), "{host}");
    }
    let mut connection = connection(ProviderKind::AwsS3);
    connection.config.endpoint = panel();
    assert!(validate_dial_targets(&connection, &panel(), &[ip("8.8.8.8")], None, 1000).is_err());
    connection.config.endpoint = endpoint("8.8.8.8");
    assert!(validate_dial_targets(&connection, &panel(), &[ip("9.9.9.9")], None, 1000).is_err());
    assert!(validate_dial_targets(&connection, &panel(), &[ip("8.8.8.8")], None, 1000).is_ok());
    assert!(
        serde_json::from_value::<StorageEndpoint>(
            serde_json::json!({"host":"s3.example.net","port":0})
        )
        .is_err()
    );
}

#[test]
fn credential_id_is_resolved_against_server_ownership_and_not_request_claims() {
    let connection = connection(ProviderKind::AwsS3);
    let reference = connection.config.credential.expect("reference");
    let draft = ConnectionDraft {
        target: connection.config.target,
        endpoint: connection.config.endpoint.clone(),
        region: connection.config.region.clone(),
        addressing: connection.config.addressing,
        credential_id: Some(reference.id),
    };
    assert!(draft.clone().resolve(tenant(101), Some(reference)).is_ok());
    assert!(draft.clone().resolve(tenant(101), None).is_err());
    let mut foreign = reference;
    foreign.owner = tenant(102);
    assert!(draft.clone().resolve(tenant(101), Some(foreign)).is_err());
    let mut different = reference;
    different.id = uuid("55555555-5555-4555-8555-555555555555")
        .try_into()
        .expect("different ref");
    assert!(draft.clone().resolve(tenant(101), Some(different)).is_err());
    let mut raw = serde_json::to_value(draft).expect("JSON");
    raw["credential"] = serde_json::json!({"id":reference.id,"owner":101,"revision":7});
    assert!(serde_json::from_value::<ConnectionDraft>(raw).is_err());
}

#[test]
fn metadata_services_cannot_receive_private_approvals_even_for_an_admin() {
    let connection = connection(ProviderKind::S3Compatible);
    for address in [
        "169.254.169.254",
        "100.100.100.200",
        "168.63.129.16",
        "fd00:ec2::254",
        "fd20:ce::254",
        "::ffff:169.254.169.254",
    ] {
        assert!(
            PrivateEndpointApproval::approve(admin(), &connection, vec![ip(address)], 1000, 1200)
                .is_err(),
            "{address}"
        );
    }
}

#[test]
fn managed_instance_epoch_changes_invalidate_existing_capability_reports() {
    let mut connection = connection(ProviderKind::SeaweedFs450);
    let bucket = bucket(&connection);
    let report = report(ProviderKind::SeaweedFs450);
    let StorageTarget::Managed {
        instance_id,
        instance_revision,
    } = connection.config.target
    else {
        panic!("managed fixture");
    };
    connection.config.target = StorageTarget::Managed {
        instance_id,
        instance_revision: instance_revision + 1,
    };
    assert_eq!(
        report.require_current(&connection, &bucket, 1001),
        Err(StorageError::StaleEvidence)
    );
}

#[test]
fn tenant_bucket_keys_do_not_silently_gain_project_scopes() {
    let connection = connection(ProviderKind::AwsS3);
    let bucket = StorageBucket::new(
        actor(101),
        &connection,
        uuid("22222222-2222-4222-8222-222222222222")
            .try_into()
            .expect("bucket"),
        "shared-media".to_owned().try_into().expect("name"),
        None,
    )
    .expect("tenant bucket");
    let grant = key_grant(&connection, &bucket);
    assert!(
        grant
            .authorize_metadata(
                &connection,
                &bucket,
                &grant.credential,
                None,
                StorageAction::ReadObject,
                1001
            )
            .is_ok()
    );
    assert!(
        grant
            .authorize_metadata(
                &connection,
                &bucket,
                &grant.credential,
                Some(ProjectIdentity {
                    id: project(202),
                    tenant_id: tenant(101)
                }),
                StorageAction::ReadObject,
                1001
            )
            .is_err()
    );
}
