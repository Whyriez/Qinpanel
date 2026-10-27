use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::fmt;

/// Certificate authority or generation issuer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateIssuer {
    Letsencrypt,
    LetsencryptStaging,
    SelfSigned,
    Custom,
}

impl CertificateIssuer {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Letsencrypt => "letsencrypt",
            Self::LetsencryptStaging => "letsencrypt_staging",
            Self::SelfSigned => "self_signed",
            Self::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "letsencrypt" => Some(Self::Letsencrypt),
            "letsencrypt_staging" => Some(Self::LetsencryptStaging),
            "self_signed" | "selfsigned" => Some(Self::SelfSigned),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

impl fmt::Display for CertificateIssuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Certificate validity status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateStatus {
    Active,
    Renewing,
    Expired,
    Failed,
    Revoked,
}

impl CertificateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Renewing => "renewing",
            Self::Expired => "expired",
            Self::Failed => "failed",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "renewing" => Some(Self::Renewing),
            "expired" => Some(Self::Expired),
            "failed" => Some(Self::Failed),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

impl fmt::Display for CertificateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// An SSL/TLS X.509 Certificate record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Certificate {
    pub id: i64,
    pub site_id: i64,
    pub primary_domain: String,
    pub san_domains_json: String,
    pub issuer: String,
    pub cert_path: String,
    pub key_path: String,
    pub fullchain_pem: String,
    pub privkey_pem: String,
    pub not_before: i64,
    pub not_after: i64,
    pub auto_renew: i64,
    pub ocsp_stapling: i64,
    pub last_renew_attempt: Option<i64>,
    pub renew_error: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Certificate {
    pub fn issuer_enum(&self) -> Option<CertificateIssuer> {
        CertificateIssuer::parse(&self.issuer)
    }

    pub fn status_enum(&self) -> Option<CertificateStatus> {
        CertificateStatus::parse(&self.status)
    }

    pub fn san_domains(&self) -> Vec<String> {
        serde_json::from_str(&self.san_domains_json).unwrap_or_default()
    }

    pub fn is_auto_renew(&self) -> bool {
        self.auto_renew != 0
    }

    pub fn is_ocsp_stapling(&self) -> bool {
        self.ocsp_stapling != 0
    }

    /// Checks if certificate expires within `threshold_seconds` from `now_timestamp`.
    pub fn is_expiring_within(&self, now_timestamp: i64, threshold_seconds: i64) -> bool {
        self.not_after <= now_timestamp + threshold_seconds
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCertificate {
    pub site_id: i64,
    pub primary_domain: String,
    pub san_domains_json: String,
    pub issuer: String,
    pub cert_path: String,
    pub key_path: String,
    pub fullchain_pem: String,
    pub privkey_pem: String,
    pub not_before: i64,
    pub not_after: i64,
    pub auto_renew: bool,
    pub ocsp_stapling: bool,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateCertificate {
    pub fullchain_pem: Option<String>,
    pub privkey_pem: Option<String>,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
    pub auto_renew: Option<bool>,
    pub ocsp_stapling: Option<bool>,
    pub last_renew_attempt: Option<Option<i64>>,
    pub renew_error: Option<Option<String>>,
    pub status: Option<String>,
}

/// An ACME account record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct AcmeAccount {
    pub id: i64,
    pub email: String,
    pub directory_url: String,
    pub account_uri: Option<String>,
    pub private_key_pem: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAcmeAccount {
    pub email: String,
    pub directory_url: String,
    pub account_uri: Option<String>,
    pub private_key_pem: String,
}
