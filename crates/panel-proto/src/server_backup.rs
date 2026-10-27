use serde::{Deserialize, Serialize};

/// Passphrases cross only the authenticated local socket and a root-owned handoff.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ServerBackupJob {
    pub id: String,
    pub operation: ServerBackupOperation,
    pub archive_id: Option<String>,
    pub digest: Option<String>,
    pub new_ipv4: Option<String>,
    pub passphrase: String,
}
impl std::fmt::Debug for ServerBackupJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerBackupJob")
            .field("id", &self.id)
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ServerBackupOperation {
    Create,
    Inspect,
    Restore,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "operation",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ServerBackupRequest {
    Status,
    Start(ServerBackupJob),
    Delete { id: String },
    DeleteUpload { id: String },
}
