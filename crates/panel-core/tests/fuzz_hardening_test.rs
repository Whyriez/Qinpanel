#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_core::cron::validate_cron_expression;
use panel_core::domain::DomainName;
use panel_core::hardening::{
    LOOPBACK_PORT_MAX, LOOPBACK_PORT_MIN, evaluate_systemd_service_security,
    validate_loopback_isolation_binding,
};
use panel_core::secret_rotation::DualKeyRotationVault;

#[test]
fn test_fuzz_loopback_port_validation_boundary_matrix() {
    // 1. Boundary ports
    assert!(validate_loopback_isolation_binding(1, 1000, LOOPBACK_PORT_MIN).is_ok());
    assert!(validate_loopback_isolation_binding(1, 1000, LOOPBACK_PORT_MAX).is_ok());
    assert!(validate_loopback_isolation_binding(1, 1000, 25000).is_ok());

    // 2. Off-by-one below and above
    assert!(validate_loopback_isolation_binding(1, 1000, LOOPBACK_PORT_MIN - 1).is_err());
    assert!(validate_loopback_isolation_binding(1, 1000, LOOPBACK_PORT_MAX + 1).is_err());

    // 3. Well-known and dangerous ports
    for p in [0, 22, 80, 443, 3306, 5432, 6379, 8080, 65535] {
        assert!(
            validate_loopback_isolation_binding(1, 1000, p).is_err(),
            "Port {} should be rejected",
            p
        );
    }

    // 4. Pseudo-random fuzzing across wide range
    for p in (1..=65535).step_by(137) {
        let res = validate_loopback_isolation_binding(1, 1001, p);
        if (LOOPBACK_PORT_MIN..=LOOPBACK_PORT_MAX).contains(&p) {
            assert!(res.is_ok(), "Port {} in range should be accepted", p);
        } else {
            assert!(res.is_err(), "Port {} out of range should be rejected", p);
        }
    }

    // 5. UID boundaries: root (0) and system (<1000) must be rejected
    for uid in [0, 1, 33, 999] {
        assert!(validate_loopback_isolation_binding(1, uid, 20050).is_err());
    }
    for uid in [1000, 1001, 50000, 65534] {
        assert!(validate_loopback_isolation_binding(1, uid, 20050).is_ok());
    }
}

#[test]
fn test_property_domain_validation_invariants() {
    let valid_samples = [
        "example.com",
        "sub.example.com",
        "api-v2.service.id",
        "my-site123.org",
        "a.b.c.d.co.uk",
    ];
    for d in valid_samples {
        assert!(
            DomainName::parse(d).is_ok(),
            "Domain '{}' should be valid",
            d
        );
    }

    let invalid_samples = [
        "",
        ".",
        "..",
        "-example.com",
        "example-.com",
        "ex..ample.com",
        "http://example.com",
        "example.com/path",
        "example.com:8080",
        "domain with spaces.com",
        "*.example.com",
        "alert('xss').com",
        "example;drop table sites;.com",
    ];
    for d in invalid_samples {
        assert!(
            DomainName::parse(d).is_err(),
            "Domain '{}' should be invalid",
            d
        );
    }
}

#[test]
fn test_property_cron_validation_invariants() {
    let valid_crons = [
        "* * * * *",
        "0 * * * *",
        "*/15 * * * *",
        "0 0 * * *",
        "30 2 * * 1-5",
        "0 12 1,15 * *",
    ];
    for c in valid_crons {
        assert!(
            validate_cron_expression(c).is_ok(),
            "Cron '{}' should be valid",
            c
        );
    }

    let invalid_crons = [
        "",
        "* * * *",      // 4 fields (too short)
        "* * * * * *",  // 6 fields (too long)
        "60 * * * *",   // minute out of range
        "* 25 * * *",   // hour out of range
        "* * 32 * *",   // dom out of range
        "* * * 13 *",   // month out of range
        "* * * * 8",    // dow out of range
        "; rm -rf / ;", // command injection attempt
        "$(whoami) * * * *",
    ];
    for c in invalid_crons {
        assert!(
            validate_cron_expression(c).is_err(),
            "Cron '{}' should be invalid",
            c
        );
    }
}

#[test]
fn test_fuzz_dual_key_secret_vault_roundtrips() {
    for seed in 0u8..30u8 {
        let key1 = [seed; 32];
        let mut vault = DualKeyRotationVault::new(key1);

        let payload = format!("SECRET_DATA_PAYLOAD_TEST_SEED_{}", seed);
        let enc1 = vault.encrypt(payload.as_bytes()).expect("encrypt");
        let (dec1, out1) = vault.decrypt(&enc1).expect("decrypt");
        assert_eq!(dec1, payload.as_bytes());
        assert_eq!(
            out1,
            panel_core::secret_rotation::DecryptionOutcome::PrimaryKey
        );

        // Rotate key
        let key2 = [seed.wrapping_add(100); 32];
        vault.rotate(key2);

        // Decrypt old with fallback
        let (dec_fallback, out_fallback) = vault.decrypt(&enc1).expect("fallback decrypt");
        assert_eq!(dec_fallback, payload.as_bytes());
        assert_eq!(
            out_fallback,
            panel_core::secret_rotation::DecryptionOutcome::FallbackSecondaryKey
        );
    }
}

#[test]
fn test_systemd_scoring_robustness_with_arbitrary_units() {
    let empty_unit = "";
    let score_empty = evaluate_systemd_service_security("empty.service", empty_unit);
    assert!(!score_empty.sandboxed);
    assert!(score_empty.exposure_score > 2.0);

    let partially_hardened = r#"
[Service]
NoNewPrivileges=true
ProtectSystem=strict
PrivateTmp=true
"#;
    let score_partial = evaluate_systemd_service_security("partial.service", partially_hardened);
    assert!(score_partial.exposure_score < score_empty.exposure_score);
}
