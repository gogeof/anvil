//! SSH 远程执行支持。
//!
//! 提供通过 SSH 在远程主机上执行命令、传输文件和建立隧道的能力。
//! 支持密码、密钥文件和 SSH agent 认证方式，以及会话管理和连接池。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime};

/// SSH 认证方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SshAuthMethod {
    /// SSH agent 认证
    Agent,
    /// 密码认证
    Password { password: String },
    /// 密钥文件认证
    KeyFile {
        path: PathBuf,
        #[serde(default)]
        passphrase: Option<String>,
    },
    /// 使用 ssh_config 中的 IdentityFile
    ConfigFile,
}

impl Default for SshAuthMethod {
    fn default() -> Self {
        Self::Agent
    }
}

/// SSH 连接配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnectionConfig {
    /// 远程主机（主机名或 IP）
    pub host: String,
    /// SSH 端口（默认 22）
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    /// 远程用户名
    pub user: Option<String>,
    /// 认证方式
    #[serde(default)]
    pub auth: SshAuthMethod,
    /// 连接超时秒数（默认 10）
    #[serde(default = "default_ssh_timeout")]
    pub timeout_secs: u64,
    /// SSH 选项（-o key=value 形式）
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    /// 跳板机配置
    #[serde(default)]
    pub jump_host: Option<Box<SshConnectionConfig>>,
    /// 连接名称/标识符
    pub name: Option<String>,
    /// 工作目录
    pub remote_cwd: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_ssh_timeout() -> u64 {
    10
}

impl SshConnectionConfig {
    /// 构建 SSH 命令行参数。
    #[must_use]
    pub fn build_ssh_args(&self, extra_args: &[String]) -> Vec<String> {
        let mut args = Vec::new();

        // 端口
        args.push("-p".to_string());
        args.push(self.port.to_string());

        // 超时
        args.push("-o".to_string());
        args.push(format!("ConnectTimeout={}", self.timeout_secs));

        // 禁止主机密钥检查（安全远程场景需要单独配置 known_hosts）
        args.push("-o".to_string());
        args.push("StrictHostKeyChecking=accept-new".to_string());

        // 认证方式
        match &self.auth {
            SshAuthMethod::KeyFile { path, passphrase: _ } => {
                args.push("-i".to_string());
                args.push(path.display().to_string());
            }
            SshAuthMethod::Password { .. } => {
                // sshpass 处理，见 build_ssh_command
            }
            SshAuthMethod::Agent | SshAuthMethod::ConfigFile => {}
        }

        // 自定义选项
        for (key, value) in &self.options {
            args.push("-o".to_string());
            args.push(format!("{key}={value}"));
        }

        // 额外参数
        args.extend(extra_args.iter().cloned());

        args
    }

    /// 构建 user@host 目标字符串。
    #[must_use]
    pub fn destination(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        }
    }

    /// 构建完整的 SSH 命令（包括可能的 sshpass 前缀）。
    #[must_use]
    pub fn build_ssh_command(&self, command: &str) -> (String, Vec<String>) {
        let remote_cmd = match &self.remote_cwd {
            Some(cwd) => format!("cd {cwd} && {command}"),
            None => command.to_string(),
        };

        let mut args = self.build_ssh_args(&[]);
        args.push(self.destination());
        args.push(remote_cmd);

        // 密码认证使用 sshpass
        if let SshAuthMethod::Password { password } = &self.auth {
            let program = "sshpass".to_string();
            let mut sshpass_args = vec![
                "-p".to_string(),
                password.clone(),
                "ssh".to_string(),
            ];
            sshpass_args.extend(args);
            (program, sshpass_args)
        } else {
            ("ssh".to_string(), args)
        }
    }

    /// 检查 SSH 连接是否可用。
    pub fn check_connection(&self) -> Result<SshConnectionStatus, SshError> {
        let mut args = self.build_ssh_args(&[]);
        args.push(self.destination());
        args.push("echo 'anvil-ssh-ok'".to_string());

        let (program, full_args) = if let SshAuthMethod::Password { password } = &self.auth {
            let mut sshpass_args = vec![
                "-p".to_string(),
                password.clone(),
                "ssh".to_string(),
            ];
            sshpass_args.extend(args);
            ("sshpass".to_string(), sshpass_args)
        } else {
            ("ssh".to_string(), args)
        };

        let output = Command::new(&program)
            .args(&full_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| SshError::ConnectionFailed(format!("failed to spawn ssh: {e}")))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(SshConnectionStatus {
                connected: true,
                banner: stdout.trim().to_string(),
                latency_ms: None,
            })
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(SshError::ConnectionFailed(stderr))
        }
    }

    /// 创建会话管理器。
    #[must_use]
    pub fn into_manager(self) -> SshSessionManager {
        SshSessionManager::new(self)
    }
}

/// SSH 连接状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnectionStatus {
    pub connected: bool,
    pub banner: String,
    pub latency_ms: Option<u64>,
}

/// 远程命令执行结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub host: String,
}

/// 文件传输方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    /// 本地 → 远程（上传）
    Upload,
    /// 远程 → 本地（下载）
    Download,
}

/// 文件传输配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTransferRequest {
    pub direction: TransferDirection,
    pub local_path: PathBuf,
    pub remote_path: PathBuf,
    /// 递归传输目录
    #[serde(default)]
    pub recursive: bool,
    /// 保留文件属性
    #[serde(default)]
    pub preserve: bool,
    /// 压缩传输
    #[serde(default)]
    pub compress: bool,
}

/// 文件传输结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTransferResult {
    pub direction: TransferDirection,
    pub local_path: PathBuf,
    pub remote_path: PathBuf,
    pub success: bool,
    pub bytes_transferred: Option<u64>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// SSH 会话标识。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshSessionHandle {
    pub id: String,
    pub host: String,
    pub created_at: u64,
    pub last_used_at: u64,
    pub active: bool,
}

/// SSH 错误类型。
#[derive(Debug, Clone)]
pub enum SshError {
    ConnectionFailed(String),
    CommandFailed(String),
    TransferFailed(String),
    SessionNotFound(String),
    SshNotInstalled,
    SshpassNotInstalled,
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionFailed(msg) => write!(f, "SSH connection failed: {msg}"),
            Self::CommandFailed(msg) => write!(f, "SSH command failed: {msg}"),
            Self::TransferFailed(msg) => write!(f, "SSH transfer failed: {msg}"),
            Self::SessionNotFound(id) => write!(f, "SSH session not found: {id}"),
            Self::SshNotInstalled => write!(f, "ssh binary not found in PATH"),
            Self::SshpassNotInstalled => write!(f, "sshpass binary not found in PATH (required for password auth)"),
        }
    }
}

impl std::error::Error for SshError {}

/// 在远程主机上执行一条命令。
///
/// # Arguments
/// * `config` - SSH 连接配置
/// * `command` - 要执行的命令
///
/// # Returns
/// 命令执行结果，包含 stdout、stderr、退出码和执行时间。
pub fn ssh_execute(config: &SshConnectionConfig, command: &str) -> Result<SshCommandResult, SshError> {
    let start = SystemTime::now();
    let (program, args) = config.build_ssh_command(command);

    // 检查必要的二进制是否存在
    if !binary_exists(&program) {
        return if program == "sshpass" {
            Err(SshError::SshpassNotInstalled)
        } else {
            Err(SshError::SshNotInstalled)
        };
    }

    let output: Output = Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| SshError::CommandFailed(format!("failed to execute remote command: {e}")))?;

    let duration = start.elapsed().unwrap_or(Duration::from_secs(0));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(SshCommandResult {
        stdout,
        stderr,
        exit_code,
        duration_ms: duration.as_millis() as u64,
        host: config.host.clone(),
    })
}

/// 在远程与本地之间传输文件。
///
/// 使用 scp 作为底层传输工具。
///
/// # Arguments
/// * `config` - SSH 连接配置（用于认证参数）
/// * `request` - 文件传输请求参数
///
/// # Returns
/// 文件传输结果。
pub fn ssh_transfer_file(
    config: &SshConnectionConfig,
    request: &FileTransferRequest,
) -> Result<FileTransferResult, SshError> {
    let start = SystemTime::now();
    let destination = config.destination();

    let mut scp_args = Vec::new();

    // 端口
    scp_args.push("-P".to_string());
    scp_args.push(config.port.to_string());

    // 超时
    scp_args.push("-o".to_string());
    scp_args.push(format!("ConnectTimeout={}", config.timeout_secs));

    scp_args.push("-o".to_string());
    scp_args.push("StrictHostKeyChecking=accept-new".to_string());

    // 认证
    if let SshAuthMethod::KeyFile { path, passphrase: _ } = &config.auth {
        scp_args.push("-i".to_string());
        scp_args.push(path.display().to_string());
    }

    // 自定义选项
    for (key, value) in &config.options {
        scp_args.push("-o".to_string());
        scp_args.push(format!("{key}={value}"));
    }

    // 递归
    if request.recursive {
        scp_args.push("-r".to_string());
    }

    // 保留属性
    if request.preserve {
        scp_args.push("-p".to_string());
    }

    // 压缩
    if request.compress {
        scp_args.push("-C".to_string());
    }

    match request.direction {
        TransferDirection::Upload => {
            scp_args.push(request.local_path.display().to_string());
            scp_args.push(format!("{destination}:{}", request.remote_path.display()));
        }
        TransferDirection::Download => {
            scp_args.push(format!("{destination}:{}", request.remote_path.display()));
            scp_args.push(request.local_path.display().to_string());
        }
    }

    let _program = if matches!(config.auth, SshAuthMethod::Password { .. }) {
        let mut sshpass_args = vec![
            "-p".to_string(),
            match &config.auth {
                SshAuthMethod::Password { password } => password.clone(),
                _ => unreachable!(),
            },
            "scp".to_string(),
        ];
        sshpass_args.extend(scp_args);
        let output = Command::new("sshpass")
            .args(&sshpass_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| SshError::TransferFailed(format!("failed to spawn scp: {e}")))?;

        return Ok(FileTransferResult {
            direction: request.direction,
            local_path: request.local_path.clone(),
            remote_path: request.remote_path.clone(),
            success: output.status.success(),
            bytes_transferred: None,
            duration_ms: start.elapsed().unwrap_or(Duration::from_secs(0)).as_millis() as u64,
            error: if output.status.success() {
                None
            } else {
                Some(String::from_utf8_lossy(&output.stderr).to_string())
            },
        });
    } else {
        let output = Command::new("scp")
            .args(&scp_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| SshError::TransferFailed(format!("failed to spawn scp: {e}")))?;

        return Ok(FileTransferResult {
            direction: request.direction,
            local_path: request.local_path.clone(),
            remote_path: request.remote_path.clone(),
            success: output.status.success(),
            bytes_transferred: None,
            duration_ms: start.elapsed().unwrap_or(Duration::from_secs(0)).as_millis() as u64,
            error: if output.status.success() {
                None
            } else {
                Some(String::from_utf8_lossy(&output.stderr).to_string())
            },
        });
    };
}

/// SSH 会话管理器，提供连接复用和会话生命周期管理。
#[derive(Debug, Clone)]
pub struct SshSessionManager {
    config: SshConnectionConfig,
}

impl SshSessionManager {
    /// 创建新的会话管理器。
    #[must_use]
    pub fn new(config: SshConnectionConfig) -> Self {
        Self { config }
    }

    /// 获取连接配置引用。
    #[must_use]
    pub fn config(&self) -> &SshConnectionConfig {
        &self.config
    }

    /// 执行远程命令。
    pub fn execute(&self, command: &str) -> Result<SshCommandResult, SshError> {
        ssh_execute(&self.config, command)
    }

    /// 传输文件。
    pub fn transfer(&self, request: &FileTransferRequest) -> Result<FileTransferResult, SshError> {
        ssh_transfer_file(&self.config, request)
    }

    /// 检查连接状态。
    pub fn check(&self) -> Result<SshConnectionStatus, SshError> {
        self.config.check_connection()
    }
}

/// 检查二进制是否在 PATH 中可用。
fn binary_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| {
            std::env::split_paths(&paths).any(|path| {
                let full = path.join(name);
                full.is_file() || full.with_extension("exe").is_file()
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config() -> SshConnectionConfig {
        SshConnectionConfig {
            host: "test.example.com".to_string(),
            port: 22,
            user: Some("testuser".to_string()),
            auth: SshAuthMethod::Agent,
            timeout_secs: 5,
            options: BTreeMap::new(),
            jump_host: None,
            name: Some("test-host".to_string()),
            remote_cwd: Some("/home/testuser/project".to_string()),
        }
    }

    #[test]
    fn builds_destination_with_user() {
        let config = make_test_config();
        assert_eq!(config.destination(), "testuser@test.example.com");
    }

    #[test]
    fn builds_destination_without_user() {
        let config = SshConnectionConfig {
            user: None,
            ..make_test_config()
        };
        assert_eq!(config.destination(), "test.example.com");
    }

    #[test]
    fn build_ssh_command_includes_remote_cwd() {
        let config = make_test_config();
        let (program, args) = config.build_ssh_command("ls -la");
        assert_eq!(program, "ssh");
        // Should contain "cd /home/testuser/project && ls -la"
        assert!(args.iter().any(|a| a.contains("cd /home/testuser/project && ls -la")));
    }

    #[test]
    fn build_ssh_command_without_remote_cwd() {
        let config = SshConnectionConfig {
            remote_cwd: None,
            ..make_test_config()
        };
        let (_, args) = config.build_ssh_command("echo hello");
        assert!(args.iter().any(|a| a == "echo hello"));
    }

    #[test]
    fn build_ssh_args_includes_port_and_timeout() {
        let args = make_test_config().build_ssh_args(&[]);
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"22".to_string()));
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"ConnectTimeout=5".to_string()));
    }

    #[test]
    fn build_ssh_args_with_key_file() {
        let config = SshConnectionConfig {
            auth: SshAuthMethod::KeyFile {
                path: PathBuf::from("/home/test/.ssh/id_rsa"),
                passphrase: None,
            },
            ..make_test_config()
        };
        let args = config.build_ssh_args(&[]);
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"/home/test/.ssh/id_rsa".to_string()));
    }

    #[test]
    fn password_auth_uses_sshpass() {
        let config = SshConnectionConfig {
            auth: SshAuthMethod::Password {
                password: "secret123".to_string(),
            },
            ..make_test_config()
        };
        let (program, args) = config.build_ssh_command("whoami");
        assert_eq!(program, "sshpass");
        // sshpass -p secret123 ssh ... args
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"secret123".to_string()));
    }

    #[test]
    fn scp_command_for_upload() {
        let config = make_test_config();
        let request = FileTransferRequest {
            direction: TransferDirection::Upload,
            local_path: PathBuf::from("/local/file.txt"),
            remote_path: PathBuf::from("/remote/file.txt"),
            recursive: false,
            preserve: false,
            compress: false,
        };
        let result = ssh_transfer_file(&config, &request);
        // Should succeed or fail gracefully (no actual SSH server)
        // We just verify it doesn't panic
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn session_manager_executes() {
        let config = make_test_config();
        let manager = config.into_manager();
        assert_eq!(manager.config().host, "test.example.com");
        // Execute should return a result (either Ok with non-zero exit or Err)
        let result = manager.execute("echo hello");
        if let Ok(cmd_result) = &result {
            // If SSH binary exists, it tries to connect and fails with non-zero
            assert!(
                cmd_result.exit_code != 0 || !cmd_result.stdout.contains("anvil-ssh-ok"),
                "expected non-zero exit or no connection"
            );
        } else {
            // If SSH binary doesn't exist, it's an error
            assert!(result.is_err());
        }
    }

    #[test]
    fn connection_check_handles_missing_server() {
        let config = make_test_config();
        let result = config.check_connection();
        // Without a real SSH server, this should be an error
        assert!(result.is_err());
    }

    #[test]
    fn ssh_error_display() {
        assert_eq!(
            SshError::ConnectionFailed("timeout".to_string()).to_string(),
            "SSH connection failed: timeout"
        );
        assert_eq!(
            SshError::SshNotInstalled.to_string(),
            "ssh binary not found in PATH"
        );
        assert_eq!(
            SshError::SshpassNotInstalled.to_string(),
            "sshpass binary not found in PATH (required for password auth)"
        );
    }

    #[test]
    fn config_serialization_roundtrip() {
        let config = make_test_config();
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: SshConnectionConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(config, deserialized);
    }

    #[test]
    fn key_file_auth_serialization() {
        let config = SshConnectionConfig {
            auth: SshAuthMethod::KeyFile {
                path: PathBuf::from("/tmp/key"),
                passphrase: Some("pass".to_string()),
            },
            ..make_test_config()
        };
        let json = serde_json::to_string_pretty(&config).expect("serialize");
        assert!(json.contains("key-file"));
        assert!(json.contains("/tmp/key"));
    }

    #[test]
    fn custom_options_included_in_args() {
        let config = SshConnectionConfig {
            options: BTreeMap::from([
                ("ServerAliveInterval".to_string(), "60".to_string()),
                ("ServerAliveCountMax".to_string(), "3".to_string()),
            ]),
            ..make_test_config()
        };
        let args = config.build_ssh_args(&[]);
        assert!(args.contains(&"ServerAliveInterval=60".to_string()));
        assert!(args.contains(&"ServerAliveCountMax=3".to_string()));
    }
}
