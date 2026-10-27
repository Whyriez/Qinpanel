use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelUpdateRequest {
    Status,
    Logs,
    Check,
    Apply {
        id: String,
        version: String,
        digest: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PanelRelease {
    pub version: String,
    pub digest: String,
    pub notes: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PanelUpdateRun {
    pub id: String,
    pub version: String,
    pub digest: String,
    pub phase: UpdatePhase,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Queued,
    Running,
    Succeeded,
    Failed,
    Interrupted,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PanelUpdateStatus {
    pub diagnostics: Option<String>,
    pub current_version: String,
    pub installed_digest: Option<String>,
    pub supported: bool,
    pub checked_at: Option<i64>,
    pub available: Option<PanelRelease>,
    pub check_error: Option<String>,
    pub update: Option<PanelUpdateRun>,
}
