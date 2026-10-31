//! REST API handlers for Database Provisioner (MariaDB / PostgreSQL).
//!
//! Provides endpoints for:
//! - Engine server lifecycle and on-demand installation (admin)
//! - Multi-tenant database creation with mandatory prefixes and plan quota enforcement
//! - Password rotation and remote access management (default localhost)
//! - Asynchronous export and import dumps via the background JobRunner

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use panel_core::database::{
    DatabaseEngine, generate_secure_db_password, tenant_db_prefix, validate_allowed_hosts,
    validate_database_name, validate_database_password, validate_database_user, validate_dump_path,
};
use panel_db::models::{
    CreateAuditLog, CreateDatabaseInput, DatabaseModel, DatabaseServer, UpdateDatabaseInput,
};
use panel_db::{
    SqliteAuditLogRepository, SqliteDatabaseRepository, SqlitePlanRepository, SqliteSiteRepository,
    SqliteUserRepository,
};
use panel_proto::{
    DatabaseCreateRequest, DatabaseRotatePasswordRequest, DatabaseServerActionRequest,
    DatabaseServerStatusRequest, DatabaseUpdateRemoteAccessRequest,
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;
use utoipa::ToSchema;

/// Query parameters for listing databases.
#[derive(Debug, Deserialize)]
pub struct ListDatabasesQuery {
    pub site_id: Option<i64>,
    pub user_id: Option<i64>,
}

/// Response payload for database engine server status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DatabaseServerResponse {
    pub engine: String,
    pub is_installed: bool,
    pub is_running: bool,
    pub version: Option<String>,
    pub port: i64,
    pub socket_path: Option<String>,
    pub updated_at: i64,
}

/// Tenant-safe live capabilities; never exposes server paths or cached readiness.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AvailableDatabaseEngine {
    pub engine: String,
    pub label: String,
    pub port: u16,
}

async fn available_database_engines(
    state: &AppState,
) -> Result<Vec<AvailableDatabaseEngine>, String> {
    let response = state
        .agent_client
        .database_server_status(DatabaseServerStatusRequest {
            engines: DatabaseEngine::ALL
                .iter()
                .map(|engine| engine.as_str().to_string())
                .collect(),
        })
        .await
        .map_err(|_| {
            "Ketersediaan mesin basis data belum dapat diperiksa. Coba lagi.".to_string()
        })?;
    let mut available = Vec::new();
    for engine in DatabaseEngine::ALL {
        let mut entries = response
            .servers
            .iter()
            .filter(|item| item.engine == engine.as_str());
        let Some(status) = entries.next() else {
            return Err("Respons ketersediaan mesin basis data tidak lengkap. Coba lagi.".into());
        };
        if entries.next().is_some() {
            return Err("Respons ketersediaan mesin basis data tidak valid. Coba lagi.".into());
        }
        if status.is_installed && status.is_running {
            available.push(AvailableDatabaseEngine {
                engine: engine.as_str().into(),
                label: engine.display_name().into(),
                port: status.port,
            });
        }
    }
    Ok(available)
}

async fn check_database_creation_availability(
    state: &AppState,
    repository: &SqliteDatabaseRepository,
    engine: DatabaseEngine,
    name: &str,
) -> Result<(), (StatusCode, String)> {
    if let Ok(Some(_)) = repository.find_by_name(name).await {
        return Err((
            StatusCode::CONFLICT,
            format!("Basis data dengan nama '{name}' sudah ada"),
        ));
    }
    let available = available_database_engines(state)
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error))?;
    if available.iter().any(|item| item.engine == engine.as_str()) {
        Ok(())
    } else {
        Err((StatusCode::UNPROCESSABLE_ENTITY,
            "Mesin basis data yang dipilih belum tersedia. Minta administrator memasang atau menjalankan layanannya, lalu muat ulang pilihan mesin.".into()))
    }
}

pub async fn available_database_engines_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
) -> Response {
    let mut response = match available_database_engines(&state).await {
        Ok(engines) => Json(engines).into_response(),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":error})),
        )
            .into_response(),
    };
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

impl From<DatabaseServer> for DatabaseServerResponse {
    fn from(s: DatabaseServer) -> Self {
        Self {
            engine: s.engine,
            is_installed: s.is_installed,
            is_running: s.is_running,
            version: s.version,
            port: s.port,
            socket_path: s.socket_path,
            updated_at: s.updated_at,
        }
    }
}

/// Request to install a database engine on demand.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstallServerRequest {
    pub engine: String,
}

/// Request to execute an action on a database service.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActionServerRequest {
    pub engine: String,
    pub action: String, // "start", "stop", "restart", "reload"
}

/// Response item for a tenant database.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DatabaseItemResponse {
    pub id: i64,
    pub user_id: i64,
    pub site_id: Option<i64>,
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub remote_access: bool,
    pub allowed_hosts: String,
    pub collation: String,
    pub character_set: String,
    pub size_bytes: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<DatabaseModel> for DatabaseItemResponse {
    fn from(d: DatabaseModel) -> Self {
        Self {
            id: d.id,
            user_id: d.user_id,
            site_id: d.site_id,
            engine: d.engine,
            name: d.name,
            db_user: d.db_user,
            remote_access: d.remote_access,
            allowed_hosts: d.allowed_hosts,
            collation: d.collation,
            character_set: d.character_set,
            size_bytes: d.size_bytes,
            created_at: d.created_at,
            updated_at: d.updated_at,
        }
    }
}

/// Response payload for listing databases.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListDatabasesResponse {
    pub databases: Vec<DatabaseItemResponse>,
    pub total: usize,
    pub plan_max_databases: i64,
    pub used_databases: i64,
}

/// Request payload to create a new database.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateDatabaseRequest {
    pub site_id: Option<i64>,
    pub engine: String,
    pub name: String,
    pub db_user: Option<String>,
    pub password: Option<String>,
    pub remote_access: Option<bool>,
    pub allowed_hosts: Option<String>,
    pub collation: Option<String>,
    pub character_set: Option<String>,
}

/// Response payload when database is created.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateDatabaseResponse {
    pub database: DatabaseItemResponse,
    pub generated_password: Option<String>,
}

/// Request payload to rotate database password.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RotatePasswordRequest {
    pub new_password: Option<String>,
}

/// Response payload after rotating password.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RotatePasswordResponse {
    pub success: bool,
    pub new_password: String,
    pub message: String,
}

/// Request payload to update remote access settings.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRemoteAccessRequest {
    pub remote_access: bool,
    pub allowed_hosts: Option<String>,
}

/// Request payload to export database dump.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExportDatabaseRequest {
    pub export_file_path: Option<String>,
}

/// Request payload to import database dump.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportDatabaseRequest {
    pub import_file_path: String,
}

/// Generic message response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DatabaseActionResponse {
    pub success: bool,
    pub message: String,
    pub job_id: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers", get(list_database_servers_handler))
        .route("/engines", get(available_database_engines_handler))
        .route("/servers/install", post(install_database_server_handler))
        .route("/servers/action", post(action_database_server_handler))
        .route(
            "/",
            get(list_databases_handler).post(create_database_handler),
        )
        .route(
            "/{id}",
            get(get_database_handler).delete(delete_database_handler),
        )
        .route("/{id}/rotate-password", post(rotate_password_handler))
        .route("/{id}/remote-access", put(update_remote_access_handler))
        .route("/{id}/export", post(export_database_handler))
        .route("/{id}/import", post(import_database_handler))
}

// =========================================================================
// Server Engine Endpoints
// =========================================================================

/// Lists database servers status, syncing with live agent state.
pub async fn list_database_servers_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());

    // Sync with agent
    let engines = DatabaseEngine::ALL
        .iter()
        .map(|engine| engine.as_str().to_string())
        .collect();
    match state
        .agent_client
        .database_server_status(DatabaseServerStatusRequest { engines })
        .await
    {
        Ok(res) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            for item in res.servers {
                let s = DatabaseServer {
                    engine: item.engine,
                    is_installed: item.is_installed,
                    is_running: item.is_running,
                    version: item.version,
                    port: item.port as i64,
                    socket_path: item.socket_path,
                    updated_at: now,
                };
                let _ = db_repo.upsert_server(&s).await;
            }
        }
        Err(e) => {
            warn!("Failed to query agent for database server status: {e}");
        }
    }

    match db_repo.list_servers().await {
        Ok(servers) => {
            let items: Vec<DatabaseServerResponse> = servers
                .into_iter()
                .map(DatabaseServerResponse::from)
                .collect();
            (StatusCode::OK, Json(items)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal memuat status server: {e}") })),
        )
            .into_response(),
    }
}

/// Installs a database engine server on demand (admin only, background job).
pub async fn install_database_server_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(payload): Json<InstallServerRequest>,
) -> Response {
    if user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Hanya administrator yang dapat memasang engine database" })),
        )
            .into_response();
    }

    let engine = match DatabaseEngine::parse(&payload.engine) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let (job, _) = match state
        .job_runner
        .enqueue_database_install(user.id, engine.as_str())
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal menjadwalkan instalasi: {e}") })),
            )
                .into_response();
        }
    };

    let _ = SqliteAuditLogRepository::new(state.operations_pool.clone())
        .create(&CreateAuditLog {
            user_id: Some(user.id),
            actor_username: user.username.clone(),
            action: "database.server_install".to_string(),
            target_resource: format!("database_server:{}", engine.as_str()),
            details_json: Some(serde_json::json!({ "engine": engine.as_str() }).to_string()),
            ip_address: None,
            status: "success".to_string(),
        })
        .await;

    state.job_runner.notify.notify_one();

    (
        StatusCode::ACCEPTED,
        Json(DatabaseActionResponse {
            success: true,
            message: format!(
                "Instalasi {} telah dijadwalkan di latar belakang",
                engine.as_str()
            ),
            job_id: Some(job.id),
        }),
    )
        .into_response()
}

/// Starts, stops, or restarts a database service (admin only).
pub async fn action_database_server_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(payload): Json<ActionServerRequest>,
) -> Response {
    if user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Hanya administrator yang dapat mengelola service database" })),
        )
            .into_response();
    }

    let engine = match DatabaseEngine::parse(&payload.engine) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    match state
        .agent_client
        .database_server_action(DatabaseServerActionRequest {
            engine: engine.as_str().to_string(),
            action: payload.action.clone(),
        })
        .await
    {
        Ok(res) => {
            let _ = SqliteAuditLogRepository::new(state.operations_pool.clone())
                .create(&CreateAuditLog {
                    user_id: Some(user.id),
                    actor_username: user.username.clone(),
                    action: format!("database.server_{}", payload.action),
                    target_resource: format!("database_server:{}", engine.as_str()),
                    details_json: Some(
                        serde_json::json!({
                            "action": payload.action,
                            "engine": engine.as_str(),
                        })
                        .to_string(),
                    ),
                    ip_address: None,
                    status: "success".to_string(),
                })
                .await;

            (
                StatusCode::OK,
                Json(DatabaseActionResponse {
                    success: res.success,
                    message: res.message,
                    job_id: None,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Aksi gagal: {e}") })),
        )
            .into_response(),
    }
}

// =========================================================================
// Tenant Databases Endpoints
// =========================================================================

/// Lists databases with role isolation and plan limits info.
pub async fn list_databases_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(query): Query<ListDatabasesQuery>,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());

    let target_user_id = if let Some(site_id) = query.site_id {
        let site_repo = SqliteSiteRepository::new(state.pool.clone());
        match crate::sites::fetch_and_authorize_site(&site_repo, &user, site_id).await {
            Ok(site) => site.user_id,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({ "error": message }))).into_response();
            }
        }
    } else if user.role == "admin" {
        query.user_id.unwrap_or(user.id)
    } else {
        user.id
    };

    // Determine plan limits
    let mut plan_max_dbs = 5;
    let target_plan_id = match user_repo.find_by_id(target_user_id).await {
        Ok(Some(u)) => u.plan_id,
        _ => None,
    };
    if let Some(plan_id) = target_plan_id
        && let Ok(Some(plan)) = plan_repo.find_by_id(plan_id).await
    {
        plan_max_dbs = plan.max_databases;
    }

    let dbs_res = if let Some(site_id) = query.site_id {
        db_repo.list_by_site(site_id).await
    } else if user.role == "admin" && query.user_id.is_none() {
        db_repo.list_all().await
    } else {
        db_repo.list_by_user(target_user_id).await
    };

    match dbs_res {
        Ok(dbs) => {
            let used_count = match db_repo.count_by_user(target_user_id).await {
                Ok(c) => c,
                Err(_) => dbs.len() as i64,
            };

            let items: Vec<DatabaseItemResponse> =
                dbs.into_iter().map(DatabaseItemResponse::from).collect();

            let total = items.len();
            (
                StatusCode::OK,
                Json(ListDatabasesResponse {
                    databases: items,
                    total,
                    plan_max_databases: plan_max_dbs,
                    used_databases: used_count,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Gagal mengambil daftar basis data: {e}") })),
        )
            .into_response(),
    }
}

/// Creates a new tenant database, enforcing plan limits, prefix, and server availability.
pub async fn create_database_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(payload): Json<CreateDatabaseRequest>,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());
    let site_repo = SqliteSiteRepository::new(state.pool.clone());

    // 1. Resolve tenant identity & plan
    let tenant_user = match user_repo.find_by_id(user.id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Pengguna tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let (resolved_linux_name, _resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&tenant_user) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let tenant_username = resolved_linux_name;

    // 2. Check plan database limit
    let mut plan_max_dbs = 5;
    if let Some(plan_id) = tenant_user.plan_id
        && let Ok(Some(plan)) = plan_repo.find_by_id(plan_id).await
    {
        plan_max_dbs = plan.max_databases;
    }

    let current_dbs = match db_repo.count_by_user(user.id).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({ "error": format!("Gagal memeriksa kuota database: {e}") }),
                ),
            )
                .into_response();
        }
    };

    if plan_max_dbs > 0 && current_dbs >= plan_max_dbs {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": format!("Batas jumlah database pada paket Anda telah tercapai (maksimum {plan_max_dbs} database). Tingkatkan paket Anda untuk menambah database.")
            })),
        )
            .into_response();
    }

    // 3. Engine validation
    let engine = match DatabaseEngine::parse(&payload.engine) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // 4. Verify site ownership if site_id provided
    if let Some(sid) = payload.site_id {
        match site_repo.find_by_id(sid).await {
            Ok(Some(site)) => {
                if site.user_id != user.id && user.role != "admin" {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({ "error": "Anda tidak memiliki akses ke situs yang dipilih" })),
                    )
                        .into_response();
                }
            }
            _ => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "Situs yang dipilih tidak ditemukan" })),
                )
                    .into_response();
            }
        }
    }

    // 5. Prefix calculation and validation
    let prefix = tenant_db_prefix(&tenant_username);
    let raw_name = payload.name.trim();

    // Auto-prepend prefix if tenant omitted it
    let full_db_name = if raw_name.to_ascii_lowercase().starts_with(&prefix) {
        raw_name.to_string()
    } else {
        format!("{prefix}{raw_name}")
    };

    if let Err(e) = validate_database_name(&full_db_name, &prefix, engine) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    // User name
    let raw_user = payload.db_user.as_deref().unwrap_or(raw_name).trim();
    let full_db_user = if raw_user.to_ascii_lowercase().starts_with(&prefix) {
        raw_user.to_string()
    } else {
        format!("{prefix}{raw_user}")
    };

    if let Err(e) = validate_database_user(&full_db_user, &prefix, engine) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    // Password handling
    let (password, was_generated) = match payload.password {
        Some(p) if !p.trim().is_empty() => {
            if let Err(e) = validate_database_password(&p) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            (p, false)
        }
        _ => (generate_secure_db_password(), true),
    };

    let remote_access = payload.remote_access.unwrap_or(false);
    let allowed_hosts = payload
        .allowed_hosts
        .as_deref()
        .unwrap_or("localhost")
        .trim();

    if let Err(e) = validate_allowed_hosts(allowed_hosts, remote_access) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let collation = payload
        .collation
        .unwrap_or_else(|| "utf8mb4_unicode_ci".to_string());
    let character_set = payload
        .character_set
        .unwrap_or_else(|| "utf8mb4".to_string());

    if let Err(error) = panel_core::database::validate_database_encoding(&collation)
        .and_then(|()| panel_core::database::validate_database_encoding(&character_set))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":error.to_string()})),
        )
            .into_response();
    }

    // 6. Check name uniqueness and live engine availability before any mutation.
    if let Err((status, error)) =
        check_database_creation_availability(&state, &db_repo, engine, &full_db_name).await
    {
        return (status, Json(serde_json::json!({"error":error}))).into_response();
    }

    // 7. Invoke agent to create database & user
    match state
        .agent_client
        .database_create(DatabaseCreateRequest {
            engine: engine.as_str().to_string(),
            name: full_db_name.clone(),
            db_user: full_db_user.clone(),
            password: password.clone(),
            remote_access,
            allowed_hosts: allowed_hosts.to_string(),
            collation: collation.clone(),
            character_set: character_set.clone(),
        })
        .await
    {
        Ok(_) => {
            // 8. Insert record in repository
            let input = CreateDatabaseInput {
                user_id: user.id,
                site_id: payload.site_id,
                engine: engine.as_str().to_string(),
                name: full_db_name.clone(),
                db_user: full_db_user.clone(),
                remote_access,
                allowed_hosts: allowed_hosts.to_string(),
                collation,
                character_set,
            };

            match db_repo.create(&input).await {
                Ok(created) => {
                    let _ = SqliteAuditLogRepository::new(state.operations_pool.clone())
                        .create(&CreateAuditLog {
                            user_id: Some(user.id),
                            actor_username: user.username.clone(),
                            action: "database.create".to_string(),
                            target_resource: format!("database:{}", created.name),
                            details_json: Some(
                                serde_json::json!({
                                    "engine": created.engine,
                                    "name": created.name,
                                    "db_user": created.db_user,
                                    "remote_access": created.remote_access,
                                })
                                .to_string(),
                            ),
                            ip_address: None,
                            status: "success".to_string(),
                        })
                        .await;

                    (
                        StatusCode::CREATED,
                        Json(CreateDatabaseResponse {
                            database: DatabaseItemResponse::from(created),
                            generated_password: if was_generated {
                                Some(password)
                            } else {
                                None
                            },
                        }),
                    )
                        .into_response()
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Gagal menyimpan data basis data: {e}") })),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({ "error": format!("Pembuatan basis data di agent gagal: {e}") }),
            ),
        )
            .into_response(),
    }
}

/// Retrieves details of a specific database.
pub async fn get_database_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<i64>,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    match db_repo.find_by_id(id).await {
        Ok(Some(db)) => {
            if db.user_id != user.id && user.role != "admin" {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({ "error": "Akses ditolak" })),
                )
                    .into_response();
            }
            (StatusCode::OK, Json(DatabaseItemResponse::from(db))).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Basis data tidak ditemukan" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Kesalahan database: {e}") })),
        )
            .into_response(),
    }
}

/// Deletes a database and removes it from the server engine.
pub async fn delete_database_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<i64>,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let db = match db_repo.find_by_id(id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Basis data tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Kesalahan database: {e}") })),
            )
                .into_response();
        }
    };

    if db.user_id != user.id && user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    // Call agent to drop DB and user
    let deletion = state
        .agent_client
        .database_delete(panel_proto::DatabaseDeleteRequest {
            engine: db.engine.clone(),
            name: db.name.clone(),
            db_user: db.db_user.clone(),
        })
        .await;

    if !deletion.is_ok_and(|result| result.success) {
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": "Gagal menghapus database atau akun pengelola internal. Periksa layanan database lalu coba lagi."}))).into_response();
    }

    // Delete record from repo
    match db_repo.delete(id).await {
        Ok(_) => {
            let _ = SqliteAuditLogRepository::new(state.operations_pool.clone())
                .create(&CreateAuditLog {
                    user_id: Some(user.id),
                    actor_username: user.username.clone(),
                    action: "database.delete".to_string(),
                    target_resource: format!("database:{}", db.name),
                    details_json: Some(
                        serde_json::json!({
                            "engine": db.engine,
                            "name": db.name,
                        })
                        .to_string(),
                    ),
                    ip_address: None,
                    status: "success".to_string(),
                })
                .await;

            (
                StatusCode::OK,
                Json(DatabaseActionResponse {
                    success: true,
                    message: format!("Basis data '{}' berhasil dihapus", db.name),
                    job_id: None,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({ "error": format!("Gagal menghapus rekaman basis data: {e}") }),
            ),
        )
            .into_response(),
    }
}

/// Rotates database password.
pub async fn rotate_password_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<i64>,
    Json(payload): Json<RotatePasswordRequest>,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let db = match db_repo.find_by_id(id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Basis data tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Kesalahan database: {e}") })),
            )
                .into_response();
        }
    };

    if db.user_id != user.id && user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let new_password = match payload.new_password {
        Some(p) if !p.trim().is_empty() => {
            if let Err(e) = validate_database_password(&p) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            p
        }
        _ => generate_secure_db_password(),
    };

    match state
        .agent_client
        .database_rotate_password(DatabaseRotatePasswordRequest {
            engine: db.engine.clone(),
            name: db.name.clone(),
            db_user: db.db_user.clone(),
            new_password: new_password.clone(),
            allowed_hosts: db.allowed_hosts.clone(),
        })
        .await
    {
        Ok(_) => {
            let _ = SqliteAuditLogRepository::new(state.operations_pool.clone())
                .create(&CreateAuditLog {
                    user_id: Some(user.id),
                    actor_username: user.username.clone(),
                    action: "database.rotate_password".to_string(),
                    target_resource: format!("database:{}", db.name),
                    details_json: Some(
                        serde_json::json!({
                            "db_user": db.db_user,
                        })
                        .to_string(),
                    ),
                    ip_address: None,
                    status: "success".to_string(),
                })
                .await;

            (
                StatusCode::OK,
                Json(RotatePasswordResponse {
                    success: true,
                    new_password,
                    message: "Kata sandi basis data berhasil diperbarui".to_string(),
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Rotasi password gagal: {e}") })),
        )
            .into_response(),
    }
}

/// Updates remote access configuration.
pub async fn update_remote_access_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateRemoteAccessRequest>,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let db = match db_repo.find_by_id(id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Basis data tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Kesalahan database: {e}") })),
            )
                .into_response();
        }
    };

    if db.user_id != user.id && user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let allowed_hosts = payload
        .allowed_hosts
        .as_deref()
        .unwrap_or(if payload.remote_access {
            "%"
        } else {
            "localhost"
        })
        .trim();

    if let Err(e) = validate_allowed_hosts(allowed_hosts, payload.remote_access) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    match state
        .agent_client
        .database_update_remote_access(DatabaseUpdateRemoteAccessRequest {
            engine: db.engine.clone(),
            name: db.name.clone(),
            db_user: db.db_user.clone(),
            remote_access: payload.remote_access,
            allowed_hosts: allowed_hosts.to_string(),
        })
        .await
    {
        Ok(_) => {
            let update_input = UpdateDatabaseInput {
                remote_access: Some(payload.remote_access),
                allowed_hosts: Some(allowed_hosts.to_string()),
                ..Default::default()
            };

            match db_repo.update(id, &update_input).await {
                Ok(updated) => {
                    let _ = SqliteAuditLogRepository::new(state.operations_pool.clone())
                        .create(&CreateAuditLog {
                            user_id: Some(user.id),
                            actor_username: user.username.clone(),
                            action: "database.update_remote_access".to_string(),
                            target_resource: format!("database:{}", updated.name),
                            details_json: Some(
                                serde_json::json!({
                                    "remote_access": updated.remote_access,
                                    "allowed_hosts": updated.allowed_hosts,
                                })
                                .to_string(),
                            ),
                            ip_address: None,
                            status: "success".to_string(),
                        })
                        .await;

                    (StatusCode::OK, Json(DatabaseItemResponse::from(updated))).into_response()
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Gagal memperbarui rekaman database: {e}") })),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Pengubahan akses remote di agent gagal: {e}") })),
        )
            .into_response(),
    }
}

/// Enqueues an asynchronous database export job.
pub async fn export_database_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<i64>,
    Json(payload): Json<ExportDatabaseRequest>,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());

    let db = match db_repo.find_by_id(id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Basis data tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Kesalahan database: {e}") })),
            )
                .into_response();
        }
    };

    if db.user_id != user.id && user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let tenant_user = match user_repo.find_by_id(db.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Pemilik database tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let (resolved_linux_name, resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&tenant_user) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let tenant_username = resolved_linux_name;
    let tenant_uid = resolved_linux_uid;

    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let export_path = payload
        .export_file_path
        .unwrap_or_else(|| format!("/home/{tenant_username}/dumps/{}_{now_ts}.sql", db.name));

    if let Err(e) = validate_dump_path(&export_path, &tenant_username) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let job_payload = serde_json::json!({
        "database_id": db.id,
        "engine": db.engine,
        "name": db.name,
        "db_user": db.db_user,
        "export_file_path": export_path,
        "tenant_username": tenant_username,
        "tenant_uid": tenant_uid,
    });

    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(user.id),
            None,
            "database.export".to_string(),
            job_payload.to_string(),
            Some(1),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal menjadwalkan ekspor: {e}") })),
            )
                .into_response();
        }
    };

    state.job_runner.notify.notify_one();
    (
        StatusCode::ACCEPTED,
        Json(DatabaseActionResponse {
            success: true,
            message: format!("Ekspor basis data '{}' sedang diproses", db.name),
            job_id: Some(job.id),
        }),
    )
        .into_response()
}

/// Enqueues an asynchronous database import job.
pub async fn import_database_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<i64>,
    Json(payload): Json<ImportDatabaseRequest>,
) -> Response {
    let db_repo = SqliteDatabaseRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());

    let db = match db_repo.find_by_id(id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Basis data tidak ditemukan" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Kesalahan database: {e}") })),
            )
                .into_response();
        }
    };

    if db.user_id != user.id && user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Akses ditolak" })),
        )
            .into_response();
    }

    let tenant_user = match user_repo.find_by_id(db.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Pemilik database tidak ditemukan" })),
            )
                .into_response();
        }
    };

    let (resolved_linux_name, resolved_linux_uid) =
        match crate::tenant_context::execution_identity(&tenant_user) {
            Ok(identity) => identity,
            Err((status, message)) => {
                return (status, Json(serde_json::json!({"error":message}))).into_response();
            }
        };
    let tenant_username = resolved_linux_name;
    let tenant_uid = resolved_linux_uid;

    if let Err(e) = validate_dump_path(&payload.import_file_path, &tenant_username) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let job_payload = serde_json::json!({
        "database_id": db.id,
        "engine": db.engine,
        "name": db.name,
        "db_user": db.db_user,
        "import_file_path": payload.import_file_path,
        "tenant_username": tenant_username,
        "tenant_uid": tenant_uid,
    });

    let (job, _) = match state
        .job_runner
        .enqueue(
            Some(user.id),
            None,
            "database.import".to_string(),
            job_payload.to_string(),
            Some(1),
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Gagal menjadwalkan impor: {e}") })),
            )
                .into_response();
        }
    };

    state.job_runner.notify.notify_one();
    (
        StatusCode::ACCEPTED,
        Json(DatabaseActionResponse {
            success: true,
            message: format!("Impor ke basis data '{}' sedang diproses", db.name),
            job_id: Some(job.id),
        }),
    )
        .into_response()
}
