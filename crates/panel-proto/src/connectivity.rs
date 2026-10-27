//! Closed, read-only local inspection targets. No host, port, path or command input.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityTarget {
    Panel,
    WebsiteHttp,
    WebsiteHttps,
    Ssh,
    MariaDb,
    PostgreSql,
    Imap,
    Submission,
    SmtpInbound,
}
impl ConnectivityTarget {
    pub const fn local(self) -> (&'static str, u16) {
        match self {
            Self::Panel => ("nginx.service", 17890),
            Self::WebsiteHttp => ("nginx.service", 80),
            Self::WebsiteHttps => ("nginx.service", 443),
            Self::Ssh => ("ssh.service", 22),
            Self::MariaDb => ("mariadb.service", 3306),
            Self::PostgreSql => ("postgresql.service", 5432),
            Self::Imap => ("dovecot.service", 993),
            Self::Submission => ("postfix.service", 587),
            Self::SmtpInbound => ("postfix.service", 25),
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalCheck {
    Pass,
    Fail,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectivityInspection {
    pub installed: LocalCheck,
    pub running: LocalCheck,
    pub listener_ipv4: LocalCheck,
    pub listener_ipv6: LocalCheck,
    /// Presence of the panel's loopback guard is not an inbound firewall verdict.
    pub panel_guard_present: LocalCheck,
}
