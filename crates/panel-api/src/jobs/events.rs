//! Job progress event types for SSE and background worker communication.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Real-time progress event emitted during background job execution.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobProgressEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i64>,
    /// ID of the job
    pub job_id: String,
    /// Current job status ('queued', 'running', 'succeeded', 'failed', 'cancelled')
    pub status: String,
    /// Completion percentage (0 - 100)
    pub progress: i64,
    /// Optional progress log or error message
    pub message: Option<String>,
    /// Unix timestamp (seconds) when this event was emitted
    pub timestamp: i64,
}

pub async fn publish(
    repo: &panel_db::repositories::SqliteJobRepository,
    tx: &tokio::sync::broadcast::Sender<JobProgressEvent>,
    mut event: JobProgressEvent,
) -> Result<(), panel_db::DbError> {
    // Each wire message is bounded even if a caller supplies a large diagnostic.
    if let Some(message) = &mut event.message {
        *message = message.chars().take(8192).collect();
    }
    event.sequence = Some(
        repo.append_event(&panel_db::models::job::JobEvent {
            sequence: 0,
            job_id: event.job_id.clone(),
            status: event.status.clone(),
            progress: event.progress,
            message: event.message.clone(),
            timestamp: event.timestamp,
        })
        .await?,
    );
    let _ = tx.send(event);
    Ok(())
}
