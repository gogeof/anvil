//! Background execution decision engine.
//!
//! This module provides automatic detection of whether a command should run
//! in the background based on static rules and historical execution data.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Decision for whether a command should run in background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundDecision {
    /// Always run in background (e.g., servers, watchers).
    Always,
    /// Never run in background (e.g., quick queries, interactive commands).
    Never,
    /// Run in background if historical data suggests it's slow.
    IfSlow,
    /// Ask user for confirmation (optional mode).
    AskUser,
}

impl Default for BackgroundDecision {
    fn default() -> Self {
        Self::IfSlow
    }
}

/// Configuration for background decision engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundJudgeConfig {
    /// Threshold in milliseconds for considering a command "slow".
    pub slow_threshold_ms: u64,
    /// Enable historical learning.
    pub enable_learning: bool,
    /// Minimum number of executions before learning kicks in.
    pub min_samples_for_learning: usize,
    /// Commands that should always run in background.
    pub always_background_patterns: Vec<String>,
    /// Commands that should never run in background.
    pub never_background_patterns: Vec<String>,
}

impl Default for BackgroundJudgeConfig {
    fn default() -> Self {
        Self {
            // Optimized thresholds based on real-world usage
            slow_threshold_ms: 5_000, // 5 seconds (was 30s, too high for network ops)
            enable_learning: true,
            min_samples_for_learning: 5, // Increased to reduce false positives
            always_background_patterns: vec![
                // Development servers
                r"(?i)--watch".to_string(),
                r"(?i)--serve".to_string(),
                r"(?i)npm\s+run\s+dev".to_string(),
                r"(?i)cargo\s+watch".to_string(),
                r"(?i)python\s+-m\s+http\.server".to_string(),
                r"(?i)nodemon".to_string(),
                r"(?i)webpack.*--watch".to_string(),
                r"(?i)vite".to_string(),
                r"(?i)next\s+dev".to_string(),
                r"(?i)rails\s+s".to_string(),
                r"(?i)django.*runserver".to_string(),
                // Long-running processes
                r"(?i)docker\s+run.*-d".to_string(),
                r"(?i)kubectl.*proxy".to_string(),
                // Build and test commands (often slow)
                r"(?i)cargo\s+build".to_string(),
                r"(?i)cargo\s+test".to_string(),
                r"(?i)npm\s+test".to_string(),
                r"(?i)npm\s+run\s+build".to_string(),
                r"(?i)make\s+-j".to_string(),
                r"(?i)pytest".to_string(),
                // Network operations with potential long waits
                r"(?i)curl.*--connect-timeout\s+[3-9]".to_string(),
                r"(?i)curl.*--max-time\s+[1-9][0-9]".to_string(),
                r"(?i)wget".to_string(),
                r"(?i)rsync.*-a".to_string(),
                r"(?i)scp\s+".to_string(),
                // Database operations
                r"(?i)pg_dump".to_string(),
                r"(?i)mysqldump".to_string(),
                r"(?i)mongodump".to_string(),
            ],
            never_background_patterns: vec![
                // Quick queries
                r"^ls\b".to_string(),
                r"^cat\b".to_string(),
                r"^head\b".to_string(),
                r"^tail\b".to_string(),
                r"^grep\b".to_string(),
                r"^find\b".to_string(),
                r"^echo\b".to_string(),
                r"^pwd\b".to_string(),
                r"^which\b".to_string(),
                r"^type\b".to_string(),
                r"^mkdir\b".to_string(),
                r"^rm\s+-[^r]".to_string(), // rm -f, rm -i (not rm -rf)
                r"^touch\b".to_string(),
                r"^mv\b".to_string(),
                r"^cp\b".to_string(),
                // Git queries
                r"^git\s+status\b".to_string(),
                r"^git\s+diff\b".to_string(),
                r"^git\s+log\b".to_string(),
                r"^git\s+branch\b".to_string(),
                r"^git\s+add\b".to_string(),
                r"^git\s+commit\b".to_string(),
                // Interactive commands
                r"^vim\b".to_string(),
                r"^nano\b".to_string(),
                r"^less\b".to_string(),
                r"^more\b".to_string(),
                r"^top\b".to_string(),
                r"^htop\b".to_string(),
                // Quick network checks (under 3s timeout)
                r"(?i)curl.*--connect-timeout\s+[1-2]\b".to_string(),
                r"(?i)curl.*--max-time\s+[1-2]\b".to_string(),
                r"(?i)ping\s+-c\s+[1-5]\b".to_string(),
            ],
        }
    }
}

/// Execution history record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    /// Hash of the command for quick lookup.
    pub command_hash: u64,
    /// Original command string.
    pub command: String,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Exit code.
    pub exit_code: i32,
    /// Timestamp of execution.
    pub timestamp: u64,
}

impl ExecutionRecord {
    /// Create a new execution record.
    pub fn new(command: &str, duration_ms: u64, exit_code: i32) -> Self {
        Self {
            command_hash: hash_command(command),
            command: command.to_string(),
            duration_ms,
            exit_code,
            timestamp: now_secs(),
        }
    }
}

/// Statistics for a command based on historical executions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandStats {
    /// Average execution time in milliseconds.
    pub avg_duration_ms: f64,
    /// Variance of execution time in milliseconds².
    pub variance_ms: f64,
    /// Maximum execution time in milliseconds.
    pub max_duration_ms: u64,
    /// Minimum execution time in milliseconds.
    pub min_duration_ms: u64,
    /// Number of executions.
    pub count: usize,
    /// Success rate (0.0 - 1.0).
    pub success_rate: f64,
}

/// Background execution decision engine.
#[derive(Debug, Clone)]
pub struct BackgroundJudge {
    config: BackgroundJudgeConfig,
    history: HashMap<u64, Vec<ExecutionRecord>>,
    history_path: Option<PathBuf>,
}

impl Default for BackgroundJudge {
    fn default() -> Self {
        Self::new(BackgroundJudgeConfig::default())
    }
}

impl BackgroundJudge {
    /// Create a new background judge with the given configuration.
    pub fn new(config: BackgroundJudgeConfig) -> Self {
        Self {
            config,
            history: HashMap::new(),
            history_path: None,
        }
    }

    /// Set the path for persisting execution history.
    pub fn with_history_path(mut self, path: PathBuf) -> Self {
        self.history_path = Some(path);
        self
    }

    /// Decide whether a command should run in background.
    pub fn should_run_background(&self, command: &str) -> BackgroundDecision {
        let trimmed = command.trim();

        // Check for explicit background marker
        if trimmed.ends_with('&') {
            return BackgroundDecision::Always;
        }

        // Check never-background patterns first (higher priority)
        for pattern in &self.config.never_background_patterns {
            if pattern_matches(pattern, trimmed) {
                return BackgroundDecision::Never;
            }
        }

        // Check always-background patterns
        for pattern in &self.config.always_background_patterns {
            if pattern_matches(pattern, trimmed) {
                return BackgroundDecision::Always;
            }
        }

        // Check historical data if learning is enabled
        if self.config.enable_learning {
            let stats = self.get_stats(command);
            if stats.count >= self.config.min_samples_for_learning {
                // Consider both average AND variance for better decisions
                let variance = stats.variance_ms;
                let std_dev = variance.sqrt();
                
                // High variance = unpredictable, lean towards background
                // Low variance + slow = consistently slow, definitely background
                // Low variance + fast = consistently fast, definitely foreground
                
                if stats.avg_duration_ms > self.config.slow_threshold_ms as f64 {
                    // Consistently slow command
                    return BackgroundDecision::Always;
                } else if stats.avg_duration_ms > 2_000.0 {
                    // Moderately slow (2-5s) - use background if variance is high
                    if std_dev > 1_000.0 {
                        return BackgroundDecision::Always;
                    }
                } else if stats.avg_duration_ms < 1_000.0 && std_dev < 500.0 {
                    // Fast and consistent (under 1s with low variance)
                    return BackgroundDecision::Never;
                }
            }
        }

        // Default: let the system decide based on other factors
        BackgroundDecision::IfSlow
    }

    /// Record an execution.
    pub fn record_execution(&mut self, command: &str, duration_ms: u64, exit_code: i32) {
        let record = ExecutionRecord::new(command, duration_ms, exit_code);
        let hash = record.command_hash;
        self.history.entry(hash).or_default().push(record);

        // Keep only last 100 executions per command
        if let Some(records) = self.history.get_mut(&hash) {
            if records.len() > 100 {
                records.remove(0);
            }
        }
    }

    /// Get statistics for a command.
    pub fn get_stats(&self, command: &str) -> CommandStats {
        let hash = hash_command(command);
        let records = self.history.get(&hash);

        match records {
            None => CommandStats::default(),
            Some(records) if records.is_empty() => CommandStats::default(),
            Some(records) => {
                let count = records.len();
                let total: u64 = records.iter().map(|r| r.duration_ms).sum();
                let avg_duration_ms = total as f64 / count as f64;
                
                // Calculate variance
                let variance_ms = if count > 1 {
                    let sum_squared_diff: f64 = records
                        .iter()
                        .map(|r| {
                            let diff = r.duration_ms as f64 - avg_duration_ms;
                            diff * diff
                        })
                        .sum();
                    sum_squared_diff / count as f64
                } else {
                    0.0
                };
                
                let max_duration_ms = records.iter().map(|r| r.duration_ms).max().unwrap_or(0);
                let min_duration_ms = records.iter().map(|r| r.duration_ms).min().unwrap_or(0);
                let success_count = records.iter().filter(|r| r.exit_code == 0).count();
                let success_rate = success_count as f64 / count as f64;

                CommandStats {
                    avg_duration_ms,
                    variance_ms,
                    max_duration_ms,
                    min_duration_ms,
                    count,
                    success_rate,
                }
            }
        }
    }

    /// Load history from disk.
    pub fn load_history(&mut self) -> std::io::Result<()> {
        let path = match &self.history_path {
            Some(p) => p,
            None => return Ok(()),
        };

        if !path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(path)?;
        let records: Vec<ExecutionRecord> = serde_json::from_str(&content)?;

        for record in records {
            self.history
                .entry(record.command_hash)
                .or_default()
                .push(record);
        }

        Ok(())
    }

    /// Save history to disk.
    pub fn save_history(&self) -> std::io::Result<()> {
        let path = match &self.history_path {
            Some(p) => p,
            None => return Ok(()),
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let records: Vec<_> = self.history.values().flatten().cloned().collect();
        let content = serde_json::to_string_pretty(&records)?;
        std::fs::write(path, content)?;

        Ok(())
    }

    /// Get the default history path.
    pub fn default_history_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".anvil")
            .join("execution_history.json")
    }
}

/// Hash a command string for quick lookup.
fn hash_command(command: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    command.trim().hash(&mut hasher);
    hasher.finish()
}

/// Get current timestamp in seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Check if a pattern matches a command.
fn pattern_matches(pattern: &str, command: &str) -> bool {
    // Simple regex matching
    match regex::Regex::new(pattern) {
        Ok(re) => re.is_match(command),
        Err(_) => {
            // Fallback to simple contains if regex fails
            command.contains(pattern)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explicit_background_marker() {
        let judge = BackgroundJudge::default();
        assert_eq!(
            judge.should_run_background("cargo build &"),
            BackgroundDecision::Always
        );
        assert_eq!(
            judge.should_run_background("npm run dev &"),
            BackgroundDecision::Always
        );
    }

    #[test]
    fn test_never_background_patterns() {
        let judge = BackgroundJudge::default();
        assert_eq!(judge.should_run_background("ls -la"), BackgroundDecision::Never);
        assert_eq!(
            judge.should_run_background("git status"),
            BackgroundDecision::Never
        );
        assert_eq!(
            judge.should_run_background("cat file.txt"),
            BackgroundDecision::Never
        );
    }

    #[test]
    fn test_always_background_patterns() {
        let judge = BackgroundJudge::default();
        assert_eq!(
            judge.should_run_background("npm run dev"),
            BackgroundDecision::Always
        );
        assert_eq!(
            judge.should_run_background("cargo watch -x test"),
            BackgroundDecision::Always
        );
        assert_eq!(
            judge.should_run_background("vite --port 3000"),
            BackgroundDecision::Always
        );
    }

    #[test]
    fn test_execution_recording() {
        let mut judge = BackgroundJudge::default();

        // Record a slow command
        for _ in 0..5 {
            judge.record_execution("cargo build --release", 45_000, 0);
        }

        // Should now recommend background execution
        assert_eq!(
            judge.should_run_background("cargo build --release"),
            BackgroundDecision::Always
        );
    }

    #[test]
    fn test_command_stats() {
        let mut judge = BackgroundJudge::default();

        judge.record_execution("test command", 1000, 0);
        judge.record_execution("test command", 2000, 0);
        judge.record_execution("test command", 1500, 1);

        let stats = judge.get_stats("test command");
        assert_eq!(stats.count, 3);
        assert_eq!(stats.avg_duration_ms, 1500.0);
        assert_eq!(stats.max_duration_ms, 2000);
        assert_eq!(stats.min_duration_ms, 1000);
        assert!((stats.success_rate - 0.6666666666666666).abs() < 0.01);
    }

    #[test]
    fn test_history_persistence() {
        let temp_dir = tempfile::tempdir().unwrap();
        let history_path = temp_dir.path().join("history.json");

        // Create judge and record some executions
        let mut judge = BackgroundJudge::default().with_history_path(history_path.clone());
        judge.record_execution("test1", 1000, 0);
        judge.record_execution("test2", 2000, 1);
        judge.save_history().unwrap();

        // Create new judge and load history
        let mut judge2 = BackgroundJudge::default().with_history_path(history_path);
        judge2.load_history().unwrap();

        let stats1 = judge2.get_stats("test1");
        assert_eq!(stats1.count, 1);
        assert_eq!(stats1.avg_duration_ms, 1000.0);

        let stats2 = judge2.get_stats("test2");
        assert_eq!(stats2.count, 1);
        assert_eq!(stats2.avg_duration_ms, 2000.0);
    }
}
