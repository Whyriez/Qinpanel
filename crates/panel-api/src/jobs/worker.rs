//! Background worker loop for claiming and processing queued jobs.

use crate::jobs::events::JobProgressEvent;
use crate::jobs::registry::JobContext;
use crate::jobs::runner::JobRunner;
use panel_db::error::DbError;
use panel_db::models::Job;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn handle_cancellation(
    runner: &Arc<JobRunner>,
    job_id: &str,
    progress: i64,
) -> Result<(), DbError> {
    runner
        .job_repo
        .update_status(
            job_id,
            "cancelled",
            None,
            Some("Tugas dibatalkan saat berjalan"),
        )
        .await?;

    crate::jobs::events::publish(
        &runner.job_repo,
        &runner.event_tx,
        JobProgressEvent {
            sequence: None,
            job_id: job_id.to_string(),
            status: "cancelled".to_string(),
            progress,
            message: Some("Tugas dibatalkan saat berjalan".to_string()),
            timestamp: current_timestamp(),
        },
    )
    .await?;

    Ok(())
}

async fn handle_success(
    runner: &Arc<JobRunner>,
    job_id: &str,
    result_json: Option<String>,
) -> Result<(), DbError> {
    runner
        .job_repo
        .update_status(job_id, "succeeded", result_json.as_deref(), None)
        .await?;

    crate::jobs::events::publish(
        &runner.job_repo,
        &runner.event_tx,
        JobProgressEvent {
            sequence: None,
            job_id: job_id.to_string(),
            status: "succeeded".to_string(),
            progress: 100,
            message: Some("Tugas selesai.".to_string()),
            timestamp: current_timestamp(),
        },
    )
    .await?;

    Ok(())
}

async fn handle_failure(runner: &Arc<JobRunner>, job: &Job, err: &str) -> Result<(), DbError> {
    warn!("Job {} ('{}') failed: {}", job.id, job.name, err);
    let updated = runner
        .job_repo
        .increment_retry_or_fail(&job.id, err)
        .await?;

    if updated.status == "queued" {
        let backoff = JobRunner::calculate_backoff(job.retry_count);
        crate::jobs::events::publish(
            &runner.job_repo,
            &runner.event_tx,
            JobProgressEvent {
                sequence: None,
                job_id: job.id.clone(),
                status: "queued".to_string(),
                progress: job.progress,
                message: Some(format!(
                    "Percobaan gagal, mengulangi ({}/{}): {}",
                    updated.retry_count, updated.max_retries, err
                )),
                timestamp: current_timestamp(),
            },
        )
        .await?;
        tokio::time::sleep(backoff).await;
    } else {
        crate::jobs::events::publish(
            &runner.job_repo,
            &runner.event_tx,
            JobProgressEvent {
                sequence: None,
                job_id: job.id.clone(),
                status: "failed".to_string(),
                progress: job.progress,
                message: Some(format!("Tugas gagal setelah seluruh percobaan: {}", err)),
                timestamp: current_timestamp(),
            },
        )
        .await?;

        // Trigger notification for job failure or deploy failure
        let notif_service =
            crate::notifications::NotificationService::new(runner.pool.clone(), None);
        let notif_service = if job.name.starts_with("email.") {
            notif_service
        } else {
            notif_service.with_job_runner(Arc::clone(runner))
        };
        let category = if job.name == "git.deploy" {
            panel_core::notification::NotificationCategory::DeployFailed
        } else if job.name.starts_with("backup.") {
            panel_core::notification::NotificationCategory::BackupFailed
        } else {
            panel_core::notification::NotificationCategory::JobFailed
        };

        let title = match category {
            panel_core::notification::NotificationCategory::DeployFailed => {
                "Eksekusi deploy Git gagal".to_string()
            }
            panel_core::notification::NotificationCategory::BackupFailed => {
                format!("Operasi cadangan '{}' gagal", job.name)
            }
            _ => format!("Pekerjaan sistem '{}' gagal", job.name),
        };

        let _ = notif_service
            .dispatch(crate::notifications::DispatchNotificationEvent {
                user_id: job.user_id,
                site_id: None,
                category,
                severity: panel_core::notification::NotificationSeverity::Error,
                title,
                message: format!("Eksekusi job {} ('{}') gagal: {}", job.id, job.name, err),
                metadata_json: Some(
                    serde_json::json!({
                        "job_id": job.id,
                        "job_name": job.name,
                        "error": err
                    })
                    .to_string(),
                ),
                action_url: Some("/jobs".to_string()),
            })
            .await;
    }

    Ok(())
}

/// Processes a single queued job if available.
/// Returns `Ok(true)` if a job was claimed and executed, or `Ok(false)` if the queue was empty.
pub async fn process_one_job(runner: &Arc<JobRunner>) -> Result<bool, DbError> {
    if std::path::Path::new(crate::maintenance::FLAG).exists() {
        return Ok(false);
    }
    let job = match runner.job_repo.claim_next_queued().await? {
        Some(j) => j,
        None => return Ok(false),
    };

    let cancel_token = CancellationToken::new();
    runner
        .cancellation_tokens
        .write()
        .await
        .insert(job.id.clone(), cancel_token.clone());

    // Cancellation can race with token registration after the atomic DB claim.
    if !runner
        .job_repo
        .find_by_id(&job.id)
        .await?
        .is_some_and(|current| current.status == "running")
    {
        runner.cancellation_tokens.write().await.remove(&job.id);
        return Ok(true);
    }

    crate::jobs::events::publish(
        &runner.job_repo,
        &runner.event_tx,
        JobProgressEvent {
            sequence: None,
            job_id: job.id.clone(),
            status: "running".to_string(),
            progress: job.progress,
            message: Some("Eksekusi tugas dimulai.".to_string()),
            timestamp: current_timestamp(),
        },
    )
    .await?;

    let ctx = JobContext {
        job_id: job.id.clone(),
        pool: runner.pool.clone(),
        job_repo: runner.job_repo.clone(),
        event_tx: runner.event_tx.clone(),
        cancel_token: cancel_token.clone(),
    };

    // Cross-file owner IDs cannot have a SQLite foreign key. Recheck the current
    // actor before *any* handler, including admin runtime installs, touches the host.
    let owner_active = match job.user_id {
        Some(id) => panel_db::repositories::SqliteUserRepository::new(runner.pool.clone())
            .find_by_id(id)
            .await?
            .is_some_and(|owner| owner.status == "active"),
        None => true,
    };
    let execution_result = if !owner_active {
        Err("Pemilik pekerjaan tidak ditemukan atau tidak aktif".to_owned())
    } else {
        match runner.registry.get(&job.name) {
            Some(handler) => handler.run(&job, &ctx).await,
            None => Err(format!("Unknown job handler: '{}'", job.name)),
        }
    };

    if cancel_token.is_cancelled() {
        let _ = handle_cancellation(runner, &job.id, job.progress).await;
    } else {
        match execution_result {
            Ok(result_json) => {
                handle_success(runner, &job.id, result_json).await?;
            }
            Err(err) => {
                handle_failure(runner, &job, &err).await?;
            }
        }
    }

    runner.cancellation_tokens.write().await.remove(&job.id);
    Ok(true)
}

async fn wait_for_job_event(
    runner: &Arc<JobRunner>,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = shutdown_rx.changed() => {
            *shutdown_rx.borrow()
        }
        _ = runner.notify.notified() => false,
        _ = tokio::time::sleep(Duration::from_millis(500)) => false,
    }
}

async fn worker_iteration(
    runner: &Arc<JobRunner>,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    match process_one_job(runner).await {
        Ok(true) => false,
        Ok(false) => wait_for_job_event(runner, shutdown_rx).await,
        Err(e) => {
            error!("Error in job worker execution: {}", e);
            tokio::time::sleep(Duration::from_millis(500)).await;
            false
        }
    }
}

/// Runs the background worker continuously until receiving a shutdown signal.
pub async fn run_worker(
    runner: Arc<JobRunner>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    info!("Background job worker started");
    while !*shutdown_rx.borrow() {
        if worker_iteration(&runner, &mut shutdown_rx).await {
            break;
        }
    }
    info!("Background job worker stopped");
}
