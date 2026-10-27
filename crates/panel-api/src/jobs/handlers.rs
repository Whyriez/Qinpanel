//! HTTP and SSE endpoints for job management, query, cancellation, and progress streaming.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream::StreamExt;
use panel_core::rbac::Action;
use panel_db::models::Job;
use panel_db::repositories::SqliteJobRepository;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct JobLogQuery {
    pub after: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/jobs/{id}/logs", params(("id" = String, Path), JobLogQuery),
    responses((status = 200, body = Vec<crate::jobs::events::JobProgressEvent>)), tag = "jobs")]
pub async fn job_logs_handler(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path(id): Path<String>,
    Query(query): Query<JobLogQuery>,
) -> Response {
    let repo = SqliteJobRepository::new(state.operations_read_pool.clone());
    let job = match repo.find_by_id(&id).await {
        Ok(Some(job)) => job,
        Ok(None) => return (StatusCode::NOT_FOUND, "Tugas tidak ditemukan").into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Gagal memuat tugas").into_response(),
    };
    if !user.is_admin()
        && !job
            .user_id
            .is_some_and(|owner_id| user.authorize(&Action::ManageJob { owner_id }))
    {
        return (StatusCode::FORBIDDEN, "Akses log tugas ditolak").into_response();
    }
    match repo
        .events_after(&id, query.after.unwrap_or(0).max(0))
        .await
    {
        Ok(events) => Json(
            events
                .into_iter()
                .map(|event| JobProgressEvent {
                    sequence: Some(event.sequence),
                    job_id: event.job_id,
                    status: event.status,
                    progress: event.progress,
                    message: event.message,
                    timestamp: event.timestamp,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Gagal memuat riwayat log tugas",
        )
            .into_response(),
    }
}

use crate::auth::{RequireAuth, RequireCsrf};
use crate::jobs::events::JobProgressEvent;
use crate::routes::AppState;

/// Representation of a background job in API responses.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobItem {
    pub id: String,
    pub user_id: Option<i64>,
    pub name: String,
    pub status: String,
    pub progress: i64,
    pub payload_json: String,
    pub result_json: Option<String>,
    pub error_message: Option<String>,
    pub retry_count: i64,
    pub max_retries: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<Job> for JobItem {
    fn from(j: Job) -> Self {
        Self {
            id: j.id,
            user_id: j.user_id,
            name: j.name,
            status: j.status,
            progress: j.progress,
            payload_json: j.payload_json,
            result_json: j.result_json,
            error_message: j.error_message,
            retry_count: j.retry_count,
            max_retries: j.max_retries,
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}

/// Request payload to enqueue a new background job.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateJobRequest {
    /// Optional client-supplied UUID for idempotency
    pub id: Option<String>,
    /// Registered job name, e.g. "test.sleep", "system.echo"
    pub name: String,
    /// Arbitrary JSON payload for the job handler
    pub payload: Option<serde_json::Value>,
    /// Maximum retries on failure (default: 3)
    pub max_retries: Option<i64>,
}

/// Query parameters for listing jobs.
#[derive(Debug, Deserialize, IntoParams)]
pub struct JobQuery {
    pub user_id: Option<i64>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// Paginated list of jobs.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobListResponse {
    pub items: Vec<JobItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs",
    request_body = CreateJobRequest,
    responses(
        (status = 201, description = "Job enqueued successfully", body = JobItem),
        (status = 200, description = "Existing job returned (idempotency key matched)", body = JobItem),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "CSRF verification failed"),
        (status = 500, description = "Internal database error")
    ),
    tag = "jobs"
)]
pub async fn create_job_handler(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Json(payload): Json<CreateJobRequest>,
) -> Response {
    // Host work must enter through the feature endpoint that validates tenant/site ownership.
    if !user.is_admin() && !matches!(payload.name.as_str(), "test.sleep" | "system.echo") {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error":"Gunakan endpoint fitur yang sesuai untuk membuat pekerjaan tenant."}))).into_response();
    }
    if payload.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Job name cannot be empty").into_response();
    }

    let payload_json = payload
        .payload
        .map(|v| serde_json::to_string(&v).unwrap_or_else(|_| "{}".to_string()))
        .unwrap_or_else(|| "{}".to_string());

    let (job, is_duplicate) = match state
        .job_runner
        .enqueue(
            Some(user.id),
            payload.id,
            payload.name.clone(),
            payload_json,
            payload.max_retries,
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to enqueue job: {}", e),
            )
                .into_response();
        }
    };

    if !is_duplicate {
        state.audit_writer.log_async(
            Some(user.id),
            &user.username,
            "job.create",
            format!("job:{}", job.id),
            "success",
            None,
            Some(format!(r#"{{"name":"{}"}}"#, job.name)),
        );
        (StatusCode::CREATED, Json(JobItem::from(job))).into_response()
    } else {
        (StatusCode::OK, Json(JobItem::from(job))).into_response()
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs",
    params(JobQuery),
    responses(
        (status = 200, description = "Paginated list of background jobs", body = JobListResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal database error")
    ),
    tag = "jobs"
)]
pub async fn list_jobs_handler(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Query(query): Query<JobQuery>,
) -> Response {
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;

    // RBAC: regular user can only view their own jobs
    let target_user_id = if user.is_admin() {
        query.user_id
    } else {
        Some(user.id)
    };

    let status_filter = query.status.as_deref();
    let job_repo = SqliteJobRepository::new(state.operations_read_pool.clone());

    let observation = if user.is_admin() {
        crate::server_backup::read_task_snapshot(&state).await
    } else {
        None
    };
    let mut items = match job_repo
        .list_paginated_observing(
            target_user_id,
            status_filter,
            per_page,
            offset,
            observation.as_ref(),
        )
        .await
    {
        Ok(items) => items,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
                .into_response();
        }
    };

    let total = match job_repo
        .count_filtered_observing(target_user_id, status_filter, observation.as_ref())
        .await
    {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
                .into_response();
        }
    };

    let total_pages = if total == 0 {
        0
    } else {
        (total + per_page - 1) / per_page
    };

    if let Some(observation) = &observation {
        for job in &mut items {
            crate::server_backup::apply_task_snapshot(job, observation);
        }
    }
    let response = JobListResponse {
        items: items.into_iter().map(JobItem::from).collect(),
        total,
        page,
        per_page,
        total_pages,
    };

    Json(response).into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}",
    responses(
        (status = 200, description = "Job details", body = JobItem),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Cannot access another user's job"),
        (status = 404, description = "Job not found"),
        (status = 500, description = "Internal database error")
    ),
    tag = "jobs"
)]
pub async fn get_job_handler(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path(id): Path<String>,
) -> Response {
    let job_repo = SqliteJobRepository::new(state.operations_read_pool.clone());
    let mut job = match job_repo.find_by_id(&id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("Job {} not found", id)).into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
                .into_response();
        }
    };

    // RBAC ownership check
    if !user.is_admin() {
        if let Some(owner_id) = job.user_id {
            if !user.authorize(&Action::ManageJob { owner_id }) {
                return (
                    StatusCode::FORBIDDEN,
                    "Forbidden: Not authorized to access this job",
                )
                    .into_response();
            }
        } else {
            return (
                StatusCode::FORBIDDEN,
                "Forbidden: Not authorized to access system job",
            )
                .into_response();
        }
    }

    crate::server_backup::task_view(&state, user.is_admin(), &mut job).await;

    Json(JobItem::from(job)).into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs/{id}/cancel",
    responses(
        (status = 200, description = "Tugas dibatalkan", body = JobItem),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Cannot cancel another user's job"),
        (status = 409, description = "Running installation or email operation cannot be cancelled safely"),
        (status = 404, description = "Job not found"),
        (status = 500, description = "Internal database error")
    ),
    tag = "jobs"
)]
pub async fn cancel_job_handler(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(id): Path<String>,
) -> Response {
    let job_repo = SqliteJobRepository::new(state.operations_read_pool.clone());
    let job = match job_repo.find_by_id(&id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("Job {} not found", id)).into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
                .into_response();
        }
    };

    // RBAC ownership check
    if !user.is_admin() {
        if let Some(owner_id) = job.user_id {
            if !user.authorize(&Action::ManageJob { owner_id }) {
                return (
                    StatusCode::FORBIDDEN,
                    "Forbidden: Not authorized to cancel this job",
                )
                    .into_response();
            }
        } else {
            return (
                StatusCode::FORBIDDEN,
                "Forbidden: Not authorized to cancel system job",
            )
                .into_response();
        }
    }

    let updated = match state.job_runner.cancel(&id).await {
        Ok(u) => u.unwrap_or(job),
        Err(panel_db::DbError::Conflict(message)) => {
            return (StatusCode::CONFLICT, message).into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to cancel job: {}", e),
            )
                .into_response();
        }
    };

    state.audit_writer.log_async(
        Some(user.id),
        &user.username,
        "job.cancel",
        format!("job:{}", id),
        "success",
        None,
        None,
    );

    Json(JobItem::from(updated)).into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}/events",
    responses(
        (status = 200, description = "Real-time SSE progress stream", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Cannot access another user's job"),
        (status = 404, description = "Job not found"),
        (status = 500, description = "Internal database error")
    ),
    tag = "jobs"
)]
pub async fn job_events_handler(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path(id): Path<String>,
) -> Response {
    // Subscribe before reading the snapshot, so completion cannot fall into a gap.
    let rx = state.job_runner.subscribe();
    let job_repo = SqliteJobRepository::new(state.operations_read_pool.clone());
    let mut job = match job_repo.find_by_id(&id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("Job {} not found", id)).into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
                .into_response();
        }
    };

    // RBAC ownership check
    if !user.is_admin() {
        if let Some(owner_id) = job.user_id {
            if !user.authorize(&Action::ManageJob { owner_id }) {
                return (
                    StatusCode::FORBIDDEN,
                    "Forbidden: Not authorized to stream this job's events",
                )
                    .into_response();
            }
        } else {
            return (
                StatusCode::FORBIDDEN,
                "Forbidden: Not authorized to stream system job",
            )
                .into_response();
        }
    }

    crate::server_backup::task_view(&state, user.is_admin(), &mut job).await;

    let initial_event = JobProgressEvent {
        sequence: None,
        job_id: job.id.clone(),
        status: job.status.clone(),
        progress: job.progress,
        message: initial_job_message(&job),
        timestamp: job.updated_at,
    };

    let is_already_terminal = matches!(job.status.as_str(), "succeeded" | "failed" | "cancelled");

    let initial_data = serde_json::to_string(&initial_event).unwrap_or_default();
    let initial_sse = Event::default().event("progress").data(initial_data);
    let initial_stream =
        futures_util::stream::once(async move { Ok::<_, Infallible>(initial_sse) });

    if is_already_terminal {
        // Stream only the final state and close
        let mut response = Sse::new(initial_stream.boxed())
            .keep_alive(KeepAlive::default())
            .into_response();
        response.headers_mut().insert(
            "x-accel-buffering",
            axum::http::HeaderValue::from_static("no"),
        );
        return response;
    }

    let target_id = id.clone();

    let rx_stream = futures_util::stream::unfold(Some((rx, target_id)), |state| async move {
        let (mut rx, target_id) = state?;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if event.job_id == target_id {
                        let is_terminal =
                            matches!(event.status.as_str(), "succeeded" | "failed" | "cancelled");
                        let data = serde_json::to_string(&event).unwrap_or_default();
                        let sse_event = Event::default().event("progress").data(data);
                        if is_terminal {
                            return Some((Ok::<_, Infallible>(sse_event), None));
                        } else {
                            return Some((Ok::<_, Infallible>(sse_event), Some((rx, target_id))));
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let combined_stream = initial_stream.chain(rx_stream).boxed();
    let mut response = Sse::new(combined_stream)
        .keep_alive(KeepAlive::default())
        .into_response();
    response.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    response
}

fn initial_job_message(job: &panel_db::models::Job) -> Option<String> {
    if matches!(
        job.name.as_str(),
        "server.backup.create" | "server.backup.inspect" | "server.backup.restore"
    ) {
        job.result_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value["message"].as_str().map(str::to_owned))
    } else {
        None
    }
    .or_else(|| job.error_message.clone())
    .or_else(|| {
        if job.status == "succeeded" {
            Some("Tugas selesai.".to_string())
        } else {
            None
        }
    })
}
