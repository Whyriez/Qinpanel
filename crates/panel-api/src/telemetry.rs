//! Production host metrics lifecycle and current-sample API.
use crate::{auth::AuthenticatedUser, routes::AppState};
use axum::{Json, extract::State, http::StatusCode};
use panel_analytics::{
    CgroupMetricsCollector, DiskUsageReader, ProcReader, RetentionConfig, ServerMetricsPoint,
    SystemMetricsCollector,
};
use serde::Serialize;
use std::sync::Arc;

pub const SAMPLE_INTERVAL_SECS: u64 = 10;
#[derive(Clone, Debug, Default, Serialize)]
pub struct CollectorStatus {
    pub last_success_at: Option<i64>,
    pub failed: bool,
}
#[derive(Serialize)]
pub struct MetricsResponse {
    pub state: &'static str,
    pub sample: Option<ServerMetricsPoint>,
    pub error: Option<&'static str>,
    pub sampling_interval_secs: u64,
}

#[utoipa::path(get,path="/api/v1/system/metrics",responses((status=200,description="Current stored host sample with waiting, ready, stale or failed state"),(status=401,description="Authentication required"),(status=403,description="Administrator required")),tag="analytics")]
pub async fn current_metrics(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Json<MetricsResponse>, (StatusCode, Json<serde_json::Value>)> {
    if !user.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"Metrik server hanya tersedia untuk administrator."})),
        ));
    }
    let collector = state.collector_status.read().await.clone();
    let sample = match state.get_analytics().await {
        Ok(storage) => match storage.latest_server_point().await {
            Ok(sample) => sample,
            Err(err) => {
                tracing::error!(error=%err, "Read host sample failed");
                return Ok(Json(MetricsResponse {
                    state: "failed",
                    sample: None,
                    error: Some("Gagal membaca metrik server. Periksa log layanan."),
                    sampling_interval_secs: SAMPLE_INTERVAL_SECS,
                }));
            }
        },
        Err(_) => {
            return Ok(Json(MetricsResponse {
                state: "failed",
                sample: None,
                error: Some("Database metrik server tidak dapat diakses. Periksa log layanan."),
                sampling_interval_secs: SAMPLE_INTERVAL_SECS,
            }));
        }
    };
    let status = if collector.failed {
        "failed"
    } else if sample.as_ref().is_some_and(|s| {
        chrono::Utc::now().timestamp() - s.timestamp > 3 * SAMPLE_INTERVAL_SECS as i64
    }) {
        "stale"
    } else if sample.is_some() {
        "ready"
    } else {
        "waiting"
    };
    Ok(Json(MetricsResponse {
        state: status,
        sample,
        error: (status == "failed")
            .then_some("Pengumpulan metrik server gagal. Periksa log layanan."),
        sampling_interval_secs: SAMPLE_INTERVAL_SECS,
    }))
}

async fn store_host_sample(state: &AppState, collector: &SystemMetricsCollector, now: i64) -> bool {
    let result = async {
        let point = collector.collect_server_point(now).await?;
        collector.storage().insert_server_point(&point).await
    }
    .await;
    match result {
        Ok(()) => {
            *state.collector_status.write().await = CollectorStatus {
                last_success_at: Some(now),
                failed: false,
            };
            true
        }
        Err(err) => {
            state.collector_status.write().await.failed = true;
            tracing::error!(error=%err, "Host metrics collection failed");
            false
        }
    }
}

async fn store_tenant_sample(
    collector: &SystemMetricsCollector,
    uid: u32,
    now: i64,
) -> Result<(), panel_analytics::AnalyticsError> {
    let points = collector.collect_tenant_points(&[uid], now).await?;
    collector.storage().insert_tenant_points(&points).await
}
async fn store_tenant_samples(state: &AppState, collector: &SystemMetricsCollector, now: i64) {
    let users = match panel_db::repositories::SqliteUserRepository::new(state.pool.clone())
        .list_all()
        .await
    {
        Ok(users) => users,
        Err(err) => {
            tracing::warn!(error=%err,"List monitored tenants failed");
            return;
        }
    };
    for user in users {
        if let Ok((_, uid)) = crate::tenant_context::execution_identity(&user)
            && let Err(err) = store_tenant_sample(collector, uid, now).await
        {
            tracing::warn!(uid,error=%err,"Tenant cgroup metrics unavailable");
        }
    }
}
async fn store_service_sample(
    collector: &SystemMetricsCollector,
    service: &str,
    now: i64,
) -> Result<(), panel_analytics::AnalyticsError> {
    let points = collector.collect_service_points(&[service], now).await?;
    collector.storage().insert_service_points(&points).await
}
async fn store_service_samples(collector: &SystemMetricsCollector, now: i64) {
    for service in ["panel-api", "panel-agent", "nginx"] {
        if let Err(err) = store_service_sample(collector, service, now).await {
            tracing::warn!(service,error=%err,"Service cgroup metrics unavailable");
        }
    }
}
async fn rollup_samples(collector: &SystemMetricsCollector, now: i64) {
    if let Err(err) = collector.execute_rollups_and_retention(now).await {
        tracing::warn!(error=%err,"System metrics rollup failed");
    }
}

pub async fn run_collector(
    state: AppState,
    collector: SystemMetricsCollector,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(SAMPLE_INTERVAL_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut rollup_at = 0;
    loop {
        tokio::select! {
            changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { break; } }
            _ = interval.tick() => {
                if *shutdown.borrow() { break; }
                let now = chrono::Utc::now().timestamp();
                if !store_host_sample(&state, &collector, now).await { continue; }
                // Missing cgroups never prevent the host sample from being persisted.
                store_tenant_samples(&state, &collector, now).await;
                store_service_samples(&collector, now).await;
                if now >= rollup_at { rollup_samples(&collector, now).await; rollup_at = now + 60; }
            }
        }
    }
    tracing::info!("System metrics collector stopped");
}

pub async fn run_system_metrics_worker(
    state: AppState,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        match state.get_analytics().await {
            Ok(storage) => {
                let collector = SystemMetricsCollector::new(
                    Arc::new(ProcReader::default_system()),
                    Arc::new(CgroupMetricsCollector::new("/sys/fs/cgroup")),
                    Arc::new(DiskUsageReader::default_root()),
                    (*storage).clone(),
                    RetentionConfig::default(),
                );
                tracing::info!("Starting system metrics collector (10 second interval)");
                run_collector(state, collector, shutdown).await;
                return;
            }
            Err((_, message)) => {
                state.collector_status.write().await.failed = true;
                tracing::error!(%message, "Analytics initialization failed; retrying");
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(SAMPLE_INTERVAL_SECS)) => {}
            changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return; }
        }
    }
}
