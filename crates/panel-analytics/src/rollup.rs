//! Site traffic rollup processor for aggregating 1m -> 1h -> 1d intervals.

use crate::histogram::{BUCKET_BOUNDS_MS, BUCKET_LOWERS_MS, LATENCY_BUCKET_COUNT};
use crate::models::{ErrorUrlItem, SiteMetricsPoint, SiteMetricsRollup, SlowUrlItem, TopItem};
use std::collections::HashMap;

pub struct SiteMetricsRollupProcessor;

impl SiteMetricsRollupProcessor {
    /// Aggregate multiple 1-minute `SiteMetricsPoint` instances belonging to the same hour into a `SiteMetricsRollup`.
    pub fn rollup_1m_to_1h(
        site_id: i64,
        hour_bucket_ts: i64,
        points: &[SiteMetricsPoint],
    ) -> Option<SiteMetricsRollup> {
        if points.is_empty() {
            return None;
        }

        Some(Self::combine_points(site_id, hour_bucket_ts, points))
    }

    /// Aggregate multiple 1-hour `SiteMetricsRollup` instances belonging to the same day into a 1-day `SiteMetricsRollup`.
    pub fn rollup_1h_to_1d(
        site_id: i64,
        day_bucket_ts: i64,
        rollups: &[SiteMetricsRollup],
    ) -> Option<SiteMetricsRollup> {
        if rollups.is_empty() {
            return None;
        }

        Some(Self::combine_rollups(site_id, day_bucket_ts, rollups))
    }

    fn combine_points(
        site_id: i64,
        bucket_ts: i64,
        points: &[SiteMetricsPoint],
    ) -> SiteMetricsRollup {
        let mut total_requests = 0u64;
        let mut total_bytes = 0u64;
        let mut status_2xx = 0u64;
        let mut status_3xx = 0u64;
        let mut status_4xx = 0u64;
        let mut status_5xx = 0u64;
        let mut cache_hit = 0u64;
        let mut cache_miss = 0u64;
        let mut cache_bypass = 0u64;
        let mut cache_other = 0u64;
        let mut human_requests = 0u64;
        let mut bot_requests = 0u64;
        let mut http_v1 = 0u64;
        let mut http_v2 = 0u64;
        let mut http_v3 = 0u64;
        let mut tls_v12 = 0u64;
        let mut tls_v13 = 0u64;
        let mut tls_other = 0u64;
        let mut unique_visitors = 0u64;

        let mut latency_sum_weighted = 0.0;
        let mut upstream_sum_weighted = 0.0;
        let mut upstream_count = 0u64;

        let mut merged_buckets = [0u64; LATENCY_BUCKET_COUNT];
        let mut paths_map = HashMap::<String, u64>::new();
        let mut referrers_map = HashMap::<String, u64>::new();
        let mut countries_map = HashMap::<String, u64>::new();
        let mut browsers_map = HashMap::<String, u64>::new();
        let mut os_map = HashMap::<String, u64>::new();
        let mut devices_map = HashMap::<String, u64>::new();
        let mut slow_map = HashMap::<String, (f64, u64)>::new();
        let mut error_map = HashMap::<(String, u16), u64>::new();

        for pt in points {
            total_requests += pt.total_requests;
            total_bytes += pt.total_bytes;
            status_2xx += pt.status_2xx;
            status_3xx += pt.status_3xx;
            status_4xx += pt.status_4xx;
            status_5xx += pt.status_5xx;
            cache_hit += pt.cache_hit;
            cache_miss += pt.cache_miss;
            cache_bypass += pt.cache_bypass;
            cache_other += pt.cache_other;
            human_requests += pt.human_requests;
            bot_requests += pt.bot_requests;
            http_v1 += pt.http_v1;
            http_v2 += pt.http_v2;
            http_v3 += pt.http_v3;
            tls_v12 += pt.tls_v12;
            tls_v13 += pt.tls_v13;
            tls_other += pt.tls_other;
            unique_visitors += pt.unique_visitors;

            latency_sum_weighted += pt.latency_avg_ms * (pt.total_requests as f64);
            if let Some(urt) = pt.upstream_time_avg_ms {
                upstream_sum_weighted += urt * (pt.total_requests as f64);
                upstream_count += pt.total_requests;
            }

            for (i, &count) in pt
                .latency_buckets
                .iter()
                .enumerate()
                .take(LATENCY_BUCKET_COUNT)
            {
                merged_buckets[i] += count;
            }

            for item in &pt.top_paths {
                *paths_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &pt.top_referrers {
                *referrers_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &pt.top_countries {
                *countries_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &pt.top_browsers {
                *browsers_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &pt.top_os {
                *os_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &pt.top_devices {
                *devices_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for slow in &pt.top_slow_urls {
                let entry = slow_map.entry(slow.uri.clone()).or_insert((0.0, 0));
                entry.0 += slow.avg_time_ms * (slow.count as f64);
                entry.1 += slow.count;
            }
            for err in &pt.top_error_urls {
                *error_map.entry((err.uri.clone(), err.status)).or_insert(0) += err.count;
            }
        }

        let latency_avg_ms = if total_requests > 0 {
            latency_sum_weighted / (total_requests as f64)
        } else {
            0.0
        };

        let upstream_time_avg_ms = if upstream_count > 0 {
            Some(upstream_sum_weighted / (upstream_count as f64))
        } else {
            None
        };

        let (latency_p50_ms, latency_p95_ms, latency_p99_ms) =
            Self::calculate_percentiles(&merged_buckets, total_requests);

        SiteMetricsRollup {
            site_id,
            bucket_ts,
            total_requests,
            total_bytes,
            status_2xx,
            status_3xx,
            status_4xx,
            status_5xx,
            cache_hit,
            cache_miss,
            cache_bypass,
            cache_other,
            latency_p50_ms,
            latency_p95_ms,
            latency_p99_ms,
            latency_avg_ms,
            upstream_time_avg_ms,
            unique_visitors,
            human_requests,
            bot_requests,
            http_v1,
            http_v2,
            http_v3,
            tls_v12,
            tls_v13,
            tls_other,
            top_paths: Self::map_to_top_items(paths_map, 10),
            top_referrers: Self::map_to_top_items(referrers_map, 10),
            top_countries: Self::map_to_top_items(countries_map, 10),
            top_browsers: Self::map_to_top_items(browsers_map, 10),
            top_os: Self::map_to_top_items(os_map, 10),
            top_devices: Self::map_to_top_items(devices_map, 10),
            top_slow_urls: Self::map_to_slow_items(slow_map, 10),
            top_error_urls: Self::map_to_error_items(error_map, 10),
            latency_buckets: merged_buckets.to_vec(),
            sample_count: points.len() as u32,
        }
    }

    fn combine_rollups(
        site_id: i64,
        bucket_ts: i64,
        rollups: &[SiteMetricsRollup],
    ) -> SiteMetricsRollup {
        let mut total_requests = 0u64;
        let mut total_bytes = 0u64;
        let mut status_2xx = 0u64;
        let mut status_3xx = 0u64;
        let mut status_4xx = 0u64;
        let mut status_5xx = 0u64;
        let mut cache_hit = 0u64;
        let mut cache_miss = 0u64;
        let mut cache_bypass = 0u64;
        let mut cache_other = 0u64;
        let mut human_requests = 0u64;
        let mut bot_requests = 0u64;
        let mut http_v1 = 0u64;
        let mut http_v2 = 0u64;
        let mut http_v3 = 0u64;
        let mut tls_v12 = 0u64;
        let mut tls_v13 = 0u64;
        let mut tls_other = 0u64;
        let mut unique_visitors = 0u64;
        let mut total_samples = 0u32;

        let mut latency_sum_weighted = 0.0;
        let mut upstream_sum_weighted = 0.0;
        let mut upstream_count = 0u64;

        let mut merged_buckets = [0u64; LATENCY_BUCKET_COUNT];
        let mut paths_map = HashMap::<String, u64>::new();
        let mut referrers_map = HashMap::<String, u64>::new();
        let mut countries_map = HashMap::<String, u64>::new();
        let mut browsers_map = HashMap::<String, u64>::new();
        let mut os_map = HashMap::<String, u64>::new();
        let mut devices_map = HashMap::<String, u64>::new();
        let mut slow_map = HashMap::<String, (f64, u64)>::new();
        let mut error_map = HashMap::<(String, u16), u64>::new();

        for r in rollups {
            total_requests += r.total_requests;
            total_bytes += r.total_bytes;
            status_2xx += r.status_2xx;
            status_3xx += r.status_3xx;
            status_4xx += r.status_4xx;
            status_5xx += r.status_5xx;
            cache_hit += r.cache_hit;
            cache_miss += r.cache_miss;
            cache_bypass += r.cache_bypass;
            cache_other += r.cache_other;
            human_requests += r.human_requests;
            bot_requests += r.bot_requests;
            http_v1 += r.http_v1;
            http_v2 += r.http_v2;
            http_v3 += r.http_v3;
            tls_v12 += r.tls_v12;
            tls_v13 += r.tls_v13;
            tls_other += r.tls_other;
            unique_visitors += r.unique_visitors;
            total_samples += r.sample_count;

            latency_sum_weighted += r.latency_avg_ms * (r.total_requests as f64);
            if let Some(urt) = r.upstream_time_avg_ms {
                upstream_sum_weighted += urt * (r.total_requests as f64);
                upstream_count += r.total_requests;
            }

            for (i, &count) in r
                .latency_buckets
                .iter()
                .enumerate()
                .take(LATENCY_BUCKET_COUNT)
            {
                merged_buckets[i] += count;
            }

            for item in &r.top_paths {
                *paths_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &r.top_referrers {
                *referrers_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &r.top_countries {
                *countries_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &r.top_browsers {
                *browsers_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &r.top_os {
                *os_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for item in &r.top_devices {
                *devices_map.entry(item.name.clone()).or_insert(0) += item.count;
            }
            for slow in &r.top_slow_urls {
                let entry = slow_map.entry(slow.uri.clone()).or_insert((0.0, 0));
                entry.0 += slow.avg_time_ms * (slow.count as f64);
                entry.1 += slow.count;
            }
            for err in &r.top_error_urls {
                *error_map.entry((err.uri.clone(), err.status)).or_insert(0) += err.count;
            }
        }

        let latency_avg_ms = if total_requests > 0 {
            latency_sum_weighted / (total_requests as f64)
        } else {
            0.0
        };

        let upstream_time_avg_ms = if upstream_count > 0 {
            Some(upstream_sum_weighted / (upstream_count as f64))
        } else {
            None
        };

        let (latency_p50_ms, latency_p95_ms, latency_p99_ms) =
            Self::calculate_percentiles(&merged_buckets, total_requests);

        SiteMetricsRollup {
            site_id,
            bucket_ts,
            total_requests,
            total_bytes,
            status_2xx,
            status_3xx,
            status_4xx,
            status_5xx,
            cache_hit,
            cache_miss,
            cache_bypass,
            cache_other,
            latency_p50_ms,
            latency_p95_ms,
            latency_p99_ms,
            latency_avg_ms,
            upstream_time_avg_ms,
            unique_visitors,
            human_requests,
            bot_requests,
            http_v1,
            http_v2,
            http_v3,
            tls_v12,
            tls_v13,
            tls_other,
            top_paths: Self::map_to_top_items(paths_map, 10),
            top_referrers: Self::map_to_top_items(referrers_map, 10),
            top_countries: Self::map_to_top_items(countries_map, 10),
            top_browsers: Self::map_to_top_items(browsers_map, 10),
            top_os: Self::map_to_top_items(os_map, 10),
            top_devices: Self::map_to_top_items(devices_map, 10),
            top_slow_urls: Self::map_to_slow_items(slow_map, 10),
            top_error_urls: Self::map_to_error_items(error_map, 10),
            latency_buckets: merged_buckets.to_vec(),
            sample_count: total_samples,
        }
    }

    fn calculate_percentiles(
        buckets: &[u64; LATENCY_BUCKET_COUNT],
        total_requests: u64,
    ) -> (f64, f64, f64) {
        if total_requests == 0 {
            return (0.0, 0.0, 0.0);
        }

        let p50 = Self::percentile_from_buckets(buckets, total_requests, 0.50);
        let p95 = Self::percentile_from_buckets(buckets, total_requests, 0.95);
        let p99 = Self::percentile_from_buckets(buckets, total_requests, 0.99);

        (p50, p95, p99)
    }

    fn percentile_from_buckets(
        buckets: &[u64; LATENCY_BUCKET_COUNT],
        total_requests: u64,
        p: f64,
    ) -> f64 {
        let target_rank = p * (total_requests as f64);
        let mut cumulative = 0.0;

        for i in 0..LATENCY_BUCKET_COUNT {
            let count = buckets[i] as f64;
            if count == 0.0 {
                continue;
            }

            let next_cumulative = cumulative + count;
            if next_cumulative >= target_rank {
                let lower = BUCKET_LOWERS_MS[i];
                let upper = if BUCKET_BOUNDS_MS[i].is_infinite() {
                    lower * 2.0
                } else {
                    BUCKET_BOUNDS_MS[i]
                };

                let rank_in_bucket = (target_rank - cumulative).max(0.0);
                let fraction = (rank_in_bucket / count).clamp(0.0, 1.0);
                return lower + fraction * (upper - lower);
            }
            cumulative = next_cumulative;
        }

        BUCKET_LOWERS_MS[LATENCY_BUCKET_COUNT - 1]
    }

    fn map_to_top_items(map: HashMap<String, u64>, limit: usize) -> Vec<TopItem> {
        let mut items: Vec<TopItem> = map
            .into_iter()
            .map(|(name, count)| TopItem { name, count })
            .collect();
        items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        items.truncate(limit);
        items
    }

    fn map_to_slow_items(map: HashMap<String, (f64, u64)>, limit: usize) -> Vec<SlowUrlItem> {
        let mut items: Vec<SlowUrlItem> = map
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
        items.sort_by(|a, b| {
            b.avg_time_ms
                .partial_cmp(&a.avg_time_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        items.truncate(limit);
        items
    }

    fn map_to_error_items(map: HashMap<(String, u16), u64>, limit: usize) -> Vec<ErrorUrlItem> {
        let mut items: Vec<ErrorUrlItem> = map
            .into_iter()
            .map(|((uri, status), count)| ErrorUrlItem { uri, status, count })
            .collect();
        items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.uri.cmp(&b.uri)));
        items.truncate(limit);
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rollup_1m_to_1h() {
        let pt1 = SiteMetricsPoint {
            site_id: 10,
            bucket_ts: 1000,
            total_requests: 100,
            total_bytes: 50000,
            status_2xx: 90,
            status_3xx: 5,
            status_4xx: 3,
            status_5xx: 2,
            cache_hit: 80,
            cache_miss: 20,
            cache_bypass: 0,
            cache_other: 0,
            latency_p50_ms: 20.0,
            latency_p95_ms: 80.0,
            latency_p99_ms: 150.0,
            latency_avg_ms: 30.0,
            upstream_time_avg_ms: Some(25.0),
            unique_visitors: 50,
            human_requests: 95,
            bot_requests: 5,
            http_v1: 10,
            http_v2: 90,
            http_v3: 0,
            tls_v12: 5,
            tls_v13: 95,
            tls_other: 0,
            top_paths: vec![TopItem {
                name: "/home".to_string(),
                count: 60,
            }],
            top_referrers: vec![TopItem {
                name: "google.com".to_string(),
                count: 40,
            }],
            top_countries: vec![TopItem {
                name: "ID".to_string(),
                count: 70,
            }],
            top_browsers: vec![TopItem {
                name: "Chrome".to_string(),
                count: 80,
            }],
            top_os: vec![TopItem {
                name: "Android".to_string(),
                count: 70,
            }],
            top_devices: vec![TopItem {
                name: "Mobile".to_string(),
                count: 70,
            }],
            top_slow_urls: vec![],
            top_error_urls: vec![],
            latency_buckets: vec![10, 20, 30, 20, 10, 5, 5, 0, 0, 0, 0, 0],
        };

        let pt2 = SiteMetricsPoint {
            site_id: 10,
            bucket_ts: 1060,
            total_requests: 50,
            total_bytes: 25000,
            status_2xx: 45,
            status_3xx: 2,
            status_4xx: 2,
            status_5xx: 1,
            cache_hit: 40,
            cache_miss: 10,
            cache_bypass: 0,
            cache_other: 0,
            latency_p50_ms: 15.0,
            latency_p95_ms: 60.0,
            latency_p99_ms: 100.0,
            latency_avg_ms: 24.0,
            upstream_time_avg_ms: Some(20.0),
            unique_visitors: 30,
            human_requests: 48,
            bot_requests: 2,
            http_v1: 5,
            http_v2: 45,
            http_v3: 0,
            tls_v12: 2,
            tls_v13: 48,
            tls_other: 0,
            top_paths: vec![TopItem {
                name: "/home".to_string(),
                count: 30,
            }],
            top_referrers: vec![TopItem {
                name: "google.com".to_string(),
                count: 20,
            }],
            top_countries: vec![TopItem {
                name: "ID".to_string(),
                count: 40,
            }],
            top_browsers: vec![TopItem {
                name: "Chrome".to_string(),
                count: 40,
            }],
            top_os: vec![TopItem {
                name: "Android".to_string(),
                count: 35,
            }],
            top_devices: vec![TopItem {
                name: "Mobile".to_string(),
                count: 35,
            }],
            top_slow_urls: vec![],
            top_error_urls: vec![],
            latency_buckets: vec![5, 10, 15, 10, 5, 3, 2, 0, 0, 0, 0, 0],
        };

        let rollup = SiteMetricsRollupProcessor::rollup_1m_to_1h(10, 1000, &[pt1, pt2]).unwrap();
        assert_eq!(rollup.site_id, 10);
        assert_eq!(rollup.total_requests, 150);
        assert_eq!(rollup.total_bytes, 75000);
        assert_eq!(rollup.status_2xx, 135);
        assert_eq!(rollup.sample_count, 2);
        assert_eq!(rollup.top_paths[0].name, "/home");
        assert_eq!(rollup.top_paths[0].count, 90);
        assert!((rollup.latency_avg_ms - 28.0).abs() < 1e-4);
    }
}
