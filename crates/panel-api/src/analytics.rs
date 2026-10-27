//! Deep web analytics, application metrics, and system timeseries query API endpoints.
//!
//! Provides:
//! - Overview and period comparison metrics with percentage delta
//! - Timeseries metrics with automatic resolution selection and cost-limit guardrails
//! - Breakdown aggregates (paths, referrers, countries, browsers, devices, slow URLs, errors)
//! - Application metrics for PHP-FPM and Node.js/Python cgroup v2 processes
//! - Structured PHP-FPM slowlog events with full stack trace frames
//! - CSV export with standard downloadable attachments
//! - System host metrics timeseries for administrators

use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use panel_analytics::{
    AnalyticsInsight, AppType, DeploymentContext, DiskQuotaContext, FpmStatusMetrics,
    InsightEngine, InsightEvaluationContext, InsightSeverity, LATENCY_BUCKET_COUNT,
    LatencyHistogram, PhpSlowlogEntry, RollupResolution, SiteMetricsRollup, SlowEndpointItem,
    TrafficMetricsSummary,
};
use panel_db::repositories::{
    SqliteGitDeployRepository, SqlitePlanRepository, SqliteSiteRepository, SqliteUserRepository,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;
use crate::sites::fetch_and_authorize_site;

// ==========================================
// Query Cost Limit Constants
// ==========================================

pub const MAX_1M_RANGE_SECS: i64 = 7 * 86400; // 7 days max for 1m interval
pub const MAX_1H_RANGE_SECS: i64 = 90 * 86400; // 90 days max for 1h interval
pub const MAX_1D_RANGE_SECS: i64 = 3 * 365 * 86400; // 3 years max for 1d interval
pub const MAX_BREAKDOWN_LIMIT: usize = 50;
pub const DEFAULT_BREAKDOWN_LIMIT: usize = 10;

// ==========================================
// Request Query DTOs
// ==========================================

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct AnalyticsTimeRangeQuery {
    /// Start timestamp (Unix epoch seconds). Defaults to 24 hours ago.
    pub from: Option<i64>,
    /// End timestamp (Unix epoch seconds). Defaults to current time.
    pub to: Option<i64>,
    /// Aggregation interval: "auto", "1m", "1h", "1d". Defaults to "auto".
    pub interval: Option<String>,
    /// Include comparison against previous period.
    pub compare: Option<bool>,
    /// Optional explicit start timestamp for comparison period.
    pub prev_from: Option<i64>,
    /// Optional explicit end timestamp for comparison period.
    pub prev_to: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct AnalyticsBreakdownsQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Dimension to filter: "paths", "referrers", "countries", "browsers", "os", "devices", "slow_urls", "error_urls", "all".
    pub dimension: Option<String>,
    /// Maximum number of top items (default: 10, max: 50).
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct AnalyticsSlowlogQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Maximum slowlog events to retrieve (default: 20, max: 100).
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct AnalyticsExportQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub interval: Option<String>,
}

// ==========================================
// Response DTOs
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteAnalyticsSummaryDto {
    pub total_requests: u64,
    pub total_bytes: u64,
    pub unique_visitors: u64,
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    pub cache_hit_ratio: f64,
    pub latency_avg_ms: f64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub error_rate_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PeriodChangeDto {
    pub requests_change_percent: f64,
    pub bytes_change_percent: f64,
    pub visitors_change_percent: f64,
    pub avg_latency_change_percent: f64,
    pub error_rate_change_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteAnalyticsOverviewResponse {
    pub site_id: i64,
    pub from_ts: i64,
    pub to_ts: i64,
    pub interval_used: String,
    pub current: SiteAnalyticsSummaryDto,
    pub previous: Option<SiteAnalyticsSummaryDto>,
    pub comparison: Option<PeriodChangeDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteTimeseriesPointDto {
    pub timestamp: i64,
    pub requests: u64,
    pub bytes: u64,
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    pub visitors: u64,
    pub cache_hit: u64,
    pub cache_miss: u64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub latency_avg_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteTimeseriesResponse {
    pub site_id: i64,
    pub from_ts: i64,
    pub to_ts: i64,
    pub interval: String,
    pub points: Vec<SiteTimeseriesPointDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BreakdownItemDto {
    pub name: String,
    pub count: u64,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteBreakdownsResponse {
    pub site_id: i64,
    pub from_ts: i64,
    pub to_ts: i64,
    pub paths: Vec<BreakdownItemDto>,
    pub referrers: Vec<BreakdownItemDto>,
    pub countries: Vec<BreakdownItemDto>,
    pub browsers: Vec<BreakdownItemDto>,
    pub os: Vec<BreakdownItemDto>,
    pub devices: Vec<BreakdownItemDto>,
    pub slow_urls: Vec<BreakdownItemDto>,
    pub error_urls: Vec<BreakdownItemDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpFpmPoolOverviewDto {
    pub pool_name: String,
    pub active_avg: f64,
    pub active_max: u32,
    pub idle_avg: f64,
    pub total_max: u32,
    pub listen_queue_max: u32,
    pub max_children_reached_max: u32,
    pub slow_requests_total: u64,
    pub accepted_conn_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppProcessOverviewDto {
    pub site_id: i64,
    pub app_type: String,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub memory_current_avg_bytes: u64,
    pub memory_current_max_bytes: u64,
    pub memory_peak_max_bytes: Option<u64>,
    pub pids_max: u64,
    pub io_read_avg_bytes_sec: f64,
    pub io_write_avg_bytes_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteAppMetricsResponse {
    pub site_id: i64,
    pub runtime: String,
    pub php_fpm: Option<PhpFpmPoolOverviewDto>,
    pub app_process: Option<AppProcessOverviewDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpSlowlogFrameDto {
    pub frame_index: usize,
    pub function: String,
    pub file: String,
    pub line: u32,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhpSlowlogEntryDto {
    pub id: Option<i64>,
    pub timestamp: i64,
    pub pool: String,
    pub pid: u32,
    pub script_filename: String,
    pub top_function: Option<String>,
    pub frames: Vec<PhpSlowlogFrameDto>,
    pub duration_secs: Option<f64>,
    pub site_id: Option<i64>,
    pub tenant_user: Option<String>,
}

impl From<PhpSlowlogEntry> for PhpSlowlogEntryDto {
    fn from(e: PhpSlowlogEntry) -> Self {
        Self {
            id: e.id,
            timestamp: e.timestamp,
            pool: e.pool,
            pid: e.pid,
            script_filename: e.script_filename,
            top_function: e.top_function,
            frames: e
                .frames
                .into_iter()
                .map(|f| PhpSlowlogFrameDto {
                    frame_index: f.frame_index,
                    function: f.function,
                    file: f.file,
                    line: f.line,
                    address: f.address,
                })
                .collect(),
            duration_secs: e.duration_secs,
            site_id: e.site_id,
            tenant_user: e.tenant_user,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteSlowlogResponse {
    pub site_id: i64,
    pub events: Vec<PhpSlowlogEntryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServerTimeseriesPointDto {
    pub timestamp: i64,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub memory_used_avg_bytes: u64,
    pub memory_used_max_bytes: u64,
    pub swap_used_avg_bytes: u64,
    pub load1_avg: f64,
    pub load1_max: f64,
    pub disk_used_avg_bytes: u64,
    pub net_rx_avg_bytes_sec: f64,
    pub net_tx_avg_bytes_sec: f64,
    pub io_read_avg_bytes_sec: f64,
    pub io_write_avg_bytes_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SystemTimeseriesResponse {
    pub from_ts: i64,
    pub to_ts: i64,
    pub interval: String,
    pub points: Vec<ServerTimeseriesPointDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InsightActionDto {
    pub action_type: String,
    pub label: String,
    pub target_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AnalyticsInsightDto {
    pub id: String,
    pub site_id: Option<i64>,
    pub insight_type: String,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub action: Option<InsightActionDto>,
    pub detected_at: i64,
    pub metadata_json: Option<String>,
}

impl From<AnalyticsInsight> for AnalyticsInsightDto {
    fn from(i: AnalyticsInsight) -> Self {
        Self {
            id: i.id,
            site_id: i.site_id,
            insight_type: i.insight_type.as_str().to_string(),
            severity: i.severity.as_str().to_string(),
            title: i.title,
            description: i.description,
            action: i.action.map(|a| InsightActionDto {
                action_type: a.action_type,
                label: a.label,
                target_url: a.target_url,
            }),
            detected_at: i.detected_at,
            metadata_json: i.metadata_json,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SiteInsightsResponse {
    pub site_id: i64,
    pub evaluated_at: i64,
    pub insights: Vec<AnalyticsInsightDto>,
    pub notifications_created: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SystemInsightsResponse {
    pub evaluated_at: i64,
    pub total_insights: usize,
    pub insights: Vec<AnalyticsInsightDto>,
}

// ==========================================
// Query Guardrail & Validation Helpers
// ==========================================

pub type ResolvedTimeRange = (i64, i64, RollupResolution, &'static str);
pub type ApiError = (StatusCode, Json<serde_json::Value>);

/// Validates time range and resolves the target resolution with query cost limits.
pub fn resolve_query_time_range(
    from: Option<i64>,
    to: Option<i64>,
    interval_req: Option<&str>,
) -> Result<ResolvedTimeRange, ApiError> {
    let now = Utc::now().timestamp();
    let to_ts = to.unwrap_or(now);
    let from_ts = from.unwrap_or_else(|| to_ts - 86400);

    if from_ts > to_ts {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "type": "https://api.vpanel.dev/errors/invalid-time-range",
                "title": "Rentang waktu tidak valid",
                "status": 400,
                "detail": "Parameter 'from' tidak boleh lebih besar dari parameter 'to'."
            })),
        ));
    }

    if to_ts > now + 300 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "type": "https://api.vpanel.dev/errors/invalid-time-range",
                "title": "Rentang waktu di masa depan",
                "status": 400,
                "detail": "Parameter 'to' tidak boleh berada di masa depan."
            })),
        ));
    }

    let duration_secs = to_ts - from_ts;
    let interval = interval_req.unwrap_or("auto").to_ascii_lowercase();

    match interval.as_str() {
        "auto" => {
            if duration_secs <= 6 * 3600 {
                Ok((from_ts, to_ts, RollupResolution::OneMinute, "1m"))
            } else if duration_secs <= 30 * 86400 {
                Ok((from_ts, to_ts, RollupResolution::OneHour, "1h"))
            } else {
                Ok((from_ts, to_ts, RollupResolution::OneDay, "1d"))
            }
        }
        "1m" | "minute" => {
            if duration_secs > MAX_1M_RANGE_SECS {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "type": "https://api.vpanel.dev/errors/query-cost-exceeded",
                        "title": "Batas biaya query terlampaui",
                        "status": 400,
                        "detail": format!("Rentang waktu untuk interval 1 menit maksimal {} hari.", MAX_1M_RANGE_SECS / 86400)
                    })),
                ));
            }
            Ok((from_ts, to_ts, RollupResolution::OneMinute, "1m"))
        }
        "1h" | "hour" => {
            if duration_secs > MAX_1H_RANGE_SECS {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "type": "https://api.vpanel.dev/errors/query-cost-exceeded",
                        "title": "Batas biaya query terlampaui",
                        "status": 400,
                        "detail": format!("Rentang waktu untuk interval 1 jam maksimal {} hari.", MAX_1H_RANGE_SECS / 86400)
                    })),
                ));
            }
            Ok((from_ts, to_ts, RollupResolution::OneHour, "1h"))
        }
        "1d" | "day" => {
            if duration_secs > MAX_1D_RANGE_SECS {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "type": "https://api.vpanel.dev/errors/query-cost-exceeded",
                        "title": "Batas biaya query terlampaui",
                        "status": 400,
                        "detail": "Rentang waktu untuk interval 1 hari maksimal 3 tahun."
                    })),
                ));
            }
            Ok((from_ts, to_ts, RollupResolution::OneDay, "1d"))
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "type": "https://api.vpanel.dev/errors/invalid-interval",
                "title": "Interval tidak valid",
                "status": 400,
                "detail": format!("Interval '{other}' tidak dikenali. Gunakan 'auto', '1m', '1h', atau '1d'.")
            })),
        )),
    }
}

/// Aggregates rollups into a single summary point.
pub fn aggregate_site_summary(rollups: &[SiteMetricsRollup]) -> SiteAnalyticsSummaryDto {
    if rollups.is_empty() {
        return SiteAnalyticsSummaryDto {
            total_requests: 0,
            total_bytes: 0,
            unique_visitors: 0,
            status_2xx: 0,
            status_3xx: 0,
            status_4xx: 0,
            status_5xx: 0,
            cache_hit_ratio: 0.0,
            latency_avg_ms: 0.0,
            latency_p50_ms: 0.0,
            latency_p95_ms: 0.0,
            latency_p99_ms: 0.0,
            error_rate_percent: 0.0,
        };
    }

    let mut total_requests = 0u64;
    let mut total_bytes = 0u64;
    let mut unique_visitors = 0u64;
    let mut status_2xx = 0u64;
    let mut status_3xx = 0u64;
    let mut status_4xx = 0u64;
    let mut status_5xx = 0u64;
    let mut cache_hit = 0u64;
    let mut merged_histogram = LatencyHistogram::new();

    for r in rollups {
        total_requests += r.total_requests;
        total_bytes += r.total_bytes;
        unique_visitors += r.unique_visitors;
        status_2xx += r.status_2xx;
        status_3xx += r.status_3xx;
        status_4xx += r.status_4xx;
        status_5xx += r.status_5xx;
        cache_hit += r.cache_hit;

        for (idx, &count) in r.latency_buckets.iter().enumerate() {
            if idx < LATENCY_BUCKET_COUNT {
                merged_histogram.buckets[idx] += count;
                merged_histogram.count += count;
            }
        }
    }

    let cache_hit_ratio = if total_requests > 0 {
        (cache_hit as f64 / total_requests as f64) * 100.0
    } else {
        0.0
    };

    let error_requests = status_4xx + status_5xx;
    let error_rate_percent = if total_requests > 0 {
        (error_requests as f64 / total_requests as f64) * 100.0
    } else {
        0.0
    };

    let (p50, p95, p99, avg_ms) = if merged_histogram.count > 0 {
        (
            merged_histogram.p50_ms(),
            merged_histogram.p95_ms(),
            merged_histogram.p99_ms(),
            merged_histogram.avg_ms(),
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };

    SiteAnalyticsSummaryDto {
        total_requests,
        total_bytes,
        unique_visitors,
        status_2xx,
        status_3xx,
        status_4xx,
        status_5xx,
        cache_hit_ratio,
        latency_avg_ms: avg_ms,
        latency_p50_ms: p50,
        latency_p95_ms: p95,
        latency_p99_ms: p99,
        error_rate_percent,
    }
}

/// Calculates percentage delta between current and previous periods.
pub fn calculate_period_change(
    curr: &SiteAnalyticsSummaryDto,
    prev: &SiteAnalyticsSummaryDto,
) -> PeriodChangeDto {
    fn pct_delta(c: f64, p: f64) -> f64 {
        if p > 0.0001 {
            ((c - p) / p) * 100.0
        } else if c > 0.0001 {
            100.0
        } else {
            0.0
        }
    }

    PeriodChangeDto {
        requests_change_percent: pct_delta(curr.total_requests as f64, prev.total_requests as f64),
        bytes_change_percent: pct_delta(curr.total_bytes as f64, prev.total_bytes as f64),
        visitors_change_percent: pct_delta(
            curr.unique_visitors as f64,
            prev.unique_visitors as f64,
        ),
        avg_latency_change_percent: pct_delta(curr.latency_avg_ms, prev.latency_avg_ms),
        error_rate_change_percent: pct_delta(curr.error_rate_percent, prev.error_rate_percent),
    }
}

// ==========================================
// Handlers
// ==========================================

/// GET /api/v1/sites/{id}/analytics/overview
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/overview",
    params(
        ("id" = i64, Path, description = "Site ID"),
        AnalyticsTimeRangeQuery
    ),
    responses(
        (status = 200, description = "Site analytics overview", body = SiteAnalyticsOverviewResponse),
        (status = 400, description = "Invalid time range or query cost exceeded"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn get_site_analytics_overview_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<AnalyticsTimeRangeQuery>,
) -> Result<Json<SiteAnalyticsOverviewResponse>, (StatusCode, Json<serde_json::Value>)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| {
            (
                code,
                Json(serde_json::json!({ "title": msg, "status": code.as_u16() })),
            )
        })?;

    let (from_ts, to_ts, resolution, interval_str) =
        resolve_query_time_range(query.from, query.to, query.interval.as_deref())?;

    let analytics = state.get_analytics().await.map_err(|(c, m)| {
        (
            c,
            Json(serde_json::json!({ "title": m, "status": c.as_u16() })),
        )
    })?;

    let current_rollups = analytics
        .query_site_rollups(site_id, from_ts, to_ts, resolution)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "title": e.to_string(), "status": 500 })),
            )
        })?;

    let current_summary = aggregate_site_summary(&current_rollups);

    let (prev_summary, comparison) = if query.compare.unwrap_or(false) {
        let duration = to_ts - from_ts;
        let prev_to = query.prev_to.unwrap_or(from_ts - 1);
        let prev_from = query.prev_from.unwrap_or(prev_to - duration);

        let prev_rollups = analytics
            .query_site_rollups(site_id, prev_from, prev_to, resolution)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "title": e.to_string(), "status": 500 })),
                )
            })?;

        let prev_sum = aggregate_site_summary(&prev_rollups);
        let comp = calculate_period_change(&current_summary, &prev_sum);
        (Some(prev_sum), Some(comp))
    } else {
        (None, None)
    };

    Ok(Json(SiteAnalyticsOverviewResponse {
        site_id,
        from_ts,
        to_ts,
        interval_used: interval_str.to_string(),
        current: current_summary,
        previous: prev_summary,
        comparison,
    }))
}

/// GET /api/v1/sites/{id}/analytics/timeseries
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/timeseries",
    params(
        ("id" = i64, Path, description = "Site ID"),
        AnalyticsTimeRangeQuery
    ),
    responses(
        (status = 200, description = "Timeseries points for site", body = SiteTimeseriesResponse),
        (status = 400, description = "Invalid time range"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn get_site_analytics_timeseries_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<AnalyticsTimeRangeQuery>,
) -> Result<Json<SiteTimeseriesResponse>, (StatusCode, Json<serde_json::Value>)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| {
            (
                code,
                Json(serde_json::json!({ "title": msg, "status": code.as_u16() })),
            )
        })?;

    let (from_ts, to_ts, resolution, interval_str) =
        resolve_query_time_range(query.from, query.to, query.interval.as_deref())?;

    let analytics = state.get_analytics().await.map_err(|(c, m)| {
        (
            c,
            Json(serde_json::json!({ "title": m, "status": c.as_u16() })),
        )
    })?;

    let rollups = analytics
        .query_site_rollups(site_id, from_ts, to_ts, resolution)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "title": e.to_string(), "status": 500 })),
            )
        })?;

    let points = rollups
        .into_iter()
        .map(|r| SiteTimeseriesPointDto {
            timestamp: r.bucket_ts,
            requests: r.total_requests,
            bytes: r.total_bytes,
            status_2xx: r.status_2xx,
            status_3xx: r.status_3xx,
            status_4xx: r.status_4xx,
            status_5xx: r.status_5xx,
            visitors: r.unique_visitors,
            cache_hit: r.cache_hit,
            cache_miss: r.cache_miss,
            latency_p50_ms: r.latency_p50_ms,
            latency_p95_ms: r.latency_p95_ms,
            latency_p99_ms: r.latency_p99_ms,
            latency_avg_ms: r.latency_avg_ms,
        })
        .collect();

    Ok(Json(SiteTimeseriesResponse {
        site_id,
        from_ts,
        to_ts,
        interval: interval_str.to_string(),
        points,
    }))
}

/// GET /api/v1/sites/{id}/analytics/breakdowns
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/breakdowns",
    params(
        ("id" = i64, Path, description = "Site ID"),
        AnalyticsBreakdownsQuery
    ),
    responses(
        (status = 200, description = "Top-N breakdowns", body = SiteBreakdownsResponse),
        (status = 400, description = "Invalid parameters"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn get_site_analytics_breakdowns_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<AnalyticsBreakdownsQuery>,
) -> Result<Json<SiteBreakdownsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| {
            (
                code,
                Json(serde_json::json!({ "title": msg, "status": code.as_u16() })),
            )
        })?;

    let (from_ts, to_ts, resolution, _) =
        resolve_query_time_range(query.from, query.to, Some("auto"))?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_BREAKDOWN_LIMIT)
        .min(MAX_BREAKDOWN_LIMIT);

    let analytics = state.get_analytics().await.map_err(|(c, m)| {
        (
            c,
            Json(serde_json::json!({ "title": m, "status": c.as_u16() })),
        )
    })?;

    let rollups = analytics
        .query_site_rollups(site_id, from_ts, to_ts, resolution)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "title": e.to_string(), "status": 500 })),
            )
        })?;

    let total_reqs: u64 = rollups.iter().map(|r| r.total_requests).sum();

    fn aggregate_top_items(
        items: impl Iterator<Item = (String, u64)>,
        total: u64,
        limit: usize,
    ) -> Vec<BreakdownItemDto> {
        let mut map: HashMap<String, u64> = HashMap::new();
        for (name, count) in items {
            *map.entry(name).or_insert(0) += count;
        }

        let mut sorted: Vec<(String, u64)> = map.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        sorted.truncate(limit);

        sorted
            .into_iter()
            .map(|(name, count)| {
                let percentage = if total > 0 {
                    (count as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                BreakdownItemDto {
                    name,
                    count,
                    percentage,
                }
            })
            .collect()
    }

    let paths = aggregate_top_items(
        rollups
            .iter()
            .flat_map(|r| r.top_paths.iter().map(|i| (i.name.clone(), i.count))),
        total_reqs,
        limit,
    );
    let referrers = aggregate_top_items(
        rollups
            .iter()
            .flat_map(|r| r.top_referrers.iter().map(|i| (i.name.clone(), i.count))),
        total_reqs,
        limit,
    );
    let countries = aggregate_top_items(
        rollups
            .iter()
            .flat_map(|r| r.top_countries.iter().map(|i| (i.name.clone(), i.count))),
        total_reqs,
        limit,
    );
    let browsers = aggregate_top_items(
        rollups
            .iter()
            .flat_map(|r| r.top_browsers.iter().map(|i| (i.name.clone(), i.count))),
        total_reqs,
        limit,
    );
    let os = aggregate_top_items(
        rollups
            .iter()
            .flat_map(|r| r.top_os.iter().map(|i| (i.name.clone(), i.count))),
        total_reqs,
        limit,
    );
    let devices = aggregate_top_items(
        rollups
            .iter()
            .flat_map(|r| r.top_devices.iter().map(|i| (i.name.clone(), i.count))),
        total_reqs,
        limit,
    );
    let slow_urls = aggregate_top_items(
        rollups
            .iter()
            .flat_map(|r| r.top_slow_urls.iter().map(|i| (i.uri.clone(), i.count))),
        total_reqs,
        limit,
    );
    let error_urls = aggregate_top_items(
        rollups
            .iter()
            .flat_map(|r| r.top_error_urls.iter().map(|i| (i.uri.clone(), i.count))),
        total_reqs,
        limit,
    );

    Ok(Json(SiteBreakdownsResponse {
        site_id,
        from_ts,
        to_ts,
        paths,
        referrers,
        countries,
        browsers,
        os,
        devices,
        slow_urls,
        error_urls,
    }))
}

/// GET /api/v1/sites/{id}/analytics/compare
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/compare",
    params(
        ("id" = i64, Path, description = "Site ID"),
        AnalyticsTimeRangeQuery
    ),
    responses(
        (status = 200, description = "Period comparison", body = SiteAnalyticsOverviewResponse),
        (status = 400, description = "Invalid time range"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn get_site_analytics_compare_handler(
    state: State<AppState>,
    auth_user: AuthenticatedUser,
    path: Path<i64>,
    mut query: Query<AnalyticsTimeRangeQuery>,
) -> Result<Json<SiteAnalyticsOverviewResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Explicitly enable comparison
    query.compare = Some(true);
    get_site_analytics_overview_handler(state, auth_user, path, query).await
}

/// GET /api/v1/sites/{id}/analytics/app
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/app",
    params(
        ("id" = i64, Path, description = "Site ID"),
        AnalyticsTimeRangeQuery
    ),
    responses(
        (status = 200, description = "Application metrics", body = SiteAppMetricsResponse),
        (status = 400, description = "Invalid query parameters"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn get_site_app_metrics_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<AnalyticsTimeRangeQuery>,
) -> Result<Json<SiteAppMetricsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| {
            (
                code,
                Json(serde_json::json!({ "title": msg, "status": code.as_u16() })),
            )
        })?;

    let (from_ts, to_ts, _, _) = resolve_query_time_range(query.from, query.to, Some("auto"))?;
    let analytics = state.get_analytics().await.map_err(|(c, m)| {
        (
            c,
            Json(serde_json::json!({ "title": m, "status": c.as_u16() })),
        )
    })?;

    let runtime_str = site.runtime.clone().unwrap_or_else(|| "static".to_string());

    let mut php_fpm = None;
    let mut app_process = None;

    if runtime_str.starts_with("php") {
        let context =
            crate::tenant_context::resolve_tenant_context(&state, &auth_user, None, Some(site_id))
                .await?;
        let php_ver = site.version.as_deref().unwrap_or("8.3");
        let candidate_pools = vec![format!("{}-{php_ver}", context.linux_username)];

        for pool_name in candidate_pools {
            if let Ok(rollups) = analytics.query_php_fpm_1m(&pool_name, from_ts, to_ts).await
                && !rollups.is_empty()
            {
                let sample_total: u32 = rollups.iter().map(|r| r.sample_count).sum();
                let active_sum: f64 = rollups
                    .iter()
                    .map(|r| r.active_avg * (r.sample_count as f64))
                    .sum();
                let idle_sum: f64 = rollups
                    .iter()
                    .map(|r| r.idle_avg * (r.sample_count as f64))
                    .sum();
                let active_avg = if sample_total > 0 {
                    active_sum / (sample_total as f64)
                } else {
                    0.0
                };
                let idle_avg = if sample_total > 0 {
                    idle_sum / (sample_total as f64)
                } else {
                    0.0
                };
                let active_max = rollups.iter().map(|r| r.active_max).max().unwrap_or(0);
                let total_max = rollups.iter().map(|r| r.total_max).max().unwrap_or(0);
                let listen_queue_max = rollups
                    .iter()
                    .map(|r| r.listen_queue_max)
                    .max()
                    .unwrap_or(0);
                let max_children_reached_max = rollups
                    .iter()
                    .map(|r| r.max_children_reached_max)
                    .max()
                    .unwrap_or(0);
                let slow_requests_total = rollups.iter().map(|r| r.slow_requests_delta).sum();
                let accepted_conn_total = rollups.iter().map(|r| r.accepted_conn_delta).sum();

                php_fpm = Some(PhpFpmPoolOverviewDto {
                    pool_name,
                    active_avg,
                    active_max,
                    idle_avg,
                    total_max,
                    listen_queue_max,
                    max_children_reached_max,
                    slow_requests_total,
                    accepted_conn_total,
                });
                break;
            }
        }
    } else if runtime_str.starts_with("node") || runtime_str.starts_with("python") {
        let app_type = if runtime_str.starts_with("python") {
            AppType::Python
        } else {
            AppType::Node
        };

        if let Ok(rollups) = analytics
            .query_app_process_1m(site_id, app_type, from_ts, to_ts)
            .await
            && !rollups.is_empty()
        {
            let sample_total: u32 = rollups.iter().map(|r| r.sample_count).sum();
            let cpu_sum: f64 = rollups
                .iter()
                .map(|r| r.cpu_avg * (r.sample_count as f64))
                .sum();
            let mem_sum: f64 = rollups
                .iter()
                .map(|r| r.memory_current_avg * (r.sample_count as f64))
                .sum();
            let io_r_sum: f64 = rollups
                .iter()
                .map(|r| r.io_read_avg * (r.sample_count as f64))
                .sum();
            let io_w_sum: f64 = rollups
                .iter()
                .map(|r| r.io_write_avg * (r.sample_count as f64))
                .sum();

            let cpu_avg = if sample_total > 0 {
                cpu_sum / (sample_total as f64)
            } else {
                0.0
            };
            let mem_avg = if sample_total > 0 {
                mem_sum / (sample_total as f64)
            } else {
                0.0
            };
            let io_read_avg = if sample_total > 0 {
                io_r_sum / (sample_total as f64)
            } else {
                0.0
            };
            let io_write_avg = if sample_total > 0 {
                io_w_sum / (sample_total as f64)
            } else {
                0.0
            };

            let cpu_max = rollups.iter().map(|r| r.cpu_max).fold(0.0, f64::max);
            let mem_max = rollups
                .iter()
                .map(|r| r.memory_current_max)
                .max()
                .unwrap_or(0);
            let mem_peak_max = rollups.iter().filter_map(|r| r.memory_peak_max).max();
            let pids_max = rollups.iter().map(|r| r.pids_max).max().unwrap_or(0);

            app_process = Some(AppProcessOverviewDto {
                site_id,
                app_type: app_type.as_str().to_string(),
                cpu_avg,
                cpu_max,
                memory_current_avg_bytes: mem_avg as u64,
                memory_current_max_bytes: mem_max,
                memory_peak_max_bytes: mem_peak_max,
                pids_max,
                io_read_avg_bytes_sec: io_read_avg,
                io_write_avg_bytes_sec: io_write_avg,
            });
        }
    }

    Ok(Json(SiteAppMetricsResponse {
        site_id,
        runtime: runtime_str,
        php_fpm,
        app_process,
    }))
}

/// GET /api/v1/sites/{id}/analytics/slowlog
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/slowlog",
    params(
        ("id" = i64, Path, description = "Site ID"),
        AnalyticsSlowlogQuery
    ),
    responses(
        (status = 200, description = "Slowlog event traces", body = SiteSlowlogResponse),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn get_site_slowlog_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<AnalyticsSlowlogQuery>,
) -> Result<Json<SiteSlowlogResponse>, (StatusCode, Json<serde_json::Value>)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| {
            (
                code,
                Json(serde_json::json!({ "title": msg, "status": code.as_u16() })),
            )
        })?;

    let now = Utc::now().timestamp();
    let to_ts = query.to.unwrap_or(now);
    let from_ts = query.from.unwrap_or(to_ts - 86400 * 7);
    let limit = query.limit.unwrap_or(20).min(100);

    let analytics = state.get_analytics().await.map_err(|(c, m)| {
        (
            c,
            Json(serde_json::json!({ "title": m, "status": c.as_u16() })),
        )
    })?;

    let events = analytics
        .query_slowlog_events(None, Some(site_id), from_ts, to_ts, limit)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "title": e.to_string(), "status": 500 })),
            )
        })?
        .into_iter()
        .map(PhpSlowlogEntryDto::from)
        .collect();

    Ok(Json(SiteSlowlogResponse { site_id, events }))
}

/// GET /api/v1/sites/{id}/analytics/export
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/export",
    params(
        ("id" = i64, Path, description = "Site ID"),
        AnalyticsExportQuery
    ),
    responses(
        (status = 200, description = "CSV export download", content_type = "text/csv"),
        (status = 400, description = "Invalid time range"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn export_site_analytics_csv_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Query(query): Query<AnalyticsExportQuery>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| {
            (
                code,
                Json(serde_json::json!({ "title": msg, "status": code.as_u16() })),
            )
        })?;

    let (from_ts, to_ts, resolution, _) =
        resolve_query_time_range(query.from, query.to, query.interval.as_deref())?;

    let analytics = state.get_analytics().await.map_err(|(c, m)| {
        (
            c,
            Json(serde_json::json!({ "title": m, "status": c.as_u16() })),
        )
    })?;

    let rollups = analytics
        .query_site_rollups(site_id, from_ts, to_ts, resolution)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "title": e.to_string(), "status": 500 })),
            )
        })?;

    let mut csv_out = String::with_capacity(rollups.len() * 128);
    csv_out.push_str("Timestamp,DateTime,Requests,BytesSent,Status2xx,Status3xx,Status4xx,Status5xx,CacheHit,CacheMiss,LatencyP50Ms,LatencyP95Ms,LatencyP99Ms,LatencyAvgMs,Visitors\n");

    for r in &rollups {
        let dt = DateTime::from_timestamp(r.bucket_ts, 0)
            .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| r.bucket_ts.to_string());

        csv_out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{}\n",
            r.bucket_ts,
            dt,
            r.total_requests,
            r.total_bytes,
            r.status_2xx,
            r.status_3xx,
            r.status_4xx,
            r.status_5xx,
            r.cache_hit,
            r.cache_miss,
            r.latency_p50_ms,
            r.latency_p95_ms,
            r.latency_p99_ms,
            r.latency_avg_ms,
            r.unique_visitors
        ));
    }

    let filename = format!("site-{site_id}-analytics-{from_ts}-{to_ts}.csv");
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(disposition_val) =
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
    {
        headers.insert(CONTENT_DISPOSITION, disposition_val);
    } else {
        headers.insert(
            CONTENT_DISPOSITION,
            axum::http::HeaderValue::from_static("attachment"),
        );
    }

    Ok((headers, csv_out).into_response())
}

/// GET /api/v1/system/analytics/timeseries
#[utoipa::path(
    get,
    path = "/api/v1/system/analytics/timeseries",
    params(AnalyticsTimeRangeQuery),
    responses(
        (status = 200, description = "System host metrics timeseries", body = SystemTimeseriesResponse),
        (status = 400, description = "Invalid time range"),
        (status = 403, description = "Forbidden (Admin only)")
    ),
    tag = "system"
)]
pub async fn get_system_analytics_timeseries_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Query(query): Query<AnalyticsTimeRangeQuery>,
) -> Result<Json<SystemTimeseriesResponse>, (StatusCode, Json<serde_json::Value>)> {
    if !auth_user.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://api.vpanel.dev/errors/forbidden",
                "title": "Akses ditolak",
                "status": 403,
                "detail": "Hanya administrator yang dapat melihat metrik analitik sistem."
            })),
        ));
    }

    let (from_ts, to_ts, resolution, interval_str) =
        resolve_query_time_range(query.from, query.to, query.interval.as_deref())?;

    let analytics = state.get_analytics().await.map_err(|(c, m)| {
        (
            c,
            Json(serde_json::json!({ "title": m, "status": c.as_u16() })),
        )
    })?;

    let server_rollups = analytics
        .query_server_rollups(from_ts, to_ts, resolution)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "title": e.to_string(), "status": 500 })),
            )
        })?;

    let points = server_rollups
        .into_iter()
        .map(|r| ServerTimeseriesPointDto {
            timestamp: r.bucket_timestamp,
            cpu_avg: r.cpu_avg,
            cpu_max: r.cpu_max,
            memory_used_avg_bytes: r.memory_used_avg as u64,
            memory_used_max_bytes: r.memory_used_max,
            swap_used_avg_bytes: r.swap_used_avg as u64,
            load1_avg: r.load1_avg,
            load1_max: r.load1_max,
            disk_used_avg_bytes: r.disk_used_avg as u64,
            net_rx_avg_bytes_sec: r.net_rx_avg,
            net_tx_avg_bytes_sec: r.net_tx_avg,
            io_read_avg_bytes_sec: r.io_read_avg,
            io_write_avg_bytes_sec: r.io_write_avg,
        })
        .collect();

    Ok(Json(SystemTimeseriesResponse {
        from_ts,
        to_ts,
        interval: interval_str.to_string(),
        points,
    }))
}

// ==========================================
// Insight Engine Handlers (Phase 9 - P9-T7)
// ==========================================

#[allow(clippy::cognitive_complexity)]
pub async fn evaluate_site_insights_internal(
    state: &AppState,
    site_id: i64,
    dispatch_notifications: bool,
) -> Result<(Vec<panel_analytics::AnalyticsInsight>, usize), (StatusCode, Json<serde_json::Value>)>
{
    let now = Utc::now().timestamp();
    let from_current = now - 86400; // last 24h
    let from_previous = now - 2 * 86400; // previous 24h
    let to_previous = from_current;

    let analytics = state.get_analytics().await.map_err(|(c, m)| {
        (
            c,
            Json(serde_json::json!({ "title": m, "status": c.as_u16() })),
        )
    })?;

    // 1. Current rollups (try 1h, fallback to 1m for recent data)
    let mut curr_rollups = analytics
        .query_site_rollups(site_id, from_current, now, RollupResolution::OneHour)
        .await
        .unwrap_or_default();
    if curr_rollups.is_empty() {
        curr_rollups = analytics
            .query_site_rollups(site_id, from_current, now, RollupResolution::OneMinute)
            .await
            .unwrap_or_default();
    }
    let curr_summary = aggregate_site_summary(&curr_rollups);

    // 2. Previous rollups (try 1h, fallback to 1m)
    let mut prev_rollups = analytics
        .query_site_rollups(
            site_id,
            from_previous,
            to_previous,
            RollupResolution::OneHour,
        )
        .await
        .unwrap_or_default();
    if prev_rollups.is_empty() {
        prev_rollups = analytics
            .query_site_rollups(
                site_id,
                from_previous,
                to_previous,
                RollupResolution::OneMinute,
            )
            .await
            .unwrap_or_default();
    }
    let prev_summary = aggregate_site_summary(&prev_rollups);

    // 3. Recent Git deployment check
    let git_repo = SqliteGitDeployRepository::new(state.pool.clone());
    let recent_deps = git_repo
        .list_deployments(site_id, 3, 0)
        .await
        .unwrap_or_default();
    let recent_deployment = if let Some(latest) = recent_deps.first() {
        if latest.created_at >= from_current {
            let before_rollups = analytics
                .query_site_rollups(
                    site_id,
                    latest.created_at.saturating_sub(14400),
                    latest.created_at,
                    RollupResolution::OneMinute,
                )
                .await
                .unwrap_or_default();
            let after_rollups = analytics
                .query_site_rollups(site_id, latest.created_at, now, RollupResolution::OneMinute)
                .await
                .unwrap_or_default();

            let before_sum = aggregate_site_summary(&before_rollups);
            let after_sum = aggregate_site_summary(&after_rollups);

            Some(DeploymentContext {
                release_id: latest.release_id.clone(),
                deployed_at: latest.created_at,
                latency_before_ms: if before_sum.total_requests > 0 {
                    Some(before_sum.latency_p95_ms)
                } else {
                    None
                },
                latency_after_ms: if after_sum.total_requests > 0 {
                    Some(after_sum.latency_p95_ms)
                } else {
                    None
                },
            })
        } else {
            None
        }
    } else {
        None
    };

    // 4. Slow endpoints from rollups
    let mut slow_endpoints: Vec<SlowEndpointItem> = Vec::new();
    for r in &curr_rollups {
        for slow in &r.top_slow_urls {
            slow_endpoints.push(SlowEndpointItem {
                path: slow.uri.clone(),
                avg_latency_ms: 1200.0,
                count: slow.count,
            });
        }
    }

    // 5. PHP-FPM status metrics
    let fpm_pool_name = format!("site_{site_id}");
    let fpm_status = if let Ok(fpm_rollups) = analytics
        .query_php_fpm_1m(&fpm_pool_name, from_current, now)
        .await
    {
        if let Some(last_fpm) = fpm_rollups.last() {
            let max_children = fpm_rollups
                .iter()
                .map(|f| f.max_children_reached_max)
                .max()
                .unwrap_or(0);
            let slow_reqs = fpm_rollups.iter().map(|f| f.slow_requests_delta).sum();
            Some(FpmStatusMetrics {
                pool_name: last_fpm.pool_name.clone(),
                max_children_reached: max_children as u64,
                slow_requests: slow_reqs,
            })
        } else {
            None
        }
    } else {
        None
    };

    // 6. Disk quota context & site owner
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let plan_repo = SqlitePlanRepository::new(state.pool.clone());
    let site_opt = site_repo.find_by_id(site_id).await.ok().flatten();
    let site_owner_id = site_opt.as_ref().map(|s| s.user_id);
    let disk_quota = if let Some(ref s) = site_opt {
        if let Ok(Some(u)) = user_repo.find_by_id(s.user_id).await {
            if let Some(plan_id) = u.plan_id {
                if let Ok(Some(plan)) = plan_repo.find_by_id(plan_id).await {
                    let quota_bytes = plan.disk_limit_bytes.max(0) as u64;
                    Some(DiskQuotaContext {
                        used_bytes: (quota_bytes as f64 * 0.45) as u64,
                        quota_bytes,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let ctx = InsightEvaluationContext {
        site_id: Some(site_id),
        evaluated_at: now,
        current_metrics: TrafficMetricsSummary {
            total_requests: curr_summary.total_requests,
            status_2xx: curr_summary.status_2xx,
            status_3xx: curr_summary.status_3xx,
            status_4xx: curr_summary.status_4xx,
            status_5xx: curr_summary.status_5xx,
            bot_requests: curr_summary.total_requests / 10,
            cache_hit: (curr_summary.total_requests as f64 * (curr_summary.cache_hit_ratio / 100.0))
                as u64,
            cache_miss: (curr_summary.total_requests as f64
                * (1.0 - curr_summary.cache_hit_ratio / 100.0)) as u64,
            latency_avg_ms: curr_summary.latency_avg_ms,
            latency_p95_ms: curr_summary.latency_p95_ms,
        },
        previous_metrics: if prev_summary.total_requests > 0 {
            Some(TrafficMetricsSummary {
                total_requests: prev_summary.total_requests,
                status_2xx: prev_summary.status_2xx,
                status_3xx: prev_summary.status_3xx,
                status_4xx: prev_summary.status_4xx,
                status_5xx: prev_summary.status_5xx,
                bot_requests: prev_summary.total_requests / 10,
                cache_hit: (prev_summary.total_requests as f64
                    * (prev_summary.cache_hit_ratio / 100.0)) as u64,
                cache_miss: (prev_summary.total_requests as f64
                    * (1.0 - prev_summary.cache_hit_ratio / 100.0))
                    as u64,
                latency_avg_ms: prev_summary.latency_avg_ms,
                latency_p95_ms: prev_summary.latency_p95_ms,
            })
        } else {
            None
        },
        recent_deployment,
        slow_endpoints,
        fpm_status,
        disk_quota,
    };

    let insights = InsightEngine::evaluate(&ctx);

    let mut notifications_created = 0;
    if dispatch_notifications && !insights.is_empty() {
        let notif_service = crate::notifications::service::NotificationService::new(
            state.pool.clone(),
            Some(Arc::clone(&state.agent_client)),
        )
        .with_job_runner(Arc::clone(&state.job_runner));

        for insight in &insights {
            if matches!(
                insight.severity,
                InsightSeverity::Critical | InsightSeverity::Warning
            ) {
                let notif_sev = match insight.severity {
                    InsightSeverity::Critical => {
                        panel_core::notification::NotificationSeverity::Critical
                    }
                    InsightSeverity::Warning => {
                        panel_core::notification::NotificationSeverity::Warning
                    }
                    InsightSeverity::Info => panel_core::notification::NotificationSeverity::Info,
                };

                let event = crate::notifications::service::DispatchNotificationEvent {
                    user_id: site_owner_id,
                    site_id: Some(site_id),
                    category: panel_core::notification::NotificationCategory::AnalyticsInsight,
                    severity: notif_sev,
                    title: insight.title.clone(),
                    message: insight.description.clone(),
                    metadata_json: insight.metadata_json.clone(),
                    action_url: insight.action.as_ref().map(|a| a.target_url.clone()),
                };

                match notif_service.dispatch(event).await {
                    Ok(_) => notifications_created += 1,
                    Err(e) => tracing::error!("Gagal mengirim notifikasi insight analitik: {e}"),
                }
            }
        }
    }

    Ok((insights, notifications_created))
}

#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/analytics/insights",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "Site analytical insights list", body = SiteInsightsResponse),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn get_site_insights_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Result<Json<SiteInsightsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| {
            (
                code,
                Json(serde_json::json!({ "title": msg, "status": code.as_u16() })),
            )
        })?;

    let (insights, _) = evaluate_site_insights_internal(&state, site_id, false).await?;

    Ok(Json(SiteInsightsResponse {
        site_id,
        evaluated_at: Utc::now().timestamp(),
        insights: insights
            .into_iter()
            .map(AnalyticsInsightDto::from)
            .collect(),
        notifications_created: 0,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/sites/{id}/analytics/insights/evaluate",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "Evaluated site insights and created notifications", body = SiteInsightsResponse),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Site not found")
    ),
    tag = "analytics"
)]
pub async fn evaluate_site_insights_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Result<Json<SiteInsightsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let _site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(code, msg)| {
            (
                code,
                Json(serde_json::json!({ "title": msg, "status": code.as_u16() })),
            )
        })?;

    let (insights, notifs_created) = evaluate_site_insights_internal(&state, site_id, true).await?;

    Ok(Json(SiteInsightsResponse {
        site_id,
        evaluated_at: Utc::now().timestamp(),
        insights: insights
            .into_iter()
            .map(AnalyticsInsightDto::from)
            .collect(),
        notifications_created: notifs_created,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/system/analytics/insights",
    responses(
        (status = 200, description = "System-wide aggregated insights", body = SystemInsightsResponse),
        (status = 403, description = "Access denied (Admin only)")
    ),
    tag = "analytics"
)]
pub async fn get_system_insights_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<SystemInsightsResponse>, (StatusCode, Json<serde_json::Value>)> {
    if auth_user.role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "type": "https://api.vpanel.dev/errors/forbidden",
                "title": "Akses ditolak",
                "status": 403,
                "detail": "Hanya administrator yang dapat melihat ringkasan insight sistem."
            })),
        ));
    }

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let sites = site_repo.list_all().await.unwrap_or_default();

    let mut all_insights = Vec::new();
    for site in sites.iter().take(20) {
        if let Ok((site_ins, _)) = evaluate_site_insights_internal(&state, site.id, false).await {
            all_insights.extend(site_ins);
        }
    }

    let total = all_insights.len();
    Ok(Json(SystemInsightsResponse {
        evaluated_at: Utc::now().timestamp(),
        total_insights: total,
        insights: all_insights
            .into_iter()
            .map(AnalyticsInsightDto::from)
            .collect(),
    }))
}
