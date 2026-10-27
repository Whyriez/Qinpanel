//! HTTP handlers for tenant lifecycle, resources, and impersonation.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::SET_COOKIE},
    response::IntoResponse,
};
use panel_core::Username;
use panel_core::auth::{hash_password, hash_token};
use panel_db::models::{CreateSession, CreateUser};
use panel_db::repositories::{SqlitePlanRepository, SqliteSessionRepository, SqliteUserRepository};
use panel_proto::{HardeningApplyRequest, SliceResourceMetrics};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;
use utoipa::ToSchema;

use crate::agent_client::{plan_to_quota_set_request, plan_to_slice_apply_request};
use crate::auth::UserInfo;
use crate::auth::csrf::generate_csrf_token;
use crate::auth::middleware::{RequireAdmin, RequireAdminCsrf, RequireAuth, RequireCsrf};
use crate::auth::session::build_session_cookie;
use crate::routes::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct TenantListQuery {
    pub search: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TenantListItem {
    pub id: i64,
    pub username: String,
    pub email: Option<String>,
    pub status: String,
    pub linux_uid: Option<i64>,
    pub linux_username: Option<String>,
    pub plan_id: Option<i64>,
    pub plan_name: Option<String>,
    pub shell_policy: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TenantListResponse {
    pub tenants: Vec<TenantListItem>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TenantDetailResponse {
    pub tenant: TenantListItem,
    pub plan: Option<crate::plans::PlanItem>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTenantRequest {
    #[schema(example = "alice")]
    pub username_slug: String,
    #[schema(example = "alice@example.com")]
    pub email: Option<String>,
    #[schema(example = "StrongPassword123!")]
    pub password: String,
    #[schema(example = 1)]
    pub plan_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SliceResourceMetricsDto {
    pub uid: u32,
    pub slice_name: String,
    pub exists: bool,
    pub cpu_usage_usec: u64,
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub pids_current: u64,
    pub pids_peak: Option<u64>,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
}

impl From<SliceResourceMetrics> for SliceResourceMetricsDto {
    fn from(m: SliceResourceMetrics) -> Self {
        Self {
            uid: m.uid,
            slice_name: m.slice_name,
            exists: m.exists,
            cpu_usage_usec: m.cpu_usage_usec,
            memory_current_bytes: m.memory_current_bytes,
            memory_peak_bytes: m.memory_peak_bytes,
            pids_current: m.pids_current,
            pids_peak: m.pids_peak,
            io_read_bytes: m.io_read_bytes,
            io_write_bytes: m.io_write_bytes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuotaMetricsDto {
    pub uid: u32,
    pub username: String,
    pub bytes_used: u64,
    pub bytes_limit: u64,
    pub inodes_used: u64,
    pub inodes_limit: u64,
    pub backend: String,
}

impl From<panel_proto::QuotaGetResponse> for QuotaMetricsDto {
    fn from(q: panel_proto::QuotaGetResponse) -> Self {
        Self {
            uid: q.uid,
            username: q.username,
            bytes_used: q.bytes_used,
            bytes_limit: q.bytes_limit,
            inodes_used: q.inodes_used,
            inodes_limit: q.inodes_limit,
            backend: q.backend,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TenantResourceUsage {
    pub uid: u32,
    pub linux_username: String,
    pub metrics: SliceResourceMetricsDto,
    pub quota: Option<QuotaMetricsDto>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TenantActionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteTenantQuery {
    pub purge: Option<bool>,
}

/// GET /api/v1/tenants - List all tenants (admin only).
#[utoipa::path(
    get,
    path = "/api/v1/tenants",
    responses(
        (status = 200, description = "List of tenants", body = TenantListResponse),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "tenants"
)]
pub async fn list_tenants_handler(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Query(query): Query<TenantListQuery>,
) -> impl IntoResponse {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());

    let users = match user_repo.list_all().await {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let plans = plan_repo.list_all().await.unwrap_or_default();
    let plans_map: std::collections::HashMap<i64, String> =
        plans.into_iter().map(|p| (p.id, p.name)).collect();

    let mut tenant_items: Vec<TenantListItem> = users
        .into_iter()
        .filter(|u| u.role == "user")
        .filter(|u| {
            if let Some(ref st) = query.status {
                &u.status == st
            } else {
                true
            }
        })
        .filter(|u| {
            if let Some(ref term) = query.search {
                let term_lower = term.to_lowercase();
                u.username.to_lowercase().contains(&term_lower)
                    || u.email
                        .as_deref()
                        .map(|e| e.to_lowercase().contains(&term_lower))
                        .unwrap_or(false)
            } else {
                true
            }
        })
        .map(|u| {
            let plan_name = u.plan_id.and_then(|pid| plans_map.get(&pid).cloned());
            TenantListItem {
                id: u.id,
                username: u.username,
                email: u.email,
                status: u.status,
                linux_uid: u.linux_uid,
                linux_username: u.linux_username,
                plan_id: u.plan_id,
                plan_name,
                shell_policy: u.shell_policy,
                created_at: u.created_at,
                updated_at: u.updated_at,
            }
        })
        .collect();

    tenant_items.sort_by_key(|a| std::cmp::Reverse(a.id));

    (
        StatusCode::OK,
        Json(TenantListResponse {
            tenants: tenant_items,
        }),
    )
        .into_response()
}

/// POST /api/v1/tenants - Wizard to provision a new tenant user (admin only).
#[utoipa::path(
    post,
    path = "/api/v1/tenants",
    request_body = CreateTenantRequest,
    responses(
        (status = 201, description = "Tenant provisioned", body = TenantDetailResponse),
        (status = 400, description = "Validation or provisioning error"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "tenants"
)]
pub async fn create_tenant_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Json(payload): Json<CreateTenantRequest>,
) -> impl IntoResponse {
    // 1. Validate username slug format (Username generates "p_<slug>")
    let tenant_uname = match Username::from_slug(&payload.username_slug) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Invalid username slug: {e}") })),
            )
                .into_response();
        }
    };
    let linux_username = tenant_uname.as_str().to_string();

    // 2. Fetch hosting plan
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());
    let plan = match plan_repo.find_by_id(payload.plan_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Plan {} not found", payload.plan_id) })),
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

    // 3. Allocate Linux tenant UID
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let uid = match user_repo.allocate_next_uid().await {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to allocate tenant UID: {e}") })),
            )
                .into_response();
        }
    };

    // 4. Provision system account via agent
    let shell_path = panel_core::ShellPolicy::parse(&plan.shell_policy)
        .map(|p| p.as_shell_path().to_string())
        .unwrap_or_else(|_| "/usr/sbin/nologin".to_string());

    if let Err(e) = state
        .agent_client
        .create_user(&linux_username, uid, Some(shell_path), None, None)
        .await
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Agent user creation failed: {e}") })),
        )
            .into_response();
    }

    // 5. Apply cgroup v2 slice limits
    let slice_req = plan_to_slice_apply_request(uid, &plan);
    if let Err(e) = state.agent_client.apply_slice(slice_req).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Agent slice limits failed: {e}") })),
        )
            .into_response();
    }

    // 6. Apply filesystem quota
    let quota_req = plan_to_quota_set_request(uid, &linux_username, &plan);
    if let Err(e) = state.agent_client.set_quota(quota_req).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Agent disk quota failed: {e}") })),
        )
            .into_response();
    }

    // 7. Hash password and insert tenant into database
    let pw_hash = match hash_password(&payload.password) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Password hashing failed: {e}") })),
            )
                .into_response();
        }
    };

    let user_in = CreateUser {
        username: payload.username_slug.clone(),
        password_hash: pw_hash,
        role: "user".to_string(),
        email: payload.email.clone(),
        linux_username: Some(linux_username.clone()),
        linux_uid: Some(uid as i64),
        plan_id: Some(plan.id),
    };

    let created_user = match user_repo.create(&user_in).await {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to save tenant to DB: {e}") })),
            )
                .into_response();
        }
    };

    state.audit_writer.log_async(
        Some(admin.0.id),
        &admin.0.username,
        "tenant.create",
        &created_user.username,
        "success",
        None,
        Some(serde_json::json!({ "uid": uid, "plan_id": plan.id }).to_string()),
    );

    let res_item = TenantListItem {
        id: created_user.id,
        username: created_user.username,
        email: created_user.email,
        status: created_user.status,
        linux_uid: created_user.linux_uid,
        linux_username: created_user.linux_username,
        plan_id: created_user.plan_id,
        plan_name: Some(plan.name.clone()),
        shell_policy: created_user.shell_policy,
        created_at: created_user.created_at,
        updated_at: created_user.updated_at,
    };

    (
        StatusCode::CREATED,
        Json(TenantDetailResponse {
            tenant: res_item,
            plan: Some(plan.into()),
        }),
    )
        .into_response()
}

/// GET /api/v1/tenants/{id} - Get tenant details (admin or tenant self).
#[utoipa::path(
    get,
    path = "/api/v1/tenants/{id}",
    responses(
        (status = 200, description = "Tenant detail", body = TenantDetailResponse),
        (status = 404, description = "Tenant not found"),
        (status = 403, description = "Forbidden")
    ),
    tag = "tenants"
)]
pub async fn get_tenant_handler(
    State(state): State<AppState>,
    auth: RequireAuth,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());

    let user = match user_repo.find_by_id(id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Tenant {id} not found") })),
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

    // RBAC: user can only view their own profile, admin can view any
    if auth.0.role != "admin" && auth.0.id != user.id {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Access forbidden" })),
        )
            .into_response();
    }

    let plan = if let Some(pid) = user.plan_id {
        plan_repo.find_by_id(pid).await.unwrap_or(None)
    } else {
        None
    };

    let item = TenantListItem {
        id: user.id,
        username: user.username,
        email: user.email,
        status: user.status,
        linux_uid: user.linux_uid,
        linux_username: user.linux_username,
        plan_id: user.plan_id,
        plan_name: plan.as_ref().map(|p| p.name.clone()),
        shell_policy: user.shell_policy,
        created_at: user.created_at,
        updated_at: user.updated_at,
    };

    (
        StatusCode::OK,
        Json(TenantDetailResponse {
            tenant: item,
            plan: plan.map(Into::into),
        }),
    )
        .into_response()
}

/// GET /api/v1/tenants/{id}/resources - Read live cgroup v2 and quota metrics for tenant.
#[utoipa::path(
    get,
    path = "/api/v1/tenants/{id}/resources",
    responses(
        (status = 200, description = "Tenant resource usage", body = TenantResourceUsage),
        (status = 404, description = "Tenant not found"),
        (status = 403, description = "Forbidden")
    ),
    tag = "tenants"
)]
pub async fn get_tenant_resources_handler(
    State(state): State<AppState>,
    auth: RequireAuth,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_id(id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Tenant {id} not found") })),
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

    if auth.0.role != "admin" && auth.0.id != user.id {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Access forbidden" })),
        )
            .into_response();
    }

    let (uname, uid) = match crate::tenant_context::execution_identity(&user) {
        Ok(identity) => identity,
        Err((status, message)) => {
            return (status, Json(serde_json::json!({"error": message}))).into_response();
        }
    };

    // 1. Read live cgroup v2 metrics
    let cgroup_metrics = state
        .agent_client
        .read_slice_resources(uid)
        .await
        .unwrap_or_else(|_| SliceResourceMetrics {
            uid,
            slice_name: format!("panel-u{uid}.slice"),
            exists: false,
            ..Default::default()
        });

    // 2. Read quota metrics
    let quota_metrics = state
        .agent_client
        .get_quota(panel_proto::QuotaGetRequest {
            uid,
            username: uname.clone(),
            mount_point: None,
        })
        .await
        .ok();

    (
        StatusCode::OK,
        Json(TenantResourceUsage {
            uid,
            linux_username: uname,
            metrics: cgroup_metrics.into(),
            quota: quota_metrics.map(Into::into),
        }),
    )
        .into_response()
}

/// POST /api/v1/tenants/{id}/suspend - Suspend tenant account and kill active processes (admin only).
#[utoipa::path(
    post,
    path = "/api/v1/tenants/{id}/suspend",
    responses(
        (status = 200, description = "Tenant suspended", body = TenantActionResponse),
        (status = 404, description = "Tenant not found"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "tenants"
)]
pub async fn suspend_tenant_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_id(id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Tenant {id} not found") })),
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

    if let (Some(uid), Some(uname)) = (user.linux_uid, &user.linux_username)
        && let Err(e) = state.agent_client.suspend_user(uname, uid as u32).await
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Agent suspend failed: {e}") })),
        )
            .into_response();
    }

    if let Err(e) = user_repo.update_status(id, "suspended").await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    state.audit_writer.log_async(
        Some(admin.0.id),
        &admin.0.username,
        "tenant.suspend",
        &user.username,
        "success",
        None,
        Some(serde_json::json!({ "tenant_id": id }).to_string()),
    );

    (
        StatusCode::OK,
        Json(TenantActionResponse {
            success: true,
            message: format!("Tenant '{}' berhasil ditangguhkan", user.username),
        }),
    )
        .into_response()
}

/// POST /api/v1/tenants/{id}/unsuspend - Unsuspend tenant account and restore access (admin only).
#[utoipa::path(
    post,
    path = "/api/v1/tenants/{id}/unsuspend",
    responses(
        (status = 200, description = "Tenant unsuspended", body = TenantActionResponse),
        (status = 404, description = "Tenant not found"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "tenants"
)]
pub async fn unsuspend_tenant_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_id(id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Tenant {id} not found") })),
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

    let shell_path = if let Some(plan_id) = user.plan_id {
        let plan_repo = SqlitePlanRepository::new(state.pool.clone());
        if let Ok(Some(plan)) = plan_repo.find_by_id(plan_id).await {
            panel_core::ShellPolicy::parse(&plan.shell_policy)
                .ok()
                .map(|p| p.as_shell_path().to_string())
        } else {
            None
        }
    } else {
        None
    };

    if let (Some(uid), Some(uname)) = (user.linux_uid, &user.linux_username)
        && let Err(e) = state
            .agent_client
            .unsuspend_user(uname, uid as u32, shell_path)
            .await
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Agent unsuspend failed: {e}") })),
        )
            .into_response();
    }

    if let Err(e) = user_repo.update_status(id, "active").await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    state.audit_writer.log_async(
        Some(admin.0.id),
        &admin.0.username,
        "tenant.unsuspend",
        &user.username,
        "success",
        None,
        Some(serde_json::json!({ "tenant_id": id }).to_string()),
    );

    (
        StatusCode::OK,
        Json(TenantActionResponse {
            success: true,
            message: format!("Akses tenant '{}' berhasil dipulihkan", user.username),
        }),
    )
        .into_response()
}

/// DELETE /api/v1/tenants/{id} - Delete tenant account (admin only).
#[utoipa::path(
    delete,
    path = "/api/v1/tenants/{id}",
    responses(
        (status = 200, description = "Tenant deleted", body = TenantActionResponse),
        (status = 404, description = "Tenant not found"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "tenants"
)]
pub async fn delete_tenant_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Path(id): Path<i64>,
    Query(query): Query<DeleteTenantQuery>,
) -> impl IntoResponse {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_id(id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Tenant {id} not found") })),
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

    let purge = query.purge.unwrap_or(false);

    // Reserve deletion before touching Linux. The storage INSERT trigger reads
    // this status in the same SQLite write order, closing an allocation race.
    let marked = sqlx::query("UPDATE users SET status='deleting' WHERE id=? AND NOT EXISTS(SELECT 1 FROM storage_connections WHERE owner_id=? AND managed=1)")
        .bind(id).bind(id).execute(&state.pool).await;
    match marked {
        Ok(result) if result.rows_affected() == 1 => {}
        Ok(_) => return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"Tenant masih memiliki bucket Qinpanel. Kosongkan dan hapus bucket melalui Storage sebelum menghapus tenant."}))).into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"Status storage tenant belum dapat diperiksa. Penghapusan belum dijalankan."}))).into_response(),
    }

    if let (Some(uid), Some(uname)) = (user.linux_uid, &user.linux_username) {
        let _ = state
            .agent_client
            .delete_user(uname, uid as u32, purge, None)
            .await;
        let _ = state.agent_client.remove_slice(uid as u32).await;
    }

    if purge {
        let _ = user_repo.delete(id).await;
    } else {
        let _ = user_repo.update_status(id, "deleting").await;
    }

    state.audit_writer.log_async(
        Some(admin.0.id),
        &admin.0.username,
        "tenant.delete",
        &user.username,
        "success",
        None,
        Some(serde_json::json!({ "tenant_id": id, "purge": purge }).to_string()),
    );

    (
        StatusCode::OK,
        Json(TenantActionResponse {
            success: true,
            message: format!("Tenant '{}' berhasil dihapus", user.username),
        }),
    )
        .into_response()
}

/// POST /api/v1/tenants/{id}/impersonate - Admin login as tenant with audit logging and UI banner.
#[utoipa::path(
    post,
    path = "/api/v1/tenants/{id}/impersonate",
    responses(
        (status = 200, description = "Impersonation started", body = UserInfo),
        (status = 404, description = "Tenant not found"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "tenants"
)]
pub async fn impersonate_tenant_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let tenant = match user_repo.find_by_id(id).await {
        Ok(Some(u)) if u.role == "user" => u,
        Ok(Some(_)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Cannot impersonate administrative accounts" })),
            )
                .into_response();
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Tenant {id} not found") })),
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

    // 1. Create a session for target tenant
    let session_id = uuid::Uuid::new_v4().to_string();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let token_hash = hash_token(&token);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let expires_at = now + state.auth_config.session_duration_secs as i64;

    let session_repo = SqliteSessionRepository::new(state.pool.clone());
    if let Err(e) = session_repo
        .create(&CreateSession {
            id: session_id.clone(),
            user_id: tenant.id,
            token_hash,
            ip_address: None,
            user_agent: Some(format!("impersonated_by:{}", admin.0.username)),
            expires_at,
        })
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    // 2. Audit log impersonation
    let _ = state
        .audit_writer
        .log(
            Some(admin.0.id),
            &admin.0.username,
            "tenant.impersonate",
            &tenant.username,
            "success",
            None,
            Some(serde_json::json!({ "tenant_id": tenant.id }).to_string()),
        )
        .await;

    // 3. Set tenant session cookie and preserve backup cookie
    let is_secure = state
        .auth_config
        .cookie_secure
        .unwrap_or(state.environment != "development");
    let session_cookie =
        build_session_cookie(&token, state.auth_config.session_duration_secs, is_secure);

    let mut headers = HeaderMap::new();
    if let Ok(c) = session_cookie.parse() {
        headers.append(SET_COOKIE, c);
    }

    let csrf = generate_csrf_token(&session_id);
    let user_info = UserInfo {
        id: tenant.id,
        username: tenant.username,
        role: tenant.role,
        email: tenant.email,
        totp_enabled: tenant.totp_enabled,
        impersonated_by: Some(admin.0.username.clone()),
    };

    (
        headers,
        Json(serde_json::json!({
            "user": user_info,
            "csrf_token": csrf,
            "impersonated_by": admin.0.username,
        })),
    )
        .into_response()
}

/// POST /api/v1/auth/stop-impersonating - End tenant impersonation.
#[utoipa::path(
    post,
    path = "/api/v1/auth/stop-impersonating",
    responses(
        (status = 200, description = "Impersonation ended", body = TenantActionResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "auth"
)]
pub async fn stop_impersonating_handler(
    State(state): State<AppState>,
    auth: RequireAuth,
    _csrf: RequireCsrf,
) -> impl IntoResponse {
    let session_repo = SqliteSessionRepository::new(state.pool.clone());
    let _ = session_repo.delete(&auth.0.session_id).await;

    let _ = state
        .audit_writer
        .log(
            Some(auth.0.id),
            &auth.0.username,
            "tenant.impersonate.stop",
            &auth.0.username,
            "success",
            None,
            None,
        )
        .await;

    (
        StatusCode::OK,
        Json(TenantActionResponse {
            success: true,
            message: "Sesi impersonasi telah berakhir".to_string(),
        }),
    )
        .into_response()
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrerequisiteCheckDto {
    pub name: String,
    pub ok: bool,
    pub message: String,
    pub details: Option<String>,
}

impl From<panel_proto::PrerequisiteCheckItem> for PrerequisiteCheckDto {
    fn from(p: panel_proto::PrerequisiteCheckItem) -> Self {
        Self {
            name: p.name,
            ok: p.ok,
            message: p.message,
            details: p.details,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HardeningStatusDto {
    pub cgroup_v2: PrerequisiteCheckDto,
    pub acl: PrerequisiteCheckDto,
    pub quota: PrerequisiteCheckDto,
    pub hidepid: PrerequisiteCheckDto,
    pub tenant_group: PrerequisiteCheckDto,
    pub overall_ok: bool,
}

impl From<panel_proto::HardeningStatus> for HardeningStatusDto {
    fn from(h: panel_proto::HardeningStatus) -> Self {
        Self {
            cgroup_v2: h.cgroup_v2.into(),
            acl: h.acl.into(),
            quota: h.quota.into(),
            hidepid: h.hidepid.into(),
            tenant_group: h.tenant_group.into(),
            overall_ok: h.overall_ok,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HardeningApplyRequestDto {
    pub enable_hidepid: bool,
    pub ensure_tenant_group: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HardeningApplyResponseDto {
    pub hidepid_applied: bool,
    pub tenant_group_created: bool,
    pub message: String,
}

/// GET /api/v1/system/hardening - Diagnostic check of host hardening & prerequisites.
#[utoipa::path(
    get,
    path = "/api/v1/system/hardening",
    responses(
        (status = 200, description = "Hardening status", body = HardeningStatusDto),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "system"
)]
pub async fn get_hardening_status_handler(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> impl IntoResponse {
    match state.agent_client.check_hardening().await {
        Ok(status) => {
            let dto: HardeningStatusDto = status.into();
            (StatusCode::OK, Json(dto)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /api/v1/system/hardening/apply - Apply host hardening actions (admin only).
#[utoipa::path(
    post,
    path = "/api/v1/system/hardening/apply",
    request_body = HardeningApplyRequestDto,
    responses(
        (status = 200, description = "Hardening applied", body = HardeningApplyResponseDto),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "system"
)]
pub async fn apply_hardening_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Json(payload): Json<HardeningApplyRequestDto>,
) -> impl IntoResponse {
    let req = HardeningApplyRequest {
        enable_hidepid: payload.enable_hidepid,
        ensure_tenant_group: payload.ensure_tenant_group,
    };
    match state.agent_client.apply_hardening(req).await {
        Ok(res) => {
            state.audit_writer.log_async(
                Some(admin.0.id),
                &admin.0.username,
                "system.hardening.apply",
                "host",
                "success",
                None,
                Some(serde_json::json!({ "message": &res.message }).to_string()),
            );
            let dto = HardeningApplyResponseDto {
                hidepid_applied: res.hidepid_applied,
                tenant_group_created: res.tenant_group_created,
                message: res.message,
            };
            (StatusCode::OK, Json(dto)).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TenantShellPolicyResponse {
    pub user_id: i64,
    pub username: String,
    pub shell_policy: Option<String>,
    pub effective_policy: String,
    pub inherited_from_plan: bool,
    pub plan_policy: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTenantShellPolicyRequest {
    /// Desired shell policy ("none", "sftp", "full"), or null to inherit from plan.
    pub shell_policy: Option<String>,
}

/// GET /api/v1/tenants/{id}/shell-policy - Get tenant's current and effective shell policy
#[utoipa::path(
    get,
    path = "/api/v1/tenants/{id}/shell-policy",
    responses(
        (status = 200, description = "Tenant shell policy", body = TenantShellPolicyResponse),
        (status = 404, description = "Tenant not found"),
        (status = 403, description = "Forbidden")
    ),
    tag = "tenants"
)]
pub async fn get_tenant_shell_policy_handler(
    State(state): State<AppState>,
    auth: RequireAuth,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if !auth.0.is_admin() && auth.0.id != id {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Forbidden" })),
        )
            .into_response();
    }

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_id(id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Tenant not found" })),
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

    let plan_repo = SqlitePlanRepository::new(state.pool.clone());
    let plan = if let Some(pid) = user.plan_id {
        plan_repo.find_by_id(pid).await.ok().flatten()
    } else {
        None
    };

    let plan_policy = plan.as_ref().map(|p| p.shell_policy.clone());
    let inherited = user.shell_policy.is_none();
    let effective = user
        .shell_policy
        .as_deref()
        .or(plan_policy.as_deref())
        .unwrap_or("none")
        .to_string();

    (
        StatusCode::OK,
        Json(TenantShellPolicyResponse {
            user_id: user.id,
            username: user.username,
            shell_policy: user.shell_policy,
            effective_policy: effective,
            inherited_from_plan: inherited,
            plan_policy,
        }),
    )
        .into_response()
}

/// PUT /api/v1/tenants/{id}/shell-policy - Update tenant's shell policy override (admin only)
#[utoipa::path(
    put,
    path = "/api/v1/tenants/{id}/shell-policy",
    request_body = UpdateTenantShellPolicyRequest,
    responses(
        (status = 200, description = "Tenant shell policy updated", body = TenantShellPolicyResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Tenant not found"),
        (status = 403, description = "Forbidden - Admin only")
    ),
    tag = "tenants"
)]
pub async fn update_tenant_shell_policy_handler(
    State(state): State<AppState>,
    admin: RequireAdmin,
    _csrf: RequireAdminCsrf,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateTenantShellPolicyRequest>,
) -> impl IntoResponse {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = match user_repo.find_by_id(id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Tenant not found" })),
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

    if let Some(ref pol) = payload.shell_policy
        && let Err(e) = panel_core::ShellPolicy::parse(pol)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    if let Err(e) = user_repo
        .update_shell_policy(id, payload.shell_policy.as_deref())
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let plan_repo = SqlitePlanRepository::new(state.pool.clone());
    let plan = if let Some(pid) = user.plan_id {
        plan_repo.find_by_id(pid).await.ok().flatten()
    } else {
        None
    };

    let plan_policy = plan.as_ref().map(|p| p.shell_policy.clone());
    let inherited = payload.shell_policy.is_none();
    let effective = payload
        .shell_policy
        .as_deref()
        .or(plan_policy.as_deref())
        .unwrap_or("none")
        .to_string();

    // Propagate to agent if linux user exists
    if let (Some(uid), Some(uname)) = (user.linux_uid, &user.linux_username)
        && let Err(e) = state
            .agent_client
            .apply_ssh_policy(uname, uid as u32, &effective)
            .await
    {
        warn!("Failed to apply shell policy to agent for {uname}: {e}");
    }

    state.audit_writer.log_async(
        Some(admin.0.id),
        &admin.0.username,
        "tenant.shell_policy_update",
        &user.username,
        "success",
        None,
        Some(
            serde_json::json!({
                "user_id": user.id,
                "shell_policy": payload.shell_policy,
                "effective_policy": effective,
                "inherited": inherited,
            })
            .to_string(),
        ),
    );

    (
        StatusCode::OK,
        Json(TenantShellPolicyResponse {
            user_id: user.id,
            username: user.username,
            shell_policy: payload.shell_policy,
            effective_policy: effective,
            inherited_from_plan: inherited,
            plan_policy,
        }),
    )
        .into_response()
}
