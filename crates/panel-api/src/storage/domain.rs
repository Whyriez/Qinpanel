use axum::body::Bytes;
use panel_core::{
    connectivity::EndpointHost,
    object_storage::{StorageError, identity::*},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::num::NonZeroU16;
use uuid::Uuid;

pub const UPLOAD_LIMIT: usize = 32 * 1024 * 1024;
pub const LIST_LIMIT: usize = 1024 * 1024;

#[derive(Clone, FromRow)]
pub struct Connection {
    pub id: String,
    pub owner_id: i64,
    pub site_id: Option<i64>,
    pub label: String,
    pub provider: String,
    pub endpoint: String,
    pub region: String,
    pub addressing: String,
    pub bucket: String,
    pub credentials_enc: String,
    pub revision: i64,
    pub verified_revision: Option<i64>,
    pub verified_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    #[sqlx(default)]
    pub last_operation: Option<String>,
    #[sqlx(default)]
    pub managed: bool,
    #[sqlx(default)]
    pub managed_state: Option<String>,
    #[sqlx(default)]
    pub application_key_generation: i64,
    #[sqlx(default)]
    pub application_key_state: Option<String>,
}
impl Connection {
    pub fn host(&self) -> Result<String, StorageError> {
        let uri: hyper::Uri = self
            .endpoint
            .parse()
            .map_err(|_| StorageError::InvalidConnection)?;
        let host = uri.host().ok_or(StorageError::InvalidConnection)?;
        Ok(host.trim_start_matches('[').trim_end_matches(']').into())
    }
    pub fn core(&self) -> Result<StorageConnection, StorageError> {
        let owner = self.owner_id.try_into()?;
        let id = Uuid::parse_str(&self.id)
            .map_err(|_| StorageError::InvalidIdentity)?
            .try_into()?;
        let revision = u64::try_from(self.revision).map_err(|_| StorageError::InvalidIdentity)?;
        let credential_id = Uuid::parse_str(&self.id)
            .map_err(|_| StorageError::InvalidIdentity)?
            .try_into()?;
        let provider = match self.provider.as_str() {
            "aws" => ExternalProvider::AwsS3,
            "r2" => ExternalProvider::CloudflareR2,
            "supabase" | "compatible" => ExternalProvider::S3Compatible,
            _ => return Err(StorageError::InvalidConnection),
        };
        let endpoint = StorageEndpoint {
            host: EndpointHost::parse(&self.host()?)
                .map_err(|_| StorageError::InvalidConnection)?,
            port: NonZeroU16::new(443).ok_or(StorageError::InvalidConnection)?,
        };
        let config = ConnectionConfig {
            target: if self.managed {
                StorageTarget::Managed {
                    instance_id: Uuid::from_u128(0xac54dc4aefc64ae6b54150fb6871f400).try_into()?,
                    instance_revision: 1,
                }
            } else {
                StorageTarget::External { provider }
            },
            endpoint,
            region: self.region.clone(),
            addressing: if self.addressing == "path" {
                AddressingStyle::Path
            } else {
                AddressingStyle::VirtualHost
            },
            credential: if self.managed {
                None
            } else {
                Some(CredentialRef {
                    id: credential_id,
                    owner,
                    revision,
                })
            },
        };
        config.validate(owner)?;
        Ok(StorageConnection {
            id,
            revision,
            owner,
            config,
        })
    }
    pub fn conditional_put(&self) -> bool {
        matches!(self.provider.as_str(), "aws" | "r2")
    }
    pub fn request_host(&self) -> Result<String, StorageError> {
        let host = self.host()?;
        Ok(if self.addressing == "virtual_host" {
            format!("{}.{host}", self.bucket)
        } else {
            host
        })
    }
    pub fn base_path(&self) -> &'static str {
        if self.provider == "supabase" {
            "/storage/v1/s3"
        } else {
            ""
        }
    }
    pub fn validate(&self) -> Result<(), StorageError> {
        if self.label.trim().is_empty()
            || self.label.chars().count() > 80
            || self.label.chars().any(char::is_control)
            || self.endpoint.contains(['#', '\\', '%'])
            || !matches!(self.addressing.as_str(), "path" | "virtual_host")
        {
            return Err(StorageError::InvalidConnection);
        }
        let uri: hyper::Uri = self
            .endpoint
            .parse()
            .map_err(|_| StorageError::InvalidConnection)?;
        if uri.scheme_str() != Some("https")
            || uri.query().is_some()
            || uri.port_u16().is_some_and(|p| p != 443)
            || uri.authority().is_none_or(|v| v.as_str().contains('@'))
            || !matches!(uri.path(), "" | "/")
                && uri.path().trim_end_matches('/') != self.base_path()
            || self.provider == "supabase"
                && (uri.path().trim_end_matches('/') != "/storage/v1/s3"
                    || self.addressing != "path")
        {
            return Err(StorageError::InvalidConnection);
        }
        if self.host()?.parse::<std::net::IpAddr>().is_ok() {
            return Err(StorageError::InvalidConnection);
        }
        let host = self.host()?.to_ascii_lowercase();
        if self.provider == "aws"
            && (!(host.starts_with("s3.") || host.starts_with("s3-"))
                || !(host.ends_with(".amazonaws.com") || host.ends_with(".amazonaws.com.cn")))
            || self.provider == "r2" && !host.ends_with(".r2.cloudflarestorage.com")
        {
            return Err(StorageError::InvalidConnection);
        }
        let bucket: BucketName = self.bucket.clone().try_into()?;
        if self.addressing == "virtual_host" && bucket.as_str().contains('.') {
            return Err(StorageError::InvalidBucketName);
        }
        self.core()?.config.validate(self.owner_id.try_into()?)
    }
}

pub enum Operation {
    List {
        prefix: String,
        token: Option<String>,
    },
    Upload {
        key: String,
        body: Bytes,
        overwrite: bool,
    },
    Delete {
        key: String,
    },
    CreateBucket,
    ReadObject {
        key: String,
    },
    Download {
        key: String,
    },
    AccessLink {
        key: String,
        expires_in: u32,
    },
}
pub enum Outcome {
    Listing(Listing),
    Done,
    Content(Bytes),
    Download { url: String, expires_at: i64 },
}

#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error("Conditional PUT belum terverifikasi pada provider ini.")]
    ConditionalUnsupported,
    #[error("Endpoint storage tidak diizinkan.")]
    Endpoint,
    #[error("Provider storage tidak tersedia.")]
    Unavailable,
    #[error("Jawaban provider tidak valid.")]
    Invalid,
    #[error("Provider menolak operasi.")]
    Provider(u16),
    #[error("Operasi melewati batas waktu; periksa keadaan provider.")]
    Timeout,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Object {
    pub key: String,
    pub size: u64,
    pub modified_at: String,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Listing {
    pub objects: Vec<Object>,
    pub prefixes: Vec<String>,
    pub next_token: Option<String>,
}

pub fn key(value: &str) -> Result<(), Failure> {
    ObjectKey::try_from(value.to_owned())
        .map(|_| ())
        .map_err(|_| Failure::Invalid)
}
pub fn list_input(prefix: &str, token: Option<&str>) -> bool {
    (prefix.is_empty() || key(prefix).is_ok())
        && prefix.len() <= 1024
        && token
            .is_none_or(|v| !v.is_empty() && v.len() <= 4096 && !v.chars().any(char::is_control))
}
/// Internal runtime output; never returned by a storage HTTP endpoint.
pub struct StorageCertificate {
    pub chain: String,
    pub key: panel_proto::StorageSecret,
}
