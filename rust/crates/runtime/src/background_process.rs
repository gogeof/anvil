//! Background process management for bash tool execution.
//!
//! This module provides process lifecycle management, output capture,
//! and control operations for background shell commands.
//!
//! # Features
//!
//! - Process lifecycle: spawn, monitor, stop, remove
//! - Signal handling: SIGINT -> SIGTERM -> SIGKILL escalation
//! - Timeout management: automatic termination after configured duration
//! - Output capture: real-time stdout/stderr capture to files
//! - Process groups: manage related processes together
//! - Restart policies: automatic restart on failure
//! - Resource limits: CPU and memory constraints
//! - Event notifications: status change callbacks

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Status of a background process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    /// Process is currently running.
    Running,
    /// Process is paused (SIGSTOP).
    Paused,
    /// Process completed successfully (exit code 0).
    Completed,
    /// Process failed (non-zero exit code).
    Failed,
    /// Process was stopped by user or timeout.
    Stopped,
    /// Process state is unknown (e.g., couldn't be determined).
    Unknown,
}

/// Signal to send to a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessSignal {
    /// Interrupt signal (Ctrl+C). Allows graceful shutdown.
    Interrupt,
    /// Termination signal. Request graceful shutdown.
    Terminate,
    /// Kill signal. Immediate termination.
    Kill,
    /// Stop signal. Pause process execution.
    Stop,
    /// Continue signal. Resume stopped process.
    Continue,
    /// User-defined signal 1.
    User1,
    /// User-defined signal 2.
    User2,
    /// Hangup signal. Often used for reload.
    Hangup,
}

impl ProcessSignal {
    /// Convert to Unix signal number.
    #[cfg(unix)]
    pub fn to_signal_num(&self) -> i32 {
        match self {
            Self::Interrupt => 2,   // SIGINT
            Self::Terminate => 15,  // SIGTERM
            Self::Kill => 9,        // SIGKILL
            Self::Stop => 19,       // SIGSTOP
            Self::Continue => 18,   // SIGCONT
            Self::User1 => 10,      // SIGUSR1
            Self::User2 => 12,      // SIGUSR2
            Self::Hangup => 1,      // SIGHUP
        }
    }

    /// Convert to Windows equivalent action.
    #[cfg(windows)]
    pub fn to_windows_action(&self) -> WindowsSignalAction {
        match self {
            Self::Interrupt => WindowsSignalAction::CtrlC,
            Self::Terminate => WindowsSignalAction::Terminate,
            Self::Kill => WindowsSignalAction::ForceKill,
            Self::Stop | Self::Continue | Self::User1 | Self::User2 | Self::Hangup => {
                WindowsSignalAction::Unsupported
            }
        }
    }

    /// Get signal name as string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
            Self::Kill => "SIGKILL",
            Self::Stop => "SIGSTOP",
            Self::Continue => "SIGCONT",
            Self::User1 => "SIGUSR1",
            Self::User2 => "SIGUSR2",
            Self::Hangup => "SIGHUP",
        }
    }
}

/// Windows signal action (simplified).
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsSignalAction {
    CtrlC,
    Terminate,
    ForceKill,
    Unsupported,
}

impl std::fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Stopped => write!(f, "stopped"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Restart policy for background processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Never restart automatically.
    Never,
    /// Restart on failure only.
    OnFailure,
    /// Always restart when stopped.
    Always,
    /// Restart on failure with unlimited attempts.
    Unlimited,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self::Never
    }
}

/// Resource limits for a background process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum CPU time in seconds.
    pub cpu_time_seconds: Option<u64>,
    /// Maximum memory in megabytes.
    pub memory_mb: Option<u64>,
    /// Maximum number of open file descriptors.
    pub max_files: Option<u64>,
    /// Maximum number of processes.
    pub max_processes: Option<u64>,
    /// Maximum file size in megabytes that can be created.
    pub max_file_size_mb: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            cpu_time_seconds: None,
            memory_mb: None,
            max_files: None,
            max_processes: None,
            max_file_size_mb: None,
        }
    }
}

/// Resource usage statistics for a running process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceUsage {
    /// CPU time used (user) in seconds.
    pub cpu_user_seconds: f64,
    /// CPU time used (system) in seconds.
    pub cpu_system_seconds: f64,
    /// Resident set size (physical memory) in bytes.
    pub memory_rss_bytes: u64,
    /// Virtual memory size in bytes.
    pub memory_vms_bytes: u64,
    /// Percentage of CPU used (0.0 - 100.0 * num_cores).
    pub cpu_percent: f64,
    /// Percentage of memory used (0.0 - 100.0).
    pub memory_percent: f64,
    /// Number of threads.
    pub num_threads: u64,
    /// Number of open file descriptors.
    pub num_fds: u64,
    /// Time when the stats were collected.
    pub collected_at: u64,
}

impl Default for ResourceUsage {
    fn default() -> Self {
        Self {
            cpu_user_seconds: 0.0,
            cpu_system_seconds: 0.0,
            memory_rss_bytes: 0,
            memory_vms_bytes: 0,
            cpu_percent: 0.0,
            memory_percent: 0.0,
            num_threads: 0,
            num_fds: 0,
            collected_at: 0,
        }
    }
}

/// Process state change event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessEvent {
    /// ID of the process that changed.
    pub process_id: String,
    /// Previous status.
    pub previous_status: ProcessStatus,
    /// New status.
    pub new_status: ProcessStatus,
    /// Timestamp of the event.
    pub timestamp: u64,
    /// Optional message describing the change.
    pub message: Option<String>,
    /// Exit code if the process exited.
    pub exit_code: Option<i32>,
}

/// Callback type for process events.
pub type ProcessEventCallback = Box<dyn Fn(ProcessEvent) + Send + Sync>;

/// Output read position for incremental reading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputPosition {
    /// The process ID.
    pub process_id: String,
    /// Which stream ("stdout" or "stderr").
    pub stream: String,
    /// Byte offset in the file.
    pub offset: u64,
    /// Line number (1-based).
    pub line_number: u64,
}

/// Incremental output result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalOutput {
    /// The process ID.
    pub process_id: String,
    /// Which stream ("stdout" or "stderr").
    pub stream: String,
    /// New content since last read.
    pub content: String,
    /// New position for next read.
    pub new_position: OutputPosition,
    /// Whether we've reached EOF (process finished).
    pub eof: bool,
}

/// Child process information for process tree tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildProcessInfo {
    /// PID of the child process.
    pub pid: u32,
    /// Parent PID.
    pub parent_pid: u32,
    /// Process name (command name).
    pub name: String,
    /// Full command line.
    pub command: String,
    /// Current status (running, sleeping, zombie, etc.).
    pub state: String,
    /// Memory usage in bytes.
    pub memory_bytes: u64,
    /// CPU time in seconds.
    pub cpu_seconds: f64,
    /// Start time (relative to boot).
    pub start_time: u64,
}

/// Configuration for creating a background process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfig {
    /// The command to execute.
    pub command: String,
    /// Optional description.
    pub description: Option<String>,
    /// Working directory.
    pub working_dir: Option<String>,
    /// Environment variables.
    pub env: Option<HashMap<String, String>>,
    /// Restart policy.
    pub restart_policy: RestartPolicy,
    /// Maximum restart attempts.
    pub max_restart_attempts: u32,
    /// Resource limits.
    pub resource_limits: ResourceLimits,
    /// Timeout in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Process group name.
    pub process_group: Option<String>,
    /// Whether sandbox is enabled.
    pub sandbox_enabled: bool,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            description: None,
            working_dir: None,
            env: None,
            restart_policy: RestartPolicy::Never,
            max_restart_attempts: 3,
            resource_limits: ResourceLimits::default(),
            timeout_ms: None,
            process_group: None,
            sandbox_enabled: true,
        }
    }
}

/// A background process with metadata and output capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundProcess {
    /// Unique identifier for this process.
    pub process_id: String,
    /// The command being executed.
    pub command: String,
    /// Optional description of the command.
    pub description: Option<String>,
    /// Current status of the process.
    pub status: ProcessStatus,
    /// System PID of the process.
    pub pid: Option<u32>,
    /// Exit code (if completed).
    pub exit_code: Option<i32>,
    /// Timestamp when the process was created.
    pub created_at: u64,
    /// Timestamp when the process was last updated.
    pub updated_at: u64,
    /// Path to the stdout output file.
    pub stdout_path: Option<String>,
    /// Path to the stderr output file.
    pub stderr_path: Option<String>,
    /// Size of captured stdout in bytes.
    pub stdout_size: u64,
    /// Size of captured stderr in bytes.
    pub stderr_size: u64,
    /// Whether sandbox is enabled.
    pub sandbox_enabled: bool,
    /// Working directory.
    pub working_dir: String,
    /// Optional timeout in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Process group ID (for managing related processes).
    pub process_group: Option<String>,
    /// Number of restart attempts.
    pub restart_count: u32,
    /// Maximum restart attempts allowed.
    pub max_restart_attempts: u32,
    /// Restart policy.
    pub restart_policy: RestartPolicy,
    /// Resource limits.
    pub resource_limits: ResourceLimits,
    /// Last signal sent to the process.
    pub last_signal: Option<String>,
    /// Time when the process was started (for timeout calculation).
    #[serde(skip)]
    pub started_at: Option<Instant>,
}

/// Internal state for tracking a live process.
#[derive(Debug, Clone)]
struct LiveProcess {
    pid: u32,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    command: String,
    working_dir: PathBuf,
    started_at: Instant,
    timeout_ms: Option<u64>,
    process_group: Option<String>,
    /// Restart policy for this process.
    restart_policy: RestartPolicy,
    /// Max restart attempts.
    max_restart_attempts: u32,
    /// Resource limits.
    resource_limits: ResourceLimits,
    /// Environment variables for restart.
    env: HashMap<String, String>,
}

/// Manager for background processes.
#[derive(Debug, Clone)]
pub struct BackgroundProcessManager {
    inner: Arc<Mutex<ProcessManagerInner>>,
    /// Condition variable for wait operations.
    wait_condvar: Arc<Condvar>,
}

impl Default for BackgroundProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct ProcessManagerInner {
    processes: HashMap<String, BackgroundProcess>,
    live_processes: HashMap<String, LiveProcess>,
    /// Process groups for managing related processes.
    process_groups: HashMap<String, Vec<String>>,
    output_dir: PathBuf,
    counter: u64,
    /// Timeout monitor thread handle.
    timeout_monitor_handle: Option<JoinHandle<()>>,
    /// Flag to signal timeout monitor to stop.
    timeout_monitor_running: bool,
    /// Event callbacks for process state changes.
    event_callbacks: Vec<ProcessEventCallback>,
}

impl std::fmt::Debug for ProcessManagerInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessManagerInner")
            .field("processes", &self.processes)
            .field("live_processes", &self.live_processes.len())
            .field("process_groups", &self.process_groups)
            .field("output_dir", &self.output_dir)
            .field("counter", &self.counter)
            .field("timeout_monitor_running", &self.timeout_monitor_running)
            .field("event_callbacks", &format!("{} callbacks", self.event_callbacks.len()))
            .finish()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl BackgroundProcessManager {
    /// Creates a new process manager with default output directory.
    #[must_use]
    pub fn new() -> Self {
        let inner = Arc::new(Mutex::new(ProcessManagerInner {
            timeout_monitor_running: true,
            ..Default::default()
        }));
        let wait_condvar = Arc::new(Condvar::new());

        // Start timeout monitor thread
        let monitor_inner = Arc::clone(&inner);
        let monitor_condvar = Arc::clone(&wait_condvar);
        let handle = thread::spawn(move || {
            Self::timeout_monitor_loop(monitor_inner, monitor_condvar);
        });

        // Store the handle
        {
            let mut guard = inner.lock().expect("manager lock poisoned");
            guard.timeout_monitor_handle = Some(handle);
        }

        Self { inner, wait_condvar }
    }

    /// Add an event callback for process state changes.
    pub fn add_event_callback(&self, callback: ProcessEventCallback) {
        let mut inner = self.inner.lock().expect("manager lock poisoned");
        inner.event_callbacks.push(callback);
    }

    /// Emit a process event to all registered callbacks.
    fn emit_event(&self, event: ProcessEvent) {
        let inner = self.inner.lock().expect("manager lock poisoned");
        for callback in &inner.event_callbacks {
            callback(event.clone());
        }
    }

    /// Create a process group for managing related processes together.
    pub fn create_process_group(&self, group_name: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().expect("manager lock poisoned");
        if inner.process_groups.contains_key(group_name) {
            return Err(format!("process group '{}' already exists", group_name));
        }
        inner.process_groups.insert(group_name.to_string(), Vec::new());
        Ok(())
    }

    /// List all process groups.
    pub fn list_process_groups(&self) -> Vec<String> {
        let inner = self.inner.lock().expect("manager lock poisoned");
        inner.process_groups.keys().cloned().collect()
    }

    /// Get processes in a specific group.
    pub fn get_process_group(&self, group_name: &str) -> Option<Vec<BackgroundProcess>> {
        let inner = self.inner.lock().expect("manager lock poisoned");
        let ids = inner.process_groups.get(group_name)?;
        Some(
            ids.iter()
                .filter_map(|id| inner.processes.get(id).cloned())
                .collect(),
        )
    }

    /// Stop all processes in a group.
    pub fn stop_process_group(&self, group_name: &str) -> Result<Vec<BackgroundProcess>, String> {
        let ids: Vec<String> = {
            let inner = self.inner.lock().expect("manager lock poisoned");
            inner
                .process_groups
                .get(group_name)
                .ok_or_else(|| format!("process group '{}' not found", group_name))?
                .clone()
        };

        let mut results = Vec::new();
        for id in ids {
            match self.stop(&id) {
                Ok(process) => results.push(process),
                Err(e) => eprintln!("Failed to stop process {}: {}", id, e),
            }
        }
        Ok(results)
    }

    /// Remove a process group (processes must be stopped first).
    pub fn remove_process_group(&self, group_name: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().expect("manager lock poisoned");

        let ids = inner
            .process_groups
            .get(group_name)
            .ok_or_else(|| format!("process group '{}' not found", group_name))?
            .clone();

        // Check if any processes are still running
        for id in &ids {
            if let Some(process) = inner.processes.get(id) {
                if process.status == ProcessStatus::Running {
                    return Err(format!(
                        "cannot remove group '{}': process {} is still running",
                        group_name, id
                    ));
                }
            }
        }

        inner.process_groups.remove(group_name);
        Ok(())
    }

    /// Timeout monitor loop - checks for expired processes periodically.
    fn timeout_monitor_loop(
        inner: Arc<Mutex<ProcessManagerInner>>,
        condvar: Arc<Condvar>,
    ) {
        loop {
            let process_ids_to_timeout: Vec<String> = {
                let mut guard = inner.lock().expect("manager lock poisoned");

                // Check if we should stop
                if !guard.timeout_monitor_running {
                    return;
                }

                // Collect processes that have timed out
                let now = Instant::now();
                guard
                    .live_processes
                    .iter()
                    .filter(|(_, live)| {
                        if let Some(timeout_ms) = live.timeout_ms {
                            let elapsed = now.duration_since(live.started_at);
                            elapsed.as_millis() as u64 >= timeout_ms
                        } else {
                            false
                        }
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            };

            // Terminate timed out processes
            for process_id in process_ids_to_timeout {
                let _ = Self::terminate_process_by_id(&inner, &process_id, "timeout");
            }

            // Sleep for a short interval before checking again
            let guard = inner.lock().expect("manager lock poisoned");
            let _ = condvar.wait_timeout(guard, Duration::from_millis(500)).expect("condvar wait");
        }
    }

    /// Check and handle automatic restart for a process.
    fn handle_auto_restart(
        inner: &Arc<Mutex<ProcessManagerInner>>,
        process_id: &str,
    ) -> Option<String> {
        let (restart_policy, restart_count, max_attempts, command, working_dir, env) = {
            let guard = inner.lock().expect("manager lock poisoned");
            let process = guard.processes.get(process_id)?;

            // Check if we should restart
            let should_restart = match process.restart_policy {
                RestartPolicy::Never => false,
                RestartPolicy::OnFailure => {
                    process.status == ProcessStatus::Failed && process.restart_count < process.max_restart_attempts
                }
                RestartPolicy::Always => process.restart_count < process.max_restart_attempts,
                RestartPolicy::Unlimited => process.status == ProcessStatus::Failed,
            };

            if !should_restart {
                return None;
            }

            let live = guard.live_processes.get(process_id)?;
            (
                process.restart_policy,
                process.restart_count,
                process.max_restart_attempts,
                live.command.clone(),
                live.working_dir.clone(),
                live.env.clone(),
            )
        };

        // Attempt to restart the process
        let new_pid = Self::spawn_process(&command, &working_dir, &env).ok()?;

        // Update the process record
        {
            let mut guard = inner.lock().expect("manager lock poisoned");
            if let Some(process) = guard.processes.get_mut(process_id) {
                process.pid = Some(new_pid);
                process.status = ProcessStatus::Running;
                process.restart_count += 1;
                process.updated_at = now_secs();
                process.started_at = Some(Instant::now());
            }
            if let Some(live) = guard.live_processes.get_mut(process_id) {
                live.pid = new_pid;
                live.started_at = Instant::now();
            }
        }

        Some(format!("Process {} restarted (attempt {})", process_id, restart_count + 1))
    }

    /// Spawn a new process with the given command and environment.
    fn spawn_process(
        command: &str,
        working_dir: &PathBuf,
        env: &HashMap<String, String>,
    ) -> std::io::Result<u32> {
        let mut child = std::process::Command::new("sh")
            .arg("-lc")
            .arg(command)
            .current_dir(working_dir)
            .envs(env.clone())
            .spawn()?;
        Ok(child.id())
    }

    /// Terminate a process by ID with a reason.
    fn terminate_process_by_id(
        inner: &Arc<Mutex<ProcessManagerInner>>,
        process_id: &str,
        reason: &str,
    ) -> Result<BackgroundProcess, String> {
        let mut guard = inner.lock().expect("manager lock poisoned");

        // First, get the process info and live process pid
        let (process_status, process_info) = {
            let process = guard
                .processes
                .get(process_id)
                .ok_or_else(|| format!("process not found: {}", process_id))?;
            let pid = guard.live_processes.get(process_id).map(|l| l.pid);
            (process.status, (process.clone(), pid))
        };

        let (mut process_info, pid) = process_info;

        // Terminate the live process if running
        if process_status == ProcessStatus::Running {
            if let Some(pid) = pid {
                Self::send_signal_to_pid(pid, ProcessSignal::Terminate);
                thread::sleep(Duration::from_millis(100));
                // Check if still running, send SIGKILL if needed
                if Self::is_pid_running(pid) {
                    Self::send_signal_to_pid(pid, ProcessSignal::Kill);
                }
            }
            process_info.status = ProcessStatus::Stopped;
            process_info.last_signal = Some(format!("{} (auto)", reason));
            process_info.updated_at = now_secs();
            guard.live_processes.remove(process_id);
            guard.processes.insert(process_id.to_string(), process_info.clone());
        }

        Ok(process_info)
    }

    /// Send a signal to a process by PID.
    fn send_signal_to_pid(pid: u32, signal: ProcessSignal) {
        #[cfg(unix)]
        {
            use std::process::Command as StdCommand;
            let signal_arg = format!("-{}", signal.to_signal_num());
            let _ = StdCommand::new("kill")
                .arg(&signal_arg)
                .arg(pid.to_string())
                .output();
        }

        #[cfg(windows)]
        {
            use std::process::Command as StdCommand;
            if signal == ProcessSignal::Kill {
                let _ = StdCommand::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output();
            } else {
                let _ = StdCommand::new("taskkill")
                    .args(["/PID", &pid.to_string()])
                    .output();
            }
        }
    }

    /// Check if a PID is still running.
    fn is_pid_running(pid: u32) -> bool {
        #[cfg(unix)]
        {
            use std::process::Command as StdCommand;
            StdCommand::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }

        #[cfg(windows)]
        {
            use std::process::Command as StdCommand;
            StdCommand::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid)])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
                .unwrap_or(false)
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = pid;
            false
        }
    }

    /// Creates a process manager with a specific output directory.
    pub fn with_output_dir(output_dir: PathBuf) -> Self {
        let inner = ProcessManagerInner {
            output_dir,
            ..Default::default()
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
            wait_condvar: Arc::new(Condvar::new()),
        }
    }

    /// Sets the output directory for process logs.
    pub fn set_output_dir(&self, dir: PathBuf) {
        let mut inner = self.inner.lock().expect("manager lock poisoned");
        inner.output_dir = dir;
    }

    /// Ensures the output directory exists.
    fn ensure_output_dir(&self) -> std::io::Result<PathBuf> {
        let mut inner = self.inner.lock().expect("manager lock poisoned");
        if inner.output_dir.as_os_str().is_empty() {
            // Use a default temp directory
            inner.output_dir = std::env::temp_dir().join(".anvil").join("background");
        }
        std::fs::create_dir_all(&inner.output_dir)?;
        Ok(inner.output_dir.clone())
    }

    /// Registers a new background process.
    ///
    /// # Arguments
    /// * `child` - The spawned child process
    /// * `command` - The command string
    /// * `description` - Optional description
    /// * `sandbox_enabled` - Whether sandbox was enabled
    /// * `working_dir` - Working directory of the command
    ///
    /// # Returns
    /// The registered `BackgroundProcess` with output capture configured.
    pub fn register(
        &self,
        child: &mut Child,
        command: &str,
        description: Option<&str>,
        sandbox_enabled: bool,
        working_dir: &std::path::Path,
    ) -> std::io::Result<BackgroundProcess> {
        self.register_with_config(
            child,
            ProcessConfig {
                command: command.to_string(),
                description: description.map(str::to_owned),
                working_dir: Some(working_dir.to_string_lossy().to_string()),
                sandbox_enabled,
                ..Default::default()
            },
        )
    }

    /// Registers a new background process with full configuration.
    ///
    /// # Arguments
    /// * `child` - The spawned child process
    /// * `config` - Process configuration
    ///
    /// # Returns
    /// The registered `BackgroundProcess` with output capture configured.
    pub fn register_with_config(
        &self,
        child: &mut Child,
        config: ProcessConfig,
    ) -> std::io::Result<BackgroundProcess> {
        let output_dir = self.ensure_output_dir()?;
        let pid = child.id();

        let mut inner = self.inner.lock().expect("manager lock poisoned");
        inner.counter += 1;

        let ts = now_secs();
        let process_id = format!("bgp_{:08x}_{}", ts, inner.counter);

        // Create output files
        let stdout_path = output_dir.join(format!("{}.stdout", process_id));
        let stderr_path = output_dir.join(format!("{}.stderr", process_id));

        // Initialize empty output files
        File::create(&stdout_path)?;
        File::create(&stderr_path)?;

        let working_dir = config
            .working_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().to_string_lossy().to_string());

        // Apply resource limits (platform-specific)
        #[cfg(unix)]
        {
            let _ = Self::apply_resource_limits(pid, &config.resource_limits);
        }

        // Add to process group if specified
        if let Some(ref group) = config.process_group {
            inner
                .process_groups
                .entry(group.clone())
                .or_default()
                .push(process_id.clone());
        }

        let process = BackgroundProcess {
            process_id: process_id.clone(),
            command: config.command.clone(),
            description: config.description.clone(),
            status: ProcessStatus::Running,
            pid: Some(pid),
            exit_code: None,
            created_at: ts,
            updated_at: ts,
            stdout_path: Some(stdout_path.to_string_lossy().to_string()),
            stderr_path: Some(stderr_path.to_string_lossy().to_string()),
            stdout_size: 0,
            stderr_size: 0,
            sandbox_enabled: config.sandbox_enabled,
            working_dir,
            timeout_ms: config.timeout_ms,
            process_group: config.process_group.clone(),
            restart_count: 0,
            max_restart_attempts: config.max_restart_attempts,
            restart_policy: config.restart_policy,
            resource_limits: config.resource_limits.clone(),
            last_signal: None,
            started_at: Some(Instant::now()),
        };

        // Track the live process
        inner.live_processes.insert(
            process_id.clone(),
            LiveProcess {
                pid,
                stdout_path,
                stderr_path,
                command: config.command,
                working_dir: PathBuf::from(&process.working_dir),
                started_at: Instant::now(),
                timeout_ms: config.timeout_ms,
                process_group: config.process_group,
                restart_policy: config.restart_policy,
                max_restart_attempts: config.max_restart_attempts,
                resource_limits: config.resource_limits,
                env: config.env.unwrap_or_default(),
            },
        );

        inner.processes.insert(process_id.clone(), process.clone());

        Ok(process)
    }

    /// Apply resource limits to a process (Unix only).
    #[cfg(unix)]
    fn apply_resource_limits(pid: u32, limits: &ResourceLimits) -> std::io::Result<()> {
        use std::process::Command as StdCommand;

        // Use prlimit to set resource limits
        let mut args = Vec::new();

        if let Some(cpu_time) = limits.cpu_time_seconds {
            args.push(format!("--cpu={}", cpu_time));
        }

        if let Some(memory_mb) = limits.memory_mb {
            // Convert MB to bytes
            let bytes = memory_mb * 1024 * 1024;
            args.push(format!("--as={}", bytes));
        }

        if let Some(max_files) = limits.max_files {
            args.push(format!("--nofile={}", max_files));
        }

        if let Some(max_procs) = limits.max_processes {
            args.push(format!("--nproc={}", max_procs));
        }

        if let Some(max_file_size) = limits.max_file_size_mb {
            let bytes = max_file_size * 1024 * 1024;
            args.push(format!("--fsize={}", bytes));
        }

        if !args.is_empty() {
            let mut cmd = StdCommand::new("prlimit");
            cmd.arg("--pid").arg(pid.to_string());
            for arg in args {
                cmd.arg(&arg);
            }
            let _ = cmd.output();
        }

        Ok(())
    }

    /// Gets a process by ID.
    pub fn get(&self, process_id: &str) -> Option<BackgroundProcess> {
        let inner = self.inner.lock().expect("manager lock poisoned");
        inner.processes.get(process_id).cloned()
    }

    /// Lists all processes, optionally filtered by status.
    pub fn list(&self, status_filter: Option<ProcessStatus>) -> Vec<BackgroundProcess> {
        let inner = self.inner.lock().expect("manager lock poisoned");
        inner
            .processes
            .values()
            .filter(|p| status_filter.map_or(true, |s| p.status == s))
            .cloned()
            .collect()
    }

    /// Stops a running process.
    ///
    /// On Unix, sends SIGTERM first, then SIGKILL after 5 seconds if still running.
    /// On Windows, terminates the process immediately.
    pub fn stop(&self, process_id: &str) -> Result<BackgroundProcess, String> {
        let mut inner = self.inner.lock().expect("manager lock poisoned");

        // First, get the process status and live process pid
        let (process_status, live_pid) = {
            let process = inner
                .processes
                .get(process_id)
                .ok_or_else(|| format!("process not found: {}", process_id))?;
            let pid = inner.live_processes.get(process_id).map(|l| l.pid);
            (process.status, pid)
        };

        // Check if already in terminal state
        match process_status {
            ProcessStatus::Completed | ProcessStatus::Failed | ProcessStatus::Stopped => {
                return Err(format!(
                    "process {} is already in terminal state: {}",
                    process_id, process_status
                ));
            }
            ProcessStatus::Unknown | ProcessStatus::Running | ProcessStatus::Paused => {}
        }

        // Try to kill the process
        if let Some(pid) = live_pid {
            #[cfg(unix)]
            {
                use std::process::Command as StdCommand;
                // Send SIGTERM first for graceful shutdown
                let _ = StdCommand::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .output();

                // Wait briefly for graceful shutdown
                std::thread::sleep(Duration::from_millis(500));

                // Check if still running, send SIGKILL if needed
                let check = StdCommand::new("kill")
                    .arg("-0")
                    .arg(pid.to_string())
                    .output();
                if check.map(|o| o.status.success()).unwrap_or(false) {
                    let _ = StdCommand::new("kill")
                        .arg("-KILL")
                        .arg(pid.to_string())
                        .output();
                }
            }

            #[cfg(windows)]
            {
                use std::process::Command as StdCommand;
                let _ = StdCommand::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output();
            }
        }

        // Update process status
        {
            let process = inner
                .processes
                .get_mut(process_id)
                .ok_or_else(|| format!("process not found: {}", process_id))?;
            process.status = ProcessStatus::Stopped;
            process.updated_at = now_secs();
        }

        // Remove from live tracking
        inner.live_processes.remove(process_id);

        // Return the updated process
        let process = inner
            .processes
            .get(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;
        Ok(process.clone())
    }

    /// Refreshes the status of a process by checking if it's still running.
    pub fn refresh_status(&self, process_id: &str) -> Result<BackgroundProcess, String> {
        let mut inner = self.inner.lock().expect("manager lock poisoned");

        // First, get the process status
        let current_status = {
            let process = inner
                .processes
                .get(process_id)
                .ok_or_else(|| format!("process not found: {}", process_id))?;
            process.status
        };

        if current_status != ProcessStatus::Running {
            let process = inner
                .processes
                .get(process_id)
                .ok_or_else(|| format!("process not found: {}", process_id))?;
            return Ok(process.clone());
        }

        // Get live process info if available
        let live_info = inner.live_processes.get(process_id).cloned();

        // Check if the process is still running
        if let Some(live) = live_info {
            let is_running = self.is_process_running(live.pid);

            if !is_running {
                // Process has finished, update output sizes and determine exit status
                let stdout_size = self.get_file_size(&live.stdout_path);
                let stderr_size = self.get_file_size(&live.stderr_path);

                // Try to get exit code from wait
                let exit_status = self.try_wait_process(live.pid);

                // Now get mutable reference to update
                let process = inner
                    .processes
                    .get_mut(process_id)
                    .ok_or_else(|| format!("process not found: {}", process_id))?;

                process.stdout_size = stdout_size;
                process.stderr_size = stderr_size;

                match exit_status {
                    Some(code) if code == 0 => {
                        process.status = ProcessStatus::Completed;
                        process.exit_code = Some(0);
                    }
                    Some(code) => {
                        process.status = ProcessStatus::Failed;
                        process.exit_code = Some(code);
                    }
                    None => {
                        process.status = ProcessStatus::Unknown;
                    }
                }

                process.updated_at = now_secs();
                inner.live_processes.remove(process_id);
            }
        }

        let process = inner
            .processes
            .get(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;
        Ok(process.clone())
    }

    /// Gets the output from a process.
    ///
    /// # Arguments
    /// * `process_id` - The process ID
    /// * `stream` - Which output stream to read ("stdout", "stderr", or "both")
    /// * `tail` - Number of lines to read from the end (None = all)
    ///
    /// # Returns
    /// The output content as a string.
    pub fn get_output(
        &self,
        process_id: &str,
        stream: &str,
        tail: Option<usize>,
    ) -> Result<String, String> {
        let inner = self.inner.lock().expect("manager lock poisoned");

        let process = inner
            .processes
            .get(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;

        let read_file = |path: &Option<String>| -> Result<String, String> {
            let path = path
                .as_ref()
                .ok_or_else(|| "output file not available".to_string())?;
            let file = File::open(path).map_err(|e| format!("failed to open output: {}", e))?;
            let reader = BufReader::new(file);

            if let Some(n) = tail {
                let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
                let start = lines.len().saturating_sub(n);
                Ok(lines[start..].join("\n"))
            } else {
                let content: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
                Ok(content.join("\n"))
            }
        };

        match stream {
            "stdout" => read_file(&process.stdout_path),
            "stderr" => read_file(&process.stderr_path),
            "both" => {
                let stdout = read_file(&process.stdout_path).unwrap_or_default();
                let stderr = read_file(&process.stderr_path).unwrap_or_default();
                if stdout.is_empty() && stderr.is_empty() {
                    Ok(String::new())
                } else if stdout.is_empty() {
                    Ok(format!("[stderr]\n{}", stderr))
                } else if stderr.is_empty() {
                    Ok(format!("[stdout]\n{}", stdout))
                } else {
                    Ok(format!("[stdout]\n{}\n\n[stderr]\n{}", stdout, stderr))
                }
            }
            _ => Err(format!("invalid stream: {}. Use 'stdout', 'stderr', or 'both'", stream)),
        }
    }

    /// Removes a completed process from the registry.
    pub fn remove(&self, process_id: &str) -> Result<BackgroundProcess, String> {
        let mut inner = self.inner.lock().expect("manager lock poisoned");

        let process = inner
            .processes
            .get(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;

        // Only allow removal of non-running processes
        if process.status == ProcessStatus::Running {
            return Err("cannot remove a running process, stop it first".to_string());
        }

        inner.live_processes.remove(process_id);
        let removed = inner.processes.remove(process_id).expect("process exists");

        // Clean up output files
        if let Some(ref path) = removed.stdout_path {
            let _ = std::fs::remove_file(path);
        }
        if let Some(ref path) = removed.stderr_path {
            let _ = std::fs::remove_file(path);
        }

        Ok(removed)
    }

    /// Returns the number of tracked processes.
    #[must_use]
    pub fn len(&self) -> usize {
        let inner = self.inner.lock().expect("manager lock poisoned");
        inner.processes.len()
    }

    /// Returns true if no processes are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Refreshes all running processes.
    pub fn refresh_all(&self) {
        // Collect IDs while holding the lock, then release before individual refreshes
        let process_ids: Vec<String> = {
            let inner = self.inner.lock().expect("manager lock poisoned");
            inner
                .processes
                .iter()
                .filter(|(_, p)| p.status == ProcessStatus::Running)
                .map(|(id, _)| id.clone())
                .collect()
        };

        // Refresh each process without holding the global lock
        for id in process_ids {
            let _ = self.refresh_status(&id);
        }
    }

    // Helper methods

    fn is_process_running(&self, pid: u32) -> bool {
        #[cfg(unix)]
        {
            use std::process::Command as StdCommand;
            StdCommand::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }

        #[cfg(windows)]
        {
            use std::process::Command as StdCommand;
            StdCommand::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid)])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
                .unwrap_or(false)
        }

        #[cfg(not(any(unix, windows)))]
        {
            false
        }
    }

    fn try_wait_process(&self, pid: u32) -> Option<i32> {
        #[cfg(unix)]
        {
            use std::process::Command as StdCommand;
            // Try to reap the zombie and get exit status
            let output = StdCommand::new("sh")
                .arg("-c")
                .arg(format!("wait {} 2>/dev/null; echo $?", pid))
                .output()
                .ok()?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.trim().parse().ok()
        }

        #[cfg(not(unix))]
        {
            let _ = pid;
            None
        }
    }

    fn get_file_size(&self, path: &PathBuf) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    /// Send a signal to a running process.
    ///
    /// # Arguments
    /// * `process_id` - The process ID
    /// * `signal` - The signal to send
    ///
    /// # Returns
    /// The updated process state after sending the signal.
    pub fn send_signal(&self, process_id: &str, signal: ProcessSignal) -> Result<BackgroundProcess, String> {
        let mut inner = self.inner.lock().expect("manager lock poisoned");

        // First, get the process status
        let current_status = {
            let process = inner
                .processes
                .get(process_id)
                .ok_or_else(|| format!("process not found: {}", process_id))?;
            process.status
        };

        if current_status != ProcessStatus::Running {
            return Err(format!("process {} is not running (status: {})", process_id, current_status));
        }

        // Get live process info
        let live = inner
            .live_processes
            .get(process_id)
            .ok_or_else(|| format!("live process not found: {}", process_id))?
            .clone();

        // Send the signal
        Self::send_signal_to_pid(live.pid, signal);

        // Update process state based on signal
        let new_status = match signal {
            ProcessSignal::Stop => ProcessStatus::Paused,
            ProcessSignal::Kill | ProcessSignal::Terminate | ProcessSignal::Interrupt => {
                // Give the process a moment to handle the signal
                std::thread::sleep(Duration::from_millis(50));

                // Check if still running
                if !Self::is_pid_running(live.pid) {
                    if signal == ProcessSignal::Kill {
                        ProcessStatus::Stopped
                    } else {
                        ProcessStatus::Completed
                    }
                } else {
                    ProcessStatus::Running
                }
            }
            ProcessSignal::Continue => ProcessStatus::Running,
            _ => ProcessStatus::Running, // User signals don't change status
        };

        // Now get mutable reference to update
        {
            let process = inner
                .processes
                .get_mut(process_id)
                .ok_or_else(|| format!("process not found: {}", process_id))?;

            process.status = new_status;
            process.last_signal = Some(signal.as_str().to_string());
            process.updated_at = now_secs();
        }

        // If process stopped, remove from live tracking
        if matches!(new_status, ProcessStatus::Stopped | ProcessStatus::Completed) {
            inner.live_processes.remove(process_id);
        }

        // Return the updated process
        let process = inner
            .processes
            .get(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;
        Ok(process.clone())
    }

    /// Pause a running process (send SIGSTOP).
    pub fn pause(&self, process_id: &str) -> Result<BackgroundProcess, String> {
        self.send_signal(process_id, ProcessSignal::Stop)
    }

    /// Resume a paused process (send SIGCONT).
    pub fn resume(&self, process_id: &str) -> Result<BackgroundProcess, String> {
        let mut inner = self.inner.lock().expect("manager lock poisoned");

        // First, get the process status
        let current_status = {
            let process = inner
                .processes
                .get(process_id)
                .ok_or_else(|| format!("process not found: {}", process_id))?;
            process.status
        };

        if current_status != ProcessStatus::Paused {
            return Err(format!("process {} is not paused (status: {})", process_id, current_status));
        }

        // Get live process info
        let live = inner
            .live_processes
            .get(process_id)
            .ok_or_else(|| format!("live process not found: {}", process_id))?
            .clone();

        Self::send_signal_to_pid(live.pid, ProcessSignal::Continue);

        // Now get mutable reference to update
        let process = inner
            .processes
            .get_mut(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;

        process.status = ProcessStatus::Running;
        process.last_signal = Some("SIGCONT".to_string());
        process.updated_at = now_secs();

        Ok(process.clone())
    }

    /// Get resource usage for a running process.
    ///
    /// Returns detailed resource usage statistics including CPU, memory,
    /// thread count, and file descriptors.
    pub fn get_resource_usage(&self, process_id: &str) -> Result<ResourceUsage, String> {
        let inner = self.inner.lock().expect("manager lock poisoned");

        let process = inner
            .processes
            .get(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;

        if process.status != ProcessStatus::Running {
            return Err(format!("process {} is not running", process_id));
        }

        let live = inner
            .live_processes
            .get(process_id)
            .ok_or_else(|| format!("live process not found: {}", process_id))?;

        Ok(self.collect_resource_usage(live.pid))
    }

    /// Collect resource usage for a PID.
    fn collect_resource_usage(&self, pid: u32) -> ResourceUsage {
        #[cfg(unix)]
        {
            use std::fs;

            // Read from /proc/[pid]/stat on Linux
            let stat_path = format!("/proc/{}/stat", pid);
            if let Ok(stat) = fs::read_to_string(&stat_path) {
                return Self::parse_proc_stat(&stat);
            }

            // Fallback: use ps command
            Self::get_usage_via_ps(pid)
        }

        #[cfg(windows)]
        {
            Self::get_usage_via_wmic(pid)
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = pid;
            ResourceUsage::default()
        }
    }

    #[cfg(unix)]
    fn parse_proc_stat(stat: &str) -> ResourceUsage {
        // Format: pid (comm) state ppid pgrp session tty_nr tpgid flags ...
        // We need fields: utime (14), stime (15), num_threads (20), vsize (23), rss (24)
        let fields: Vec<&str> = stat.split_whitespace().collect();

        let cpu_user_seconds = fields.get(13)
            .and_then(|s| s.parse::<u64>().ok())
            .map(|ticks| ticks as f64 / 100.0) // Assuming 100 Hz
            .unwrap_or(0.0);

        let cpu_system_seconds = fields.get(14)
            .and_then(|s| s.parse::<u64>().ok())
            .map(|ticks| ticks as f64 / 100.0)
            .unwrap_or(0.0);

        let num_threads = fields.get(19)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1);

        let memory_vms_bytes = fields.get(22)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let memory_rss_bytes = fields.get(23)
            .and_then(|s| s.parse::<u64>().ok())
            .map(|pages| pages * 4096) // Assuming 4KB pages
            .unwrap_or(0);

        ResourceUsage {
            cpu_user_seconds,
            cpu_system_seconds,
            memory_rss_bytes,
            memory_vms_bytes,
            cpu_percent: 0.0, // Would need previous measurement
            memory_percent: 0.0, // Would need total system memory
            num_threads,
            num_fds: 0, // Would need to count /proc/[pid]/fd
            collected_at: now_secs(),
        }
    }

    #[cfg(unix)]
    fn get_usage_via_ps(pid: u32) -> ResourceUsage {
        use std::process::Command as StdCommand;

        let output = StdCommand::new("ps")
            .args(["-p", &pid.to_string(), "-o", "pcpu,pmem,rss,vsz,nlwp"])
            .output();

        let mut usage = ResourceUsage::default();
        usage.collected_at = now_secs();

        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Skip header line
            if let Some(line) = stdout.lines().nth(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
                    usage.cpu_percent = parts[0].parse().unwrap_or(0.0);
                    usage.memory_percent = parts[1].parse().unwrap_or(0.0);
                    usage.memory_rss_bytes = parts[2].parse::<u64>().unwrap_or(0) * 1024;
                    usage.memory_vms_bytes = parts[3].parse::<u64>().unwrap_or(0) * 1024;
                    usage.num_threads = parts[4].parse().unwrap_or(1);
                }
            }
        }

        usage
    }

    #[cfg(windows)]
    fn get_usage_via_wmic(pid: u32) -> ResourceUsage {
        use std::process::Command as StdCommand;

        let output = StdCommand::new("wmic")
            .args(["process", "where", &format!("ProcessId={}", pid), "get", "WorkingSetSize,PageFileUsage"])
            .output();

        let mut usage = ResourceUsage::default();
        usage.collected_at = now_secs();

        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Parse WMIC output
            for line in stdout.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    usage.memory_rss_bytes = parts[0].parse().unwrap_or(0);
                    usage.memory_vms_bytes = parts[1].parse().unwrap_or(0) * 1024;
                    break;
                }
            }
        }

        usage
    }

    /// Get incremental output from a process since a given position.
    ///
    /// This allows reading new output without re-reading the entire file.
    pub fn get_incremental_output(
        &self,
        process_id: &str,
        stream: &str,
        position: Option<OutputPosition>,
    ) -> Result<IncrementalOutput, String> {
        let inner = self.inner.lock().expect("manager lock poisoned");

        let process = inner
            .processes
            .get(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;

        let path = match stream {
            "stdout" => process.stdout_path.as_ref(),
            "stderr" => process.stderr_path.as_ref(),
            _ => return Err(format!("invalid stream: {}. Use 'stdout' or 'stderr'", stream)),
        };

        let path = path.ok_or_else(|| "output file not available".to_string())?;

        // Determine starting position
        let start_offset = position.as_ref().map(|p| p.offset).unwrap_or(0);
        let start_line = position.as_ref().map(|p| p.line_number).unwrap_or(0);

        // Read new content
        let file = File::open(path).map_err(|e| format!("failed to open output: {}", e))?;
        let metadata = file.metadata().map_err(|e| format!("failed to get metadata: {}", e))?;
        let file_size = metadata.len();

        if start_offset >= file_size {
            // No new content
            return Ok(IncrementalOutput {
                process_id: process_id.to_string(),
                stream: stream.to_string(),
                content: String::new(),
                new_position: OutputPosition {
                    process_id: process_id.to_string(),
                    stream: stream.to_string(),
                    offset: file_size,
                    line_number: start_line,
                },
                eof: process.status != ProcessStatus::Running,
            });
        }

        // Read from start_offset to end
        use std::io::{Read, Seek, SeekFrom};
        let mut file = file;
        file.seek(SeekFrom::Start(start_offset))
            .map_err(|e| format!("seek failed: {}", e))?;

        let bytes_to_read = (file_size - start_offset) as usize;
        let mut buffer = vec![0u8; bytes_to_read];
        file.read_exact(&mut buffer)
            .map_err(|e| format!("read failed: {}", e))?;

        let content = String::from_utf8_lossy(&buffer).to_string();
        let new_lines = content.matches('\n').count() as u64;

        Ok(IncrementalOutput {
            process_id: process_id.to_string(),
            stream: stream.to_string(),
            content,
            new_position: OutputPosition {
                process_id: process_id.to_string(),
                stream: stream.to_string(),
                offset: file_size,
                line_number: start_line + new_lines,
            },
            eof: process.status != ProcessStatus::Running,
        })
    }

    /// Wait for a process to complete with optional timeout.
    ///
    /// Blocks until the process finishes or timeout is reached.
    pub fn wait_for_completion(
        &self,
        process_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<BackgroundProcess, String> {
        let start = Instant::now();
        let timeout_duration = timeout_ms.map(Duration::from_millis);

        loop {
            let process = self.refresh_status(process_id)?;

            if process.status != ProcessStatus::Running {
                return Ok(process);
            }

            // Check timeout
            if let Some(timeout) = timeout_duration {
                if start.elapsed() >= timeout {
                    return Err(format!("wait timeout exceeded: {} ms", timeout_ms.unwrap()));
                }
            }

            // Small sleep to avoid busy-waiting
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Get child processes of a background process.
    ///
    /// Returns information about all child processes spawned by the given process.
    pub fn get_child_processes(&self, process_id: &str) -> Result<Vec<ChildProcessInfo>, String> {
        let inner = self.inner.lock().expect("manager lock poisoned");

        let process = inner
            .processes
            .get(process_id)
            .ok_or_else(|| format!("process not found: {}", process_id))?;

        let pid = process.pid.ok_or_else(|| "process has no PID".to_string())?;

        #[cfg(unix)]
        {
            Self::get_child_processes_unix(pid)
        }

        #[cfg(windows)]
        {
            Self::get_child_processes_windows(pid)
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = pid;
            Ok(Vec::new())
        }
    }

    #[cfg(unix)]
    fn get_child_processes_unix(parent_pid: u32) -> Result<Vec<ChildProcessInfo>, String> {
        use std::process::Command as StdCommand;

        // Use ps to get process tree
        let output = StdCommand::new("ps")
            .args(["-e", "-o", "pid,ppid,comm,args,stat,rss,etime"])
            .output()
            .map_err(|e| format!("failed to run ps: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut children = Vec::new();

        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                let pid: u32 = parts[0].parse().unwrap_or(0);
                let ppid: u32 = parts[1].parse().unwrap_or(0);

                if ppid == parent_pid {
                    children.push(ChildProcessInfo {
                        pid,
                        parent_pid,
                        name: parts[2].to_string(),
                        command: parts.get(3).unwrap_or(&"").to_string(),
                        state: parts.get(4).unwrap_or(&"?").to_string(),
                        memory_bytes: parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0) * 1024,
                        cpu_seconds: 0.0, // Would need more complex parsing
                        start_time: 0,
                    });
                }
            }
        }

        Ok(children)
    }

    #[cfg(windows)]
    fn get_child_processes_windows(parent_pid: u32) -> Result<Vec<ChildProcessInfo>, String> {
        use std::process::Command as StdCommand;

        let output = StdCommand::new("wmic")
            .args(["process", "where", &format!("ParentProcessId={}", parent_pid), "get", "ProcessId,Name,CommandLine,WorkingSetSize"])
            .output()
            .map_err(|e| format!("failed to run wmic: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut children = Vec::new();

        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                children.push(ChildProcessInfo {
                    pid: parts.last().and_then(|s| s.parse().ok()).unwrap_or(0),
                    parent_pid,
                    name: parts[0].to_string(),
                    command: parts.get(1).unwrap_or(&"").to_string(),
                    state: "running".to_string(),
                    memory_bytes: 0,
                    cpu_seconds: 0.0,
                    start_time: 0,
                });
            }
        }

        Ok(children)
    }
}

/// Writes output to a file, handling line buffering.
pub struct OutputWriter {
    writer: BufWriter<File>,
}

impl OutputWriter {
    /// Creates a new output writer for the given path.
    pub fn new(path: &PathBuf) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            writer: BufWriter::new(file),
        })
    }

    /// Writes a line of output.
    pub fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    /// Writes raw bytes.
    pub fn write_bytes(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn creates_manager() {
        let manager = BackgroundProcessManager::new();
        assert!(manager.is_empty());
    }

    #[test]
    fn registers_process() {
        let manager = BackgroundProcessManager::new();
        let temp_dir = std::env::temp_dir();

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 0.1 && echo hello")
            .spawn()
            .expect("failed to spawn");

        let process = manager
            .register(&mut child, "sleep 0.1 && echo hello", Some("test"), false, &temp_dir)
            .expect("register should succeed");

        assert!(process.process_id.starts_with("bgp_"));
        assert_eq!(process.status, ProcessStatus::Running);
        assert!(process.pid.is_some());
        assert!(process.stdout_path.is_some());

        // Clean up
        let _ = child.wait();
    }

    #[test]
    fn lists_processes() {
        let manager = BackgroundProcessManager::new();
        let temp_dir = std::env::temp_dir();

        let mut child1 = Command::new("sh")
            .arg("-c")
            .arg("sleep 0.1")
            .spawn()
            .expect("failed to spawn");

        let mut child2 = Command::new("sh")
            .arg("-c")
            .arg("echo done")
            .spawn()
            .expect("failed to spawn");

        let p1 = manager
            .register(&mut child1, "sleep 0.1", None, false, &temp_dir)
            .expect("register");
        let p2 = manager
            .register(&mut child2, "echo done", None, false, &temp_dir)
            .expect("register");

        let all = manager.list(None);
        assert_eq!(all.len(), 2);

        let running = manager.list(Some(ProcessStatus::Running));
        assert_eq!(running.len(), 2);

        // Clean up
        let _ = child1.wait();
        let _ = child2.wait();
    }

    #[test]
    fn stops_process() {
        let manager = BackgroundProcessManager::new();
        let temp_dir = std::env::temp_dir();

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 10")
            .spawn()
            .expect("failed to spawn");

        let process = manager
            .register(&mut child, "sleep 10", None, false, &temp_dir)
            .expect("register");

        // Give it a moment to start
        std::thread::sleep(Duration::from_millis(50));

        let stopped = manager.stop(&process.process_id).expect("stop should succeed");
        assert_eq!(stopped.status, ProcessStatus::Stopped);

        // Stopping again should fail
        let result = manager.stop(&process.process_id);
        assert!(result.is_err());
    }

    #[test]
    fn gets_process() {
        let manager = BackgroundProcessManager::new();
        let temp_dir = std::env::temp_dir();

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("echo test")
            .spawn()
            .expect("failed to spawn");

        let registered = manager
            .register(&mut child, "echo test", Some("a test"), false, &temp_dir)
            .expect("register");

        let fetched = manager.get(&registered.process_id).expect("should exist");
        assert_eq!(fetched.command, "echo test");
        assert_eq!(fetched.description, Some("a test".to_string()));

        let _ = child.wait();
    }

    #[test]
    fn removes_completed_process() {
        let manager = BackgroundProcessManager::new();
        let temp_dir = std::env::temp_dir();

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("echo done")
            .spawn()
            .expect("failed to spawn");

        let process = manager
            .register(&mut child, "echo done", None, false, &temp_dir)
            .expect("register");

        // Wait for completion
        let _ = child.wait();

        // Refresh status to mark as completed
        let _ = manager.refresh_status(&process.process_id);

        // Now removal should succeed
        let removed = manager.remove(&process.process_id).expect("remove should succeed");
        assert!(manager.get(&process.process_id).is_none());
    }

    #[test]
    fn cannot_remove_running_process() {
        let manager = BackgroundProcessManager::new();
        let temp_dir = std::env::temp_dir();

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 1")
            .spawn()
            .expect("failed to spawn");

        let process = manager
            .register(&mut child, "sleep 1", None, false, &temp_dir)
            .expect("register");

        let result = manager.remove(&process.process_id);
        assert!(result.is_err());

        // Clean up
        let _ = child.kill();
        let _ = child.wait();
    }
}
