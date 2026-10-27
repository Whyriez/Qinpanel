//! Metadata persistence. Mutations require an active administrator; reads resolve
//! project ownership on every query. There are no RPC, probe or provider effects.
use crate::{DbError, DbPool, EndpointEvidenceRecord, ServiceEndpoint};
use panel_core::{DatabaseEngine, connectivity::*};
use sqlx::{FromRow, SqliteConnection};
use std::num::NonZeroU16;

// Explicit projection prevents credentials/bindings from entering Debug or JSON.
const COLUMNS: &str = "e.id,e.revision,e.service,e.scope_kind,e.site_id,e.engine,e.transport,e.host,e.port,e.tls_mode,e.certificate_id,e.enabled,e.is_primary,e.created_at,e.updated_at";
const VISIBLE: &str = "EXISTS (SELECT 1 FROM users actor WHERE actor.id=? AND actor.status='active' AND (actor.role='admin' OR (actor.role='user' AND EXISTS (SELECT 1 FROM sites s WHERE s.id=e.site_id AND s.user_id=actor.id AND s.status!='suspended'))))";

#[derive(FromRow)]
struct EndpointRow {
    id: String,
    revision: i64,
    service: String,
    scope_kind: String,
    site_id: Option<i64>,
    engine: Option<String>,
    transport: String,
    host: String,
    port: i64,
    tls_mode: String,
    certificate_id: Option<i64>,
    enabled: bool,
    is_primary: bool,
    created_at: i64,
    updated_at: i64,
}

fn validation(error: impl std::fmt::Display) -> DbError {
    DbError::Validation(error.to_string())
}

impl EndpointRow {
    fn into_endpoint(self) -> Result<ServiceEndpoint, DbError> {
        let scope = match (
            self.scope_kind.as_str(),
            self.site_id,
            self.engine.as_deref(),
        ) {
            ("host", None, None) => EndpointScope::Host,
            ("project", Some(site_id), None) => EndpointScope::Project { site_id },
            ("engine", None, Some(engine)) => EndpointScope::Engine {
                engine: DatabaseEngine::parse(engine).map_err(validation)?,
            },
            _ => return Err(validation("Scope endpoint tersimpan tidak valid.")),
        };
        let port = u16::try_from(self.port)
            .ok()
            .and_then(NonZeroU16::new)
            .ok_or_else(|| validation("Port endpoint tersimpan tidak valid."))?;
        let config = EndpointConfig {
            service: EndpointService::parse(&self.service).map_err(validation)?,
            scope,
            transport: EndpointTransport::parse(&self.transport).map_err(validation)?,
            host: EndpointHost::parse(&self.host).map_err(validation)?,
            port,
            tls_mode: EndpointTlsMode::parse(&self.tls_mode).map_err(validation)?,
            certificate_id: self.certificate_id,
            enabled: self.enabled,
            primary: self.is_primary,
        };
        config.validate().map_err(validation)?;
        Ok(ServiceEndpoint {
            id: self.id,
            revision: self.revision,
            client_requirement: config.transport.client_requirement(),
            config,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Clone)]
pub struct SqliteConnectivityRepository {
    pool: DbPool,
}

impl SqliteConnectivityRepository {
    /// Durable lease and cooldown shared by API workers. Only administrators can
    /// trigger network work; clients cannot supply results or probe destinations.
    pub async fn claim_diagnosis(
        &self,
        actor_id: i64,
        id: &str,
        revision: i64,
        token: &str,
    ) -> Result<ServiceEndpoint, DbError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_admin(&mut tx, actor_id).await?;
        let endpoint = load(&mut tx, id).await?;
        validate_references(&mut tx, &endpoint.config).await?;
        if endpoint.revision != revision || !endpoint.config.enabled {
            return Err(DbError::Conflict(
                "Alamat berubah atau dinonaktifkan. Muat ulang sebelum memeriksa.".into(),
            ));
        }
        let now = chrono::Utc::now().timestamp();
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM connectivity_diagnosis_leases WHERE lease_until>?",
        )
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if active >= 2 {
            return Err(DbError::Conflict(
                "Dua pemeriksaan sedang berjalan. Coba kembali setelah selesai.".into(),
            ));
        }
        let changed = sqlx::query("INSERT INTO connectivity_diagnosis_leases(endpoint_id,token,lease_until,cooldown_until) VALUES(?,?,?,?) ON CONFLICT(endpoint_id) DO UPDATE SET token=excluded.token,lease_until=excluded.lease_until,cooldown_until=excluded.cooldown_until WHERE connectivity_diagnosis_leases.cooldown_until<=?")
            .bind(id).bind(token).bind(now+30).bind(now+60).bind(now).execute(&mut *tx).await?.rows_affected();
        if changed == 0 {
            return Err(DbError::Conflict(
                "Alamat baru saja diperiksa. Tunggu 60 detik sebelum mencoba lagi.".into(),
            ));
        }
        tx.commit().await?;
        Ok(endpoint)
    }

    /// Publish a complete run atomically. A concurrent edit/delete, expired lease
    /// or revoked administrator discards the whole run, never a partial result.
    pub async fn finish_diagnosis(
        &self,
        actor_id: i64,
        id: &str,
        revision: i64,
        token: &str,
        reports: &[EndpointEvidence],
    ) -> Result<(), DbError> {
        if reports.is_empty() || reports.len() > 32 {
            return Err(validation("Jumlah bukti diagnosis tidak valid."));
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_admin(&mut tx, actor_id).await?;
        let endpoint = load(&mut tx, id).await?;
        validate_references(&mut tx, &endpoint.config).await?;
        if endpoint.revision != revision || !endpoint.config.enabled {
            return Err(DbError::Conflict(
                "Alamat berubah saat diperiksa. Hasil dibuang; muat ulang konfigurasi.".into(),
            ));
        }
        let now = chrono::Utc::now().timestamp();
        let lease: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM connectivity_diagnosis_leases WHERE endpoint_id=? AND token=? AND lease_until>?)")
            .bind(id).bind(token).bind(now).fetch_one(&mut *tx).await?;
        if !lease {
            return Err(DbError::Conflict(
                "Pemeriksaan telah berakhir. Jalankan kembali setelah masa tunggu.".into(),
            ));
        }
        for report in reports {
            report.validate(now).map_err(validation)?;
            if report.endpoint_revision != revision
                || report.observation_point != ObservationPoint::LocalServer
            {
                return Err(validation("Bukti diagnosis lokal tidak valid."));
            }
            if let (Ok(ip), Some(family)) = (
                endpoint.config.host.as_str().parse::<std::net::IpAddr>(),
                report.address_family,
            ) && ip.is_ipv4() != (family == AddressFamily::Ipv4)
            {
                return Err(validation("Keluarga IP bukti tidak cocok dengan endpoint."));
            }
            sqlx::query("INSERT INTO endpoint_evidence(endpoint_id,endpoint_revision,kind,observation_point,observer_id,address_family,observed_at,expires_at,outcome,reason) VALUES(?,?,?,?,?,?,?,?,?,?)")
                .bind(id).bind(revision).bind(report.kind.as_str()).bind(report.observation_point.as_str()).bind(&report.observer_id)
                .bind(report.address_family.map(AddressFamily::as_str)).bind(report.observed_at).bind(report.expires_at).bind(report.outcome.as_str()).bind(report.reason.as_str())
                .execute(&mut *tx).await?;
        }
        // Keep bounded history per endpoint. Existing evidence from other observers
        // remains subject to the same cap, with newest rows retained.
        sqlx::query("DELETE FROM endpoint_evidence WHERE endpoint_id=? AND id NOT IN (SELECT id FROM endpoint_evidence WHERE endpoint_id=? ORDER BY observed_at DESC,id DESC LIMIT 500)")
            .bind(id).bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE connectivity_diagnosis_leases SET lease_until=0 WHERE endpoint_id=? AND token=?")
            .bind(id).bind(token).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Narrow tenant connection projection. Global SSH metadata can be used by
    /// active tenants without exposing global panel/database/mail inventory.
    pub async fn ssh_candidates(
        &self,
        tenant_id: i64,
        site_id: Option<i64>,
    ) -> Result<Vec<ServiceEndpoint>, DbError> {
        let rows = sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {COLUMNS} FROM service_endpoints e WHERE e.service='ssh' AND e.enabled=1 AND e.is_primary=1 AND EXISTS(SELECT 1 FROM users u WHERE u.id=? AND u.role='user' AND u.status='active') AND (e.scope_kind='host' OR (e.scope_kind='project' AND e.site_id=? AND EXISTS(SELECT 1 FROM sites s WHERE s.id=e.site_id AND s.user_id=? AND s.status!='suspended'))) ORDER BY CASE e.scope_kind WHEN 'project' THEN 0 ELSE 1 END,e.id"
        )).bind(tenant_id).bind(site_id).bind(tenant_id).fetch_all(&self.pool).await?;
        rows.into_iter().map(EndpointRow::into_endpoint).collect()
    }

    pub async fn list(&self, actor_id: i64) -> Result<Vec<ServiceEndpoint>, DbError> {
        let rows = sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {COLUMNS} FROM service_endpoints e WHERE {VISIBLE} ORDER BY e.service,e.id"
        ))
        .bind(actor_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(EndpointRow::into_endpoint).collect()
    }

    pub async fn find(&self, actor_id: i64, id: &str) -> Result<Option<ServiceEndpoint>, DbError> {
        let row = sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {COLUMNS} FROM service_endpoints e WHERE {VISIBLE} AND e.id=?"
        ))
        .bind(actor_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(EndpointRow::into_endpoint).transpose()
    }

    pub async fn list_page(
        &self,
        actor_id: i64,
        service: Option<EndpointService>,
        limit: u32,
        offset: u64,
    ) -> Result<Vec<ServiceEndpoint>, DbError> {
        let rows = sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {COLUMNS} FROM service_endpoints e WHERE {VISIBLE} AND (? IS NULL OR e.service=?) ORDER BY e.service,e.id LIMIT ? OFFSET ?"
        )).bind(actor_id).bind(service.map(EndpointService::as_str)).bind(service.map(EndpointService::as_str))
            .bind(i64::from(limit)).bind(i64::try_from(offset).map_err(|_| validation("Halaman tidak valid."))?).fetch_all(&self.pool).await?;
        rows.into_iter().map(EndpointRow::into_endpoint).collect()
    }

    pub async fn service_counts(
        &self,
        actor_id: i64,
    ) -> Result<std::collections::BTreeMap<String, i64>, DbError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(&format!(
            "SELECT e.service,COUNT(*) FROM service_endpoints e WHERE {VISIBLE} GROUP BY e.service"
        ))
        .bind(actor_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn create(
        &self,
        actor_id: i64,
        config: &EndpointConfig,
    ) -> Result<ServiceEndpoint, DbError> {
        self.create_once(actor_id, config, None)
            .await
            .map(|(endpoint, _)| endpoint)
    }

    /// An optional create key survives API restart and endpoint deletion. A replay
    /// returns the existing resource, or a conflict if deleted/different payload.
    pub async fn create_once(
        &self,
        actor_id: i64,
        config: &EndpointConfig,
        request_key: Option<uuid::Uuid>,
    ) -> Result<(ServiceEndpoint, bool), DbError> {
        config.validate().map_err(validation)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_admin(&mut tx, actor_id).await?;
        let payload = serde_json::to_vec(config)
            .map_err(|_| validation("Konfigurasi endpoint tidak valid."))?;
        let payload_hash = panel_core::compute_bytes_sha256(&payload);
        let request_key = request_key.map(|key| key.to_string());
        if let Some(key) = &request_key {
            let existing: Option<(String, String)> = sqlx::query_as("SELECT endpoint_id,payload_hash FROM connectivity_create_requests WHERE actor_id=? AND request_key=?")
                .bind(actor_id).bind(key).fetch_optional(&mut *tx).await?;
            if let Some((id, hash)) = existing {
                if hash != payload_hash {
                    return Err(DbError::Conflict("Kunci permintaan sudah dipakai untuk isian berbeda. Muat ulang daftar sebelum membuat alamat baru.".into()));
                }
                let endpoint = load(&mut tx, &id).await.map_err(|error| match error {
                    DbError::NotFound(_) => DbError::Conflict("Alamat dari permintaan ini sudah dihapus. Muat ulang daftar sebelum membuat alamat baru.".into()),
                    other => other,
                })?;
                tx.commit().await?;
                return Ok((endpoint, false));
            }
        }
        validate_references(&mut tx, config).await?;
        let (scope_kind, site_id, engine) = scope_columns(config.scope);
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO service_endpoints (id,revision,service,scope_kind,site_id,engine,transport,host,port,tls_mode,certificate_id,enabled,is_primary,created_at,updated_at) VALUES (?,1,?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(&id).bind(config.service.as_str()).bind(scope_kind).bind(site_id).bind(engine)
            .bind(config.transport.as_str()).bind(config.host.as_str()).bind(i64::from(config.port.get()))
            .bind(config.tls_mode.as_str()).bind(config.certificate_id).bind(config.enabled).bind(config.primary)
            .bind(now).bind(now).execute(&mut *tx).await.map_err(write_error)?;
        let result = load(&mut tx, &id).await?;
        if let Some(key) = request_key {
            sqlx::query("INSERT INTO connectivity_create_requests(actor_id,request_key,payload_hash,endpoint_id) VALUES (?,?,?,?)")
                .bind(actor_id).bind(key).bind(payload_hash).bind(&id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok((result, true))
    }

    /// Full replacement with optimistic concurrency. Service/scope are immutable;
    /// moving an endpoint to another resource requires creating a new identity.
    pub async fn update(
        &self,
        actor_id: i64,
        id: &str,
        expected_revision: i64,
        config: &EndpointConfig,
    ) -> Result<ServiceEndpoint, DbError> {
        config.validate().map_err(validation)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_admin(&mut tx, actor_id).await?;
        let old = load(&mut tx, id).await?;
        if old.revision != expected_revision {
            return Err(DbError::Conflict(
                "Endpoint telah berubah. Muat ulang konfigurasi.".into(),
            ));
        }
        if old.config.service != config.service || old.config.scope != config.scope {
            return Err(validation(
                "Layanan dan scope endpoint tidak dapat dipindahkan.",
            ));
        }
        validate_references(&mut tx, config).await?;
        sqlx::query("UPDATE service_endpoints SET revision=revision+1,transport=?,host=?,port=?,tls_mode=?,certificate_id=?,enabled=?,is_primary=?,updated_at=? WHERE id=? AND revision=?")
            .bind(config.transport.as_str()).bind(config.host.as_str()).bind(i64::from(config.port.get()))
            .bind(config.tls_mode.as_str()).bind(config.certificate_id).bind(config.enabled).bind(config.primary)
            .bind(chrono::Utc::now().timestamp()).bind(id).bind(expected_revision)
            .execute(&mut *tx).await.map_err(write_error)?;
        let result = load(&mut tx, id).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn delete(
        &self,
        actor_id: i64,
        id: &str,
        expected_revision: i64,
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_admin(&mut tx, actor_id).await?;
        let result = sqlx::query("DELETE FROM service_endpoints WHERE id=? AND revision=?")
            .bind(id)
            .bind(expected_revision)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() != 1 {
            return Err(DbError::Conflict(
                "Endpoint hilang atau telah berubah. Muat ulang konfigurasi.".into(),
            ));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Trusted persistence boundary for future diagnosis jobs, not a public
    /// endpoint accepting unverified tenant/client claims.
    pub async fn record_evidence(
        &self,
        actor_id: i64,
        id: &str,
        evidence: &EndpointEvidence,
    ) -> Result<i64, DbError> {
        evidence
            .validate(chrono::Utc::now().timestamp())
            .map_err(validation)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_admin(&mut tx, actor_id).await?;
        let endpoint = load(&mut tx, id).await?;
        if endpoint.revision != evidence.endpoint_revision || !endpoint.config.enabled {
            return Err(DbError::Conflict(
                "Bukti tidak cocok dengan endpoint aktif saat ini.".into(),
            ));
        }
        if let (Ok(ip), Some(family)) = (
            endpoint.config.host.as_str().parse::<std::net::IpAddr>(),
            evidence.address_family,
        ) && ip.is_ipv4() != (family == AddressFamily::Ipv4)
        {
            return Err(validation("Keluarga IP bukti tidak cocok dengan endpoint."));
        }
        let result = sqlx::query("INSERT INTO endpoint_evidence (endpoint_id,endpoint_revision,kind,observation_point,observer_id,address_family,observed_at,expires_at,outcome,reason) VALUES (?,?,?,?,?,?,?,?,?,?)")
            .bind(id).bind(evidence.endpoint_revision).bind(evidence.kind.as_str())
            .bind(evidence.observation_point.as_str()).bind(&evidence.observer_id)
            .bind(evidence.address_family.map(AddressFamily::as_str)).bind(evidence.observed_at)
            .bind(evidence.expires_at).bind(evidence.outcome.as_str()).bind(evidence.reason.as_str())
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result.last_insert_rowid())
    }

    /// History preserves expired/revision-stale reports; only the newest report
    /// per check, observer and IP family can be current. Authorization and current
    /// endpoint revision come from the same SQL snapshot.
    pub async fn evidence(
        &self,
        actor_id: i64,
        id: &str,
    ) -> Result<Vec<EndpointEvidenceRecord>, DbError> {
        let rows = sqlx::query_as::<_, EvidenceRow>(&format!(
            "SELECT r.id,r.endpoint_id,e.service,e.revision AS current_revision,e.enabled,r.endpoint_revision,r.kind,r.observation_point,r.observer_id,r.address_family,r.observed_at,r.expires_at,r.outcome,r.reason,
            NOT EXISTS (SELECT 1 FROM endpoint_evidence newer WHERE newer.endpoint_id=r.endpoint_id AND newer.endpoint_revision=r.endpoint_revision AND newer.kind=r.kind AND newer.observation_point=r.observation_point AND newer.observer_id=r.observer_id AND newer.address_family IS r.address_family AND (newer.observed_at>r.observed_at OR (newer.observed_at=r.observed_at AND newer.id>r.id))) AS latest
            FROM endpoint_evidence r JOIN service_endpoints e ON e.id=r.endpoint_id WHERE {VISIBLE} AND e.id=? ORDER BY r.observed_at DESC,r.id DESC"
        )).bind(actor_id).bind(id).fetch_all(&self.pool).await?;
        let now = chrono::Utc::now().timestamp();
        rows.into_iter().map(|row| row.into_record(now)).collect()
    }
}

async fn require_admin(conn: &mut SqliteConnection, actor_id: i64) -> Result<(), DbError> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin' AND status='active')",
    )
    .bind(actor_id)
    .fetch_one(conn)
    .await?;
    if !allowed {
        return Err(validation(
            "Pengaturan konektivitas memerlukan administrator aktif.",
        ));
    }
    Ok(())
}

async fn load(conn: &mut SqliteConnection, id: &str) -> Result<ServiceEndpoint, DbError> {
    sqlx::query_as::<_, EndpointRow>(&format!(
        "SELECT {COLUMNS} FROM service_endpoints e WHERE e.id=?"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| DbError::NotFound("Endpoint tidak ditemukan.".into()))?
    .into_endpoint()
}

fn scope_columns(scope: EndpointScope) -> (&'static str, Option<i64>, Option<&'static str>) {
    match scope {
        EndpointScope::Host => ("host", None, None),
        EndpointScope::Project { site_id } => ("project", Some(site_id), None),
        EndpointScope::Engine { engine } => ("engine", None, Some(engine.as_str())),
    }
}

async fn validate_references(
    conn: &mut SqliteConnection,
    config: &EndpointConfig,
) -> Result<(), DbError> {
    if let EndpointScope::Project { site_id } = config.scope {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sites s JOIN users u ON u.id=s.user_id WHERE s.id=? AND u.status='active' AND s.status!='suspended')")
            .bind(site_id).fetch_one(&mut *conn).await?;
        if !exists {
            return Err(validation(
                "Project endpoint tidak ditemukan atau tidak aktif.",
            ));
        }
        if let Some(certificate_id) = config.certificate_id {
            let matches: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM certificates WHERE id=? AND site_id=?)",
            )
            .bind(certificate_id)
            .bind(site_id)
            .fetch_one(&mut *conn)
            .await?;
            if !matches {
                return Err(validation("Sertifikat bukan milik project endpoint."));
            }
        }
    }
    Ok(())
}

fn write_error(error: sqlx::Error) -> DbError {
    if error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
    {
        DbError::Conflict("Alamat utama sudah ada untuk layanan dan scope ini.".into())
    } else {
        DbError::Sqlx(error)
    }
}

#[derive(FromRow)]
struct EvidenceRow {
    id: i64,
    endpoint_id: String,
    service: String,
    current_revision: i64,
    enabled: bool,
    endpoint_revision: i64,
    kind: String,
    observation_point: String,
    observer_id: String,
    address_family: Option<String>,
    observed_at: i64,
    expires_at: i64,
    outcome: String,
    reason: String,
    latest: bool,
}

impl EvidenceRow {
    fn into_record(self, now: i64) -> Result<EndpointEvidenceRecord, DbError> {
        let evidence = EndpointEvidence {
            endpoint_revision: self.endpoint_revision,
            kind: EvidenceKind::parse(&self.kind).map_err(validation)?,
            observation_point: ObservationPoint::parse(&self.observation_point)
                .map_err(validation)?,
            observer_id: self.observer_id,
            address_family: self
                .address_family
                .as_deref()
                .map(AddressFamily::parse)
                .transpose()
                .map_err(validation)?,
            observed_at: self.observed_at,
            expires_at: self.expires_at,
            outcome: EvidenceOutcome::parse(&self.outcome).map_err(validation)?,
            reason: EvidenceReason::parse(&self.reason).map_err(validation)?,
        };
        evidence.validate(now).map_err(validation)?;
        Ok(EndpointEvidenceRecord {
            id: self.id,
            endpoint_id: self.endpoint_id,
            service: EndpointService::parse(&self.service).map_err(validation)?,
            current: self.enabled && self.latest && evidence.is_current(self.current_revision, now),
            expired: now >= evidence.expires_at,
            revision_matches: evidence.endpoint_revision == self.current_revision,
            suggested_action: evidence.reason.suggested_action(),
            evidence,
        })
    }
}
