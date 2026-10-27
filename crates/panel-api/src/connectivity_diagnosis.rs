//! Bounded read-only diagnosis of persisted endpoints. DNS is resolved once;
//! sockets use checked IPs and TLS uses the original name. No HTTP/redirects.
use crate::agent_client::AgentClient;
use futures_util::future::BoxFuture;
use panel_core::{DatabaseEngine, connectivity::*};
use panel_proto::{ConnectivityInspection, ConnectivityTarget, LocalCheck};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::{net::TcpStream, time::timeout};

const STEP: Duration = Duration::from_secs(2);
const TTL: i64 = 300;

/// Conservative public-probe policy, based on IANA special-purpose registries.
/// Deliberately excludes special-purpose exceptions and transition mechanisms.
pub fn public_probe_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let n = u32::from(ip);
            ![
                (0x00000000, 8),
                (0x0a000000, 8),
                (0x64400000, 10),
                (0x7f000000, 8),
                (0xa9fe0000, 16),
                (0xa83f8110, 32), // Azure host-node WireServer, not a public probe target.
                (0xac100000, 12),
                (0xc0000000, 24),
                (0xc0000200, 24),
                (0xc01fc400, 24),
                (0xc034c100, 24),
                (0xc0586300, 24),
                (0xc0a80000, 16),
                (0xc0af3000, 24),
                (0xc6120000, 15),
                (0xc6336400, 24),
                (0xcb007100, 24),
                (0xe0000000, 3),
            ]
            .iter()
            .any(|&(base, prefix)| n >> (32 - prefix) == base >> (32 - prefix))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
                && !(s[0] == 0x2620 && s[1] == 0x4f && s[2] == 0x8000)
        }
    }
}

trait ProbeIo: Sync {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, EvidenceReason>>;
    fn connect<'a>(
        &'a self,
        target: SocketAddr,
        host: &'a str,
        tls: bool,
    ) -> BoxFuture<'a, (EvidenceReason, EvidenceReason)>;
}
struct Network;
impl ProbeIo for Network {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, EvidenceReason>> {
        Box::pin(async move {
            if let Ok(ip) = host.parse::<IpAddr>() {
                return Ok(vec![SocketAddr::new(ip, port)]);
            }
            match timeout(STEP, tokio::net::lookup_host((host, port))).await {
                Ok(Ok(addresses)) => Ok(addresses.take(17).collect()),
                Ok(Err(_)) => Err(EvidenceReason::DnsFailure),
                Err(_) => Err(EvidenceReason::Timeout),
            }
        })
    }
    fn connect<'a>(
        &'a self,
        target: SocketAddr,
        host: &'a str,
        tls: bool,
    ) -> BoxFuture<'a, (EvidenceReason, EvidenceReason)> {
        Box::pin(async move {
            let socket = match timeout(STEP, TcpStream::connect(target)).await {
                Ok(Ok(socket)) => socket,
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                    return (EvidenceReason::ConnectionRefused, EvidenceReason::NotTested);
                }
                Ok(Err(_)) => {
                    return (
                        EvidenceReason::NetworkUnavailable,
                        EvidenceReason::NotTested,
                    );
                }
                Err(_) => return (EvidenceReason::Timeout, EvidenceReason::NotTested),
            };
            if !tls {
                return (EvidenceReason::Verified, EvidenceReason::Unsupported);
            }
            let Ok(config) = crate::mail::system_trust() else {
                return (EvidenceReason::Verified, EvidenceReason::Unsupported);
            };
            let result = tls_verdict(socket, host, config).await;
            (EvidenceReason::Verified, result)
        })
    }
}
async fn tls_verdict(
    socket: TcpStream,
    host: &str,
    config: std::sync::Arc<tokio_rustls::rustls::ClientConfig>,
) -> EvidenceReason {
    let Ok(name) = tokio_rustls::rustls::pki_types::ServerName::try_from(host.to_owned()) else {
        return EvidenceReason::TlsFailure;
    };
    match timeout(
        STEP,
        tokio_rustls::TlsConnector::from(config).connect(name, socket),
    )
    .await
    {
        Ok(Ok(_)) => EvidenceReason::Verified,
        Ok(Err(_)) => EvidenceReason::TlsFailure,
        Err(_) => EvidenceReason::Timeout,
    }
}
fn report(
    revision: i64,
    kind: EvidenceKind,
    family: Option<AddressFamily>,
    reason: EvidenceReason,
    observer: &str,
) -> EndpointEvidence {
    use EvidenceReason::*;
    let outcome = match reason {
        Verified => EvidenceOutcome::Pass,
        NotTested | Unsupported | Timeout | ProviderPending | PolicyBlocked | FamilyUnavailable
        | AgentUnavailable | LocalGuardOnly | NetworkUnavailable => EvidenceOutcome::Unknown,
        _ => EvidenceOutcome::Fail,
    };
    let now = chrono::Utc::now().timestamp();
    EndpointEvidence {
        endpoint_revision: revision,
        kind,
        observation_point: ObservationPoint::LocalServer,
        observer_id: observer.into(),
        address_family: family,
        observed_at: now,
        expires_at: now + TTL,
        outcome,
        reason,
    }
}
fn families(config: &EndpointConfig) -> Vec<AddressFamily> {
    match config.host.as_str().parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => vec![AddressFamily::Ipv4],
        Ok(IpAddr::V6(_)) => vec![AddressFamily::Ipv6],
        Err(_) => vec![AddressFamily::Ipv4, AddressFamily::Ipv6],
    }
}
pub fn local_target(config: &EndpointConfig) -> Option<ConnectivityTarget> {
    use ConnectivityTarget as T;
    if config.transport == EndpointTransport::Provider {
        return None;
    }
    Some(match config.service {
        EndpointService::Panel => T::Panel,
        EndpointService::Website => {
            if config.tls_mode == EndpointTlsMode::Tls {
                T::WebsiteHttps
            } else {
                T::WebsiteHttp
            }
        }
        EndpointService::Ssh => T::Ssh,
        EndpointService::Database => match config.scope {
            EndpointScope::Engine {
                engine: DatabaseEngine::MariaDb,
            } => T::MariaDb,
            EndpointScope::Engine {
                engine: DatabaseEngine::PostgreSql,
            } => T::PostgreSql,
            _ => return None,
        },
        EndpointService::Imap => T::Imap,
        EndpointService::Submission => T::Submission,
        EndpointService::SmtpInbound => T::SmtpInbound,
    })
}
fn local_reports(
    config: &EndpointConfig,
    revision: i64,
    result: Option<&ConnectivityInspection>,
) -> Vec<EndpointEvidence> {
    use EvidenceKind as K;
    use EvidenceReason as R;
    let mut out = Vec::new();
    let target = local_target(config);
    let reason = |value, fail| match value {
        Some(LocalCheck::Pass) => R::Verified,
        Some(LocalCheck::Fail) => fail,
        Some(LocalCheck::Unknown) => R::NotTested,
        _ => {
            if target.is_none() {
                R::NotTested
            } else {
                R::AgentUnavailable
            }
        }
    };
    out.push(report(
        revision,
        K::Installation,
        None,
        reason(result.map(|r| r.installed), R::ServiceMissing),
        "server-local",
    ));
    out.push(report(
        revision,
        K::Service,
        None,
        reason(result.map(|r| r.running), R::ServiceStopped),
        "server-local",
    ));
    for family in families(config) {
        // A registered public/custom/NAT port is not proof of the local mapping.
        let why = if target.is_none()
            || target.is_some_and(|t| t.local().1 != config.port.get())
            || config.transport != EndpointTransport::Direct
        {
            R::NotTested
        } else {
            reason(
                result.map(|r| {
                    if family == AddressFamily::Ipv4 {
                        // An IPv6 socket may accept IPv4-mapped clients. /proc
                        // does not expose per-socket IPV6_V6ONLY: absence of an
                        // AF_INET row alone cannot prove IPv4 unavailability.
                        if r.listener_ipv4 == LocalCheck::Fail
                            && r.listener_ipv6 != LocalCheck::Fail
                        {
                            LocalCheck::Unknown
                        } else {
                            r.listener_ipv4
                        }
                    } else {
                        r.listener_ipv6
                    }
                }),
                R::ListenerUnavailable,
            )
        };
        out.push(report(
            revision,
            K::Listener,
            Some(family),
            why,
            "server-local",
        ));
    }
    out.push(report(
        revision,
        K::Firewall,
        None,
        if result.is_some_and(|r| r.panel_guard_present == LocalCheck::Pass) {
            R::LocalGuardOnly
        } else {
            R::NotTested
        },
        "server-local",
    ));
    if config.transport != EndpointTransport::Direct {
        // Provider metadata has no verified binding yet (separate provisioning tasks).
        out.push(report(
            revision,
            K::Provider,
            None,
            R::ProviderPending,
            "server-provider",
        ));
    }
    out.push(report(
        revision,
        K::Authentication,
        None,
        R::NotTested,
        "server-local",
    ));
    out
}
async fn network_reports(
    io: &impl ProbeIo,
    config: &EndpointConfig,
    revision: i64,
) -> Vec<EndpointEvidence> {
    use EvidenceKind as K;
    use EvidenceReason as R;
    let resolved = io.resolve(config.host.as_str(), config.port.get()).await;
    let blocked = resolved.as_ref().is_ok_and(|all| {
        all.len() > 16
            || all
                .iter()
                .any(|a| a.port() != config.port.get() || !public_probe_ip(a.ip()))
    });
    let allow_socket = !matches!(
        config.transport,
        EndpointTransport::PrivateNetwork | EndpointTransport::CloudflareAccess
    );
    let implicit_tls =
        config.tls_mode == EndpointTlsMode::Tls && config.service != EndpointService::Database;
    let mut out = Vec::new();
    for family in families(config) {
        let target = resolved
            .as_ref()
            .ok()
            .and_then(|all| {
                all.iter()
                    .find(|a| a.is_ipv4() == (family == AddressFamily::Ipv4))
            })
            .copied();
        let dns = match &resolved {
            _ if config.host.as_str().parse::<IpAddr>().is_ok() => R::NotTested,
            Err(reason) => *reason,
            Ok(_) if target.is_none() => R::FamilyUnavailable,
            Ok(_) => R::Verified,
        };
        out.push(report(
            revision,
            K::Dns,
            Some(family),
            dns,
            "server-resolver",
        ));
        let (route, tls) = if blocked {
            (R::PolicyBlocked, R::NotTested)
        } else if !allow_socket {
            (R::NotTested, R::NotTested)
        } else if let Some(target) = target {
            io.connect(target, config.host.as_str(), implicit_tls).await
        } else {
            (
                if dns == R::Timeout {
                    R::Timeout
                } else {
                    R::NotTested
                },
                R::NotTested,
            )
        };
        out.push(report(
            revision,
            K::Route,
            Some(family),
            route,
            "server-network",
        ));
        out.push(report(
            revision,
            K::Tls,
            Some(family),
            tls,
            "server-network",
        ));
    }
    out
}
pub async fn diagnose(
    agent: &AgentClient,
    config: &EndpointConfig,
    revision: i64,
) -> Vec<EndpointEvidence> {
    let local = async {
        match local_target(config) {
            Some(target) => agent.connectivity_inspect(target).await.ok(),
            None => None,
        }
    };
    let network = async {
        timeout(
            Duration::from_secs(11),
            network_reports(&Network, config, revision),
        )
        .await
        .unwrap_or_else(|_| {
            families(config)
                .into_iter()
                .flat_map(|family| {
                    [EvidenceKind::Dns, EvidenceKind::Route, EvidenceKind::Tls].map(|kind| {
                        report(
                            revision,
                            kind,
                            Some(family),
                            EvidenceReason::Timeout,
                            if kind == EvidenceKind::Dns {
                                "server-resolver"
                            } else {
                                "server-network"
                            },
                        )
                    })
                })
                .collect()
        })
    };
    let (local, mut network) = tokio::join!(local, network);
    let mut reports = local_reports(config, revision, local.as_ref());
    reports.append(&mut network);
    reports
}

#[cfg(test)]
#[path = "connectivity_diagnosis_tests.rs"]
mod tests;
