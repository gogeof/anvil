//! Performance optimization module for reducing latency and improving throughput.
//!
//! Target metrics:
//! - First token latency: < 1s (current ~2s)
//! - Token generation rate: > 30 tokens/s (current ~20 tokens/s)

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Performance metrics for a single operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfMetric {
    /// Operation name.
    pub operation: String,
    /// Start timestamp (Unix epoch millis).
    pub start_ms: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Additional metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Performance statistics for an operation type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerfStats {
    /// Total number of operations.
    pub count: u64,
    /// Total duration in milliseconds.
    pub total_ms: u64,
    /// Minimum duration.
    pub min_ms: u64,
    /// Maximum duration.
    pub max_ms: u64,
    /// P50 latency.
    pub p50_ms: u64,
    /// P95 latency.
    pub p95_ms: u64,
    /// P99 latency.
    pub p99_ms: u64,
}

/// Streaming metrics for first-token latency tracking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamMetrics {
    /// Time from request start to first token.
    pub first_token_ms: u64,
    /// Total tokens generated.
    pub total_tokens: u64,
    /// Tokens per second.
    pub tokens_per_second: f64,
    /// Total duration.
    pub total_ms: u64,
}

/// Performance tracker for recording and analyzing metrics.
#[derive(Debug)]
pub struct PerfTracker {
    /// Metrics storage.
    metrics: Arc<Mutex<Vec<PerfMetric>>>,
    /// Start time for timing operations.
    start: Instant,
}

impl Default for PerfTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl PerfTracker {
    /// Create a new performance tracker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(Mutex::new(Vec::new())),
            start: Instant::now(),
        }
    }

    /// Start timing an operation.
    pub fn start_operation(&self, operation: impl Into<String>) -> OperationTimer {
        OperationTimer {
            operation: operation.into(),
            start: Instant::now(),
            tracker: self.metrics.clone(),
            metadata: None,
        }
    }

    /// Record a metric directly.
    pub fn record(&self, metric: PerfMetric) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.push(metric);
        // Keep only last 10000 metrics to avoid memory bloat
        if metrics.len() > 10000 {
            metrics.remove(0);
        }
    }

    /// Get statistics for an operation.
    pub fn get_stats(&self, operation: &str) -> PerfStats {
        let metrics = self.metrics.lock().unwrap();
        let mut durations: Vec<u64> = metrics
            .iter()
            .filter(|m| m.operation == operation)
            .map(|m| m.duration_ms)
            .collect();

        if durations.is_empty() {
            return PerfStats::default();
        }

        durations.sort();

        let count = durations.len() as u64;
        let total_ms: u64 = durations.iter().sum();
        let min_ms = *durations.first().unwrap();
        let max_ms = *durations.last().unwrap();
        let p50_ms = percentile(&durations, 50);
        let p95_ms = percentile(&durations, 95);
        let p99_ms = percentile(&durations, 99);

        PerfStats {
            count,
            total_ms,
            min_ms,
            max_ms,
            p50_ms,
            p95_ms,
            p99_ms,
        }
    }

    /// Get all metrics.
    pub fn get_all_metrics(&self) -> Vec<PerfMetric> {
        let metrics = self.metrics.lock().unwrap();
        metrics.clone()
    }

    /// Clear all metrics.
    pub fn clear(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.clear();
    }

    /// Get streaming metrics summary.
    pub fn get_stream_metrics(&self) -> Option<StreamMetrics> {
        let metrics = self.metrics.lock().unwrap();

        let first_token_metrics: Vec<_> = metrics
            .iter()
            .filter(|m| m.operation == "stream_first_token")
            .collect();

        let token_metrics: Vec<_> = metrics
            .iter()
            .filter(|m| m.operation == "stream_complete")
            .collect();

        if first_token_metrics.is_empty() || token_metrics.is_empty() {
            return None;
        }

        let avg_first_token: f64 = first_token_metrics
            .iter()
            .map(|m| m.duration_ms as f64)
            .sum::<f64>()
            / first_token_metrics.len() as f64;

        let total_tokens: u64 = token_metrics
            .iter()
            .filter_map(|m| m.metadata.as_ref())
            .filter_map(|meta| meta.get("tokens").and_then(|t| t.as_u64()))
            .sum();

        let total_duration: u64 = token_metrics.iter().map(|m| m.duration_ms).sum();

        let tokens_per_second = if total_duration > 0 {
            (total_tokens as f64) / (total_duration as f64 / 1000.0)
        } else {
            0.0
        };

        Some(StreamMetrics {
            first_token_ms: avg_first_token as u64,
            total_tokens,
            tokens_per_second,
            total_ms: total_duration,
        })
    }

    /// Generate a performance report.
    pub fn report(&self) -> PerfReport {
        let metrics = self.metrics.lock().unwrap();

        let mut operations: std::collections::HashMap<String, Vec<u64>> =
            std::collections::HashMap::new();

        for metric in metrics.iter() {
            operations
                .entry(metric.operation.clone())
                .or_default()
                .push(metric.duration_ms);
        }

        let mut stats = std::collections::HashMap::new();
        for (op, durations) in operations {
            if durations.is_empty() {
                continue;
            }

            let mut sorted = durations.clone();
            sorted.sort();

            let count = sorted.len() as u64;
            let total_ms: u64 = sorted.iter().sum();
            let min_ms = *sorted.first().unwrap();
            let max_ms = *sorted.last().unwrap();

            stats.insert(
                op,
                PerfStats {
                    count,
                    total_ms,
                    min_ms,
                    max_ms,
                    p50_ms: percentile(&sorted, 50),
                    p95_ms: percentile(&sorted, 95),
                    p99_ms: percentile(&sorted, 99),
                },
            );
        }

        PerfReport {
            uptime_secs: self.start.elapsed().as_secs(),
            total_operations: metrics.len() as u64,
            stats,
        }
    }
}

/// Timer for an operation.
pub struct OperationTimer {
    operation: String,
    start: Instant,
    tracker: Arc<Mutex<Vec<PerfMetric>>>,
    metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

impl OperationTimer {
    /// Add metadata to the timer.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata
            .get_or_insert_with(serde_json::Map::new)
            .insert(key.into(), value);
        self
    }

    /// Complete the operation and record the metric.
    pub fn finish(self) {
        let duration_ms = self.start.elapsed().as_millis() as u64;
        let metric = PerfMetric {
            operation: self.operation,
            start_ms: self.start.elapsed().as_millis() as u64,
            duration_ms,
            metadata: self.metadata,
        };

        let mut metrics = self.tracker.lock().unwrap();
        metrics.push(metric);
        if metrics.len() > 10000 {
            metrics.remove(0);
        }
    }
}

/// Performance report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfReport {
    /// Uptime in seconds.
    pub uptime_secs: u64,
    /// Total number of operations.
    pub total_operations: u64,
    /// Statistics by operation.
    pub stats: std::collections::HashMap<String, PerfStats>,
}

/// Calculate percentile of a sorted slice.
fn percentile(sorted: &[u64], p: u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }

    let idx = ((p as f64 / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Global performance tracker.
static GLOBAL_TRACKER: std::sync::OnceLock<PerfTracker> = std::sync::OnceLock::new();

/// Get the global performance tracker.
pub fn global_tracker() -> &'static PerfTracker {
    GLOBAL_TRACKER.get_or_init(PerfTracker::new)
}

/// Record a metric to the global tracker.
pub fn record_metric(metric: PerfMetric) {
    global_tracker().record(metric);
}

/// Start timing an operation.
pub fn start_op(operation: impl Into<String>) -> OperationTimer {
    global_tracker().start_operation(operation)
}

/// Get performance report.
pub fn perf_report() -> PerfReport {
    global_tracker().report()
}

/// Connection pool warming utilities.
pub mod connection_warmup {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Connection pool warmer.
    #[derive(Debug)]
    pub struct ConnectionWarmer {
        warmed: Arc<AtomicBool>,
    }

    impl Default for ConnectionWarmer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ConnectionWarmer {
        /// Create a new warmer.
        #[must_use]
        pub fn new() -> Self {
            Self {
                warmed: Arc::new(AtomicBool::new(false)),
            }
        }

        /// Check if connections are warmed.
        pub fn is_warmed(&self) -> bool {
            self.warmed.load(Ordering::Relaxed)
        }

        /// Mark connections as warmed.
        pub fn mark_warmed(&self) {
            self.warmed.store(true, Ordering::Relaxed);
        }

        /// Warm up HTTP connections synchronously.
        pub fn warm_http_sync(&self, endpoint: &str) -> Result<(), String> {
            let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            rt.block_on(async {
                let client = reqwest::Client::new();
                let result = client
                    .get(endpoint)
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await;
                match result {
                    Ok(_) => {
                        self.mark_warmed();
                        Ok(())
                    }
                    Err(e) => Err(format!("Warmup failed: {}", e)),
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perf_tracker() {
        let tracker = PerfTracker::new();

        {
            let timer = tracker.start_operation("test_op");
            std::thread::sleep(std::time::Duration::from_millis(10));
            timer.finish();
        }

        let stats = tracker.get_stats("test_op");
        assert_eq!(stats.count, 1);
        assert!(stats.min_ms >= 10);
    }

    #[test]
    fn test_percentile() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        // P50 index = (50/100) * 9 = 4.5 -> round to 5 -> data[5] = 6
        assert_eq!(percentile(&data, 50), 6);
        assert_eq!(percentile(&data, 95), 10);
        assert_eq!(percentile(&data, 99), 10);
    }

    #[test]
    fn test_perf_report() {
        let tracker = PerfTracker::new();

        for _ in 0..5 {
            let timer = tracker.start_operation("api_call");
            std::thread::sleep(std::time::Duration::from_millis(5));
            timer.finish();
        }

        let report = tracker.report();
        assert!(report.total_operations >= 5);
        assert!(report.stats.contains_key("api_call"));
    }
}
