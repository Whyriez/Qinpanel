#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::sync::Mutex;
struct Mock {
    addresses: Result<Vec<SocketAddr>, EvidenceReason>,
    connections: Mutex<Vec<(SocketAddr, String, bool)>>,
}
impl ProbeIo for Mock {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, EvidenceReason>> {
        Box::pin(async { self.addresses.clone() })
    }
    fn connect<'a>(
        &'a self,
        target: SocketAddr,
        host: &'a str,
        tls: bool,
    ) -> BoxFuture<'a, (EvidenceReason, EvidenceReason)> {
        self.connections
            .lock()
            .unwrap()
            .push((target, host.into(), tls));
        Box::pin(async move {
            if target.is_ipv4() {
                (EvidenceReason::Verified, EvidenceReason::Verified)
            } else {
                (EvidenceReason::ConnectionRefused, EvidenceReason::NotTested)
            }
        })
    }
}
fn config() -> EndpointConfig {
    serde_json::from_value(serde_json::json!({"service":"website","scope":{"kind":"project","site_id":1},"transport":"direct","host":"app.example.test","port":8443,"tls_mode":"tls","certificate_id":null,"enabled":true,"primary":true})).unwrap()
}
fn mock(ips: &[&str]) -> Mock {
    Mock {
        addresses: Ok(ips.iter().map(|s| s.parse().unwrap()).collect()),
        connections: Mutex::new(vec![]),
    }
}
#[test]
fn public_policy_blocks_metadata_private_transition_documentation_and_special_addresses() {
    for value in [
        "0.0.0.0",
        "10.0.0.1",
        "100.64.0.1",
        "100.100.100.200",
        "127.0.0.1",
        "169.254.169.254",
        "168.63.129.16",
        "172.16.0.1",
        "192.168.1.1",
        "192.0.0.9",
        "192.0.2.1",
        "192.88.99.2",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:8.8.8.8",
        "64:ff9b::808:808",
        "100::1",
        "fc00::1",
        "fe80::1",
        "ff02::1",
        "2001::1",
        "2001:20::1",
        "2001:db8::1",
        "2002:808:808::1",
        "3fff::1",
        "2620:4f:8000::1",
    ] {
        assert!(!public_probe_ip(value.parse().unwrap()), "{value}");
    }
    for value in [
        "8.8.8.8",
        "1.1.1.1",
        "172.32.0.1",
        "2001:4860:4860::8888",
        "2606:4700:4700::1111",
    ] {
        assert!(public_probe_ip(value.parse().unwrap()), "{value}");
    }
}
#[tokio::test]
async fn mixed_dns_answer_blocks_all_sockets_even_when_public_address_is_first() {
    let io = mock(&["8.8.8.8:8443", "127.0.0.1:8443"]);
    let reports = network_reports(&io, &config(), 7).await;
    assert!(io.connections.lock().unwrap().is_empty());
    assert!(
        reports
            .iter()
            .filter(|r| r.kind == EvidenceKind::Route)
            .all(|r| r.reason == EvidenceReason::PolicyBlocked
                && r.outcome == EvidenceOutcome::Unknown)
    );
    assert!(
        reports
            .iter()
            .all(|r| r.kind != EvidenceKind::ExternalReachability)
    );
    let io = mock(&["127.0.0.1:8443"]);
    let mut cfg = config();
    cfg.host = EndpointHost::parse("127.0.0.1").unwrap();
    let reports = network_reports(&io, &cfg, 7).await;
    assert!(reports.iter().filter(|r| r.kind == EvidenceKind::Dns).all(|r| r.outcome == EvidenceOutcome::Unknown && r.reason == EvidenceReason::NotTested));
}
#[tokio::test]
async fn pins_one_checked_address_per_family_and_keeps_failures_independent() {
    let io = mock(&[
        "8.8.8.8:8443",
        "1.1.1.1:8443",
        "[2606:4700:4700::1111]:8443",
    ]);
    let reports = network_reports(&io, &config(), 7).await;
    let calls = io.connections.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0],
        (
            "8.8.8.8:8443".parse().unwrap(),
            "app.example.test".into(),
            true
        )
    );
    assert!(reports.iter().any(|r| r.kind == EvidenceKind::Route
        && r.address_family == Some(AddressFamily::Ipv4)
        && r.outcome == EvidenceOutcome::Pass));
    assert!(reports.iter().any(|r| r.kind == EvidenceKind::Route
        && r.address_family == Some(AddressFamily::Ipv6)
        && r.reason == EvidenceReason::ConnectionRefused));
    for report in reports {
        report.validate(chrono::Utc::now().timestamp()).unwrap();
        assert_eq!(report.endpoint_revision, 7);
        assert_eq!(report.expires_at - report.observed_at, 300);
        assert_eq!(report.observation_point, ObservationPoint::LocalServer);
    }
}
#[tokio::test]
async fn absent_family_is_unknown_and_private_access_never_dials_a_socket() {
    let io = mock(&["8.8.8.8:8443"]);
    let reports = network_reports(&io, &config(), 1).await;
    assert!(reports.iter().any(|r| r.kind == EvidenceKind::Dns
        && r.address_family == Some(AddressFamily::Ipv6)
        && r.reason == EvidenceReason::FamilyUnavailable
        && r.outcome == EvidenceOutcome::Unknown));
    let io = mock(&["8.8.8.8:8443"]);
    let mut cfg = config();
    cfg.transport = EndpointTransport::PrivateNetwork;
    network_reports(&io, &cfg, 1).await;
    assert!(io.connections.lock().unwrap().is_empty());
    cfg.transport = EndpointTransport::CloudflareAccess;
    network_reports(&io, &cfg, 1).await;
    assert!(io.connections.lock().unwrap().is_empty());
}
#[tokio::test]
async fn resolver_failure_timeout_oversized_answers_and_wrong_ports_never_connect() {
    for addresses in [
        Err(EvidenceReason::DnsFailure),
        Err(EvidenceReason::Timeout),
        Ok(vec!["8.8.8.8:8443".parse().unwrap(); 17]),
        Ok(vec!["8.8.8.8:7700".parse().unwrap()]),
    ] {
        let io = Mock {
            addresses,
            connections: Mutex::new(vec![]),
        };
        network_reports(&io, &config(), 1).await;
        assert!(io.connections.lock().unwrap().is_empty());
    }
}
#[test]
fn custom_port_provider_and_local_service_are_never_route_or_firewall_proof() {
    let cfg = config();
    let local = ConnectivityInspection {
        installed: LocalCheck::Pass,
        running: LocalCheck::Pass,
        listener_ipv4: LocalCheck::Pass,
        listener_ipv6: LocalCheck::Fail,
        panel_guard_present: LocalCheck::Pass,
    };
    let reports = local_reports(&cfg, 1, Some(&local));
    assert!(
        reports
            .iter()
            .any(|r| r.kind == EvidenceKind::Service && r.outcome == EvidenceOutcome::Pass)
    );
    assert!(
        reports
            .iter()
            .filter(|r| r.kind == EvidenceKind::Listener)
            .all(|r| r.outcome == EvidenceOutcome::Unknown)
    );
    assert!(reports.iter().any(|r| r.kind == EvidenceKind::Firewall
        && r.reason == EvidenceReason::LocalGuardOnly
        && r.outcome == EvidenceOutcome::Unknown));
    assert!(
        !reports
            .iter()
            .any(|r| r.kind == EvidenceKind::Route || r.kind == EvidenceKind::ExternalReachability)
    );
    let mut cfg = cfg;
    cfg.transport = EndpointTransport::Provider;
    let reports = local_reports(&cfg, 1, None);
    assert!(
        reports.iter().any(
            |r| r.kind == EvidenceKind::Provider && r.reason == EvidenceReason::ProviderPending
        )
    );
    assert!(reports.iter().all(|r| r.outcome != EvidenceOutcome::Pass));
}
#[test]
fn local_listener_only_checks_matching_fixed_target_and_never_claims_public_binding() {
    let mut cfg = config();
    cfg.port = std::num::NonZeroU16::new(443).unwrap();
    let local = ConnectivityInspection {
        installed: LocalCheck::Pass,
        running: LocalCheck::Unknown,
        listener_ipv4: LocalCheck::Pass,
        listener_ipv6: LocalCheck::Fail,
        panel_guard_present: LocalCheck::Unknown,
    };
    let reports = local_reports(&cfg, 2, Some(&local));
    assert!(reports.iter().any(|r| r.kind == EvidenceKind::Listener
        && r.address_family == Some(AddressFamily::Ipv4)
        && r.outcome == EvidenceOutcome::Pass));
    assert!(reports.iter().any(|r| r.kind == EvidenceKind::Listener
        && r.address_family == Some(AddressFamily::Ipv6)
        && r.outcome == EvidenceOutcome::Fail));
    assert!(
        reports
            .iter()
            .any(|r| r.kind == EvidenceKind::Service && r.reason == EvidenceReason::NotTested)
    );
    let dual = ConnectivityInspection {
        listener_ipv4: LocalCheck::Fail,
        listener_ipv6: LocalCheck::Pass,
        ..local
    };
    let reports = local_reports(&cfg, 2, Some(&dual));
    assert!(reports.iter().any(|r| r.kind == EvidenceKind::Listener
        && r.address_family == Some(AddressFamily::Ipv4)
        && r.outcome == EvidenceOutcome::Unknown));
}

#[tokio::test]
async fn real_tls_fixture_checks_dns_name_ip_san_trust_expiry_and_deadline_without_application_data()
 {
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use tokio_rustls::rustls::{
        self,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    };
    for (name, trusted, expired, expected) in [
        (
            "fixture.example.test",
            true,
            false,
            EvidenceReason::Verified,
        ),
        ("127.0.0.1", true, false, EvidenceReason::Verified),
        (
            "wrong.example.test",
            true,
            false,
            EvidenceReason::TlsFailure,
        ),
        (
            "fixture.example.test",
            false,
            false,
            EvidenceReason::TlsFailure,
        ),
        (
            "fixture.example.test",
            true,
            true,
            EvidenceReason::TlsFailure,
        ),
    ] {
        let mut params =
            rcgen::CertificateParams::new(vec!["fixture.example.test".into(), "127.0.0.1".into()])
                .unwrap();
        if expired {
            params.not_before = rcgen::date_time_ymd(2000, 1, 1);
            params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        }
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        let cert = CertificateDer::from(cert.der().to_vec());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server_config = rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.clone()],
                PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            )
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        if trusted {
            roots.add(cert).unwrap();
        }
        let trust = Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            if let Ok(mut tls) = tokio_rustls::TlsAcceptor::from(Arc::new(server_config))
                .accept(socket)
                .await
            {
                let mut byte = [0];
                let result = tls.read(&mut byte).await;
                assert!(
                    matches!(result, Ok(0) | Err(_)),
                    "probe must not send HTTP or credentials"
                );
            }
        });
        let socket = TcpStream::connect(address).await.unwrap();
        assert_eq!(
            tls_verdict(socket, name, trust).await,
            expected,
            "{name}, trusted={trusted}, expired={expired}"
        );
        server.await.unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let socket = TcpStream::connect(address).await.unwrap();
    let (_silent, _) = listener.accept().await.unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let trust = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth(),
    );
    let start = std::time::Instant::now();
    assert_eq!(
        tls_verdict(socket, "fixture.example.test", trust).await,
        EvidenceReason::Timeout
    );
    assert!(start.elapsed() < Duration::from_secs(4));
}
