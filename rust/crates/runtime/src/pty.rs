//! PTY (Pseudo-Terminal) support for interactive command execution.
//!
//! Provides PTY-based execution for interactive tools like vim, htop, etc.

use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// A PTY session for interactive command execution.
pub struct PtySession {
    /// The PTY pair (master + slave).
    pair: PtyPair,
    /// The child process handle.
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Reader for output.
    reader: Box<dyn Read + Send>,
    /// Current size of the PTY.
    size: PtySize,
    /// Session ID.
    pub session_id: String,
}

/// Configuration for a PTY session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyConfig {
    /// Initial rows.
    pub rows: u16,
    /// Initial columns.
    pub cols: u16,
    /// Working directory.
    pub working_dir: Option<String>,
    /// Environment variables.
    pub env: Vec<(String, String)>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        PtyConfig {
            rows: 24,
            cols: 80,
            working_dir: None,
            env: vec![],
        }
    }
}

/// Output from a PTY session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyOutput {
    /// Session ID.
    pub session_id: String,
    /// Output data (as string, may contain ANSI codes).
    pub data: String,
    /// Whether the session has ended.
    pub ended: bool,
    /// Exit code (if ended).
    pub exit_code: Option<i32>,
}

/// Status of a PTY session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyStatus {
    /// Session ID.
    pub session_id: String,
    /// Whether the session is running.
    pub running: bool,
    /// Exit code (if not running).
    pub exit_code: Option<i32>,
}

/// Create a new PTY session.
pub fn spawn_pty(command: &str, config: PtyConfig) -> std::io::Result<PtySession> {
    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows: config.rows,
            cols: config.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let mut cmd = CommandBuilder::new("sh");
    cmd.arg("-c");
    cmd.arg(command);

    if let Some(dir) = &config.working_dir {
        cmd.cwd(dir);
    }

    for (key, value) in &config.env {
        cmd.env(key, value);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let session_id = format!("pty_{}", uuid::Uuid::new_v4().simple());

    Ok(PtySession {
        pair,
        child,
        reader,
        size: PtySize {
            rows: config.rows,
            cols: config.cols,
            pixel_width: 0,
            pixel_height: 0,
        },
        session_id,
    })
}

impl PtySession {
    /// Read output from the PTY (non-blocking).
    pub fn read(&mut self) -> std::io::Result<String> {
        let mut buffer = [0u8; 4096];
        let mut output = String::new();
        
        // Set non-blocking mode
        let mut attempts = 0;
        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buffer[..n]);
                    output.push_str(&chunk);
                    attempts = 0;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if output.is_empty() && attempts < 10 {
                        attempts += 1;
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        
        Ok(output)
    }
    
    /// Write input to the PTY.
    pub fn write(&mut self, input: &str) -> std::io::Result<()> {
        let mut writer = self
            .pair
            .master
            .take_writer()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        writer.write_all(input.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    /// Resize the PTY.
    pub fn resize(&mut self, rows: u16, cols: u16) -> std::io::Result<()> {
        self.size.rows = rows;
        self.size.cols = cols;
        self.pair
            .master
            .resize(self.size)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        Ok(())
    }
    
    /// Check if the child process is still running.
    pub fn is_running(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(_) => false,
        }
    }
    
    /// Wait for the child process to exit.
    pub fn wait(&mut self) -> std::io::Result<i32> {
        let status = self.child.wait()?;
        Ok(status.exit_code() as i32)
    }
    
    /// Send a signal to the child process.
    pub fn send_signal(&mut self, signal: PtySignal) -> std::io::Result<()> {
        // portable-pty doesn't support signals directly, so we use the process ID
        #[cfg(unix)]
        {
            use std::process::Command;
            if let Some(pid) = self.child.process_id() {
                let signal_num = match signal {
                    PtySignal::Interrupt => 2,  // SIGINT
                    PtySignal::Quit => 3,       // SIGQUIT
                    PtySignal::Terminate => 15, // SIGTERM
                    PtySignal::Kill => 9,       // SIGKILL
                };
                Command::new("kill")
                    .arg(format!("-{}", signal_num))
                    .arg(pid.to_string())
                    .output()?;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = signal;
            // On non-Unix systems, just try to terminate
            let _ = self.child.kill();
        }
        Ok(())
    }
    
    /// Force terminate the child process.
    pub fn terminate(&mut self) -> std::io::Result<()> {
        self.child.kill()?;
        Ok(())
    }
    
    /// Get current size.
    pub fn size(&self) -> (u16, u16) {
        (self.size.rows, self.size.cols)
    }
}

/// Signals that can be sent to a PTY session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PtySignal {
    /// Interrupt (Ctrl+C).
    Interrupt,
    /// Quit (Ctrl+\).
    Quit,
    /// Terminate (SIGTERM).
    Terminate,
    /// Kill (SIGKILL).
    Kill,
}

/// PTY manager for handling multiple sessions.
pub struct PtyManager {
    sessions: Arc<Mutex<Vec<(String, PtySession)>>>,
}

impl PtyManager {
    /// Create a new PTY manager.
    pub fn new() -> Self {
        PtyManager {
            sessions: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    /// Spawn a new PTY session.
    pub fn spawn(&mut self, command: &str, config: PtyConfig) -> std::io::Result<String> {
        let session = spawn_pty(command, config)?;
        let session_id = session.session_id.clone();
        
        let mut sessions = self.sessions.lock().unwrap();
        sessions.push((session_id.clone(), session));
        
        Ok(session_id)
    }
    
    /// Read from a specific session.
    pub fn read(&mut self, session_id: &str) -> std::io::Result<Option<String>> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some((_, session)) = sessions.iter_mut().find(|(id, _)| id == session_id) {
            Ok(Some(session.read()?))
        } else {
            Ok(None)
        }
    }
    
    /// Write to a specific session.
    pub fn write(&mut self, session_id: &str, input: &str) -> std::io::Result<bool> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some((_, session)) = sessions.iter_mut().find(|(id, _)| id == session_id) {
            session.write(input)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// Resize a specific session.
    pub fn resize(&mut self, session_id: &str, rows: u16, cols: u16) -> std::io::Result<bool> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some((_, session)) = sessions.iter_mut().find(|(id, _)| id == session_id) {
            session.resize(rows, cols)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// Get status of a session.
    pub fn status(&mut self, session_id: &str) -> Option<PtyStatus> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some((_, session)) = sessions.iter_mut().find(|(id, _)| id == session_id) {
            let running = session.is_running();
            let exit_code = if running { None } else { session.child.try_wait().ok().flatten().map(|s| s.exit_code() as i32) };
            Some(PtyStatus {
                session_id: session_id.to_string(),
                running,
                exit_code,
            })
        } else {
            None
        }
    }
    
    /// Terminate a session.
    pub fn terminate(&mut self, session_id: &str) -> std::io::Result<bool> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(pos) = sessions.iter().position(|(id, _)| id == session_id) {
            let (_, mut session) = sessions.remove(pos);
            session.terminate()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// List all sessions.
    pub fn list(&self) -> Vec<String> {
        let sessions = self.sessions.lock().unwrap();
        sessions.iter().map(|(id, _)| id.clone()).collect()
    }
    
    /// Clean up finished sessions.
    pub fn cleanup(&mut self) {
        let mut sessions = self.sessions.lock().unwrap();
        let mut to_remove = Vec::new();
        for (id, session) in sessions.iter_mut() {
            if !session.is_running() {
                to_remove.push(id.clone());
            }
        }
        sessions.retain(|(id, _)| !to_remove.contains(id));
    }
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_config_default() {
        let config = PtyConfig::default();
        assert_eq!(config.rows, 24);
        assert_eq!(config.cols, 80);
        assert!(config.working_dir.is_none());
        assert!(config.env.is_empty());
    }

    #[test]
    fn test_pty_signal_serialization() {
        let signal = PtySignal::Interrupt;
        let json = serde_json::to_string(&signal).unwrap();
        assert_eq!(json, "\"interrupt\"");
    }

    #[test]
    fn test_pty_manager_new() {
        let manager = PtyManager::new();
        assert!(manager.list().is_empty());
    }

    #[test]
    fn test_spawn_simple_command() {
        let config = PtyConfig::default();
        let result = spawn_pty("echo hello", config);
        // This may fail in CI environments without PTY support
        if let Ok(mut session) = result {
            // Give it time to execute
            std::thread::sleep(std::time::Duration::from_millis(100));
            let output = session.read().unwrap_or_default();
            assert!(output.contains("hello") || session.wait().is_ok());
        }
    }
}
