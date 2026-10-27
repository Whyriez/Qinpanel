use super::{
    StorageError,
    capability::{
        ProviderContract, QuotaEnforcement, StorageProviderContract, configuration_fingerprint,
    },
    identity::{StorageBucket, StorageConnection},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum StorageQuota {
    #[default]
    Observe,
    PanelUploads {
        limit_bytes: u64,
    },
    RequireProviderHard {
        limit_bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageSample {
    configuration_fingerprint: String,
    committed_bytes: u64,
    reserved_bytes: u64,
    observed_at: u64,
}

impl UsageSample {
    /// Measurements and reservations must come from trusted repositories or
    /// adapters, never a caller's claimed usage. This does not reserve bytes.
    pub fn from_measurement(
        connection: &StorageConnection,
        bucket: &StorageBucket,
        committed_bytes: u64,
        reserved_bytes: u64,
        observed_at: u64,
    ) -> Result<Self, StorageError> {
        if observed_at == 0 {
            return Err(StorageError::UnknownUsage);
        }
        if bucket.connection_id != connection.id || bucket.owner != connection.owner {
            return Err(StorageError::Forbidden);
        }
        Ok(Self {
            configuration_fingerprint: configuration_fingerprint(connection, bucket)?,
            committed_bytes,
            reserved_bytes,
            observed_at,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadQuotaResult {
    NotEnforced,
    /// A panel-path admission check, not a reservation or direct-SDK limit.
    PanelCheckOnly {
        remaining_bytes: u64,
    },
}

impl StorageQuota {
    pub fn validate(self, provider: &dyn StorageProviderContract) -> Result<(), StorageError> {
        match self {
            Self::Observe => Ok(()),
            Self::PanelUploads { limit_bytes } if limit_bytes > 0 => Ok(()),
            Self::RequireProviderHard { limit_bytes } if limit_bytes > 0 => {
                if provider.quota_enforcement() == QuotaEnforcement::ProviderHard {
                    Ok(())
                } else {
                    Err(StorageError::HardQuotaUnavailable)
                }
            }
            _ => Err(StorageError::QuotaExceeded),
        }
    }
    pub fn check_panel_upload(
        self,
        connection: &StorageConnection,
        bucket: &StorageBucket,
        usage: Option<UsageSample>,
        incoming_bytes: u64,
        now: u64,
    ) -> Result<UploadQuotaResult, StorageError> {
        self.validate(&ProviderContract::for_connection(connection))?;
        if bucket.connection_id != connection.id || bucket.owner != connection.owner {
            return Err(StorageError::Forbidden);
        }
        let limit = match self {
            Self::Observe => return Ok(UploadQuotaResult::NotEnforced),
            Self::PanelUploads { limit_bytes } => limit_bytes,
            // A future hard-quota driver must still admit/commit at the engine.
            Self::RequireProviderHard { .. } => return Err(StorageError::Unverified),
        };
        let usage = usage.ok_or(StorageError::UnknownUsage)?;
        if now < usage.observed_at
            || now - usage.observed_at > 60
            || usage.configuration_fingerprint != configuration_fingerprint(connection, bucket)?
        {
            return Err(StorageError::UnknownUsage);
        }
        // Count the complete new object size. Overwrite savings cannot be
        // guessed from a name or an ETag. A future caller must reserve/commit
        // multipart bytes atomically; this pure check does not do so.
        let total = usage
            .committed_bytes
            .checked_add(usage.reserved_bytes)
            .and_then(|value| value.checked_add(incoming_bytes))
            .ok_or(StorageError::QuotaExceeded)?;
        if total > limit {
            return Err(StorageError::QuotaExceeded);
        }
        Ok(UploadQuotaResult::PanelCheckOnly {
            remaining_bytes: limit - total,
        })
    }
}
