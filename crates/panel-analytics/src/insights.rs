//! Automatic insight engine based on analytical metrics, application status, and host resources.
//!
//! Rule-based anomaly detection following PRD 5.6:
//! - 5xx server error spike
//! - Latency regression after deployment (correlated with git deploy markers)
//! - Bot crawler / scraper traffic spike
//! - Sudden incoming traffic surge
//! - Extremely slow URL endpoints
//! - Low cache hit ratio
//! - PHP-FPM `pm.max_children` saturation
//! - Disk storage quota threshold warning

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightType {
    FiveXxSpike,
    DeployLatencyRegression,
    BotSpike,
    TrafficSpike,
    SlowEndpoint,
    LowCacheHit,
    FpmMaxChildrenReached,
    DiskQuotaWarning,
}

impl InsightType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FiveXxSpike => "five_xx_spike",
            Self::DeployLatencyRegression => "deploy_latency_regression",
            Self::BotSpike => "bot_spike",
            Self::TrafficSpike => "traffic_spike",
            Self::SlowEndpoint => "slow_endpoint",
            Self::LowCacheHit => "low_cache_hit",
            Self::FpmMaxChildrenReached => "fpm_max_children_reached",
            Self::DiskQuotaWarning => "disk_quota_warning",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightSeverity {
    Critical,
    Warning,
    Info,
}

impl InsightSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightAction {
    pub action_type: String,
    pub label: String,
    pub target_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsInsight {
    pub id: String,
    pub site_id: Option<i64>,
    pub insight_type: InsightType,
    pub severity: InsightSeverity,
    pub title: String,
    pub description: String,
    pub action: Option<InsightAction>,
    pub detected_at: i64,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TrafficMetricsSummary {
    pub total_requests: u64,
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    pub bot_requests: u64,
    pub cache_hit: u64,
    pub cache_miss: u64,
    pub latency_avg_ms: f64,
    pub latency_p95_ms: f64,
}

#[derive(Debug, Clone)]
pub struct DeploymentContext {
    pub release_id: String,
    pub deployed_at: i64,
    pub latency_before_ms: Option<f64>,
    pub latency_after_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct SlowEndpointItem {
    pub path: String,
    pub avg_latency_ms: f64,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct FpmStatusMetrics {
    pub pool_name: String,
    pub max_children_reached: u64,
    pub slow_requests: u64,
}

#[derive(Debug, Clone)]
pub struct DiskQuotaContext {
    pub used_bytes: u64,
    pub quota_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct InsightEvaluationContext {
    pub site_id: Option<i64>,
    pub evaluated_at: i64,
    pub current_metrics: TrafficMetricsSummary,
    pub previous_metrics: Option<TrafficMetricsSummary>,
    pub recent_deployment: Option<DeploymentContext>,
    pub slow_endpoints: Vec<SlowEndpointItem>,
    pub fpm_status: Option<FpmStatusMetrics>,
    pub disk_quota: Option<DiskQuotaContext>,
}

pub struct InsightEngine;

impl InsightEngine {
    /// Evaluates rules against the supplied evaluation context and produces structured insights.
    pub fn evaluate(ctx: &InsightEvaluationContext) -> Vec<AnalyticsInsight> {
        let mut insights = Vec::new();
        let site_prefix = ctx
            .site_id
            .map(|id| format!("site_{id}_"))
            .unwrap_or_default();
        let target_base = ctx
            .site_id
            .map(|id| format!("/sites/{id}"))
            .unwrap_or_else(|| "/analytics".to_string());

        // --------------------------------------------------------------------
        // Rule 1: 5xx Server Error Spike
        // --------------------------------------------------------------------
        let cur = &ctx.current_metrics;
        if cur.status_5xx >= 5 {
            let error_rate = if cur.total_requests > 0 {
                (cur.status_5xx as f64 / cur.total_requests as f64) * 100.0
            } else {
                0.0
            };

            let prev_5xx = ctx
                .previous_metrics
                .as_ref()
                .map(|p| p.status_5xx)
                .unwrap_or(0);
            let has_spike = error_rate >= 3.0 || (prev_5xx > 0 && cur.status_5xx >= prev_5xx * 3);

            if has_spike {
                insights.push(AnalyticsInsight {
                    id: format!("{site_prefix}5xx_spike_{}", ctx.evaluated_at / 3600),
                    site_id: ctx.site_id,
                    insight_type: InsightType::FiveXxSpike,
                    severity: InsightSeverity::Critical,
                    title: "Lonjakan Server Error 5xx Terdeteksi".to_string(),
                    description: format!(
                        "Terdeteksi {} error 5xx ({:.1}% dari total {} permintaan). Pengunjung kemungkinan menemui kegagalan layanan.",
                        cur.status_5xx, error_rate, cur.total_requests
                    ),
                    action: Some(InsightAction {
                        action_type: "view_logs".to_string(),
                        label: "Periksa Log Error Situs".to_string(),
                        target_url: format!("{target_base}?tab=logs"),
                    }),
                    detected_at: ctx.evaluated_at,
                    metadata_json: Some(format!(
                        r#"{{"count":{},"error_rate_pct":{:.2}}}"#,
                        cur.status_5xx, error_rate
                    )),
                });
            }
        }

        // --------------------------------------------------------------------
        // Rule 2: Deployment Latency Regression
        // --------------------------------------------------------------------
        if let Some(dep) = &ctx.recent_deployment
            && let (Some(before), Some(after)) = (dep.latency_before_ms, dep.latency_after_ms)
        {
            // Regression: post-deploy latency jumped by >= 40% AND delta >= 80ms, OR after >= 1500ms
            let is_significant_jump =
                before > 0.0 && (after >= before * 1.4) && (after - before >= 80.0);
            let is_unacceptable_tail = after >= 1500.0 && after > before;

            if is_significant_jump || is_unacceptable_tail {
                insights.push(AnalyticsInsight {
                        id: format!("{site_prefix}deploy_latency_{}", dep.release_id),
                        site_id: ctx.site_id,
                        insight_type: InsightType::DeployLatencyRegression,
                        severity: InsightSeverity::Critical,
                        title: "Regresi Latensi Terdeteksi Pasca Deploy".to_string(),
                        description: format!(
                            "Latensi respons meningkat signifikan dari {:.0} ms menjadi {:.0} ms setelah rilis {} dideploy.",
                            before, after, dep.release_id
                        ),
                        action: Some(InsightAction {
                            action_type: "rollback_deploy".to_string(),
                            label: "Rollback ke Rilis Sebelumnya".to_string(),
                            target_url: format!("{target_base}?tab=deploy"),
                        }),
                        detected_at: ctx.evaluated_at,
                        metadata_json: Some(format!(
                            r#"{{"release_id":"{}","latency_before_ms":{:.1},"latency_after_ms":{:.1}}}"#,
                            dep.release_id, before, after
                        )),
                    });
            }
        }

        // --------------------------------------------------------------------
        // Rule 3: Bot Scraper/Crawler Spike
        // --------------------------------------------------------------------
        if cur.total_requests >= 50 && cur.bot_requests >= 20 {
            let bot_ratio = (cur.bot_requests as f64 / cur.total_requests as f64) * 100.0;
            let prev_bot = ctx
                .previous_metrics
                .as_ref()
                .map(|p| p.bot_requests)
                .unwrap_or(0);
            let bot_surge = bot_ratio >= 40.0 || (prev_bot > 0 && cur.bot_requests >= prev_bot * 3);

            if bot_surge {
                insights.push(AnalyticsInsight {
                    id: format!("{site_prefix}bot_spike_{}", ctx.evaluated_at / 3600),
                    site_id: ctx.site_id,
                    insight_type: InsightType::BotSpike,
                    severity: InsightSeverity::Warning,
                    title: "Lonjakan Trafik Bot Crawler Terdeteksi".to_string(),
                    description: format!(
                        "Permintaan bot mencapai {:.1}% ({} dari {} permintaan). Aktivitas crawling tinggi dapat menguras CPU dan bandwidth.",
                        bot_ratio, cur.bot_requests, cur.total_requests
                    ),
                    action: Some(InsightAction {
                        action_type: "bot_protection".to_string(),
                        label: "Tinjau Aturan Akses & Proteksi".to_string(),
                        target_url: format!("{target_base}?tab=rules"),
                    }),
                    detected_at: ctx.evaluated_at,
                    metadata_json: Some(format!(
                        r#"{{"bot_requests":{},"bot_ratio_pct":{:.2}}}"#,
                        cur.bot_requests, bot_ratio
                    )),
                });
            }
        }

        // --------------------------------------------------------------------
        // Rule 4: Traffic Surge
        // --------------------------------------------------------------------
        if let Some(prev) = &ctx.previous_metrics
            && prev.total_requests >= 20
            && cur.total_requests >= prev.total_requests * 3
        {
            let increase_pct = ((cur.total_requests as f64 - prev.total_requests as f64)
                / prev.total_requests as f64)
                * 100.0;
            insights.push(AnalyticsInsight {
                    id: format!("{site_prefix}traffic_spike_{}", ctx.evaluated_at / 3600),
                    site_id: ctx.site_id,
                    insight_type: InsightType::TrafficSpike,
                    severity: InsightSeverity::Info,
                    title: "Lonjakan Volume Permintaan Masuk".to_string(),
                    description: format!(
                        "Lalu lintas meningkat {:.0}% dibandingkan periode sebelumnya ({} vs {} permintaan).",
                        increase_pct, cur.total_requests, prev.total_requests
                    ),
                    action: Some(InsightAction {
                        action_type: "view_analytics".to_string(),
                        label: "Tinjau Grafik Lalu Lintas".to_string(),
                        target_url: format!("{target_base}?tab=analytics"),
                    }),
                    detected_at: ctx.evaluated_at,
                    metadata_json: Some(format!(
                        r#"{{"current_requests":{},"previous_requests":{},"increase_pct":{:.1}}}"#,
                        cur.total_requests, prev.total_requests, increase_pct
                    )),
                });
        }

        // --------------------------------------------------------------------
        // Rule 5: Extremely Slow URL Endpoint
        // --------------------------------------------------------------------
        if let Some(slowest) = ctx
            .slow_endpoints
            .iter()
            .find(|e| e.avg_latency_ms >= 1000.0 && e.count >= 10)
        {
            insights.push(AnalyticsInsight {
                id: format!("{site_prefix}slow_endpoint_{}", slowest.path.replace('/', "_")),
                site_id: ctx.site_id,
                insight_type: InsightType::SlowEndpoint,
                severity: InsightSeverity::Warning,
                title: "Endpoint Eksekusi Lambat Terdeteksi".to_string(),
                description: format!(
                    "Jalur '{}' memiliki latensi rata-rata tinggi ({:.0} ms dari {} permintaan). Pertimbangkan optimasi query atau caching.",
                    slowest.path, slowest.avg_latency_ms, slowest.count
                ),
                action: Some(InsightAction {
                    action_type: "view_slowlog".to_string(),
                    label: "Periksa Analitik & Slowlog".to_string(),
                    target_url: format!("{target_base}?tab=analytics"),
                }),
                detected_at: ctx.evaluated_at,
                metadata_json: Some(format!(
                    r#"{{"path":"{}","avg_latency_ms":{:.1},"count":{}}}"#,
                    slowest.path, slowest.avg_latency_ms, slowest.count
                )),
            });
        }

        // --------------------------------------------------------------------
        // Rule 6: Low Cache Hit Ratio
        // --------------------------------------------------------------------
        let total_cacheable = cur.cache_hit + cur.cache_miss;
        if cur.total_requests >= 100 && total_cacheable >= 50 {
            let hit_ratio = (cur.cache_hit as f64 / total_cacheable as f64) * 100.0;
            if hit_ratio < 20.0 {
                insights.push(AnalyticsInsight {
                    id: format!("{site_prefix}low_cache_hit_{}", ctx.evaluated_at / 3600),
                    site_id: ctx.site_id,
                    insight_type: InsightType::LowCacheHit,
                    severity: InsightSeverity::Info,
                    title: "Rasio Cache Hit Rendah".to_string(),
                    description: format!(
                        "Hanya {:.1}% permintaan dilayani dari cache. Backend memproses hampir seluruh permintaan secara dinamis.",
                        hit_ratio
                    ),
                    action: Some(InsightAction {
                        action_type: "configure_cache".to_string(),
                        label: "Tinjau Pengaturan Caching Nginx".to_string(),
                        target_url: format!("{target_base}?tab=rules"),
                    }),
                    detected_at: ctx.evaluated_at,
                    metadata_json: Some(format!(
                        r#"{{"cache_hit":{},"cache_miss":{},"hit_ratio_pct":{:.2}}}"#,
                        cur.cache_hit, cur.cache_miss, hit_ratio
                    )),
                });
            }
        }

        // --------------------------------------------------------------------
        // Rule 7: PHP-FPM pm.max_children Reached
        // --------------------------------------------------------------------
        if let Some(fpm) = &ctx.fpm_status
            && fpm.max_children_reached > 0
        {
            insights.push(AnalyticsInsight {
                    id: format!("{site_prefix}fpm_max_children_{}", ctx.evaluated_at / 3600),
                    site_id: ctx.site_id,
                    insight_type: InsightType::FpmMaxChildrenReached,
                    severity: InsightSeverity::Warning,
                    title: "Batas Proses PHP-FPM Tercapai (pm.max_children)".to_string(),
                    description: format!(
                        "Pool PHP-FPM '{}' mencapai batas pm.max_children sebanyak {} kali. Permintaan masuk berpotensi tertahan dalam antrean.",
                        fpm.pool_name, fpm.max_children_reached
                    ),
                    action: Some(InsightAction {
                        action_type: "configure_php".to_string(),
                        label: "Tingkatkan Worker di Pengaturan PHP".to_string(),
                        target_url: format!("{target_base}?tab=php"),
                    }),
                    detected_at: ctx.evaluated_at,
                    metadata_json: Some(format!(
                        r#"{{"pool":"{}","max_children_reached":{}}}"#,
                        fpm.pool_name, fpm.max_children_reached
                    )),
                });
        }

        // --------------------------------------------------------------------
        // Rule 8: Disk Quota Warning
        // --------------------------------------------------------------------
        if let Some(quota) = &ctx.disk_quota
            && quota.quota_bytes > 0
        {
            let usage_ratio = (quota.used_bytes as f64 / quota.quota_bytes as f64) * 100.0;
            if usage_ratio >= 85.0 {
                let severity = if usage_ratio >= 95.0 {
                    InsightSeverity::Critical
                } else {
                    InsightSeverity::Warning
                };

                insights.push(AnalyticsInsight {
                        id: format!("{site_prefix}disk_quota_{}", ctx.evaluated_at / 3600),
                        site_id: ctx.site_id,
                        insight_type: InsightType::DiskQuotaWarning,
                        severity,
                        title: "Kapasitas Penyimpanan Hampir Penuh".to_string(),
                        description: format!(
                            "Penggunaan disk telah mencapai {:.1}% ({:.1} MB dari {:.1} MB). Segera bersihkan berkas atau naikkan paket kuota.",
                            usage_ratio,
                            quota.used_bytes as f64 / (1024.0 * 1024.0),
                            quota.quota_bytes as f64 / (1024.0 * 1024.0)
                        ),
                        action: Some(InsightAction {
                            action_type: "view_storage".to_string(),
                            label: "Buka Pengelola Berkas".to_string(),
                            target_url: format!("{target_base}?tab=files"),
                        }),
                        detected_at: ctx.evaluated_at,
                        metadata_json: Some(format!(
                            r#"{{"used_bytes":{},"quota_bytes":{},"usage_pct":{:.1}}}"#,
                            quota.used_bytes, quota.quota_bytes, usage_ratio
                        )),
                    });
            }
        }

        insights
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;

    #[test]
    fn test_five_xx_spike_detection() {
        let mut ctx = InsightEvaluationContext::default();
        ctx.site_id = Some(42);
        ctx.evaluated_at = 1700000000;
        ctx.current_metrics.total_requests = 100;
        ctx.current_metrics.status_5xx = 10; // 10% rate (> 3%)

        let insights = InsightEngine::evaluate(&ctx);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].insight_type, InsightType::FiveXxSpike);
        assert_eq!(insights[0].severity, InsightSeverity::Critical);
        assert!(insights[0].description.contains("10 error 5xx"));
    }

    #[test]
    fn test_deploy_latency_regression() {
        let mut ctx = InsightEvaluationContext::default();
        ctx.site_id = Some(10);
        ctx.evaluated_at = 1700000000;
        ctx.recent_deployment = Some(DeploymentContext {
            release_id: "rel-xyz-123".to_string(),
            deployed_at: 1700000000 - 3600,
            latency_before_ms: Some(100.0),
            latency_after_ms: Some(250.0), // +150% jump, delta 150ms >= 80ms
        });

        let insights = InsightEngine::evaluate(&ctx);
        assert_eq!(insights.len(), 1);
        assert_eq!(
            insights[0].insight_type,
            InsightType::DeployLatencyRegression
        );
        assert_eq!(insights[0].severity, InsightSeverity::Critical);
        assert!(insights[0].description.contains("rel-xyz-123"));
        assert_eq!(
            insights[0].action.as_ref().unwrap().action_type,
            "rollback_deploy"
        );
    }

    #[test]
    fn test_bot_spike_and_traffic_spike() {
        let mut ctx = InsightEvaluationContext::default();
        ctx.evaluated_at = 1700000000;
        ctx.current_metrics.total_requests = 300;
        ctx.current_metrics.bot_requests = 150; // 50% bot ratio

        let mut prev = TrafficMetricsSummary::default();
        prev.total_requests = 80;
        prev.bot_requests = 20;
        ctx.previous_metrics = Some(prev);

        let insights = InsightEngine::evaluate(&ctx);
        assert!(
            insights
                .iter()
                .any(|i| i.insight_type == InsightType::BotSpike)
        );
        assert!(
            insights
                .iter()
                .any(|i| i.insight_type == InsightType::TrafficSpike)
        );
    }

    #[test]
    fn test_slow_endpoint_and_fpm_max_children() {
        let mut ctx = InsightEvaluationContext::default();
        ctx.evaluated_at = 1700000000;
        ctx.slow_endpoints.push(SlowEndpointItem {
            path: "/api/heavy-report".to_string(),
            avg_latency_ms: 2450.0,
            count: 25,
        });
        ctx.fpm_status = Some(FpmStatusMetrics {
            pool_name: "site_prod".to_string(),
            max_children_reached: 4,
            slow_requests: 12,
        });

        let insights = InsightEngine::evaluate(&ctx);
        assert!(
            insights
                .iter()
                .any(|i| i.insight_type == InsightType::SlowEndpoint)
        );
        assert!(
            insights
                .iter()
                .any(|i| i.insight_type == InsightType::FpmMaxChildrenReached)
        );
    }

    #[test]
    fn test_disk_quota_critical_warning() {
        let mut ctx = InsightEvaluationContext::default();
        ctx.evaluated_at = 1700000000;
        ctx.disk_quota = Some(DiskQuotaContext {
            used_bytes: 9_600_000_000,
            quota_bytes: 10_000_000_000, // 96%
        });

        let insights = InsightEngine::evaluate(&ctx);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].insight_type, InsightType::DiskQuotaWarning);
        assert_eq!(insights[0].severity, InsightSeverity::Critical);
    }
}
