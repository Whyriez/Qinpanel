//! Audit recorder for `panel-agent`.
//!
//! Rule 4.3(8): "Setiap command agent menulis audit record (siapa, apa, hasil)."
//! Records structured events for system operations executed by the privileged daemon.

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentAuditRecord {
    pub timestamp: u64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub status: String,
    pub details: Option<String>,
}

impl AgentAuditRecord {
    pub fn new(
        actor: impl Into<String>,
        action: impl Into<String>,
        target: impl Into<String>,
        status: impl Into<String>,
        details: Option<String>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            timestamp: now,
            actor: actor.into(),
            action: action.into(),
            target: target.into(),
            status: status.into(),
            details,
        }
    }
}

/// Emits a structured audit event for an agent operation.
pub fn record_audit(record: &AgentAuditRecord) {
    tracing::info!(
        target: "panel_agent::audit",
        timestamp = record.timestamp,
        actor = %record.actor,
        action = %record.action,
        target_resource = %record.target,
        status = %record.status,
        details = ?record.details,
        "AGENT_AUDIT: {} executed {} on {} -> {}",
        record.actor,
        record.action,
        record.target,
        record.status
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_audit_record_creation() {
        let record = AgentAuditRecord::new(
            "uid:1000",
            "ping",
            "system",
            "success",
            Some("req-1".to_string()),
        );
        assert_eq!(record.actor, "uid:1000");
        assert_eq!(record.action, "ping");
        assert_eq!(record.target, "system");
        assert_eq!(record.status, "success");
        assert_eq!(record.details.as_deref(), Some("req-1"));
        assert!(record.timestamp > 0);

        // Does not panic
        record_audit(&record);
    }
}
