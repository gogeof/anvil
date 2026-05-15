use std::collections::HashMap;
use std::env;
use std::io;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;
use tokio::runtime::Builder;
use tokio::time::timeout;

use crate::background_judge::{BackgroundDecision, BackgroundJudge};
use crate::background_process::{
    BackgroundProcessManager, ProcessStatus, ResourceLimits, RestartPolicy,
};
use crate::lane_events::{LaneEvent, ShipMergeMethod, ShipProvenance};
use crate::sandbox::{
    build_linux_sandbox_command, resolve_sandbox_status_for_request, FilesystemIsolationMode,
    SandboxConfig, SandboxStatus,
};
use crate::telemetry::Telemetry;
use crate::ConfigLoader;

/// Global background process manager instance.
static PROCESS_MANAGER: OnceLock<BackgroundProcessManager> = OnceLock::new();

/// Global background judge instance for auto-detecting background execution.
static BACKGROUND_JUDGE: OnceLock<std::sync::Mutex<BackgroundJudge>> = OnceLock::new();

/// Get the global process manager, initializing if necessary.
pub fn process_manager() -> &'static BackgroundProcessManager {
    PROCESS_MANAGER.get_or_init(BackgroundProcessManager::new)
}

/// Get the global background judge, initializing if necessary.
pub fn background_judge() -> &'static std::sync::Mutex<BackgroundJudge> {
    BACKGROUND_JUDGE.get_or_init(|| {
        let judge = BackgroundJudge::default()
            .with_history_path(BackgroundJudge::default_history_path());
        std::sync::Mutex::new(judge)
    })
}

/// Resource limits input in user-friendly format.
///
/// These limits are applied to the process to constrain resource usage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLimitsInput {
    /// Maximum CPU time in seconds.
    pub cpu_time_seconds: Option<u64>,
    /// Maximum memory in megabytes.
    pub memory_mb: Option<u64>,
    /// Maximum number of open file descriptors.
    pub max_files: Option<u64>,
    /// Maximum number of child processes.
    pub max_processes: Option<u64>,
    /// Maximum file size that can be created (MB).
    pub max_file_size_mb: Option<u64>,
}

impl From<ResourceLimitsInput> for ResourceLimits {
    fn from(input: ResourceLimitsInput) -> Self {
        Self {
            cpu_time_seconds: input.cpu_time_seconds,
            memory_mb: input.memory_mb,
            max_files: input.max_files,
            max_processes: input.max_processes,
            max_file_size_mb: input.max_file_size_mb,
        }
    }
}

/// Restart policy input configuration.
///
/// Controls automatic restart behavior when a background process exits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestartPolicyInput {
    /// Restart policy type.
    pub policy: RestartPolicy,
    /// Maximum restart attempts (ignored for Unlimited policy).
    #[serde(default)]
    pub max_attempts: u32,
    /// Delay between restart attempts in milliseconds.
    #[serde(default)]
    pub delay_ms: u64,
    /// Exponential backoff multiplier for delay (1.0 = no backoff).
    #[serde(default)]
    pub backoff_multiplier: f64,
    /// Maximum delay cap in milliseconds.
    pub max_delay_ms: Option<u64>,
}

impl Default for RestartPolicyInput {
    fn default() -> Self {
        Self {
            policy: RestartPolicy::Never,
            max_attempts: 3,
            delay_ms: 0,
            backoff_multiplier: 2.0,
            max_delay_ms: None,
        }
    }
}

/// Process priority for scheduling.
///
/// Higher values indicate higher priority. On Unix, this maps to nice values
/// (inverted: higher priority = lower nice value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProcessPriority(pub i32);

impl Default for ProcessPriority {
    fn default() -> Self {
        Self(0)
    }
}

/// Input schema for the built-in bash execution tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BashCommandInput {
    pub command: String,
    pub timeout: Option<u64>,
    pub description: Option<String>,
    #[serde(rename = "run_in_background")]
    pub run_in_background: Option<bool>,
    #[serde(rename = "dangerouslyDisableSandbox")]
    pub dangerously_disable_sandbox: Option<bool>,
    #[serde(rename = "namespaceRestrictions")]
    pub namespace_restrictions: Option<bool>,
    #[serde(rename = "isolateNetwork")]
    pub isolate_network: Option<bool>,
    #[serde(rename = "filesystemMode")]
    pub filesystem_mode: Option<FilesystemIsolationMode>,
    #[serde(rename = "allowedMounts")]
    pub allowed_mounts: Option<Vec<String>>,
}

/// Output returned from a bash tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BashCommandOutput {
    pub stdout: String,
    pub stderr: String,
    #[serde(rename = "rawOutputPath")]
    pub raw_output_path: Option<String>,
    pub interrupted: bool,
    #[serde(rename = "isImage")]
    pub is_image: Option<bool>,
    #[serde(rename = "backgroundTaskId")]
    pub background_task_id: Option<String>,
    #[serde(rename = "backgroundedByUser")]
    pub backgrounded_by_user: Option<bool>,
    #[serde(rename = "assistantAutoBackgrounded")]
    pub assistant_auto_backgrounded: Option<bool>,
    #[serde(rename = "dangerouslyDisableSandbox")]
    pub dangerously_disable_sandbox: Option<bool>,
    #[serde(rename = "returnCodeInterpretation")]
    pub return_code_interpretation: Option<String>,
    #[serde(rename = "noOutputExpected")]
    pub no_output_expected: Option<bool>,
    #[serde(rename = "structuredContent")]
    pub structured_content: Option<Vec<serde_json::Value>>,
    #[serde(rename = "persistedOutputPath")]
    pub persisted_output_path: Option<String>,
    #[serde(rename = "persistedOutputSize")]
    pub persisted_output_size: Option<u64>,
    #[serde(rename = "sandboxStatus")]
    pub sandbox_status: Option<SandboxStatus>,
}

/// Executes a shell command with the requested sandbox settings.
pub fn execute_bash(input: BashCommandInput) -> io::Result<BashCommandOutput> {
    let cwd = env::current_dir()?;
    let sandbox_status = sandbox_status_for_input(&input, &cwd);

    // Check if user explicitly requested background execution
    let user_requested_background = input.run_in_background.unwrap_or(false);

    // Auto-detect if command should run in background
    let auto_background_decision = if !user_requested_background {
        let judge = background_judge();
        let judge_guard = judge.lock().unwrap();
        judge_guard.should_run_background(&input.command)
    } else {
        BackgroundDecision::Always
    };

    // Decide whether to run in background
    let should_run_background = user_requested_background
        || matches!(auto_background_decision, BackgroundDecision::Always);

    let start_time = Instant::now();
    let result = if should_run_background {
        execute_bash_background(input.clone(), sandbox_status.clone(), cwd.clone())
    } else {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(execute_bash_async(
            input.clone(),
            sandbox_status.clone(),
            cwd.clone(),
        ))
    };

    // Record execution time for learning
    let duration_ms = start_time.elapsed().as_millis() as u64;
    let exit_code = match &result {
        Ok(output) => output.return_code_interpretation.as_ref().and_then(|s| {
            s.strip_prefix("exit_code:")
                .and_then(|n| n.parse::<i32>().ok())
        }).unwrap_or(0),
        Err(_) => 1,
    };

    // Record telemetry for metrics
    let success = exit_code == 0;
    let mut context = HashMap::new();
    context.insert("exit_code".to_string(), serde_json::json!(exit_code));
    context.insert("auto_background".to_string(), serde_json::json!(!user_requested_background && should_run_background));
    Telemetry::new().record(
        "bash",
        Duration::from_millis(duration_ms),
        success,
        context,
    );

    // Update execution history (async, don't block on error)
    if let Ok(judge) = background_judge().lock() {
        let mut judge = judge;
        judge.record_execution(&input.command, duration_ms, exit_code);
        let _ = judge.save_history();
    }

    // Mark output with auto-background info
    let mut result = result?;
    if !user_requested_background && should_run_background {
        result.assistant_auto_backgrounded = Some(true);
    }

    Ok(result)
}

/// Execute a command in the background with output capture.
fn execute_bash_background(
    input: BashCommandInput,
    sandbox_status: SandboxStatus,
    cwd: std::path::PathBuf,
) -> io::Result<BashCommandOutput> {
    let output_dir = std::env::temp_dir().join(".anvil").join("background");
    std::fs::create_dir_all(&output_dir)?;

    let manager = process_manager();
    manager.set_output_dir(output_dir);

    // Create output files for capturing stdout/stderr
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stdout_path = std::env::temp_dir()
        .join(".anvil")
        .join("background")
        .join(format!("bg_{}.stdout", timestamp));
    let stderr_path = std::env::temp_dir()
        .join(".anvil")
        .join("background")
        .join(format!("bg_{}.stderr", timestamp));

    // Create the output files
    std::fs::File::create(&stdout_path)?;
    std::fs::File::create(&stderr_path)?;

    let stdout_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_path)?;
    let stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)?;

    let mut child = prepare_command(&input.command, &cwd, &sandbox_status, false);
    let mut child = child
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()?;

    let sandbox_enabled = !input.dangerously_disable_sandbox.unwrap_or(false);
    let process = manager.register(
        &mut child,
        &input.command,
        input.description.as_deref(),
        sandbox_enabled,
        &cwd,
    )?;

    Ok(BashCommandOutput {
        stdout: String::new(),
        stderr: String::new(),
        raw_output_path: Some(stdout_path.to_string_lossy().to_string()),
        interrupted: false,
        is_image: None,
        background_task_id: Some(process.process_id),
        backgrounded_by_user: Some(true),
        assistant_auto_backgrounded: Some(false),
        dangerously_disable_sandbox: input.dangerously_disable_sandbox,
        return_code_interpretation: None,
        no_output_expected: Some(true),
        structured_content: None,
        persisted_output_path: Some(stdout_path.to_string_lossy().to_string()),
        persisted_output_size: Some(0),
        sandbox_status: Some(sandbox_status),
    })
}

/// Detect git push to main and emit ship provenance event
fn detect_and_emit_ship_prepared(command: &str) {
    let trimmed = command.trim();
    // Simple detection: git push with main/master
    if trimmed.contains("git push") && (trimmed.contains("main") || trimmed.contains("master")) {
        // Emit ship.prepared event
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let provenance = ShipProvenance {
            source_branch: get_current_branch().unwrap_or_else(|| "unknown".to_string()),
            base_commit: get_head_commit().unwrap_or_default(),
            commit_count: 0, // Would need to calculate from range
            commit_range: "unknown..HEAD".to_string(),
            merge_method: ShipMergeMethod::DirectPush,
            actor: get_git_actor().unwrap_or_else(|| "unknown".to_string()),
            pr_number: None,
        };
        let _event = LaneEvent::ship_prepared(format!("{now}"), &provenance);
        // Log to stderr as interim routing before event stream integration
        eprintln!(
            "[ship.prepared] branch={} -> main, commits={}, actor={}",
            provenance.source_branch, provenance.commit_count, provenance.actor
        );
    }
}

fn get_current_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn get_head_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn get_git_actor() -> Option<String> {
    let name = Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;
    Some(name)
}

async fn execute_bash_async(
    input: BashCommandInput,
    sandbox_status: SandboxStatus,
    cwd: std::path::PathBuf,
) -> io::Result<BashCommandOutput> {
    // Detect and emit ship provenance for git push operations
    detect_and_emit_ship_prepared(&input.command);

    let mut command = prepare_tokio_command(&input.command, &cwd, &sandbox_status, true);

    let output_result = if let Some(timeout_ms) = input.timeout {
        match timeout(Duration::from_millis(timeout_ms), command.output()).await {
            Ok(result) => (result?, false),
            Err(_) => {
                return Ok(BashCommandOutput {
                    stdout: String::new(),
                    stderr: format!("Command exceeded timeout of {timeout_ms} ms"),
                    raw_output_path: None,
                    interrupted: true,
                    is_image: None,
                    background_task_id: None,
                    backgrounded_by_user: None,
                    assistant_auto_backgrounded: None,
                    dangerously_disable_sandbox: input.dangerously_disable_sandbox,
                    return_code_interpretation: Some(String::from("timeout")),
                    no_output_expected: Some(true),
                    structured_content: None,
                    persisted_output_path: None,
                    persisted_output_size: None,
                    sandbox_status: Some(sandbox_status),
                });
            }
        }
    } else {
        (command.output().await?, false)
    };

    let (output, interrupted) = output_result;
    let stdout = truncate_output(&String::from_utf8_lossy(&output.stdout));
    let stderr = truncate_output(&String::from_utf8_lossy(&output.stderr));
    let no_output_expected = Some(stdout.trim().is_empty() && stderr.trim().is_empty());
    let return_code_interpretation = output.status.code().and_then(|code| {
        if code == 0 {
            None
        } else {
            Some(format!("exit_code:{code}"))
        }
    });

    Ok(BashCommandOutput {
        stdout,
        stderr,
        raw_output_path: None,
        interrupted,
        is_image: None,
        background_task_id: None,
        backgrounded_by_user: None,
        assistant_auto_backgrounded: None,
        dangerously_disable_sandbox: input.dangerously_disable_sandbox,
        return_code_interpretation,
        no_output_expected,
        structured_content: None,
        persisted_output_path: None,
        persisted_output_size: None,
        sandbox_status: Some(sandbox_status),
    })
}

fn sandbox_status_for_input(input: &BashCommandInput, cwd: &std::path::Path) -> SandboxStatus {
    let config = ConfigLoader::default_for(cwd).load().map_or_else(
        |_| SandboxConfig::default(),
        |runtime_config| runtime_config.sandbox().clone(),
    );
    let request = config.resolve_request(
        input.dangerously_disable_sandbox.map(|disabled| !disabled),
        input.namespace_restrictions,
        input.isolate_network,
        input.filesystem_mode,
        input.allowed_mounts.clone(),
    );
    resolve_sandbox_status_for_request(&request, cwd)
}

fn prepare_command(
    command: &str,
    cwd: &std::path::Path,
    sandbox_status: &SandboxStatus,
    create_dirs: bool,
) -> Command {
    if create_dirs {
        prepare_sandbox_dirs(cwd);
    }

    if let Some(launcher) = build_linux_sandbox_command(command, cwd, sandbox_status) {
        let mut prepared = Command::new(launcher.program);
        prepared.args(launcher.args);
        prepared.current_dir(cwd);
        prepared.envs(launcher.env);
        return prepared;
    }

    let mut prepared = Command::new("sh");
    prepared.arg("-lc").arg(command).current_dir(cwd);
    if sandbox_status.filesystem_active {
        prepared.env("HOME", cwd.join(".sandbox-home"));
        prepared.env("TMPDIR", cwd.join(".sandbox-tmp"));
    }
    prepared
}

fn prepare_tokio_command(
    command: &str,
    cwd: &std::path::Path,
    sandbox_status: &SandboxStatus,
    create_dirs: bool,
) -> TokioCommand {
    if create_dirs {
        prepare_sandbox_dirs(cwd);
    }

    if let Some(launcher) = build_linux_sandbox_command(command, cwd, sandbox_status) {
        let mut prepared = TokioCommand::new(launcher.program);
        prepared.args(launcher.args);
        prepared.current_dir(cwd);
        prepared.envs(launcher.env);
        return prepared;
    }

    let mut prepared = TokioCommand::new("sh");
    prepared.arg("-lc").arg(command).current_dir(cwd);
    if sandbox_status.filesystem_active {
        prepared.env("HOME", cwd.join(".sandbox-home"));
        prepared.env("TMPDIR", cwd.join(".sandbox-tmp"));
    }
    prepared
}

fn prepare_sandbox_dirs(cwd: &std::path::Path) {
    let _ = std::fs::create_dir_all(cwd.join(".sandbox-home"));
    let _ = std::fs::create_dir_all(cwd.join(".sandbox-tmp"));
}

#[cfg(test)]
mod tests {
    use super::{execute_bash, BashCommandInput};
    use crate::sandbox::FilesystemIsolationMode;

    #[test]
    fn executes_simple_command() {
        let output = execute_bash(BashCommandInput {
            command: String::from("printf 'hello'"),
            timeout: Some(1_000),
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(false),
            namespace_restrictions: Some(false),
            isolate_network: Some(false),
            filesystem_mode: Some(FilesystemIsolationMode::WorkspaceOnly),
            allowed_mounts: None,
        })
        .expect("bash command should execute");

        assert_eq!(output.stdout, "hello");
        assert!(!output.interrupted);
        assert!(output.sandbox_status.is_some());
    }

    #[test]
    fn disables_sandbox_when_requested() {
        let output = execute_bash(BashCommandInput {
            command: String::from("printf 'hello'"),
            timeout: Some(1_000),
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(true),
            namespace_restrictions: None,
            isolate_network: None,
            filesystem_mode: None,
            allowed_mounts: None,
        })
        .expect("bash command should execute");

        assert!(!output.sandbox_status.expect("sandbox status").enabled);
    }
}

/// Maximum output bytes before truncation (16 KiB, matching upstream).
const MAX_OUTPUT_BYTES: usize = 16_384;

/// Truncate output to `MAX_OUTPUT_BYTES`, appending a marker when trimmed.
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s.to_string();
    }
    // Find the last valid UTF-8 boundary at or before MAX_OUTPUT_BYTES
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = s[..end].to_string();
    truncated.push_str("\n\n[output truncated — exceeded 16384 bytes]");
    truncated
}

// ============================================================================
// Background Process Control API
// ============================================================================

/// Output from listing background processes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundProcessList {
    pub processes: Vec<BackgroundProcessInfo>,
    pub total: usize,
}

/// Summary information about a background process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundProcessInfo {
    #[serde(rename = "processId")]
    pub process_id: String,
    pub command: String,
    pub description: Option<String>,
    pub status: String,
    pub pid: Option<u32>,
    #[serde(rename = "exitCode")]
    pub exit_code: Option<i32>,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    #[serde(rename = "updatedAt")]
    pub updated_at: u64,
    #[serde(rename = "stdoutSize")]
    pub stdout_size: u64,
    #[serde(rename = "stderrSize")]
    pub stderr_size: u64,
}

/// Result of stopping a background process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopProcessResult {
    #[serde(rename = "processId")]
    pub process_id: String,
    pub status: String,
    pub message: String,
}

/// List all background processes.
pub fn list_background_processes() -> BackgroundProcessList {
    let manager = process_manager();
    let processes: Vec<BackgroundProcessInfo> = manager
        .list(None)
        .into_iter()
        .map(|p| BackgroundProcessInfo {
            process_id: p.process_id,
            command: p.command,
            description: p.description,
            status: p.status.to_string(),
            pid: p.pid,
            exit_code: p.exit_code,
            created_at: p.created_at,
            updated_at: p.updated_at,
            stdout_size: p.stdout_size,
            stderr_size: p.stderr_size,
        })
        .collect();
    let total = processes.len();
    BackgroundProcessList { processes, total }
}

/// List background processes filtered by status.
pub fn list_background_processes_by_status(status: ProcessStatus) -> BackgroundProcessList {
    let manager = process_manager();
    let processes: Vec<BackgroundProcessInfo> = manager
        .list(Some(status))
        .into_iter()
        .map(|p| BackgroundProcessInfo {
            process_id: p.process_id,
            command: p.command,
            description: p.description,
            status: p.status.to_string(),
            pid: p.pid,
            exit_code: p.exit_code,
            created_at: p.created_at,
            updated_at: p.updated_at,
            stdout_size: p.stdout_size,
            stderr_size: p.stderr_size,
        })
        .collect();
    let total = processes.len();
    BackgroundProcessList { processes, total }
}

/// Get information about a specific background process.
pub fn get_background_process(process_id: &str) -> Option<BackgroundProcessInfo> {
    let manager = process_manager();
    manager.get(process_id).map(|p| BackgroundProcessInfo {
        process_id: p.process_id,
        command: p.command,
        description: p.description,
        status: p.status.to_string(),
        pid: p.pid,
        exit_code: p.exit_code,
        created_at: p.created_at,
        updated_at: p.updated_at,
        stdout_size: p.stdout_size,
        stderr_size: p.stderr_size,
    })
}

/// Stop a running background process.
pub fn stop_background_process(process_id: &str) -> Result<StopProcessResult, String> {
    let manager = process_manager();
    let process = manager.stop(process_id)?;
    Ok(StopProcessResult {
        process_id: process.process_id,
        status: process.status.to_string(),
        message: "Process stopped successfully".to_string(),
    })
}

/// Refresh the status of a background process.
pub fn refresh_background_process(process_id: &str) -> Result<BackgroundProcessInfo, String> {
    let manager = process_manager();
    let process = manager.refresh_status(process_id)?;
    Ok(BackgroundProcessInfo {
        process_id: process.process_id,
        command: process.command,
        description: process.description,
        status: process.status.to_string(),
        pid: process.pid,
        exit_code: process.exit_code,
        created_at: process.created_at,
        updated_at: process.updated_at,
        stdout_size: process.stdout_size,
        stderr_size: process.stderr_size,
    })
}

/// Remove a completed background process from tracking.
pub fn remove_background_process(process_id: &str) -> Result<BackgroundProcessInfo, String> {
    let manager = process_manager();
    let process = manager.remove(process_id)?;
    Ok(BackgroundProcessInfo {
        process_id: process.process_id,
        command: process.command,
        description: process.description,
        status: process.status.to_string(),
        pid: process.pid,
        exit_code: process.exit_code,
        created_at: process.created_at,
        updated_at: process.updated_at,
        stdout_size: process.stdout_size,
        stderr_size: process.stderr_size,
    })
}

/// Refresh all running background processes.
pub fn refresh_all_background_processes() {
    let manager = process_manager();
    manager.refresh_all();
}

// ============================================================================
// Extended Background Process Control API
// ============================================================================

use crate::background_process::{
    ChildProcessInfo, IncrementalOutput, OutputPosition, ProcessSignal, ResourceUsage,
};

/// Result of sending a signal to a process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalResult {
    #[serde(rename = "processId")]
    pub process_id: String,
    pub signal: String,
    pub status: String,
    pub message: String,
}

/// Send a signal to a background process.
///
/// # Arguments
/// * `process_id` - The process ID
/// * `signal` - The signal to send (e.g., "SIGINT", "SIGTERM", "SIGKILL", "SIGSTOP", "SIGCONT", "SIGUSR1", "SIGUSR2", "SIGHUP")
pub fn send_signal_to_process(process_id: &str, signal: &str) -> Result<SignalResult, String> {
    let signal_enum = match signal.to_uppercase().as_str() {
        "SIGINT" | "INT" => ProcessSignal::Interrupt,
        "SIGTERM" | "TERM" => ProcessSignal::Terminate,
        "SIGKILL" | "KILL" => ProcessSignal::Kill,
        "SIGSTOP" | "STOP" => ProcessSignal::Stop,
        "SIGCONT" | "CONT" => ProcessSignal::Continue,
        "SIGUSR1" | "USR1" => ProcessSignal::User1,
        "SIGUSR2" | "USR2" => ProcessSignal::User2,
        "SIGHUP" | "HUP" => ProcessSignal::Hangup,
        _ => return Err(format!("unknown signal: {}", signal)),
    };

    let manager = process_manager();
    let process = manager.send_signal(process_id, signal_enum)?;

    Ok(SignalResult {
        process_id: process.process_id,
        signal: signal.to_uppercase(),
        status: process.status.to_string(),
        message: format!("Signal {} sent successfully", signal.to_uppercase()),
    })
}

/// Pause a running background process (send SIGSTOP).
pub fn pause_background_process(process_id: &str) -> Result<BackgroundProcessInfo, String> {
    let manager = process_manager();
    let process = manager.pause(process_id)?;
    Ok(BackgroundProcessInfo {
        process_id: process.process_id,
        command: process.command,
        description: process.description,
        status: process.status.to_string(),
        pid: process.pid,
        exit_code: process.exit_code,
        created_at: process.created_at,
        updated_at: process.updated_at,
        stdout_size: process.stdout_size,
        stderr_size: process.stderr_size,
    })
}

/// Resume a paused background process (send SIGCONT).
pub fn resume_background_process(process_id: &str) -> Result<BackgroundProcessInfo, String> {
    let manager = process_manager();
    let process = manager.resume(process_id)?;
    Ok(BackgroundProcessInfo {
        process_id: process.process_id,
        command: process.command,
        description: process.description,
        status: process.status.to_string(),
        pid: process.pid,
        exit_code: process.exit_code,
        created_at: process.created_at,
        updated_at: process.updated_at,
        stdout_size: process.stdout_size,
        stderr_size: process.stderr_size,
    })
}

/// Get resource usage for a running background process.
pub fn get_process_resource_usage(process_id: &str) -> Result<ResourceUsage, String> {
    let manager = process_manager();
    manager.get_resource_usage(process_id)
}

/// Get incremental output from a background process.
///
/// This reads new output since the last read position, enabling efficient
/// streaming of process output without re-reading the entire file.
///
/// # Arguments
/// * `process_id` - The process ID
/// * `stream` - Which stream to read ("stdout" or "stderr")
/// * `position` - Optional previous position to continue from
pub fn get_incremental_output(
    process_id: &str,
    stream: &str,
    position: Option<OutputPosition>,
) -> Result<IncrementalOutput, String> {
    let manager = process_manager();
    manager.get_incremental_output(process_id, stream, position)
}

/// Wait for a background process to complete.
///
/// Blocks until the process finishes or the timeout is reached.
///
/// # Arguments
/// * `process_id` - The process ID
/// * `timeout_ms` - Optional timeout in milliseconds
/// * `block` - Whether to block (true) or return immediately if still running (false)
pub fn wait_for_process(
    process_id: &str,
    timeout_ms: Option<u64>,
    block: bool,
) -> Result<BackgroundProcessInfo, String> {
    let manager = process_manager();

    let process = if block {
        manager.wait_for_completion(process_id, timeout_ms)?
    } else {
        manager.refresh_status(process_id)?
    };

    Ok(BackgroundProcessInfo {
        process_id: process.process_id,
        command: process.command,
        description: process.description,
        status: process.status.to_string(),
        pid: process.pid,
        exit_code: process.exit_code,
        created_at: process.created_at,
        updated_at: process.updated_at,
        stdout_size: process.stdout_size,
        stderr_size: process.stderr_size,
    })
}

/// Get child processes of a background process.
pub fn get_child_processes(process_id: &str) -> Result<Vec<ChildProcessInfo>, String> {
    let manager = process_manager();
    manager.get_child_processes(process_id)
}

/// Create a new process group for managing related processes together.
pub fn create_process_group(group_name: &str) -> Result<(), String> {
    let manager = process_manager();
    manager.create_process_group(group_name)
}

/// List all process groups.
pub fn list_process_groups() -> Vec<String> {
    let manager = process_manager();
    manager.list_process_groups()
}

/// Get all processes in a specific group.
pub fn get_process_group(group_name: &str) -> Option<Vec<BackgroundProcessInfo>> {
    let manager = process_manager();
    manager.get_process_group(group_name).map(|processes| {
        processes
            .into_iter()
            .map(|p| BackgroundProcessInfo {
                process_id: p.process_id,
                command: p.command,
                description: p.description,
                status: p.status.to_string(),
                pid: p.pid,
                exit_code: p.exit_code,
                created_at: p.created_at,
                updated_at: p.updated_at,
                stdout_size: p.stdout_size,
                stderr_size: p.stderr_size,
            })
            .collect()
    })
}

/// Stop all processes in a group.
pub fn stop_process_group(group_name: &str) -> Result<Vec<BackgroundProcessInfo>, String> {
    let manager = process_manager();
    let processes = manager.stop_process_group(group_name)?;
    Ok(processes
        .into_iter()
        .map(|p| BackgroundProcessInfo {
            process_id: p.process_id,
            command: p.command,
            description: p.description,
            status: p.status.to_string(),
            pid: p.pid,
            exit_code: p.exit_code,
            created_at: p.created_at,
            updated_at: p.updated_at,
            stdout_size: p.stdout_size,
            stderr_size: p.stderr_size,
        })
        .collect())
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    #[test]
    fn short_output_unchanged() {
        let s = "hello world";
        assert_eq!(truncate_output(s), s);
    }

    #[test]
    fn long_output_truncated() {
        let s = "x".repeat(20_000);
        let result = truncate_output(&s);
        assert!(result.len() < 20_000);
        assert!(result.ends_with("[output truncated — exceeded 16384 bytes]"));
    }

    #[test]
    fn exact_boundary_unchanged() {
        let s = "a".repeat(MAX_OUTPUT_BYTES);
        assert_eq!(truncate_output(&s), s);
    }

    #[test]
    fn one_over_boundary_truncated() {
        let s = "a".repeat(MAX_OUTPUT_BYTES + 1);
        let result = truncate_output(&s);
        assert!(result.contains("[output truncated"));
    }
}
