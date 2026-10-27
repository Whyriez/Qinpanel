//! OWASP ASVS L2 & nftables loopback security hardening handlers and DTO schemas.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use panel_core::hardening::{
    OwaspAsvsAuditReport, SystemdServiceSecurityScore, generate_asvs_l2_audit,
    get_default_service_security_evaluations,
};
use panel_proto::{
    NftablesStatusRequest, NftablesSyncRequest, TenantPortBindingDto as ProtoTenantPortBindingDto,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{AuthenticatedUser, RequireAdminCsrf};
use crate::routes::AppState;

/// Loopback port binding for tenant application isolation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantPortBindingDto {
    pub site_id: u64,
    pub tenant_uid: u32,
    pub port: u16,
}

impl From<TenantPortBindingDto> for ProtoTenantPortBindingDto {
    fn from(b: TenantPortBindingDto) -> Self {
        Self {
            site_id: b.site_id,
            tenant_uid: b.tenant_uid,
            port: b.port,
        }
    }
}

/// Nftables firewall isolation status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct NftablesStatusResponseDto {
    pub enabled: bool,
    pub table_name: String,
    pub active_rules_count: usize,
    pub protected_ports_count: usize,
    pub last_synced_at: u64,
}

impl From<panel_proto::NftablesStatusDto> for NftablesStatusResponseDto {
    fn from(s: panel_proto::NftablesStatusDto) -> Self {
        Self {
            enabled: s.enabled,
            table_name: s.table_name,
            active_rules_count: s.active_rules_count,
            protected_ports_count: s.protected_ports_count,
            last_synced_at: s.last_synced_at,
        }
    }
}

/// Consolidated overview of system security hardening, OWASP ASVS audit, and firewall isolation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SecurityHardeningOverviewResponse {
    pub asvs_audit: OwaspAsvsAuditReport,
    pub nftables_status: NftablesStatusResponseDto,
    pub systemd_security: Vec<SystemdServiceSecurityScore>,
    pub secret_vault_active: bool,
    pub secret_key_version: u32,
}

/// Request payload to trigger synchronization of nftables loopback guard rules.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct NftablesSyncPayload {
    pub custom_bindings: Option<Vec<TenantPortBindingDto>>,
}

/// Response returned when nftables rules synchronization completes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NftablesSyncResponseDto {
    pub success: bool,
    pub rules_count: usize,
    pub message: String,
}

/// Retrieve comprehensive OWASP ASVS audit status, systemd security exposure, and nftables firewall state.
#[utoipa::path(
    get,
    path = "/api/v1/system/security-hardening",
    responses(
        (status = 200, description = "Security hardening overview and OWASP ASVS audit", body = SecurityHardeningOverviewResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Admin access required"),
        (status = 502, description = "Bad Gateway: Agent communication failure")
    ),
    tag = "system"
)]
pub async fn get_security_hardening_overview_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Response {
    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Forbidden",
                "status": 403,
                "detail": "Hanya administrator yang berhak mengakses status security hardening sistem."
            })),
        )
            .into_response();
    }

    let asvs_audit = generate_asvs_l2_audit();
    let systemd_security = get_default_service_security_evaluations();

    // Query nftables loopback guard status from panel-agent
    let nftables_status = match state
        .agent_client
        .nftables_status(NftablesStatusRequest::default())
        .await
    {
        Ok(res) => NftablesStatusResponseDto::from(res.status),
        Err(e) => {
            tracing::warn!(
                target: "panel_api::security_hardening",
                "Could not fetch nftables status from agent ({}), using offline default",
                e
            );
            NftablesStatusResponseDto {
                enabled: false,
                table_name: "panel_loopback_guard".to_string(),
                active_rules_count: 0,
                protected_ports_count: 0,
                last_synced_at: 0,
            }
        }
    };

    let overview = SecurityHardeningOverviewResponse {
        asvs_audit,
        nftables_status,
        systemd_security,
        secret_vault_active: true,
        secret_key_version: 1,
    };

    (StatusCode::OK, Json(overview)).into_response()
}

/// Synchronize nftables loopback isolation firewall rules.
#[utoipa::path(
    post,
    path = "/api/v1/system/security-hardening/nftables/sync",
    request_body = NftablesSyncPayload,
    responses(
        (status = 200, description = "Nftables loopback isolation firewall synchronized", body = NftablesSyncResponseDto),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Admin access required"),
        (status = 409, description = "Conflicting tenant port mapping"),
        (status = 422, description = "Agent rejected nftables configuration or prerequisites"),
        (status = 503, description = "Agent connection unavailable; refresh status before retrying")
    ),
    tag = "system"
)]
pub async fn sync_nftables_loopback_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(auth_user): RequireAdminCsrf,
    Json(payload): Json<NftablesSyncPayload>,
) -> Response {
    if !auth_user.is_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://tools.ietf.org/html/rfc9457",
                "title": "Forbidden",
                "status": 403,
                "detail": "Hanya administrator yang berhak mengelola firewall nftables."
            })),
        )
            .into_response();
    }

    let input_bindings = match payload.custom_bindings {
        Some(bindings) => bindings,
        None => match automatic_bindings(&state.pool).await {
            Ok(bindings) => bindings,
            Err(message) => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"detail":message})),
                )
                    .into_response();
            }
        },
    };

    // Validate loopback ports
    for b in &input_bindings {
        if let Err(err) = panel_core::hardening::validate_loopback_isolation_binding(
            b.site_id,
            b.tenant_uid,
            b.port,
        ) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "type": "https://tools.ietf.org/html/rfc9457",
                    "title": "Invalid Loopback Port Binding",
                    "status": 400,
                    "detail": err
                })),
            )
                .into_response();
        }
    }

    let bindings = input_bindings
        .into_iter()
        .map(ProtoTenantPortBindingDto::from)
        .collect();

    let req = NftablesSyncRequest { bindings };
    match state.agent_client.nftables_sync(req).await {
        Ok(res) => (
            StatusCode::OK,
            Json(NftablesSyncResponseDto {
                success: res.success,
                rules_count: res.rules_count,
                message: res.message,
            }),
        )
            .into_response(),
        Err(error) => nftables_failure(error),
    }
}

fn nftables_failure(error: crate::agent_client::AgentClientError) -> Response {
    use crate::agent_client::AgentClientError;
    let (status, code, detail) = match error {
        AgentClientError::AgentError(message)
            if message.contains("Netlink") || message.contains("netlink") =>
        {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "nftables_netlink_unavailable",
                "Agent tidak dapat membuka socket Netlink. Unit panel-agent memerlukan RestrictAddressFamilies yang mencakup AF_NETLINK dan capability CAP_NET_ADMIN. Perbarui unit, jalankan daemon-reload, lalu restart panel-agent sebelum mencoba lagi.",
            )
        }
        AgentClientError::AgentError(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "nftables_rejected",
            "Agent menolak penerapan nftables. Periksa log panel-agent dan prasyarat nft; jangan menghapus firewall atau mengirim ulang sebelum penyebabnya jelas.",
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "nftables_agent_unavailable",
            "Status penerapan nftables belum dapat dipastikan karena koneksi agent terputus. Muat ulang status sebelum mencoba lagi.",
        ),
    };
    let title = if status == StatusCode::SERVICE_UNAVAILABLE {
        "Status sinkronisasi nftables belum terkonfirmasi"
    } else {
        "Aturan nftables belum diterapkan"
    };
    (status,Json(serde_json::json!({"type":"about:blank","title":title,"status":status.as_u16(),"code":code,"detail":detail}))).into_response()
}

#[derive(sqlx::FromRow)]
struct OwnedBinding {
    site_id: i64,
    settings_json: String,
    #[sqlx(flatten)]
    owner: panel_db::models::User,
}
async fn automatic_bindings(pool: &panel_db::DbPool) -> Result<Vec<TenantPortBindingDto>, String> {
    let rows=sqlx::query_as::<_,OwnedBinding>("SELECT s.id AS site_id,s.settings_json,u.* FROM sites s JOIN users u ON u.id=s.user_id WHERE s.status='active' AND u.status='active' AND u.role='user' AND (s.kind IN ('nodejs','python') OR s.runtime IN ('nodejs','python'))")
        .fetch_all(pool).await.map_err(|_|"Pemetaan port tenant gagal dimuat; aturan tidak diubah.".to_owned())?;
    let mut ports = std::collections::BTreeMap::<u16, TenantPortBindingDto>::new();
    for row in rows {
        let settings: serde_json::Value = serde_json::from_str(&row.settings_json)
            .map_err(|_| "Konfigurasi situs tidak valid; aturan tidak diubah.".to_owned())?;
        let Some(port) = settings["port"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .filter(|port| (20000..=29999).contains(port))
        else {
            continue;
        };
        let (_, tenant_uid) = crate::tenant_context::execution_identity(&row.owner)
            .map_err(|(_, message)| message)?;
        if ports
            .get(&port)
            .is_some_and(|old| old.tenant_uid != tenant_uid)
        {
            return Err("Port aplikasi tercatat untuk dua tenant berbeda; perbaiki pemetaan sebelum sinkronisasi.".into());
        }
        let site_id =
            u64::try_from(row.site_id).map_err(|_| "Identitas situs tidak valid.".to_owned())?;
        ports.entry(port).or_insert(TenantPortBindingDto {
            site_id,
            tenant_uid,
            port,
        });
    }
    Ok(ports.into_values().collect())
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use http_body_util::BodyExt;
    #[tokio::test]
    async fn netlink_rejection_is_actionable_json_and_never_a_gateway_error() {
        let response = nftables_failure(crate::agent_client::AgentClientError::AgentError(
            "mnl.c: Unable to initialize Netlink socket: Address family not supported by protocol"
                .into(),
        ));
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body["code"], "nftables_netlink_unavailable");
        assert!(
            body["detail"]
                .as_str()
                .expect("detail")
                .contains("AF_NETLINK")
        );
    }

    #[tokio::test]
    async fn automatic_bindings_use_owned_active_runtime_ports_and_reject_uid_collisions() {
        let pool = panel_db::pool::init_pool("sqlite::memory:")
            .await
            .expect("pool");
        panel_db::pool::run_migrations(&pool)
            .await
            .expect("migrations");
        let users = panel_db::repositories::SqliteUserRepository::new(pool.clone());
        let mut owners = Vec::new();
        for (username, uid, role) in [
            ("tenantone", 10001, "user"),
            ("tenanttwo", 10002, "user"),
            ("administrator", 0, "admin"),
        ] {
            owners.push(
                users
                    .create(&panel_db::models::CreateUser {
                        username: username.into(),
                        password_hash: "fixture".into(),
                        role: role.into(),
                        email: None,
                        linux_username: Some(username.into()),
                        linux_uid: (role == "user").then_some(uid),
                        plan_id: None,
                    })
                    .await
                    .expect("user"),
            );
        }
        for (owner, kind, status, port) in [
            (0, "nodejs", "active", 20050),
            (0, "python", "active", 20060),
            (1, "nodejs", "disabled", 20070),
            (2, "nodejs", "active", 20080),
            (0, "proxy", "active", 20090),
        ] {
            sqlx::query("INSERT INTO sites(user_id,kind,docroot,status,settings_json,created_at,updated_at) VALUES(?,?,'/fixture',?,?,1,1)")
                .bind(owners[owner].id).bind(kind).bind(status).bind(serde_json::json!({"port":port}).to_string()).execute(&pool).await.expect("site");
        }
        let bindings = automatic_bindings(&pool).await.expect("bindings");
        assert_eq!(
            bindings
                .iter()
                .map(|row| (row.tenant_uid, row.port))
                .collect::<Vec<_>>(),
            vec![(10001, 20050), (10001, 20060)]
        );
        sqlx::query("UPDATE sites SET status='active',settings_json=? WHERE user_id=?")
            .bind(r#"{"port":20050}"#)
            .bind(owners[1].id)
            .execute(&pool)
            .await
            .expect("collision");
        assert!(automatic_bindings(&pool).await.is_err());
        sqlx::query("UPDATE sites SET status='disabled' WHERE user_id=?")
            .bind(owners[1].id)
            .execute(&pool)
            .await
            .expect("disable");
        sqlx::query("UPDATE users SET linux_uid=0 WHERE id=?")
            .bind(owners[0].id)
            .execute(&pool)
            .await
            .expect("invalid identity");
        assert!(automatic_bindings(&pool).await.is_err());
    }
}
