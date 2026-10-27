#![allow(clippy::unwrap_used, clippy::expect_used)]
use panel_core::connectivity::*;
use serde_json::json;
use std::num::NonZeroU16;

fn config() -> EndpointConfig {
    serde_json::from_value(json!({
        "service":"ssh", "scope":{"kind":"host"}, "transport":"direct",
        "host":"2001:db8::10", "port":2222, "tls_mode":"ssh",
        "certificate_id":null, "enabled":true, "primary":true
    }))
    .unwrap()
}

#[test]
fn hosts_are_canonical_and_authorities_preserve_ipv6_and_custom_ports() {
    let config = config();
    assert_eq!(config.host.authority(config.port), "[2001:db8::10]:2222");
    let host = EndpointHost::parse("2001:0db8:0:0::10").unwrap();
    assert_eq!(host.as_str(), "2001:db8::10");
    assert_eq!(
        EndpointHost::parse("203.0.113.10")
            .unwrap()
            .authority(NonZeroU16::new(17890).unwrap()),
        "203.0.113.10:17890"
    );
    assert_eq!(
        EndpointHost::parse("PANEL.Example.COM").unwrap().as_str(),
        "panel.example.com"
    );
    assert_eq!(
        EndpointHost::parse("café.example").unwrap().as_str(),
        "xn--caf-dma.example"
    );
    assert_eq!(
        EndpointHost::parse("panel.example")
            .unwrap()
            .authority(NonZeroU16::new(443).unwrap()),
        "panel.example:443"
    );
}

#[test]
fn untrusted_deserialization_cannot_bypass_host_port_or_typed_metadata() {
    let original = serde_json::to_value(config()).unwrap();
    for host in [
        "",
        " host.example",
        "host.example ",
        "https://host.example",
        "host.example:22",
        "[::1]",
        "fe80::1%eth0",
        "user@host.example",
        "host.example/path",
        "host.example\n",
        "a.example;shutdown",
        "999.0.0.1",
    ] {
        let mut value = original.clone();
        value["host"] = json!(host);
        assert!(
            serde_json::from_value::<EndpointConfig>(value).is_err(),
            "{host:?}"
        );
    }
    for port in [0, -1, 65536] {
        let mut value = original.clone();
        value["port"] = json!(port);
        assert!(serde_json::from_value::<EndpointConfig>(value).is_err());
    }
    for (field, value) in [
        ("secret_ref", json!("token")),
        ("client_requirement", json!("none")),
        ("service", json!("sqlserver")),
    ] {
        let mut payload = original.clone();
        payload[field] = value;
        assert!(serde_json::from_value::<EndpointConfig>(payload).is_err());
    }
}

#[test]
fn transport_and_scope_do_not_advertise_false_capabilities() {
    let mut cfg = config();
    assert!(cfg.validate().is_ok());
    cfg.transport = EndpointTransport::CloudflareHttp;
    assert!(cfg.validate().is_err());
    cfg.transport = EndpointTransport::CloudflareAccess;
    assert!(cfg.validate().is_ok());
    assert_eq!(
        cfg.transport.client_requirement(),
        ClientRequirement::Cloudflared
    );
    cfg.transport = EndpointTransport::PrivateNetwork;
    assert_eq!(
        cfg.transport.client_requirement(),
        ClientRequirement::CloudflareOne
    );
    cfg.scope = EndpointScope::Project { site_id: -1 };
    assert!(cfg.validate().is_err());
    cfg.scope = EndpointScope::Host;
    cfg.certificate_id = Some(1);
    assert!(cfg.validate().is_err());
    cfg.certificate_id = None;
    cfg.enabled = false;
    assert!(cfg.validate().is_err());
    cfg.primary = false;
    assert!(cfg.validate().is_ok());
}

#[test]
fn local_service_and_stale_results_never_prove_external_reachability() {
    let mut evidence = EndpointEvidence {
        endpoint_revision: 1,
        kind: EvidenceKind::Service,
        observation_point: ObservationPoint::LocalServer,
        observer_id: "server".into(),
        address_family: None,
        observed_at: 100,
        expires_at: 200,
        outcome: EvidenceOutcome::Pass,
        reason: EvidenceReason::Verified,
    };
    assert!(evidence.validate(110).is_ok());
    assert!(evidence.is_current(1, 199));
    assert!(!evidence.is_current(1, 200));
    assert!(!evidence.is_current(2, 110));
    assert!(!evidence.is_current(1, 99));
    evidence.kind = EvidenceKind::ExternalReachability;
    evidence.address_family = Some(AddressFamily::Ipv4);
    assert!(evidence.validate(110).is_err());
    evidence.observation_point = ObservationPoint::UserClient;
    assert!(evidence.validate(110).is_ok());
    evidence.address_family = None;
    assert!(evidence.validate(110).is_err());
    evidence.address_family = Some(AddressFamily::Ipv6);
    evidence.reason = EvidenceReason::Timeout;
    assert!(evidence.validate(110).is_err());
    evidence.outcome = EvidenceOutcome::Unknown;
    assert!(evidence.validate(110).is_ok());
    evidence.observer_id = "https://probe.example/token".into();
    assert!(evidence.validate(110).is_err());
}
