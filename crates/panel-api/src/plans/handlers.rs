//! HTTP handlers for hosting plans management.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use panel_db::models::{CreatePlan, Plan, UpdatePlan, User};
use panel_db::repositories::{SqlitePlanRepository, SqliteUserRepository};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::agent_client::{plan_to_quota_set_request, plan_to_slice_apply_request};
use crate::auth::middleware::{RequireAdmin, RequireAdminCsrf, RequireAuth};
use crate::routes::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PlanItem {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub cpu_quota_percent: i64,
    pub memory_limit_bytes: i64,
    pub memory_high_bytes: Option<i64>,
    pub tasks_max: i64,
    pub io_weight: i64,
    pub disk_limit_bytes: i64,
    pub inode_limit: i64,
    pub max_sites: i64,
    pub max_domains: i64,
    pub max_databases: i64,
    pub max_cron_jobs: i64,
    pub max_terminal_sessions: i64,
    pub max_upload_bytes: i64,
    pub shell_policy: String,
    pub allow_external_upstream: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<Plan> for PlanItem {
    fn from(p: Plan) -> Self {
        Self {
            id: p.id,
            name: p.name,
            description: p.description,
            cpu_quota_percent: p.cpu_quota_percent,
            memory_limit_bytes: p.memory_limit_bytes,
            memory_high_bytes: p.memory_high_bytes,
            tasks_max: p.tasks_max,
            io_weight: p.io_weight,
            disk_limit_bytes: p.disk_limit_bytes,
            inode_limit: p.inode_limit,
            max_sites: p.max_sites,
            max_domains: p.max_domains,
            max_databases: p.max_databases,
            max_cron_jobs: p.max_cron_jobs,
            max_terminal_sessions: p.max_terminal_sessions,
            max_upload_bytes: p.max_upload_bytes,
            shell_policy: p.shell_policy,
            allow_external_upstream: p.allow_external_upstream,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreatePlanRequest {
    pub name: String,
    pub description: Option<String>,
    pub cpu_quota_percent: Option<i64>,
    pub memory_limit_bytes: Option<i64>,
    pub memory_high_bytes: Option<i64>,
    pub tasks_max: Option<i64>,
    pub io_weight: Option<i64>,
    pub disk_limit_bytes: Option<i64>,
    pub inode_limit: Option<i64>,
    pub max_sites: Option<i64>,
    pub max_domains: Option<i64>,
    pub max_databases: Option<i64>,
    pub max_cron_jobs: Option<i64>,
    pub max_terminal_sessions: Option<i64>,
    pub max_upload_bytes: Option<i64>,
    pub shell_policy: Option<String>,
    pub allow_external_upstream: Option<bool>,
}

impl From<CreatePlanRequest> for CreatePlan {
    fn from(r: CreatePlanRequest) -> Self {
        Self {
            name: r.name,
            description: r.description,
            cpu_quota_percent: r.cpu_quota_percent,
            memory_limit_bytes: r.memory_limit_bytes,
            memory_high_bytes: r.memory_high_bytes,
            tasks_max: r.tasks_max,
            io_weight: r.io_weight,
            disk_limit_bytes: r.disk_limit_bytes,
            inode_limit: r.inode_limit,
            max_sites: r.max_sites,
            max_domains: r.max_domains,
            max_databases: r.max_databases,
            max_cron_jobs: r.max_cron_jobs,
            max_terminal_sessions: r.max_terminal_sessions,
            max_upload_bytes: r.max_upload_bytes,
            shell_policy: r.shell_policy,
            allow_external_upstream: r.allow_external_upstream,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdatePlanRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub cpu_quota_percent: Option<i64>,
    pub memory_limit_bytes: Option<i64>,
    pub memory_high_bytes: Option<i64>,
    pub tasks_max: Option<i64>,
    pub io_weight: Option<i64>,
    pub disk_limit_bytes: Option<i64>,
    pub inode_limit: Option<i64>,
    pub max_sites: Option<i64>,
    pub max_domains: Option<i64>,
    pub max_databases: Option<i64>,
    pub max_cron_jobs: Option<i64>,
    pub max_terminal_sessions: Option<i64>,
    pub max_upload_bytes: Option<i64>,
    pub shell_policy: Option<String>,
    pub allow_external_upstream: Option<bool>,
}

impl From<UpdatePlanRequest> for UpdatePlan {
    fn from(r: UpdatePlanRequest) -> Self {
        Self {
            name: r.name,
            description: r.description,
            cpu_quota_percent: r.cpu_quota_percent,
            memory_limit_bytes: r.memory_limit_bytes,
            memory_high_bytes: r.memory_high_bytes,
            tasks_max: r.tasks_max,
            io_weight: r.io_weight,
            disk_limit_bytes: r.disk_limit_bytes,
            inode_limit: r.inode_limit,
            max_sites: r.max_sites,
            max_domains: r.max_domains,
            max_databases: r.max_databases,
            max_cron_jobs: r.max_cron_jobs,
            max_terminal_sessions: r.max_terminal_sessions,
            max_upload_bytes: r.max_upload_bytes,
            shell_policy: r.shell_policy,
            allow_external_upstream: r.allow_external_upstream,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PlanListResponse {
    pub plans: Vec<PlanItem>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PlanResponse {
    pub plan: PlanItem,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PlanDeleteResponse {
    pub success: bool,
    pub message: String,
}

/// GET /api/v1/plans - List all hosting plans.
#[utoipa::path(
    get,
    path = "/api/v1/plans",
    responses(
        (status = 200, description = "List of all plans", body = PlanListResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "plans"
)]
pub async fn list_plans_handler(
    State(state): State<AppState>,
    _auth: RequireAuth,
) -> impl IntoResponse {
    let repo = SqlitePlanRepository::new(state.pool);
    match repo.list_all().await {
        Ok(plans) => {
            let items: Vec<PlanItem> = plans.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(PlanListResponse { plans: items })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/v1/plans/{id} - Get plan details by ID.
#[utoipa::path(
    get,
    path = "/api/v1/plans/{id}",
    responses(
        (status = 200, description = "Plan details", body = PlanResponse),
        (status = 404, description = "Plan not found"),
        (status = 401, description = "Unauthorized")
    ),
    tag = "plans"
)]
pub async fn get_plan_handler(
    State(state): State<AppState>,
    _auth: RequireAuth,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let repo = SqlitePlanRepository::new(state.pool);
    match repo.find_by_id(id).await {
        Ok(Some(plan)) => {
            (StatusCode::OK, Json(PlanResponse { plan: plan.into() })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("Plan with ID {id} not found") })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

fn validate_shell_policy(policy: Option<&str>) -> Result<(), &'static str> {
    match policy {
        None | Some("none" | "sftp" | "full") => Ok(()),
        Some(_) => Err("Akses shell harus none, sftp, atau full."),
    }
}

/// POST /api/v1/plans - Create a new hosting plan (admin only).
#[utoipa::path(
    post,
    path = "/api/v1/plans",
    request_body = CreatePlanRequest,
    responses(
        (status = 201, description = "Plan created", body = PlanResponse),
        (status = 400, description = "Validation error"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "plans"
)]
pub async fn create_plan_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Json(payload): Json<CreatePlanRequest>,
) -> impl IntoResponse {
    if let Err(message) = validate_shell_policy(payload.shell_policy.as_deref()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": message })),
        )
            .into_response();
    }
    let repo = SqlitePlanRepository::new(state.pool.clone());
    let create_model: CreatePlan = payload.into();
    match repo.create(&create_model).await {
        Ok(plan) => {
            state.audit_writer.log_async(
                Some(admin.0.id),
                &admin.0.username,
                "plan.create",
                &plan.name,
                "success",
                None,
                Some(serde_json::json!({ "plan_id": plan.id }).to_string()),
            );
            (
                StatusCode::CREATED,
                Json(PlanResponse { plan: plan.into() }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// PUT /api/v1/plans/{id} - Update a hosting plan and apply updated limits to active tenant slices.
#[utoipa::path(
    put,
    path = "/api/v1/plans/{id}",
    request_body = UpdatePlanRequest,
    responses(
        (status = 200, description = "Plan updated", body = PlanResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Plan not found"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "plans"
)]
pub async fn update_plan_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Path(id): Path<i64>,
    Json(payload): Json<UpdatePlanRequest>,
) -> impl IntoResponse {
    if let Err(message) = validate_shell_policy(payload.shell_policy.as_deref()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": message })),
        )
            .into_response();
    }
    let repo = SqlitePlanRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());

    let update_model: UpdatePlan = payload.into();
    let updated_plan = match repo.update(id, &update_model).await {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Propagate new limits to any tenants on this plan
    if let Ok(users) = user_repo.list_all().await {
        let tenants_on_plan: Vec<User> = users
            .into_iter()
            .filter(|u| u.plan_id == Some(id) && u.status == "active")
            .collect();

        for tenant in tenants_on_plan {
            if let (Some(uid), Some(uname)) = (tenant.linux_uid, tenant.linux_username) {
                let slice_req = plan_to_slice_apply_request(uid as u32, &updated_plan);
                let _ = state.agent_client.apply_slice(slice_req).await;

                let quota_req = plan_to_quota_set_request(uid as u32, &uname, &updated_plan);
                let _ = state.agent_client.set_quota(quota_req).await;

                if tenant.shell_policy.is_none() {
                    let _ = state
                        .agent_client
                        .apply_ssh_policy(&uname, uid as u32, &updated_plan.shell_policy)
                        .await;
                }
            }
        }
    }

    state.audit_writer.log_async(
        Some(admin.0.id),
        &admin.0.username,
        "plan.update",
        &updated_plan.name,
        "success",
        None,
        Some(serde_json::json!({ "plan_id": updated_plan.id }).to_string()),
    );

    (
        StatusCode::OK,
        Json(PlanResponse {
            plan: updated_plan.into(),
        }),
    )
        .into_response()
}

/// DELETE /api/v1/plans/{id} - Delete a hosting plan (admin only, rejected if tenants are assigned).
#[utoipa::path(
    delete,
    path = "/api/v1/plans/{id}",
    responses(
        (status = 200, description = "Plan deleted", body = PlanDeleteResponse),
        (status = 400, description = "Plan has active tenants"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "plans"
)]
pub async fn delete_plan_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let repo = SqlitePlanRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());

    // Check if tenants are assigned to this plan
    if let Ok(users) = user_repo.list_all().await {
        let has_tenants = users.iter().any(|u| u.plan_id == Some(id));
        if has_tenants {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Paket hosting tidak dapat dihapus karena masih digunakan oleh tenant"
                })),
            )
                .into_response();
        }
    }

    let plan_name = match repo.find_by_id(id).await {
        Ok(Some(p)) => p.name,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Plan {id} not found") })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    match repo.delete(id).await {
        Ok(_) => {
            state.audit_writer.log_async(
                Some(admin.0.id),
                &admin.0.username,
                "plan.delete",
                &plan_name,
                "success",
                None,
                Some(serde_json::json!({ "plan_id": id }).to_string()),
            );
            (
                StatusCode::OK,
                Json(PlanDeleteResponse {
                    success: true,
                    message: format!("Paket '{}' berhasil dihapus", plan_name),
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
