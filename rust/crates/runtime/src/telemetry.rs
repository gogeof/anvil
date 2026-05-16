//! Telemetry system for anvil metrics collection
//!
//! Records performance metrics for the four core operations:
//! - Search (grep_search, file_search)
//! - Read (read_file)
//! - Write (edit_file, write_file)
//! - Execute (bash commands)

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A single metric record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricRecord {
    /// Timestamp (UNIX epoch ms)
    pub timestamp: u64,
    /// Operation name (grep_search, edit_file, bash, etc.)
    pub operation: String,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Whether the operation succeeded
    pub success: bool,
    /// Additional context
    pub context: HashMap<String, serde_json::Value>,
}

/// Aggregated statistics for an operation
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperationStats {
    pub count: u64,
    pub success_count: u64,
    pub total_duration_ms: u64,
    pub min_duration_ms: u64,
    pub max_duration_ms: u64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
}

impl OperationStats {
    pub fn from_records(records: &[MetricRecord]) -> Self {
        if records.is_empty() {
            return Self::default();
        }

        let mut durations: Vec<u64> = records.iter().map(|r| r.duration_ms).collect();
        durations.sort_unstable();

        let count = records.len() as u64;
        let success_count = records.iter().filter(|r| r.success).count() as u64;
        let total_duration_ms = durations.iter().sum();
        let min_duration_ms = *durations.first().unwrap_or(&0);
        let max_duration_ms = *durations.last().unwrap_or(&0);

        let percentile = |p: f64| -> u64 {
            let idx = ((count as f64) * p / 100.0).floor() as usize;
            durations.get(idx.min(durations.len() - 1)).copied().unwrap_or(0)
        };

        Self {
            count,
            success_count,
            total_duration_ms,
            min_duration_ms,
            max_duration_ms,
            p50_ms: percentile(50.0),
            p95_ms: percentile(95.0),
            p99_ms: percentile(99.0),
        }
    }
}

/// Telemetry collector
#[derive(Debug)]
pub struct Telemetry {
    records: Arc<Mutex<Vec<MetricRecord>>>,
    metrics_dir: PathBuf,
}

impl Telemetry {
    /// Create a new telemetry instance
    pub fn new() -> Self {
        let metrics_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".anvil")
            .join("metrics");

        // Ensure directory exists
        let _ = fs::create_dir_all(&metrics_dir);

        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            metrics_dir,
        }
    }

    /// Get the metrics directory path
    pub fn metrics_dir(&self) -> &std::path::Path {
        &self.metrics_dir
    }

    /// Record an operation
    pub fn record(&self, operation: &str, duration: Duration, success: bool, context: HashMap<String, serde_json::Value>) {
        let record = MetricRecord {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            operation: operation.to_string(),
            duration_ms: duration.as_millis() as u64,
            success,
            context,
        };

        // Add to in-memory records
        if let Ok(mut records) = self.records.lock() {
            records.push(record.clone());
            
            // Keep only last 10000 records in memory
            if records.len() > 10000 {
                records.remove(0);
            }
        }

        // Persist to daily log file
        self.persist_record(&record);
    }

    /// Convenience method to time an operation
    pub fn time<F, T, E>(&self, operation: &str, f: F) -> Result<T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        let start = Instant::now();
        let result = f();
        let duration = start.elapsed();
        let success = result.is_ok();
        
        self.record(operation, duration, success, HashMap::new());
        
        result
    }

    /// Time with context
    pub fn time_with_context<F, T, E>(
        &self,
        operation: &str,
        context: HashMap<String, serde_json::Value>,
        f: F,
    ) -> Result<T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        let start = Instant::now();
        let result = f();
        let duration = start.elapsed();
        let success = result.is_ok();
        
        self.record(operation, duration, success, context);
        
        result
    }

    /// Persist a record to disk
    fn persist_record(&self, record: &MetricRecord) {
        let date = chrono_lite_date(record.timestamp);
        let filename = format!("metrics-{}.jsonl", date);
        let path = self.metrics_dir.join(&filename);
        
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(file, "{}", serde_json::to_string(record).unwrap_or_default());
        }
    }

    /// Load records from the last N days
    pub fn load_recent(&self, days: u64) -> Vec<MetricRecord> {
        let mut all_records = Vec::new();
        
        for i in 0..days {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
                - (i * 24 * 60 * 60 * 1000);
            
            let date = chrono_lite_date(ts);
            let filename = format!("metrics-{}.jsonl", date);
            let path = self.metrics_dir.join(&filename);
            
            if let Ok(content) = fs::read_to_string(&path) {
                for line in content.lines() {
                    if let Ok(record) = serde_json::from_str::<MetricRecord>(line) {
                        all_records.push(record);
                    }
                }
            }
        }
        
        all_records
    }

    /// Get statistics for an operation
    pub fn stats(&self, operation: &str) -> OperationStats {
        let records = self.load_recent(7);
        let op_records: Vec<_> = records.into_iter()
            .filter(|r| r.operation == operation)
            .collect();
        
        OperationStats::from_records(&op_records)
    }

    /// Generate a weekly report
    pub fn weekly_report(&self) -> String {
        let records = self.load_recent(7);
        
        // Group by operation
        let mut by_operation: HashMap<String, Vec<&MetricRecord>> = HashMap::new();
        for record in &records {
            by_operation
                .entry(record.operation.clone())
                .or_default()
                .push(record);
        }
        
        let mut report = String::new();
        report.push_str("# ANVIL Weekly Metrics Report\n\n");
        
        for op in ["grep_search", "read_file", "edit_file", "bash"] {
            if let Some(op_records) = by_operation.get(op) {
                let stats = OperationStats::from_records(&op_records.iter().cloned().cloned().collect::<Vec<_>>());
                let success_rate = if stats.count > 0 {
                    (stats.success_count as f64 / stats.count as f64 * 100.0) as u64
                } else {
                    0
                };
                
                report.push_str(&format!(
                    "## {}\n- Count: {}\n- Success Rate: {}%\n- Latency P50: {}ms\n- Latency P95: {}ms\n\n",
                    op, stats.count, success_rate, stats.p50_ms, stats.p95_ms
                ));
            }
        }
        
        report
    }
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert timestamp to YYYY-MM-DD string
fn chrono_lite_date(ts_ms: u64) -> String {
    // Simple conversion without chrono dependency
    let days_since_epoch = ts_ms / (24 * 60 * 60 * 1000);
    let _seconds = (ts_ms / 1000) % (24 * 60 * 60);
    
    // Unix epoch was 1970-01-01 (Thursday)
    // Days since epoch to year/month/day
    let (year, month, day) = days_to_ymd(days_since_epoch as i64);
    
    format!("{:04}-{:02}-{:02}", year, month, day)
}

/// Convert days since Unix epoch to (year, month, day)
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Start from 1970-01-01
    let mut year = 1970;
    let mut remaining = days;
    
    fn is_leap(y: i32) -> bool {
        (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
    }
    
   fn days_in_year(y: i32) -> i64 {
       if is_leap(y) { 366 } else { 365 }
   }
    
    while remaining >= days_in_year(year) {
        remaining -= days_in_year(year);
        year += 1;
    }
    
    let days_in_months = [
        31, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30,
        31, 31, 30, 31, 30, 31
    ];
    
    let mut month = 1;
    for &dim in &days_in_months {
        if remaining < dim as i64 {
            break;
        }
        remaining -= dim as i64;
        month += 1;
    }
    
    let day = (remaining + 1) as u32;
    
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_days_to_ymd() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        assert_eq!(days_to_ymd(1), (1970, 1, 2));
        assert_eq!(days_to_ymd(31), (1970, 2, 1));
        assert_eq!(days_to_ymd(365), (1971, 1, 1));
    }

    #[test]
    fn test_operation_stats() {
        let records = vec![
            MetricRecord {
                timestamp: 0,
                operation: "test".to_string(),
                duration_ms: 100,
                success: true,
                context: HashMap::new(),
            },
            MetricRecord {
                timestamp: 0,
                operation: "test".to_string(),
                duration_ms: 200,
                success: false,
                context: HashMap::new(),
            },
        ];
        
        let stats = OperationStats::from_records(&records);
        assert_eq!(stats.count, 2);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.min_duration_ms, 100);
        assert_eq!(stats.max_duration_ms, 200);
    }
}
