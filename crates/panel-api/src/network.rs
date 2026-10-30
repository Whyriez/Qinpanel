//! Administrator DNS guidance and typed Cloudflare management.
use crate::{
    auth::{RequireAdmin, RequireAdminCsrf},
    routes::AppState,
};
use axum::{Json, extract::State, http::StatusCode};
use panel_db::{pool::DbPool, repositories::SqliteSettingRepository};
use panel_proto::CloudflareRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use utoipa::ToSchema;

type Error = (StatusCode, Json<Value>);
fn fail(status: StatusCode, message: impl Into<String>) -> Error {
    (status, Json(json!({"error":message.into()})))
}
fn db_error(_: impl std::fmt::Display) -> Error {
    fail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Konfigurasi jaringan gagal disimpan atau dimuat.",
    )
}

#[derive(Default, Deserialize, Serialize, ToSchema)]
pub struct PublicAddresses {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}
#[derive(Deserialize, ToSchema)]
pub struct DnsCheck {
    pub domain_id: i64,
}

fn normalize(address: Option<String>, ipv6: bool) -> Result<Option<String>, Error> {
    let Some(value) = address.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let parsed: std::net::IpAddr = value
        .trim()
        .parse()
        .map_err(|_| fail(StatusCode::BAD_REQUEST, "Alamat IP tidak valid."))?;
    let public = match parsed {
        std::net::IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_multicast()
                && a != 0
                && a < 240
                && !(a == 100 && (64..=127).contains(&b))
        }
        std::net::IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && ip.to_ipv4_mapped().is_none()
        }
    };
    if !public || parsed.is_ipv6() != ipv6 {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "Gunakan IP publik dengan keluarga alamat yang sesuai.",
        ));
    }
    Ok(Some(parsed.to_string()))
}

#[utoipa::path(get, path="/api/v1/network", responses((status=200,description="Public addresses and project DNS guidance")), tag="network")]
pub async fn overview(
    State(state): State<AppState>,
    RequireAdmin(_user): RequireAdmin,
) -> Result<Json<Value>, Error> {
    let repo = SqliteSettingRepository::new(state.pool.clone());
    let ipv4 = repo.get("network.public_ipv4").await.map_err(db_error)?;
    let ipv6 = repo.get("network.public_ipv6").await.map_err(db_error)?;
    let rows = sqlx::query("SELECT d.id,d.site_id,d.name,s.kind,s.status,s.settings_json FROM domains d JOIN sites s ON s.id=d.site_id ORDER BY d.name")
        .fetch_all(&state.pool).await.map_err(db_error)?;
    let projects = rows.iter().map(|r| {
        let settings: Value = serde_json::from_str(r.get::<String,_>("settings_json").as_str()).unwrap_or_default();
        json!({"domain_id":r.get::<i64,_>("id"),"site_id":r.get::<i64,_>("site_id"),"hostname":r.get::<String,_>("name"),"kind":r.get::<String,_>("kind"),"status":r.get::<String,_>("status"),"tls":settings["has_ssl"].as_bool().unwrap_or(false)})
    }).collect::<Vec<_>>();
    Ok(Json(json!({"ipv4":ipv4,"ipv6":ipv6,"projects":projects})))
}
#[utoipa::path(put, path="/api/v1/network", request_body=PublicAddresses, responses((status=200,description="Public addresses saved")), tag="network")]
pub async fn save_addresses(
    State(state): State<AppState>,
    RequireAdminCsrf(user): RequireAdminCsrf,
    Json(payload): Json<PublicAddresses>,
) -> Result<Json<Value>, Error> {
    let ipv4 = normalize(payload.ipv4, false)?;
    let ipv6 = normalize(payload.ipv6, true)?;
    let mut tx = state.pool.begin().await.map_err(db_error)?;
    for (key, value) in [
        ("network.public_ipv4", &ipv4),
        ("network.public_ipv6", &ipv6),
    ] {
        sqlx::query("DELETE FROM settings WHERE key=?")
            .bind(key)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        if let Some(value) = value {
            sqlx::query("INSERT INTO settings(key,value,updated_at) VALUES (?,?,?)")
                .bind(key)
                .bind(value)
                .bind(chrono::Utc::now().timestamp())
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
        }
    }
    tx.commit().await.map_err(db_error)?;
    state.audit_writer.log_async(
        Some(user.id),
        &user.username,
        "network.addresses",
        "network",
        "success",
        None,
        None,
    );
    Ok(Json(json!({"ipv4":ipv4,"ipv6":ipv6})))
}
#[utoipa::path(post, path="/api/v1/network/dns-check", request_body=DnsCheck, responses((status=200,description="DNS lookup compared with configured public addresses")), tag="network")]
pub async fn dns_check(
    State(state): State<AppState>,
    RequireAdminCsrf(_user): RequireAdminCsrf,
    Json(payload): Json<DnsCheck>,
) -> Result<Json<Value>, Error> {
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM domains WHERE id=?")
        .bind(payload.domain_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db_error)?;
    let name =
        name.ok_or_else(|| fail(StatusCode::NOT_FOUND, "Domain project tidak ditemukan."))?;
    let repo = SqliteSettingRepository::new(state.pool.clone());
    let expected = [
        repo.get("network.public_ipv4").await.map_err(db_error)?,
        repo.get("network.public_ipv6").await.map_err(db_error)?,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let addresses = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::lookup_host((name.as_str(), 80)),
    )
    .await
    .map_err(|_| {
        fail(
            StatusCode::GATEWAY_TIMEOUT,
            "Pemeriksaan DNS melewati batas waktu.",
        )
    })?
    .map_err(|_| {
        fail(
            StatusCode::BAD_GATEWAY,
            "DNS belum dapat diresolusi oleh server. Periksa record dan coba kembali.",
        )
    })?;
    let found = addresses
        .map(|s| s.ip().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let matches = !expected.is_empty()
        && expected
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            == found;
    Ok(Json(
        json!({"hostname":name,"addresses":found,"expected":expected,"matches":matches}),
    ))
}

pub async fn validate_publication(
    pool: &DbPool,
    command: CloudflareRequest,
) -> Result<CloudflareRequest, Error> {
    if matches!(command, CloudflareRequest::PublishStorage { .. }) {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "Gunakan dialog aktivasi storage untuk menerbitkan rute storage.",
        ));
    }
    if let CloudflareRequest::Publish {
        zone_id,
        tunnel_id,
        hostname,
        ..
    } = command
    {
        let row = sqlx::query("SELECT s.status,s.settings_json FROM domains d JOIN sites s ON s.id=d.site_id WHERE d.name=?").bind(&hostname).fetch_optional(pool).await.map_err(db_error)?
            .ok_or_else(||fail(StatusCode::BAD_REQUEST,"Pilih hostname dari domain project panel."))?;
        if row.get::<String, _>("status") != "active" {
            return Err(fail(
                StatusCode::CONFLICT,
                "Aktifkan project sebelum menerbitkan tunnel.",
            ));
        }
        let settings: Value = serde_json::from_str(row.get::<String, _>("settings_json").as_str())
            .map_err(db_error)?;
        let tls = settings["has_ssl"].as_bool().unwrap_or(false);
        if settings["force_https"].as_bool() == Some(true) && !tls {
            return Err(fail(
                StatusCode::CONFLICT,
                "Project memaksa HTTPS tetapi sertifikat origin belum tersedia. Perbaiki pengaturan HTTPS dahulu.",
            ));
        }
        return Ok(CloudflareRequest::Publish {
            zone_id,
            tunnel_id,
            hostname,
            tls,
        });
    }
    Ok(command)
}
fn cloudflare_failure(error: crate::agent_client::AgentClientError) -> Error {
    use crate::agent_client::AgentClientError;
    match error {
        AgentClientError::AgentError(_) => fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Agent menolak pemeriksaan atau operasi Cloudflare. Periksa kredensial terenkripsi, inventaris tunnel dan log panel-agent. Jangan memasang ulang konektor untuk sekadar memuat status.",
        ),
        _ => fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Koneksi panel-api ke panel-agent belum tersedia atau terputus. Status Cloudflare belum dapat dipastikan; muat ulang status setelah layanan panel kembali tersedia.",
        ),
    }
}

#[utoipa::path(get, path="/api/v1/network/cloudflare", responses((status=200,description="Cloudflare connection and local connector status; no credentials"),(status=422,description="Agent rejected local status inspection"),(status=503,description="Agent unavailable or status deadline exceeded")), tag="network")]
pub async fn cloudflare_status(
    State(state): State<AppState>,
    RequireAdmin(_user): RequireAdmin,
) -> Result<Json<Value>, Error> {
    tokio::time::timeout(std::time::Duration::from_secs(8), state.agent_client.cloudflare(CloudflareRequest::Status))
        .await.map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE,
            "Pemeriksaan status Cloudflare melewati batas 8 detik. Tidak ada perintah perubahan konektor yang dikirim; coba muat ulang status dan periksa panel-agent."))?
        .map(Json).map_err(cloudflare_failure)
}
#[utoipa::path(post, path="/api/v1/network/cloudflare", request_body=CloudflareRequest, responses((status=200,description="Cloudflare query or DNS mutation result"),(status=202,description="Tunnel operation queued")), tag="network")]
pub async fn cloudflare_command(
    State(state): State<AppState>,
    RequireAdminCsrf(user): RequireAdminCsrf,
    Json(command): Json<CloudflareRequest>,
) -> Result<(StatusCode, Json<Value>), Error> {
    let command = validate_publication(&state.pool, command).await?;
    if matches!(
        command,
        CloudflareRequest::CreateTunnel { .. }
            | CloudflareRequest::TunnelAction { .. }
            | CloudflareRequest::Publish { .. }
            | CloudflareRequest::Unpublish { .. }
    ) {
        let payload = serde_json::to_string(&command).map_err(db_error)?;
        let (job, _) = state
            .job_runner
            .enqueue_cloudflare(user.id, "cloudflare.change", payload)
            .await
            .map_err(db_error)?;
        state.audit_writer.log_async(
            Some(user.id),
            &user.username,
            "cloudflare.queue",
            "cloudflare",
            "success",
            None,
            None,
        );
        return Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::to_value(job).map_err(db_error)?),
        ));
    }
    let mutation = matches!(
        command,
        CloudflareRequest::Configure { .. }
            | CloudflareRequest::CreateZone { .. }
            | CloudflareRequest::SaveRecord { .. }
            | CloudflareRequest::DeleteRecord { .. }
    );
    let response = state.agent_client.cloudflare(command).await;
    if mutation {
        state.audit_writer.log_async(
            Some(user.id),
            &user.username,
            "cloudflare.configure_or_dns",
            "cloudflare",
            if response.is_ok() {
                "success"
            } else {
                "failure"
            },
            None,
            None,
        );
    }
    Ok((StatusCode::OK, Json(response.map_err(cloudflare_failure)?)))
}
#[utoipa::path(post, path="/api/v1/network/cloudflare/install", responses((status=202,description="Signed-repository connector installation queued")), tag="network")]
pub async fn install(
    State(state): State<AppState>,
    RequireAdminCsrf(user): RequireAdminCsrf,
) -> Result<(StatusCode, Json<Value>), Error> {
    let (job, _) = state
        .job_runner
        .enqueue_cloudflare(user.id, "cloudflare.install", "{}".into())
        .await
        .map_err(db_error)?;
    state.audit_writer.log_async(
        Some(user.id),
        &user.username,
        "cloudflare.install",
        "cloudflare",
        "success",
        None,
        None,
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::to_value(job).map_err(db_error)?),
    ))
}

#[cfg(test)]
mod cloudflare_failure_tests {
    use super::*;
    use crate::agent_client::AgentClientError;

    #[test]
    fn cloudflare_errors_are_safe_service_or_validation_responses_instead_of_502() {
        let (status, Json(body)) = cloudflare_failure(AgentClientError::AgentError(
            "fixture-token-must-not-leak".into(),
        ));
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!body.to_string().contains("fixture-token-must-not-leak"));
        let (status, Json(body)) = cloudflare_failure(AgentClientError::Proto(
            panel_proto::ProtoError::ConnectionClosed,
        ));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            body["error"]
                .as_str()
                .expect("message")
                .contains("panel-agent")
        );
    }
}
