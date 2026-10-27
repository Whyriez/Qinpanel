//! In-memory 1-minute site traffic aggregator with normalization, Top-K, and bot/geo classification.

use crate::geoip::GeoIpResolver;
use crate::histogram::LatencyHistogram;
use crate::models::{ErrorUrlItem, ParsedLogEntry, SiteMetricsPoint, SlowUrlItem};
use crate::normalizer::PathNormalizer;
use crate::space_saving::SpaceSavingCounter;
use crate::ua_parser::UaParser;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// Per-site 1-minute accumulation buffer.
#[derive(Debug, Clone)]
pub struct SiteMinuteAccumulator {
    pub site_id: i64,
    pub bucket_ts: i64,
    pub total_requests: u64,
    pub total_bytes: u64,
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    pub cache_hit: u64,
    pub cache_miss: u64,
    pub cache_bypass: u64,
    pub cache_other: u64,
    pub latency_hist: LatencyHistogram,
    pub upstream_time_sum_ms: f64,
    pub upstream_time_count: u64,
    pub visitor_hashes: HashSet<u64>,
    pub human_requests: u64,
    pub bot_requests: u64,
    pub http_v1: u64,
    pub http_v2: u64,
    pub http_v3: u64,
    pub tls_v12: u64,
    pub tls_v13: u64,
    pub tls_other: u64,
    pub paths_counter: SpaceSavingCounter,
    pub referrers_counter: SpaceSavingCounter,
    pub countries_counter: SpaceSavingCounter,
    pub browsers_counter: SpaceSavingCounter,
    pub os_counter: SpaceSavingCounter,
    pub devices_counter: SpaceSavingCounter,
    pub slow_urls: HashMap<String, (f64, u64)>, // uri -> (total_ms, count)
    pub error_urls: HashMap<(String, u16), u64>, // (uri, status) -> count
}

impl SiteMinuteAccumulator {
    pub fn new(site_id: i64, bucket_ts: i64) -> Self {
        Self {
            site_id,
            bucket_ts,
            total_requests: 0,
            total_bytes: 0,
            status_2xx: 0,
            status_3xx: 0,
            status_4xx: 0,
            status_5xx: 0,
            cache_hit: 0,
            cache_miss: 0,
            cache_bypass: 0,
            cache_other: 0,
            latency_hist: LatencyHistogram::new(),
            upstream_time_sum_ms: 0.0,
            upstream_time_count: 0,
            visitor_hashes: HashSet::new(),
            human_requests: 0,
            bot_requests: 0,
            http_v1: 0,
            http_v2: 0,
            http_v3: 0,
            tls_v12: 0,
            tls_v13: 0,
            tls_other: 0,
            paths_counter: SpaceSavingCounter::new(50),
            referrers_counter: SpaceSavingCounter::new(50),
            countries_counter: SpaceSavingCounter::new(50),
            browsers_counter: SpaceSavingCounter::new(20),
            os_counter: SpaceSavingCounter::new(20),
            devices_counter: SpaceSavingCounter::new(10),
            slow_urls: HashMap::new(),
            error_urls: HashMap::new(),
        }
    }

    /// Record a single log entry into this 1-minute window.
    pub fn record(&mut self, entry: &ParsedLogEntry, salt: &str, geoip: &GeoIpResolver) {
        self.total_requests += 1;
        self.total_bytes += entry.body_bytes_sent;

        // Status code class
        match entry.status {
            200..=299 => self.status_2xx += 1,
            300..=399 => self.status_3xx += 1,
            400..=499 => self.status_4xx += 1,
            500..=599 => self.status_5xx += 1,
            _ => {}
        }

        // Cache status
        if let Some(cs) = &entry.upstream_cache_status {
            match cs.to_uppercase().as_str() {
                "HIT" => self.cache_hit += 1,
                "MISS" => self.cache_miss += 1,
                "BYPASS" => self.cache_bypass += 1,
                _ => self.cache_other += 1,
            }
        }

        // Latency
        self.latency_hist.record_secs(entry.request_time_secs);
        let req_time_ms = entry.request_time_secs * 1000.0;

        // Upstream response time
        if let Some(urt) = entry.upstream_response_time_secs {
            self.upstream_time_sum_ms += urt * 1000.0;
            self.upstream_time_count += 1;
        }

        // Unique visitor hashing (privacy preserving: IP + UA + salt)
        let v_hash = compute_visitor_hash(&entry.client_ip, entry.user_agent.as_deref(), salt);
        self.visitor_hashes.insert(v_hash);

        // User agent & bot classification
        let ua_info = UaParser::parse(entry.user_agent.as_deref());
        if ua_info.is_bot {
            self.bot_requests += 1;
        } else {
            self.human_requests += 1;
        }
        self.browsers_counter.increment(ua_info.browser);
        self.os_counter.increment(ua_info.os);
        self.devices_counter.increment(ua_info.device);

        // Protocol & TLS
        if let Some(proto) = &entry.protocol {
            let p_upper = proto.to_uppercase();
            if p_upper.contains("HTTP/2") {
                self.http_v2 += 1;
            } else if p_upper.contains("HTTP/3") {
                self.http_v3 += 1;
            } else {
                self.http_v1 += 1;
            }
        } else {
            self.http_v1 += 1;
        }

        if let Some(tls) = &entry.tls_version {
            let t_upper = tls.to_uppercase();
            if t_upper.contains("1.3") {
                self.tls_v13 += 1;
            } else if t_upper.contains("1.2") {
                self.tls_v12 += 1;
            } else {
                self.tls_other += 1;
            }
        }

        // Top normalized paths
        let norm_path = PathNormalizer::normalize_path(&entry.uri);
        self.paths_counter.increment(norm_path);

        // Top referrers
        if let Some(domain) = entry.referer.as_deref().and_then(extract_referrer_host) {
            self.referrers_counter.increment(domain);
        }

        // GeoIP country
        let country = geoip.lookup_country(&entry.client_ip);
        self.countries_counter.increment(country);

        // Slow URLs tracking (requests taking > 250ms)
        if req_time_ms > 250.0 {
            let clean_uri = clean_uri_path(&entry.uri);
            let entry_slow = self.slow_urls.entry(clean_uri).or_insert((0.0, 0));
            entry_slow.0 += req_time_ms;
            entry_slow.1 += 1;
        }

        // Error URLs tracking (4xx & 5xx)
        if entry.status >= 400 {
            let clean_uri = clean_uri_path(&entry.uri);
            *self
                .error_urls
                .entry((clean_uri, entry.status))
                .or_insert(0) += 1;
        }
    }

    /// Convert into aggregated 1-minute `SiteMetricsPoint`.
    pub fn into_metrics_point(self) -> SiteMetricsPoint {
        let latency_avg = self.latency_hist.avg_ms();
        let latency_p50 = self.latency_hist.p50_ms();
        let latency_p95 = self.latency_hist.p95_ms();
        let latency_p99 = self.latency_hist.p99_ms();
        let latency_buckets = self.latency_hist.buckets.to_vec();

        let upstream_time_avg_ms = if self.upstream_time_count > 0 {
            Some(self.upstream_time_sum_ms / (self.upstream_time_count as f64))
        } else {
            None
        };

        // Top slow URLs
        let mut slow_list: Vec<SlowUrlItem> = self
            .slow_urls
            .into_iter()
            .map(|(uri, (total_ms, count))| SlowUrlItem {
                uri,
                avg_time_ms: if count > 0 {
                    total_ms / (count as f64)
                } else {
                    0.0
                },
                count,
            })
            .collect();
        slow_list.sort_by(|a, b| {
            b.avg_time_ms
                .partial_cmp(&a.avg_time_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        slow_list.truncate(10);

        // Top error URLs
        let mut error_list: Vec<ErrorUrlItem> = self
            .error_urls
            .into_iter()
            .map(|((uri, status), count)| ErrorUrlItem { uri, status, count })
            .collect();
        error_list.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.uri.cmp(&b.uri)));
        error_list.truncate(10);

        SiteMetricsPoint {
            site_id: self.site_id,
            bucket_ts: self.bucket_ts,
            total_requests: self.total_requests,
            total_bytes: self.total_bytes,
            status_2xx: self.status_2xx,
            status_3xx: self.status_3xx,
            status_4xx: self.status_4xx,
            status_5xx: self.status_5xx,
            cache_hit: self.cache_hit,
            cache_miss: self.cache_miss,
            cache_bypass: self.cache_bypass,
            cache_other: self.cache_other,
            latency_p50_ms: latency_p50,
            latency_p95_ms: latency_p95,
            latency_p99_ms: latency_p99,
            latency_avg_ms: latency_avg,
            upstream_time_avg_ms,
            unique_visitors: self.visitor_hashes.len() as u64,
            human_requests: self.human_requests,
            bot_requests: self.bot_requests,
            http_v1: self.http_v1,
            http_v2: self.http_v2,
            http_v3: self.http_v3,
            tls_v12: self.tls_v12,
            tls_v13: self.tls_v13,
            tls_other: self.tls_other,
            top_paths: self.paths_counter.top_k(10),
            top_referrers: self.referrers_counter.top_k(10),
            top_countries: self.countries_counter.top_k(10),
            top_browsers: self.browsers_counter.top_k(10),
            top_os: self.os_counter.top_k(10),
            top_devices: self.devices_counter.top_k(10),
            top_slow_urls: slow_list,
            top_error_urls: error_list,
            latency_buckets,
        }
    }
}

/// Orchestrator for managing in-memory 1-minute aggregation windows across multiple sites.
pub struct SiteMetricsAggregator {
    windows: HashMap<(i64, i64), SiteMinuteAccumulator>,
    daily_salt: String,
    geoip: GeoIpResolver,
}

impl Default for SiteMetricsAggregator {
    fn default() -> Self {
        Self::new(GeoIpResolver::new_empty())
    }
}

impl SiteMetricsAggregator {
    /// Create new aggregator with GeoIP resolver and default daily salt.
    pub fn new(geoip: GeoIpResolver) -> Self {
        Self {
            windows: HashMap::new(),
            daily_salt: "panel-analytics-default-salt".to_string(),
            geoip,
        }
    }

    /// Set a custom daily salt for privacy-preserving visitor hashing.
    pub fn set_daily_salt(&mut self, salt: impl Into<String>) {
        self.daily_salt = salt.into();
    }

    /// Record a parsed log entry into its appropriate 1-minute bucket.
    pub fn record(&mut self, entry: &ParsedLogEntry) {
        let bucket_ts = (entry.timestamp / 60) * 60;
        let key = (entry.site_id, bucket_ts);

        let accumulator = self
            .windows
            .entry(key)
            .or_insert_with(|| SiteMinuteAccumulator::new(entry.site_id, bucket_ts));

        accumulator.record(entry, &self.daily_salt, &self.geoip);
    }

    /// Flush completed 1-minute windows (windows where `bucket_ts + 60 <= current_ts`).
    pub fn flush_completed_windows(&mut self, current_ts: i64) -> Vec<SiteMetricsPoint> {
        let completed_keys: Vec<(i64, i64)> = self
            .windows
            .keys()
            .filter(|(_, bucket_ts)| *bucket_ts + 60 <= current_ts)
            .cloned()
            .collect();

        let mut points = Vec::with_capacity(completed_keys.len());
        for key in completed_keys {
            if let Some(acc) = self.windows.remove(&key) {
                points.push(acc.into_metrics_point());
            }
        }
        points
    }

    /// Flush all currently buffered windows unconditionally (useful on shutdown or tests).
    pub fn flush_all(&mut self) -> Vec<SiteMetricsPoint> {
        let mut points = Vec::with_capacity(self.windows.len());
        for (_, acc) in self.windows.drain() {
            points.push(acc.into_metrics_point());
        }
        points
    }

    /// Access reference to the GeoIP resolver.
    pub fn geoip(&self) -> &GeoIpResolver {
        &self.geoip
    }

    /// Number of active 1-minute aggregation windows in memory.
    pub fn active_windows_count(&self) -> usize {
        self.windows.len()
    }
}

/// Compute a 64-bit privacy-preserving visitor hash using SHA-256 with daily salt.
pub fn compute_visitor_hash(ip: &str, user_agent: Option<&str>, salt: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(ip.as_bytes());
    hasher.update(b"|");
    hasher.update(user_agent.unwrap_or("").as_bytes());
    hasher.update(b"|");
    hasher.update(salt.as_bytes());
    let result = hasher.finalize();

    let mut buf = [0u8; 8];
    buf.copy_from_slice(&result[..8]);
    u64::from_le_bytes(buf)
}

/// Extract clean host from referer string (stripping protocol and path).
fn extract_referrer_host(referer: &str) -> Option<String> {
    let trimmed = referer.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }

    let without_proto = if let Some(stripped) = trimmed.strip_prefix("https://") {
        stripped
    } else if let Some(stripped) = trimmed.strip_prefix("http://") {
        stripped
    } else {
        trimmed
    };

    let host = without_proto.split('/').next()?.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

/// Clean URI path by stripping query strings.
fn clean_uri_path(uri: &str) -> String {
    uri.split('?').next().unwrap_or(uri).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visitor_hash_consistency() {
        let h1 = compute_visitor_hash("203.0.113.1", Some("Mozilla/5.0"), "salt1");
        let h2 = compute_visitor_hash("203.0.113.1", Some("Mozilla/5.0"), "salt1");
        let h3 = compute_visitor_hash("203.0.113.1", Some("Mozilla/5.0"), "salt2");

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_extract_referrer() {
        assert_eq!(
            extract_referrer_host("https://google.com/search?q=panel").as_deref(),
            Some("google.com")
        );
        assert_eq!(
            extract_referrer_host("http://blog.my-site.id:8080/page").as_deref(),
            Some("blog.my-site.id")
        );
        assert_eq!(extract_referrer_host("-"), None);
    }
}
