//! Connectivity metadata only: an address never grants access or proves reachability.
use crate::{DatabaseEngine, DomainName};
use serde::{Deserialize, Serialize};
use std::{net::IpAddr, num::NonZeroU16};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct ConnectivityError(pub &'static str);

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        pub enum $name { $(#[serde(rename = $wire)] $variant),+ }
        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
            pub fn parse(value: &str) -> Result<Self, ConnectivityError> {
                match value {
                    $($wire => Ok(Self::$variant)),+,
                    _ => Err(ConnectivityError("Nilai konektivitas tidak dikenal.")),
                }
            }
        }
    };
}

string_enum!(EndpointService {
    Panel => "panel", Website => "website", Ssh => "ssh", Database => "database",
    Imap => "imap", Submission => "submission", SmtpInbound => "smtp_inbound",
});
string_enum!(EndpointTransport {
    Direct => "direct", CloudflareHttp => "cloudflare_http",
    CloudflareAccess => "cloudflare_access", PrivateNetwork => "private_network",
    Provider => "provider",
});
string_enum!(ClientRequirement {
    None => "none", Cloudflared => "cloudflared", CloudflareOne => "cloudflare_one",
});
string_enum!(EndpointTlsMode {
    None => "none", Tls => "tls", Starttls => "starttls", Ssh => "ssh",
});

impl EndpointTransport {
    pub const fn client_requirement(self) -> ClientRequirement {
        match self {
            Self::CloudflareAccess => ClientRequirement::Cloudflared,
            Self::PrivateNetwork => ClientRequirement::CloudflareOne,
            _ => ClientRequirement::None,
        }
    }
}

/// Typed scope avoids a caller-supplied arbitrary scope string or resource path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EndpointScope {
    Host,
    Project { site_id: i64 },
    Engine { engine: DatabaseEngine },
}

/// Canonical IP (without brackets) or validated DNS name; never an authority/URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EndpointHost(String);

impl EndpointHost {
    pub fn parse(value: &str) -> Result<Self, ConnectivityError> {
        if value.is_empty() || value.trim() != value {
            return Err(ConnectivityError("Host endpoint tidak valid."));
        }
        if let Ok(ip) = value.parse::<IpAddr>() {
            return Ok(Self(ip.to_string()));
        }
        let domain = DomainName::parse(value)
            .map_err(|_| ConnectivityError("Gunakan hostname atau alamat IP tanpa URL/port."))?;
        Ok(Self(domain.as_ascii().to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// URL authority retains explicit ports, including default ports.
    pub fn authority(&self, port: NonZeroU16) -> String {
        if matches!(self.0.parse::<IpAddr>(), Ok(IpAddr::V6(_))) {
            format!("[{}]:{port}", self.0)
        } else {
            format!("{}:{port}", self.0)
        }
    }
}

impl TryFrom<String> for EndpointHost {
    type Error = ConnectivityError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}
impl From<EndpointHost> for String {
    fn from(value: EndpointHost) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    pub service: EndpointService,
    pub scope: EndpointScope,
    pub transport: EndpointTransport,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub host: EndpointHost,
    #[cfg_attr(feature = "openapi", schema(value_type = u16, minimum = 1, maximum = 65535))]
    pub port: NonZeroU16,
    pub tls_mode: EndpointTlsMode,
    pub certificate_id: Option<i64>,
    pub enabled: bool,
    pub primary: bool,
}

impl EndpointConfig {
    pub fn validate(&self) -> Result<(), ConnectivityError> {
        use EndpointScope as Scope;
        use EndpointService as Service;
        use EndpointTlsMode as Tls;
        use EndpointTransport as Transport;
        let scope_valid = match (self.service, self.scope) {
            (Service::Panel, Scope::Host)
            | (Service::Ssh, Scope::Host)
            | (Service::Database, Scope::Engine { .. }) => true,
            (
                Service::Website
                | Service::Ssh
                | Service::Imap
                | Service::Submission
                | Service::SmtpInbound,
                Scope::Project { site_id },
            ) => site_id > 0,
            _ => false,
        };
        if !scope_valid {
            return Err(ConnectivityError(
                "Scope tidak sesuai dengan layanan endpoint.",
            ));
        }
        let web = matches!(self.service, Service::Panel | Service::Website);
        let mail = matches!(
            self.service,
            Service::Imap | Service::Submission | Service::SmtpInbound
        );
        if (self.transport == Transport::CloudflareHttp && !web)
            || (self.transport == Transport::CloudflareAccess && (web || mail))
            || (self.transport == Transport::Provider
                && !(mail || self.service == Service::Database))
        {
            return Err(ConnectivityError(
                "Jalur tidak sesuai dengan layanan endpoint.",
            ));
        }
        if (self.service == Service::Ssh) != (self.tls_mode == Tls::Ssh)
            || (self.tls_mode == Tls::Starttls && !(mail || self.service == Service::Database))
            || (self.transport == Transport::CloudflareHttp && self.tls_mode != Tls::Tls)
        {
            return Err(ConnectivityError(
                "Kebijakan TLS tidak sesuai dengan layanan endpoint.",
            ));
        }
        if self.certificate_id.is_some()
            && (!matches!(self.scope, Scope::Project { .. })
                || !matches!(self.tls_mode, Tls::Tls | Tls::Starttls)
                || matches!(
                    self.transport,
                    Transport::CloudflareHttp | Transport::Provider
                )
                || self.certificate_id.is_some_and(|id| id <= 0))
        {
            return Err(ConnectivityError(
                "Referensi sertifikat lokal tidak sesuai scope endpoint.",
            ));
        }
        if self.primary && !self.enabled {
            return Err(ConnectivityError("Alamat utama harus diaktifkan."));
        }
        Ok(())
    }
}

string_enum!(EvidenceKind {
    Installation => "installation", Service => "service", Route => "route",
    Listener => "listener", Firewall => "firewall", Provider => "provider",
    Dns => "dns", Tls => "tls", Authentication => "authentication",
    ExternalReachability => "external_reachability",
});
string_enum!(ObservationPoint {
    LocalServer => "local_server", UserClient => "user_client", ExternalProbe => "external_probe",
});
string_enum!(AddressFamily { Ipv4 => "ipv4", Ipv6 => "ipv6" });
string_enum!(EvidenceOutcome { Pass => "pass", Fail => "fail", Unknown => "unknown" });
string_enum!(EvidenceReason {
    Verified => "verified", NotTested => "not_tested", Unsupported => "unsupported",
    Timeout => "timeout", ServiceMissing => "service_missing", ServiceStopped => "service_stopped",
    DnsFailure => "dns_failure", ConnectorUnavailable => "connector_unavailable",
    TlsFailure => "tls_failure", AuthenticationDenied => "authentication_denied",
    ListenerUnavailable => "listener_unavailable", PortBlocked => "port_blocked",
    ProviderPending => "provider_pending",
    PolicyBlocked => "policy_blocked", FamilyUnavailable => "family_unavailable",
    ConnectionRefused => "connection_refused", AgentUnavailable => "agent_unavailable",
    LocalGuardOnly => "local_guard_only", NetworkUnavailable => "network_unavailable",
});

impl EvidenceReason {
    pub const fn suggested_action(self) -> &'static str {
        match self {
            Self::Verified => "Tidak ada tindakan yang diperlukan untuk pemeriksaan ini.",
            Self::NotTested => "Jalankan pemeriksaan sesuai jalur koneksi.",
            Self::Unsupported => "Pilih jalur yang didukung atau gunakan akses browser.",
            Self::Timeout => "Periksa jalur koneksi lalu coba pemeriksaan kembali.",
            Self::ServiceMissing => "Pasang layanan melalui pengelola yang sesuai.",
            Self::ServiceStopped => "Periksa log dan jalankan layanan.",
            Self::DnsFailure => "Periksa record DNS endpoint.",
            Self::ConnectorUnavailable => "Periksa konektor dan rute Tunnel.",
            Self::TlsFailure => "Periksa sertifikat dan kebijakan TLS endpoint.",
            Self::AuthenticationDenied => "Periksa izin dan autentikasi layanan.",
            Self::ListenerUnavailable => "Periksa listener layanan bersama administrator.",
            Self::PortBlocked => "Periksa aturan port dan jalur jaringan bersama administrator.",
            Self::PolicyBlocked => {
                "Target dibatasi kebijakan probe publik. Gunakan diagnosis lokal atau pengujian client pada jaringan yang sesuai."
            }
            Self::FamilyUnavailable => {
                "Resolver server tidak mengembalikan alamat untuk keluarga IP ini. Periksa record A/AAAA."
            }
            Self::ConnectionRefused => {
                "Target menolak koneksi TCP dari server ini. Periksa listener dan aturan jaringan."
            }
            Self::AgentUnavailable => {
                "Agent belum dapat memberikan bukti lokal. Periksa koneksi agent."
            }
            Self::NetworkUnavailable => {
                "Jaringan dari server ke target belum dapat dibuktikan. Periksa routing dan coba dari client luar."
            }
            Self::LocalGuardOnly => {
                "Guard loopback panel tidak membuktikan izin inbound. Periksa firewall host, provider dan router secara terpisah."
            }
            Self::ProviderPending => "Selesaikan konfigurasi dan verifikasi provider.",
        }
    }
}

/// No arbitrary diagnostics text: reason codes avoid leaking credentials in reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct EndpointEvidence {
    pub endpoint_revision: i64,
    pub kind: EvidenceKind,
    pub observation_point: ObservationPoint,
    pub observer_id: String,
    pub address_family: Option<AddressFamily>,
    pub observed_at: i64,
    pub expires_at: i64,
    pub outcome: EvidenceOutcome,
    pub reason: EvidenceReason,
}

impl EndpointEvidence {
    pub fn validate(&self, now: i64) -> Result<(), ConnectivityError> {
        if self.endpoint_revision <= 0
            || self.observed_at < 0
            || self.observed_at > now
            || self.expires_at <= self.observed_at
        {
            return Err(ConnectivityError(
                "Revisi atau waktu bukti pemeriksaan tidak valid.",
            ));
        }
        if self.observer_id.is_empty()
            || self.observer_id.len() > 128
            || !self
                .observer_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(ConnectivityError("Identitas titik pengamatan tidak valid."));
        }
        if self.kind == EvidenceKind::ExternalReachability
            && (self.observation_point == ObservationPoint::LocalServer
                || self.address_family.is_none())
        {
            return Err(ConnectivityError(
                "Bukti koneksi dari luar memerlukan pengamat luar dan keluarga IP.",
            ));
        }
        let expected = match self.reason {
            EvidenceReason::Verified => EvidenceOutcome::Pass,
            EvidenceReason::NotTested
            | EvidenceReason::Unsupported
            | EvidenceReason::Timeout
            | EvidenceReason::ProviderPending
            | EvidenceReason::PolicyBlocked
            | EvidenceReason::FamilyUnavailable
            | EvidenceReason::AgentUnavailable
            | EvidenceReason::LocalGuardOnly
            | EvidenceReason::NetworkUnavailable => EvidenceOutcome::Unknown,
            _ => EvidenceOutcome::Fail,
        };
        if self.outcome != expected {
            return Err(ConnectivityError(
                "Hasil pemeriksaan tidak sesuai kode alasan.",
            ));
        }
        Ok(())
    }

    pub fn is_current(&self, revision: i64, now: i64) -> bool {
        self.endpoint_revision == revision && self.observed_at <= now && now < self.expires_at
    }
}
