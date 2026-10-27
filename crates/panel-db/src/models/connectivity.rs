//! Public metadata intentionally excludes provider bindings and secret references.
use panel_core::connectivity::{
    ClientRequirement, EndpointConfig, EndpointEvidence, EndpointService,
};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceEndpoint {
    pub id: String,
    pub revision: i64,
    #[serde(flatten)]
    pub config: EndpointConfig,
    pub client_requirement: ClientRequirement,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Independent evidence dimensions, not one synthetic "ready" badge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndpointEvidenceRecord {
    pub id: i64,
    pub endpoint_id: String,
    pub service: EndpointService,
    #[serde(flatten)]
    pub evidence: EndpointEvidence,
    pub current: bool,
    pub expired: bool,
    pub revision_matches: bool,
    pub suggested_action: &'static str,
}
