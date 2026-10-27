use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelTlsRequest {
    Inspect,
    Challenge {
        ip: IpAddr,
        token: String,
        key_authorization: String,
    },
    Cleanup {
        token: String,
    },
    Activate {
        id: String,
        ip: IpAddr,
        port: u16,
        expected_fingerprint: Option<String>,
        fullchain_pem: String,
        private_key_pem: String,
    },
}
impl std::fmt::Debug for PanelTlsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PanelTlsRequest([redacted])")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PanelTlsStatus {
    pub active_id: Option<String>,
    pub fingerprint: Option<String>,
    pub ip: Option<IpAddr>,
    pub port: Option<u16>,
}
