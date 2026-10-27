use serde::{Deserialize, Serialize};

/// Fixed Cloudflare zone and ACME operations; no URLs or filesystem paths.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Dns01Request {
    Status,
    Configure {
        zone_id: String,
        zone_name: String,
        api_token: crate::CloudflareSecret,
    },
    Revoke,
    Present {
        id: String,
        revision: String,
        hostname: String,
        digest: String,
    },
    Cleanup {
        id: String,
    },
    VerifyOrigin {
        id: String,
        hostname: String,
    },
    StageCertificate {
        id: String,
        hostname: String,
        names: Vec<String>,
        fullchain_pem: String,
        private_key_pem: String,
    },
}
impl std::fmt::Debug for Dns01Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Dns01Request([redacted])")
    }
}
