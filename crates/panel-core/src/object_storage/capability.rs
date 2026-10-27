//! Provider declarations are not credentials or permission evidence.
use super::{MANAGED_ENGINE_COMMIT, STORAGE_SCHEMA_VERSION, StorageError, identity::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const MAX_EVIDENCE_AGE_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAction {
    ListObjects,
    ReadObject,
    WriteObject,
    DeleteObject,
    PresignRead,
    PresignWrite,
    MultipartUpload,
    CreateBucket,
    DeleteBucket,
    ConfigureCors,
    IssueKey,
    PublicRead,
}
impl StorageAction {
    pub fn is_data_write(self) -> bool {
        matches!(
            self,
            Self::WriteObject | Self::DeleteObject | Self::PresignWrite | Self::MultipartUpload
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    #[default]
    Unknown,
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[serde(rename = "seaweedfs_4_50")]
    SeaweedFs450,
    AwsS3,
    CloudflareR2,
    S3Compatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaEnforcement {
    Unknown,
    MonitoringOnly,
    PanelUploadsOnly,
    ProviderPeriodic,
    ProviderHard,
}

pub trait StorageProviderContract: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn declared_support(&self, action: StorageAction) -> CapabilityState;
    fn quota_enforcement(&self) -> QuotaEnforcement;
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderContract {
    kind: ProviderKind,
}
impl ProviderContract {
    pub fn for_connection(connection: &StorageConnection) -> Self {
        let kind = match connection.config.target {
            StorageTarget::Managed { .. } => ProviderKind::SeaweedFs450,
            StorageTarget::External {
                provider: ExternalProvider::AwsS3,
            } => ProviderKind::AwsS3,
            StorageTarget::External {
                provider: ExternalProvider::CloudflareR2,
            } => ProviderKind::CloudflareR2,
            StorageTarget::External {
                provider: ExternalProvider::S3Compatible,
            } => ProviderKind::S3Compatible,
        };
        Self { kind }
    }
}
impl StorageProviderContract for ProviderContract {
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    fn declared_support(&self, action: StorageAction) -> CapabilityState {
        // Provider control planes (IAM/R2 tokens) and publication are separate
        // adapters. Do not infer them from an S3 endpoint or an access key.
        if matches!(action, StorageAction::IssueKey | StorageAction::PublicRead) {
            return CapabilityState::Unsupported;
        }
        if self.kind == ProviderKind::S3Compatible {
            CapabilityState::Unknown
        } else {
            CapabilityState::Supported
        }
    }
    fn quota_enforcement(&self) -> QuotaEnforcement {
        match self.kind {
            ProviderKind::SeaweedFs450 => QuotaEnforcement::ProviderPeriodic,
            _ => QuotaEnforcement::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCheck {
    pub action: StorageAction,
    pub state: CapabilityState,
}

/// Normalized output of a trusted future adapter, not an HTTP request DTO.
/// Fixture reports exercise the contract; they do not prove a live provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterProbeReport {
    pub schema_version: u32,
    pub provider: ProviderKind,
    pub engine_commit: Option<String>,
    pub connection_id: ConnectionId,
    pub connection_revision: u64,
    pub credential_revision: u64,
    pub configuration_fingerprint: String,
    pub bucket_id: BucketId,
    pub bucket_revision: u64,
    pub observed_at: u64,
    pub expires_at: u64,
    pub checks: Vec<CapabilityCheck>,
}

/// Staleness binding, not a signature or proof that a provider was contacted.
pub fn configuration_fingerprint(
    connection: &StorageConnection,
    bucket: &StorageBucket,
) -> Result<String, StorageError> {
    connection.config.validate(connection.owner)?;
    let bytes = serde_json::to_vec(&(STORAGE_SCHEMA_VERSION, connection, bucket))
        .map_err(|_| StorageError::InvalidConnection)?;
    Ok(crate::bytes_to_hex(&Sha256::digest(bytes)))
}

impl AdapterProbeReport {
    pub fn parse_fixture(bytes: &[u8]) -> Result<Self, StorageError> {
        if bytes.len() > 65_536 {
            return Err(StorageError::Unverified);
        }
        let report: Self = serde_json::from_slice(bytes).map_err(|_| StorageError::Unverified)?;
        report.validate_structure()?;
        Ok(report)
    }
    fn validate_structure(&self) -> Result<(), StorageError> {
        if self.schema_version != STORAGE_SCHEMA_VERSION
            || self.connection_revision == 0
            || self.credential_revision == 0
            || self.bucket_revision == 0
            || self.observed_at == 0
            || self.checks.len() > 12
            || self.expires_at <= self.observed_at
            || self.expires_at - self.observed_at > MAX_EVIDENCE_AGE_SECONDS
            || self.provider == ProviderKind::SeaweedFs450
                && self.engine_commit.as_deref() != Some(MANAGED_ENGINE_COMMIT)
            || self.provider != ProviderKind::SeaweedFs450 && self.engine_commit.is_some()
            || self.configuration_fingerprint.len() != 64
            || !self
                .configuration_fingerprint
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(StorageError::Unverified);
        }
        let mut seen = BTreeSet::new();
        if self.checks.iter().any(|check| !seen.insert(check.action)) {
            return Err(StorageError::Unverified);
        }
        Ok(())
    }
    pub fn require_current(
        &self,
        connection: &StorageConnection,
        bucket: &StorageBucket,
        now: u64,
    ) -> Result<(), StorageError> {
        self.validate_structure()?;
        if self.connection_id != connection.id
            || self.connection_revision != connection.revision
            || self.credential_revision != connection.credential_revision()
            || self.bucket_id != bucket.id
            || self.bucket_revision != bucket.revision
            || self.provider != ProviderContract::for_connection(connection).kind()
            || self.configuration_fingerprint != configuration_fingerprint(connection, bucket)?
            || now < self.observed_at
            || now >= self.expires_at
        {
            return Err(StorageError::StaleEvidence);
        }
        Ok(())
    }
    fn require_supported(&self, action: StorageAction) -> Result<(), StorageError> {
        match self
            .checks
            .iter()
            .find(|check| check.action == action)
            .map(|check| check.state)
        {
            Some(CapabilityState::Supported) => Ok(()),
            Some(CapabilityState::Unsupported) => Err(StorageError::Unsupported),
            _ => Err(StorageError::Unverified),
        }
    }
}

/// Capability-checked intent only. It contains no credentials and performs no I/O.
/// A future handler/job must reauthorize the active actor before executing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizedStorageIntent {
    connection_id: ConnectionId,
    connection_revision: u64,
    credential_revision: u64,
    bucket_id: BucketId,
    bucket_revision: u64,
    action: StorageAction,
    expires_at: u64,
    observed_at: u64,
    configuration_fingerprint: String,
}
impl AuthorizedStorageIntent {
    pub fn require_current(
        &self,
        connection: &StorageConnection,
        bucket: &StorageBucket,
        now: u64,
    ) -> Result<(), StorageError> {
        if self.connection_id != connection.id
            || self.connection_revision != connection.revision
            || self.credential_revision != connection.credential_revision()
            || self.bucket_id != bucket.id
            || self.bucket_revision != bucket.revision
            || now < self.observed_at
            || now >= self.expires_at
            || self.configuration_fingerprint != configuration_fingerprint(connection, bucket)?
        {
            return Err(StorageError::StaleEvidence);
        }
        Ok(())
    }
    pub fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }
    pub fn connection_revision(&self) -> u64 {
        self.connection_revision
    }
    pub fn credential_revision(&self) -> u64 {
        self.credential_revision
    }
    pub fn bucket_id(&self) -> BucketId {
        self.bucket_id
    }
    pub fn bucket_revision(&self) -> u64 {
        self.bucket_revision
    }
    pub fn action(&self) -> StorageAction {
        self.action
    }
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

pub fn authorize_operation(
    actor: StorageActor,
    connection: &StorageConnection,
    bucket: &StorageBucket,
    evidence: &AdapterProbeReport,
    action: StorageAction,
    now: u64,
) -> Result<AuthorizedStorageIntent, StorageError> {
    actor.authorize(bucket.owner)?;
    if bucket.connection_id != connection.id || bucket.owner != connection.owner {
        return Err(StorageError::Forbidden);
    }
    if ProviderContract::for_connection(connection).declared_support(action)
        == CapabilityState::Unsupported
    {
        return Err(StorageError::Unsupported);
    }
    evidence.require_current(connection, bucket, now)?;
    evidence.require_supported(action)?;
    Ok(AuthorizedStorageIntent {
        connection_id: connection.id,
        connection_revision: connection.revision,
        credential_revision: connection.credential_revision(),
        bucket_id: bucket.id,
        bucket_revision: bucket.revision,
        action,
        expires_at: evidence.expires_at,
        observed_at: evidence.observed_at,
        configuration_fingerprint: evidence.configuration_fingerprint.clone(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyPermissions {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
}
impl KeyPermissions {
    pub fn allows(self, action: StorageAction) -> bool {
        match action {
            StorageAction::ListObjects | StorageAction::ReadObject | StorageAction::PresignRead => {
                self.read
            }
            StorageAction::WriteObject
            | StorageAction::PresignWrite
            | StorageAction::MultipartUpload => self.write,
            StorageAction::DeleteObject => self.delete,
            _ => false,
        }
    }
}

/// Metadata for one bucket key; validation does not issue or reveal a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BucketKeyGrant {
    pub credential: CredentialRef,
    pub connection_id: ConnectionId,
    pub bucket_id: BucketId,
    pub project_id: Option<ProjectId>,
    pub permissions: KeyPermissions,
    pub expires_at: Option<u64>,
    pub revoked: bool,
}
impl BucketKeyGrant {
    /// Both grant and current_credential must come from trusted repositories.
    /// This checks metadata; native S3 key isolation must also be verified by
    /// the provider adapter before any credential is issued or disclosed.
    pub fn authorize_metadata(
        &self,
        connection: &StorageConnection,
        bucket: &StorageBucket,
        current_credential: &CredentialRef,
        project: Option<ProjectIdentity>,
        action: StorageAction,
        now: u64,
    ) -> Result<(), StorageError> {
        if &self.credential != current_credential
            || now == 0
            || self.revoked
            || self.credential.revision == 0
            || self.expires_at.is_some_and(|time| now >= time)
            || self.connection_id != connection.id
            || self.bucket_id != bucket.id
            || bucket.connection_id != connection.id
            || self.credential.owner != bucket.owner
            || bucket.owner != connection.owner
            || self.project_id != bucket.project_id
            || !self.permissions.allows(action)
        {
            return Err(StorageError::Forbidden);
        }
        match (self.project_id, project) {
            (Some(_), Some(project)) => bucket.require_project(project),
            (None, None) => Ok(()),
            _ => Err(StorageError::Forbidden),
        }
    }
}
