//! Closed requests for Cloudflare; no caller-supplied URL, command, or filesystem path.
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(transparent)]
pub struct CloudflareSecret(pub String);

impl std::fmt::Debug for CloudflareSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TunnelAction {
    Start,
    Stop,
    Remove,
    Import,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CloudflareDnsRecord {
    pub kind: String,
    pub name: String,
    pub content: String,
    pub ttl: u32,
    pub proxied: bool,
    pub priority: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum CloudflareRequest {
    Status,
    Configure {
        account_id: String,
        api_token: CloudflareSecret,
    },
    Zones {
        page: u32,
    },
    CreateZone {
        name: String,
    },
    Records {
        zone_id: String,
        page: u32,
    },
    SaveRecord {
        zone_id: String,
        record_id: Option<String>,
        record: CloudflareDnsRecord,
    },
    DeleteRecord {
        zone_id: String,
        record_id: String,
    },
    Tunnels {
        page: u32,
    },
    CreateTunnel {
        name: String,
    },
    TunnelAction {
        tunnel_id: String,
        operation: TunnelAction,
    },
    Publish {
        zone_id: String,
        tunnel_id: String,
        hostname: String,
        tls: bool,
    },
    Unpublish {
        zone_id: String,
        tunnel_id: String,
        hostname: String,
    },
    PublishStorage {
        zone_id: String,
        tunnel_id: String,
        hostname: String,
    },
}
