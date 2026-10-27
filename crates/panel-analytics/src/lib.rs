//! `panel-analytics`: System metrics collection, log tailing, aggregating, and deep analytics.

pub mod aggregator;
pub mod app_cgroup;
pub mod attribution;
pub mod cgroup;
pub mod collector;
pub mod disk;
pub mod engine;
pub mod error;
pub mod fpm_status;
pub mod geoip;
pub mod histogram;
pub mod insights;
pub mod log_entry;
pub mod models;
pub mod normalizer;
pub mod proc;
pub mod rollup;
pub mod security;
pub mod slowlog;
pub mod space_saving;
pub mod storage;
pub mod tailer;
pub mod ua_parser;

pub use aggregator::{SiteMetricsAggregator, SiteMinuteAccumulator, compute_visitor_hash};
pub use app_cgroup::AppProcessMetricsCollector;
pub use attribution::SiteAttributor;
pub use cgroup::CgroupMetricsCollector;
pub use collector::SystemMetricsCollector;
pub use disk::{DiskStats, DiskUsageReader};
pub use engine::LogTailerEngine;
pub use error::AnalyticsError;
pub use fpm_status::{
    FpmStatusClient, build_fcgi_status_request, convert_status_metrics, parse_fcgi_status_response,
    parse_pool_identifiers, parse_status_json,
};
pub use geoip::GeoIpResolver;
pub use histogram::{LATENCY_BUCKET_COUNT, LatencyHistogram};
pub use insights::{
    AnalyticsInsight, DeploymentContext, DiskQuotaContext, FpmStatusMetrics, InsightAction,
    InsightEngine, InsightEvaluationContext, InsightSeverity, InsightType, SlowEndpointItem,
    TrafficMetricsSummary,
};
pub use log_entry::NginxJsonLog;
pub use models::*;
pub use normalizer::PathNormalizer;
pub use proc::ProcReader;
pub use rollup::SiteMetricsRollupProcessor;
pub use security::{
    BruteForceIncident, ProbingPathStat, SecurityEvent, SecurityEventType, SecurityOverviewSummary,
    SecurityRiskLevel, TopSecurityIp, analyze_log_entry, calculate_ip_risk_level,
    classify_probing_path, is_auth_endpoint,
};
pub use slowlog::PhpSlowlogParser;
pub use space_saving::SpaceSavingCounter;
pub use storage::AnalyticsStorage;
pub use tailer::LogFileTailer;
pub use ua_parser::{UaInfo, UaParser};
