use super::*;

fn known(value: u64) -> CapacityMetric {
    CapacityMetric::measured(value, vec!["fixture".into()])
}

fn snapshot() -> CapacitySnapshot {
    CapacitySnapshot {
        schema_version: 1,
        observed_at: 1000,
        scope: "panel.slice".into(),
        cpu_quota_percent: known(400),
        memory_limit_bytes: known(8192 * MIB),
        memory_available_bytes: known(6144 * MIB),
        disk_total_bytes: known(100_000 * MIB),
        disk_available_bytes: known(60_000 * MIB),
        inodes_total: known(1_000_000),
        inodes_available: known(800_000),
        notices: vec![],
    }
}

fn personal() -> PackageDemand {
    PackageDemand {
        usage: PackageUse::Personal,
        projects_per_tenant: 1,
        tenant_slots: 1,
    }
}

#[test]
fn personal_preserves_headroom_and_does_not_reserve_all_host_resources() {
    let facts = snapshot();
    let preset = recommend_package(&facts, personal(), 1001).expect("valid preset");
    assert_eq!(preset.cpu_quota_percent, 360);
    assert_eq!(preset.cpu_headroom_percent, 40);
    assert_eq!(preset.max_sites, 1);
    assert!(preset.memory_high_bytes < preset.memory_limit_bytes);
    assert!(preset.memory_limit_bytes + preset.memory_headroom_bytes <= 6144 * MIB);
    assert_eq!(preset.expires_at, 1060);
    assert!(
        preset
            .notices
            .iter()
            .any(|v| v == "not_a_reservation_or_apply_authorization")
    );
}

#[test]
fn multi_project_does_not_multiply_one_tenants_budget_by_project_count() {
    let facts = snapshot();
    let single = recommend_package(&facts, personal(), 1001).expect("personal");
    let multi = recommend_package(
        &facts,
        PackageDemand {
            usage: PackageUse::MultiProject,
            projects_per_tenant: 8,
            tenant_slots: 1,
        },
        1001,
    )
    .expect("multi project");
    assert_eq!(single.memory_limit_bytes, multi.memory_limit_bytes);
    assert_eq!(single.cpu_quota_percent, multi.cpu_quota_percent);
    assert_eq!(multi.max_sites, 8);
    assert!(multi.tasks_max > single.tasks_max);
}

#[test]
fn equal_shares_stay_within_memory_disk_inode_and_cpu_budgets() {
    for slots in 2..=12 {
        let facts = snapshot();
        let demand = PackageDemand {
            usage: PackageUse::MultiTenant,
            projects_per_tenant: 2,
            tenant_slots: slots,
        };
        let preset = recommend_package(&facts, demand, 1001).expect("shared preset");
        let slots = u64::from(slots);
        assert!(preset.memory_limit_bytes * slots + preset.memory_headroom_bytes <= 6144 * MIB);
        assert!(preset.disk_limit_bytes * slots + preset.disk_headroom_bytes <= 60_000 * MIB);
        assert!(preset.cpu_quota_percent * slots + preset.cpu_headroom_percent <= 400);
        assert!(preset.inode_limit * slots <= 700_000);
        assert!(
            preset
                .notices
                .iter()
                .any(|v| v == "equal_shares_without_existing_plan_commitments")
        );
    }
}

#[test]
fn missing_invalid_or_empty_provenance_is_not_zero_or_unlimited() {
    let mut facts = snapshot();
    facts.cpu_quota_percent = CapacityMetric::unavailable("unreadable", vec![]);
    assert_eq!(
        recommend_package(&facts, personal(), 1001),
        Err(CapacityError::Unavailable)
    );
    facts = snapshot();
    facts.memory_limit_bytes.sources.clear();
    assert_eq!(
        recommend_package(&facts, personal(), 1001),
        Err(CapacityError::Unavailable)
    );
    facts = snapshot();
    facts.disk_available_bytes.unavailable_reason = Some("contradictory".into());
    assert_eq!(
        recommend_package(&facts, personal(), 1001),
        Err(CapacityError::Unavailable)
    );
}

#[test]
fn low_or_full_resources_do_not_produce_a_tiny_unsafe_preset() {
    for field in ["memory", "disk", "inodes", "cpu"] {
        let mut facts = snapshot();
        match field {
            "memory" => facts.memory_available_bytes = known(128 * MIB),
            "disk" => facts.disk_available_bytes = known(0),
            "inodes" => facts.inodes_available = known(0),
            _ => facts.cpu_quota_percent = known(10),
        }
        assert_eq!(
            recommend_package(&facts, personal(), 1001),
            Err(CapacityError::Insufficient)
        );
    }
}

#[test]
fn contradictory_capacity_and_unknown_schema_or_scope_are_rejected() {
    for field in ["memory", "disk", "inodes", "scope", "schema"] {
        let mut facts = snapshot();
        match field {
            "memory" => facts.memory_available_bytes = known(9000 * MIB),
            "disk" => facts.disk_available_bytes = known(200_000 * MIB),
            "inodes" => facts.inodes_available = known(2_000_000),
            "scope" => facts.scope = "panel-agent.service".into(),
            _ => facts.schema_version = 99,
        }
        assert_eq!(
            recommend_package(&facts, personal(), 1001),
            Err(CapacityError::InvalidMeasurement)
        );
    }
}

#[test]
fn expiry_and_clock_skew_require_a_new_snapshot() {
    for now in [999, 1061, u64::MAX] {
        assert_eq!(
            recommend_package(&snapshot(), personal(), now),
            Err(CapacityError::Stale)
        );
    }
    assert!(recommend_package(&snapshot(), personal(), 1060).is_ok());
    let mut facts = snapshot();
    facts.observed_at = 0;
    assert_eq!(
        recommend_package(&facts, personal(), 1),
        Err(CapacityError::Stale)
    );
    facts.observed_at = u64::MAX - 10;
    assert_eq!(
        recommend_package(&facts, personal(), u64::MAX - 10),
        Err(CapacityError::InvalidMeasurement)
    );
}

#[test]
fn contradictory_modes_counts_and_injected_fields_are_rejected() {
    for (usage, projects_per_tenant, tenant_slots) in [
        (PackageUse::Personal, 2, 1),
        (PackageUse::Personal, 1, 2),
        (PackageUse::MultiProject, 1, 1),
        (PackageUse::MultiProject, 2, 2),
        (PackageUse::MultiTenant, 1, 1),
        (PackageUse::MultiTenant, 0, 2),
        (PackageUse::MultiTenant, 257, 2),
        (PackageUse::MultiTenant, 1, 10001),
    ] {
        assert_eq!(
            recommend_package(
                &snapshot(),
                PackageDemand {
                    usage,
                    projects_per_tenant,
                    tenant_slots
                },
                1001
            ),
            Err(CapacityError::InvalidDemand)
        );
    }
    let malicious = serde_json::json!({"usage":"personal","projects_per_tenant":1,"tenant_slots":1,"shell_policy":"full"});
    assert!(serde_json::from_value::<PackageDemand>(malicious).is_err());
}

#[test]
fn cpu_list_intersections_do_not_count_disallowed_or_offline_cpus() {
    let online = parse_cpu_list("0-3,8-11\n").expect("online");
    let allowed = parse_cpu_list("2-5,10\n").expect("allowed");
    assert_eq!(intersect_cpu_count(&online, &allowed), 3);
    assert_eq!(parse_cpu_list("0").expect("one"), vec![(0, 0)]);
    for invalid in [
        "",
        "0-3,2",
        "0,0",
        "4-1",
        "-1",
        "0-1-2",
        "1,",
        "4294967295",
        "0-65536",
    ] {
        assert!(parse_cpu_list(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn cpu_quota_is_relative_to_one_core_and_rounds_down() {
    assert_eq!(parse_cpu_max("250000 100000"), Ok(Some(250)));
    assert_eq!(parse_cpu_max("1500 100000"), Ok(Some(1)));
    assert_eq!(parse_cpu_max("max 100000"), Ok(None));
    for invalid in [
        "max",
        "max 0",
        "100000 0",
        "0 100000",
        "-1 100000",
        "18446744073709551615 1",
        "1 1 extra",
    ] {
        assert!(parse_cpu_max(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn memavailable_is_required_not_a_magic_default_or_memfree_substitute() {
    assert_eq!(
        parse_memory("MemTotal: 8192 kB\nMemAvailable: 4096 kB\nSwapTotal: 999999 kB"),
        Ok((8192 * 1024, 4096 * 1024))
    );
    for invalid in [
        "",
        "MemTotal: 8192 kB\nMemFree: 4096 kB",
        "MemTotal: 8192 MB\nMemAvailable: 4 kB",
        "MemTotal: 8192 kB\nMemAvailable: 9000 kB",
        "MemTotal: 8192 kB\nMemAvailable: 4 kB\nMemAvailable: 3 kB",
        "MemTotal: 18446744073709551615 kB\nMemAvailable: 1 kB",
    ] {
        assert!(parse_memory(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn zero_usage_and_unlimited_limit_have_different_meanings() {
    assert_eq!(parse_limit("max\n"), Ok(None));
    assert_eq!(parse_limit("0\n"), Ok(Some(0)));
    assert!(parse_limit("unlimited").is_err());
}
