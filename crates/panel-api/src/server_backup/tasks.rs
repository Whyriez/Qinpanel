//! Mirror a supervised root worker into Tasks without ever executing its request.
use crate::{
    jobs::events::{JobProgressEvent, publish},
    routes::AppState,
};
use panel_db::models::{ExternalJobSnapshot, Job};
use panel_proto::ServerBackupRequest;
use serde_json::{Value, json};
use std::time::Duration;

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn snapshot(value: &Value, actor: Option<i64>) -> Option<ExternalJobSnapshot> {
    let value = value.get("job")?.as_object()?;
    let id = value.get("id")?.as_str()?;
    if !uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id) {
        return None;
    }
    let operation = value.get("operation")?.as_str()?;
    if !matches!(operation, "create" | "inspect" | "restore") {
        return None;
    }
    let phase = value.get("phase")?.as_str()?;
    let status = match phase {
        "queued" => "queued",
        "running" => "running",
        "succeeded" => "succeeded",
        "failed" | "interrupted" => "failed",
        _ => return None,
    };
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Menunggu status worker server.")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(2048)
        .collect::<String>();
    let measured = value
        .get("progress")
        .and_then(Value::as_i64)
        .filter(|value| (0..=100).contains(value));
    let progress = if status == "succeeded" {
        100
    } else {
        measured.unwrap_or(0)
    };
    let now = chrono::Utc::now().timestamp();
    let created = value
        .get("started_at")
        .and_then(Value::as_i64)
        .filter(|time| *time > 0)
        .unwrap_or(now);
    let updated = value
        .get("updated_at")
        .and_then(Value::as_i64)
        .filter(|time| *time >= created)
        .unwrap_or(created);
    let payload_json = json!({"external":true,"operation":operation}).to_string();
    let result_json=json!({"message":message,"progress_known":measured.is_some()||status=="succeeded","phase":phase}).to_string();
    Some(ExternalJobSnapshot {
        id: id.into(),
        user_id: actor,
        name: format!("server.backup.{operation}"),
        status: status.into(),
        progress,
        payload_json,
        result_json,
        error_message: (status == "failed").then_some(message),
        created_at: created,
        updated_at: updated,
    })
}
/// Maintenance freezes SQLite, so Tasks read the supervisor's metadata without
/// writing through its drain barrier. The same snapshot filters the full list.
pub(crate) async fn read_task_snapshot(state: &AppState) -> Option<ExternalJobSnapshot> {
    if !std::path::Path::new(crate::maintenance::FLAG).exists() {
        return None;
    }
    let value = tokio::time::timeout(
        Duration::from_secs(2),
        state
            .agent_client
            .server_backup(ServerBackupRequest::Status),
    )
    .await
    .ok()?
    .ok()?;
    if value["supported"] != true {
        return None;
    }
    snapshot(&value, None)
}
pub(crate) fn apply_task_snapshot(job: &mut Job, value: &ExternalJobSnapshot) {
    if job.id != value.id
        || job.name != value.name
        || !matches!(job.status.as_str(), "queued" | "running")
    {
        return;
    }
    job.status = value.status.clone();
    job.progress = job.progress.max(value.progress);
    job.result_json = Some(value.result_json.clone());
    job.error_message = value.error_message.clone();
    job.updated_at = job.updated_at.max(value.updated_at);
}
pub(crate) async fn task_view(state: &AppState, admin: bool, job: &mut Job) {
    if !admin
        || !matches!(
            job.name.as_str(),
            "server.backup.create" | "server.backup.inspect" | "server.backup.restore"
        )
    {
        return;
    }
    if let Some(value) = read_task_snapshot(state).await {
        apply_task_snapshot(job, &value);
    }
}
async fn save(state: &AppState, value: &ExternalJobSnapshot) -> Result<(), panel_db::DbError> {
    if let Some(job) = state
        .job_runner
        .job_repo
        .observe_server_backup(value)
        .await?
    {
        let detail: Value =
            serde_json::from_str(&job.result_json.unwrap_or_default()).unwrap_or(Value::Null);
        publish(
            &state.job_runner.job_repo,
            &state.job_runner.event_tx,
            JobProgressEvent {
                sequence: None,
                job_id: job.id,
                status: job.status,
                progress: job.progress,
                message: detail["message"].as_str().map(str::to_owned),
                timestamp: job.updated_at,
            },
        )
        .await?;
    }
    Ok(())
}
fn retired(job: Job, catalog: &Value) -> ExternalJobSnapshot {
    let operation = job.name.strip_prefix("server.backup.").unwrap_or("create");
    let ready = operation == "create"
        && catalog
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["id"] == job.id));
    let message = if ready {
        "Backup siap diunduh."
    } else {
        "Status worker untuk tugas ini tidak lagi tersedia, misalnya setelah restore snapshot. Periksa hasil backup atau log server."
    };
    ExternalJobSnapshot {id:job.id,user_id:job.user_id,name:job.name.clone(),status:if ready {"succeeded"} else {"failed"}.into(),
        progress:if ready {100} else {job.progress},payload_json:job.payload_json,
        result_json:json!({"message":message,"progress_known":ready,"phase":if ready {"succeeded"} else {"interrupted"}}).to_string(),
        error_message:(!ready).then(||message.into()),created_at:job.created_at,updated_at:chrono::Utc::now().timestamp().max(job.updated_at)}
}
pub(super) async fn observe(state: &AppState, value: &Value, actor: Option<i64>) {
    if value["supported"] != true {
        return;
    }
    let _lock = LOCK.lock().await;
    // The accepted POST is already inside maintenance::WriteGuard, so the root
    // worker waits for its task receipt even if it has just raised the flag.
    let _drain = if actor.is_some() {
        None
    } else {
        let Some(guard) = crate::maintenance::background_write() else {
            return;
        };
        Some(guard)
    };
    let current = snapshot(value, actor);
    if let Some(current) = &current
        && save(state, current).await.is_err()
    {
        tracing::warn!("Server backup was accepted but Tasks synchronization must be retried");
        return;
    }
    // Only one root backup worker can run. Snapshot-imported entries must not
    // stay 'running' forever or get replayed by the ordinary job runner.
    if (value["job"].is_null() || current.is_some())
        && let Ok(active) = state.job_runner.job_repo.active_server_backups().await
    {
        for job in active {
            if current.as_ref().is_some_and(|value| value.id == job.id) {
                continue;
            }
            let _ = save(state, &retired(job, &value["backups"])).await;
        }
    }
}
pub fn start_observer(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let busy = std::path::Path::new(crate::maintenance::FLAG).exists()
                || state
                    .job_runner
                    .job_repo
                    .active_server_backups()
                    .await
                    .is_ok_and(|jobs| !jobs.is_empty());
            if !std::path::Path::new(crate::maintenance::FLAG).exists()
                && let Ok(Ok(value)) = tokio::time::timeout(
                    Duration::from_secs(2),
                    state
                        .agent_client
                        .server_backup(ServerBackupRequest::Status),
                )
                .await
            {
                observe(&state, &value, None).await;
            }
            tokio::time::sleep(Duration::from_secs(if busy { 3 } else { 30 })).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_keeps_only_observation_metadata_never_secrets_or_replay_inputs() {
        let job = json!({"supported":true,"job":{"id":uuid::Uuid::new_v4(),"operation":"restore","phase":"running","message":"Memulihkan layanan.","progress":65,"passphrase":"fixture_secret","admin_password":"fixture_password","new_ipv4":"192.0.2.1"}});
        let value = snapshot(&job, Some(1)).expect("snapshot");
        assert_eq!(value.name, "server.backup.restore");
        assert_eq!(value.progress, 65);
        assert!(!value.payload_json.contains("fixture"));
        assert!(!value.result_json.contains("fixture"));
        assert!(!value.payload_json.contains("new_ipv4"));
        let mut unknown = job;
        unknown["job"]["operation"] = json!("arbitrary");
        assert!(snapshot(&unknown, None).is_none());
    }

    #[test]
    fn maintenance_view_projects_root_progress_without_replacing_owner_payload_or_terminal_state() {
        let value = json!({"job":{"id":uuid::Uuid::new_v4(),"operation":"create","phase":"running","message":"Encrypting","progress":75,"started_at":100,"updated_at":110}});
        let observation = snapshot(&value, None).expect("snapshot");
        let mut job = Job {
            id: observation.id.clone(),
            user_id: Some(1),
            name: observation.name.clone(),
            status: "queued".into(),
            progress: 0,
            payload_json: "metadata-only".into(),
            result_json: None,
            error_message: None,
            retry_count: 0,
            max_retries: 0,
            created_at: 100,
            updated_at: 100,
        };
        apply_task_snapshot(&mut job, &observation);
        assert_eq!(job.progress, 75);
        assert_eq!(job.status, "running");
        assert_eq!(job.user_id, Some(1));
        assert_eq!(job.payload_json, "metadata-only");
        job.status = "succeeded".into();
        job.progress = 100;
        apply_task_snapshot(&mut job, &observation);
        assert_eq!(job.status, "succeeded");
        assert_eq!(job.progress, 100);
    }

    #[tokio::test]
    async fn observation_persists_task_events_without_replay_and_reconciles_imported_snapshots() {
        use std::sync::Arc;
        let pool = panel_db::pool::init_pool("sqlite::memory:")
            .await
            .expect("pool");
        panel_db::pool::run_migrations(&pool)
            .await
            .expect("migrations");
        let temp = tempfile::tempdir().expect("temp");
        let state = AppState::new(
            Arc::new(crate::agent_client::AgentClient::new(
                temp.path().join("no-agent"),
            )),
            pool,
            "development".into(),
            crate::config::AuthConfig::default(),
            Arc::new(crate::auth::LockoutManager::new(5, 60)),
        );
        let repo = &state.job_runner.job_repo;
        let first = uuid::Uuid::new_v4().to_string();
        let mut value = json!({"supported":true,"job":{"id":first,"operation":"create","phase":"queued","progress":0,"message":"Queued","started_at":100,"updated_at":100},"backups":[]});
        observe(&state, &value, None).await;
        observe(&state, &value, None).await;
        assert_eq!(repo.events_after(&first, 0).await.expect("events").len(), 1);
        assert!(repo.claim_next_queued().await.expect("claim").is_none());
        value["job"]["phase"] = json!("running");
        value["job"]["progress"] = json!(40);
        value["job"]["message"] = json!("Snapshot");
        value["job"]["updated_at"] = json!(110);
        observe(&state, &value, None).await;
        assert_eq!(repo.events_after(&first, 0).await.expect("events").len(), 2);
        assert_eq!(repo.recover_interrupted_jobs().await.expect("restart"), 0);
        // A restored operations snapshot can lose its current task receipt.
        sqlx::query("DELETE FROM jobs WHERE id=?")
            .bind(&first)
            .execute(&state.operations_pool)
            .await
            .expect("restored snapshot");
        observe(&state, &value, None).await;
        assert_eq!(
            repo.find_by_id(&first)
                .await
                .expect("find")
                .expect("job")
                .progress,
            40
        );
        value["job"]["id"] = json!(uuid::Uuid::new_v4());
        value["backups"] = json!([{"id":first}]);
        observe(&state, &value, None).await;
        assert_eq!(
            repo.find_by_id(&first)
                .await
                .expect("find")
                .expect("job")
                .status,
            "succeeded"
        );
        observe(
            &state,
            &json!({"supported":true,"job":null,"backups":[]}),
            None,
        )
        .await;
        assert!(
            repo.active_server_backups()
                .await
                .expect("active")
                .is_empty()
        );
    }
}
