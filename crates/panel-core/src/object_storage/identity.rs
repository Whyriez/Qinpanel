use super::{MANAGED_ENGINE_COMMIT, MANAGED_ENGINE_VERSION, StorageError};
use crate::{connectivity::EndpointHost, rbac::Role};
use serde::{Deserialize, Serialize};
use std::{net::Ipv4Addr, num::NonZeroU16};
use uuid::Uuid;

macro_rules! numeric_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "i64", into = "i64")]
        pub struct $name(i64);
        impl TryFrom<i64> for $name {
            type Error = StorageError;
            fn try_from(value: i64) -> Result<Self, Self::Error> {
                if value > 0 {
                    Ok(Self(value))
                } else {
                    Err(StorageError::InvalidIdentity)
                }
            }
        }
        impl From<$name> for i64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
numeric_id!(TenantId);
numeric_id!(ProjectId);

macro_rules! storage_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);
        impl TryFrom<Uuid> for $name {
            type Error = StorageError;
            fn try_from(value: Uuid) -> Result<Self, Self::Error> {
                if value.is_nil() {
                    Err(StorageError::InvalidIdentity)
                } else {
                    Ok(Self(value))
                }
            }
        }
        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
storage_id!(ConnectionId);
storage_id!(BucketId);
storage_id!(InstanceId);
storage_id!(CredentialId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BucketName(String);
impl TryFrom<String> for BucketName {
    type Error = StorageError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let bytes = value.as_bytes();
        let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
        if !(3..=63).contains(&bytes.len())
            || !bytes.first().copied().is_some_and(alnum)
            || !bytes.last().copied().is_some_and(alnum)
            || !bytes.iter().all(|b| alnum(*b) || matches!(*b, b'.' | b'-'))
            || value.contains("..")
            || value.parse::<Ipv4Addr>().is_ok()
            || value.split('.').count() == 4
                && value
                    .split('.')
                    .all(|v| v.parse::<u16>().is_ok_and(|v| v <= 255))
            || ["xn--", "sthree-", "amzn-s3-demo-"]
                .iter()
                .any(|v| value.starts_with(v))
            || [
                "-s3alias",
                "--ol-s3",
                ".mrap",
                "--x-s3",
                "--table-s3",
                "-an",
            ]
            .iter()
            .any(|v| value.ends_with(v))
        {
            return Err(StorageError::InvalidBucketName);
        }
        Ok(Self(value))
    }
}
impl From<BucketName> for String {
    fn from(value: BucketName) -> Self {
        value.0
    }
}
impl BucketName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ObjectKey(String);
impl TryFrom<String> for ObjectKey {
    type Error = StorageError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 1024
            || value.chars().any(|c| c.is_control() || c == '\\')
            || value.split('/').any(|part| matches!(part, "." | ".."))
        {
            return Err(StorageError::InvalidObjectKey);
        }
        Ok(Self(value))
    }
}
impl From<ObjectKey> for String {
    fn from(value: ObjectKey) -> Self {
        value.0
    }
}
impl ObjectKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Construct only from authenticated server-side identity; never a request DTO.
#[derive(Debug, Clone, Copy)]
pub struct StorageActor {
    id: TenantId,
    admin: bool,
}
impl StorageActor {
    pub fn from_authenticated(id: i64, role: Role, active: bool) -> Result<Self, StorageError> {
        if !active || !matches!(role, Role::Admin | Role::User) {
            return Err(StorageError::Forbidden);
        }
        Ok(Self {
            id: id.try_into()?,
            admin: role == Role::Admin,
        })
    }
    pub fn require_admin(self) -> Result<(), StorageError> {
        if self.admin {
            Ok(())
        } else {
            Err(StorageError::Forbidden)
        }
    }
    pub fn authorize(self, owner: TenantId) -> Result<(), StorageError> {
        if self.admin || self.id == owner {
            Ok(())
        } else {
            Err(StorageError::Forbidden)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProjectIdentity {
    pub id: ProjectId,
    pub tenant_id: TenantId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalProvider {
    AwsS3,
    CloudflareR2,
    S3Compatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum StorageTarget {
    Managed {
        instance_id: InstanceId,
        instance_revision: u64,
    },
    External {
        provider: ExternalProvider,
    },
}

impl StorageTarget {
    pub fn managed_engine(&self) -> Option<(&'static str, &'static str, &'static str)> {
        matches!(self, Self::Managed { .. }).then_some((
            "seaweedfs",
            MANAGED_ENGINE_VERSION,
            MANAGED_ENGINE_COMMIT,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressingStyle {
    Path,
    VirtualHost,
}

/// An HTTPS origin only; no URL userinfo, path, query, redirect or insecure flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageEndpoint {
    pub host: EndpointHost,
    pub port: NonZeroU16,
}
impl StorageEndpoint {
    pub fn origin(&self) -> String {
        format!("https://{}", self.host.authority(self.port))
    }
    pub fn require_separate_origin(&self, panel: &Self) -> Result<(), StorageError> {
        if self == panel {
            Err(StorageError::ForbiddenEndpoint)
        } else {
            Ok(())
        }
    }
}

/// Secret ownership/revision reference, never secret access-key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CredentialRef {
    pub id: CredentialId,
    pub owner: TenantId,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectionConfig {
    pub target: StorageTarget,
    pub endpoint: StorageEndpoint,
    pub region: String,
    pub addressing: AddressingStyle,
    pub credential: Option<CredentialRef>,
}

/// Request shape carries an identifier, never a claim about secret ownership.
/// The caller must load the credential record from its server-side repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDraft {
    pub target: StorageTarget,
    pub endpoint: StorageEndpoint,
    pub region: String,
    pub addressing: AddressingStyle,
    pub credential_id: Option<CredentialId>,
}

impl ConnectionDraft {
    pub fn resolve(
        self,
        owner: TenantId,
        credential: Option<CredentialRef>,
    ) -> Result<ConnectionConfig, StorageError> {
        if self.credential_id != credential.map(|value| value.id) {
            return Err(StorageError::InvalidConnection);
        }
        let config = ConnectionConfig {
            target: self.target,
            endpoint: self.endpoint,
            region: self.region,
            addressing: self.addressing,
            credential,
        };
        config.validate(owner)?;
        Ok(config)
    }
}

impl ConnectionConfig {
    pub fn validate(&self, owner: TenantId) -> Result<(), StorageError> {
        if self.region.is_empty()
            || self.region.len() > 63
            || !self
                .region
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || self
                .credential
                .is_some_and(|c| c.owner != owner || c.revision == 0)
        {
            return Err(StorageError::InvalidConnection);
        }
        match self.target {
            StorageTarget::Managed {
                instance_revision, ..
            } if instance_revision == 0
                || self.credential.is_some()
                || self.addressing != AddressingStyle::Path =>
            {
                Err(StorageError::InvalidConnection)
            }
            StorageTarget::External { .. } if self.credential.is_none() => {
                Err(StorageError::InvalidConnection)
            }
            StorageTarget::External {
                provider: ExternalProvider::CloudflareR2,
            } if self.region != "auto" || self.addressing != AddressingStyle::Path => {
                Err(StorageError::InvalidConnection)
            }
            _ => Ok(()),
        }
    }
}

/// Trusted metadata built by a future repository/handler, not deserialized input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageConnection {
    pub id: ConnectionId,
    pub revision: u64,
    pub owner: TenantId,
    pub config: ConnectionConfig,
}
impl StorageConnection {
    pub fn new(
        actor: StorageActor,
        id: ConnectionId,
        revision: u64,
        owner: TenantId,
        config: ConnectionConfig,
    ) -> Result<Self, StorageError> {
        actor.require_admin()?;
        if revision == 0 {
            return Err(StorageError::InvalidIdentity);
        }
        config.validate(owner)?;
        Ok(Self {
            id,
            revision,
            owner,
            config,
        })
    }
    pub fn credential_revision(&self) -> u64 {
        match self.config.target {
            StorageTarget::Managed {
                instance_revision, ..
            } => instance_revision,
            StorageTarget::External { .. } => self.config.credential.map_or(0, |c| c.revision),
        }
    }
    pub fn fingerprint(&self) -> Result<String, StorageError> {
        use sha2::{Digest, Sha256};
        self.config.validate(self.owner)?;
        let bytes = serde_json::to_vec(&(super::STORAGE_SCHEMA_VERSION, self))
            .map_err(|_| StorageError::InvalidConnection)?;
        Ok(crate::bytes_to_hex(&Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BucketVisibility {
    #[default]
    Private,
    PublicRead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageBucket {
    pub id: BucketId,
    pub revision: u64,
    pub connection_id: ConnectionId,
    pub owner: TenantId,
    pub project_id: Option<ProjectId>,
    pub name: BucketName,
    pub visibility: BucketVisibility,
}
impl StorageBucket {
    pub fn new(
        actor: StorageActor,
        connection: &StorageConnection,
        id: BucketId,
        name: BucketName,
        project: Option<ProjectIdentity>,
    ) -> Result<Self, StorageError> {
        actor.authorize(connection.owner)?;
        if project.is_some_and(|p| p.tenant_id != connection.owner)
            || connection.config.addressing == AddressingStyle::VirtualHost
                && name.as_str().contains('.')
        {
            return Err(StorageError::Forbidden);
        }
        Ok(Self {
            id,
            revision: 1,
            connection_id: connection.id,
            owner: connection.owner,
            project_id: project.map(|p| p.id),
            name,
            visibility: BucketVisibility::Private,
        })
    }
    pub fn require_project(&self, project: ProjectIdentity) -> Result<(), StorageError> {
        if self.owner == project.tenant_id && self.project_id == Some(project.id) {
            Ok(())
        } else {
            Err(StorageError::Forbidden)
        }
    }
}
