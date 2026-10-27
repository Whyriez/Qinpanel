//! Logarithmic latency histogram for accurate percentile estimation (p50, p95, p99).
//!
//! Uses 12 logarithmic latency buckets spanning 0 ms to >10,000 ms.
//! Allows seamless merging across aggregation windows (1m -> 1h -> 1d).

use serde::{Deserialize, Serialize};

/// Number of latency buckets in the logarithmic histogram.
pub const LATENCY_BUCKET_COUNT: usize = 12;

/// Upper bounds of each bucket in milliseconds:
/// [0..5), [5..10), [10..25), [25..50), [50..100), [100..250), [250..500),
/// [500..1000), [1000..2500), [2500..5000), [5000..10000), [10000..inf)
pub const BUCKET_BOUNDS_MS: [f64; LATENCY_BUCKET_COUNT] = [
    5.0,
    10.0,
    25.0,
    50.0,
    100.0,
    250.0,
    500.0,
    1000.0,
    2500.0,
    5000.0,
    10000.0,
    f64::INFINITY,
];

/// Lower bounds of each bucket in milliseconds.
pub const BUCKET_LOWERS_MS: [f64; LATENCY_BUCKET_COUNT] = [
    0.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0,
];

/// Logarithmic bucket histogram for request latency tracking.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatencyHistogram {
    pub buckets: [u64; LATENCY_BUCKET_COUNT],
    pub count: u64,
    pub sum_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyHistogram {
    /// Create a new empty latency histogram.
    pub fn new() -> Self {
        Self {
            buckets: [0; LATENCY_BUCKET_COUNT],
            count: 0,
            sum_ms: 0.0,
            min_ms: f64::INFINITY,
            max_ms: 0.0,
        }
    }

    /// Record a latency measurement in seconds.
    pub fn record_secs(&mut self, latency_secs: f64) {
        if latency_secs < 0.0 || latency_secs.is_nan() {
            return;
        }
        self.record_ms(latency_secs * 1000.0);
    }

    /// Record a latency measurement in milliseconds.
    pub fn record_ms(&mut self, latency_ms: f64) {
        if latency_ms < 0.0 || latency_ms.is_nan() {
            return;
        }

        let idx = Self::bucket_index(latency_ms);
        self.buckets[idx] += 1;
        self.count += 1;
        self.sum_ms += latency_ms;
        if latency_ms < self.min_ms {
            self.min_ms = latency_ms;
        }
        if latency_ms > self.max_ms {
            self.max_ms = latency_ms;
        }
    }

    /// Determine bucket index for a given latency in milliseconds.
    pub fn bucket_index(ms: f64) -> usize {
        for (idx, &bound) in BUCKET_BOUNDS_MS.iter().enumerate() {
            if ms < bound {
                return idx;
            }
        }
        LATENCY_BUCKET_COUNT - 1
    }

    /// Calculate average latency in milliseconds.
    pub fn avg_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum_ms / (self.count as f64)
        }
    }

    /// Estimate percentile (e.g. 0.50 for p50, 0.95 for p95, 0.99 for p99) in milliseconds.
    pub fn percentile_ms(&self, p: f64) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        if self.count == 1 {
            return self.max_ms;
        }

        let p_clamped = p.clamp(0.0, 1.0);
        let target_rank = p_clamped * (self.count as f64);

        let mut cumulative = 0.0;
        for i in 0..LATENCY_BUCKET_COUNT {
            let bucket_count = self.buckets[i] as f64;
            if bucket_count == 0.0 {
                continue;
            }

            let next_cumulative = cumulative + bucket_count;
            if next_cumulative >= target_rank {
                let lower = BUCKET_LOWERS_MS[i];
                let upper = if BUCKET_BOUNDS_MS[i].is_infinite() {
                    self.max_ms.max(lower)
                } else {
                    BUCKET_BOUNDS_MS[i]
                };

                let rank_in_bucket = (target_rank - cumulative).max(0.0);
                let fraction = (rank_in_bucket / bucket_count).clamp(0.0, 1.0);
                return lower + fraction * (upper - lower);
            }
            cumulative = next_cumulative;
        }

        self.max_ms
    }

    /// Convenience getter for p50 latency in milliseconds.
    pub fn p50_ms(&self) -> f64 {
        self.percentile_ms(0.50)
    }

    /// Convenience getter for p95 latency in milliseconds.
    pub fn p95_ms(&self) -> f64 {
        self.percentile_ms(0.95)
    }

    /// Convenience getter for p99 latency in milliseconds.
    pub fn p99_ms(&self) -> f64 {
        self.percentile_ms(0.99)
    }

    /// Merge another latency histogram into this one.
    pub fn merge(&mut self, other: &LatencyHistogram) {
        if other.count == 0 {
            return;
        }

        for i in 0..LATENCY_BUCKET_COUNT {
            self.buckets[i] += other.buckets[i];
        }
        self.count += other.count;
        self.sum_ms += other.sum_ms;
        if other.min_ms < self.min_ms {
            self.min_ms = other.min_ms;
        }
        if other.max_ms > self.max_ms {
            self.max_ms = other.max_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram_recording_and_percentiles() {
        let mut hist = LatencyHistogram::new();
        // Record 100 requests between 20ms and 120ms
        for i in 1..=100 {
            hist.record_ms(i as f64);
        }

        assert_eq!(hist.count, 100);
        assert!((hist.avg_ms() - 50.5).abs() < 1e-6);
        assert_eq!(hist.min_ms, 1.0);
        assert_eq!(hist.max_ms, 100.0);

        let p50 = hist.p50_ms();
        let p95 = hist.p95_ms();
        let p99 = hist.p99_ms();

        // p50 should be around 50ms, p95 around 95ms, p99 around 99ms
        assert!((p50 - 50.0).abs() < 15.0, "p50 was {}", p50);
        assert!((p95 - 95.0).abs() < 15.0, "p95 was {}", p95);
        assert!((p99 - 99.0).abs() < 15.0, "p99 was {}", p99);
        assert!(p50 < p95);
        assert!(p95 <= p99);
    }

    #[test]
    fn test_histogram_merge() {
        let mut hist1 = LatencyHistogram::new();
        let mut hist2 = LatencyHistogram::new();

        hist1.record_ms(10.0);
        hist2.record_ms(20.0);

        hist1.merge(&hist2);
        assert_eq!(hist1.count, 2);
        assert!((hist1.avg_ms() - 15.0).abs() < 1e-6);
        assert_eq!(hist1.min_ms, 10.0);
        assert_eq!(hist1.max_ms, 20.0);
    }
}
