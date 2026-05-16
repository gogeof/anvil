//! Debugging assistance tools for code analysis and error diagnosis.
//!
//! Provides stack trace parsing, error analysis, breakpoint suggestions,
//! and debugging workflows.

use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};

/// Parsed stack trace frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackFrame {
    /// Function/method name
    pub function: String,
    /// Source file path
    pub file: Option<String>,
    /// Line number
    pub line: Option<usize>,
    /// Column number
    pub column: Option<usize>,
    /// Module/namespace
    pub module: Option<String>,
    /// Raw frame text
    pub raw: String,
    /// Whether this is in user code (vs library code)
    pub is_user_code: bool,
}

/// Parsed error information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedError {
    /// Error type/name
    pub error_type: String,
    /// Error message
    pub message: String,
    /// Stack trace frames
    pub stack_frames: Vec<StackFrame>,
    /// Suggested fixes
    pub suggestions: Vec<ErrorSuggestion>,
    /// Related code context
    pub context: Option<ErrorContext>,
}

/// Suggested fix for an error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorSuggestion {
    /// Description of the fix
    pub description: String,
    /// Confidence level (0-100)
    pub confidence: u8,
    /// Related file (if applicable)
    pub file: Option<String>,
    /// Related line (if applicable)
    pub line: Option<usize>,
    /// Code snippet for the fix (if applicable)
    pub code_snippet: Option<String>,
}

/// Code context around an error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    /// File path
    pub file: String,
    /// Line number
    pub line: usize,
    /// Lines of context before the error
    pub before: Vec<String>,
    /// The error line
    pub error_line: String,
    /// Lines of context after the error
    pub after: Vec<String>,
}

/// Debug configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugConfig {
    /// Project root directory
    pub project_root: PathBuf,
    /// Maximum stack frames to parse
    pub max_frames: usize,
    /// Lines of context to show around errors
    pub context_lines: usize,
    /// Whether to include library frames
    pub include_library_frames: bool,
    /// Known library paths to filter out
    pub library_paths: Vec<String>,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            max_frames: 50,
            context_lines: 5,
            include_library_frames: true,
            library_paths: vec![
                "/usr/lib".to_string(),
                "/usr/local/lib".to_string(),
                "node_modules".to_string(),
                ".cargo".to_string(),
                "site-packages".to_string(),
            ],
        }
    }
}

/// Parse a stack trace string.
pub fn parse_stack_trace(trace: &str, config: &DebugConfig) -> Vec<StackFrame> {
    let mut frames = Vec::new();
    
    for line in trace.lines() {
        if let Some(frame) = parse_stack_frame(line) {
            if !config.include_library_frames && !frame.is_user_code {
                continue;
            }
            frames.push(frame);
            if frames.len() >= config.max_frames {
                break;
            }
        }
    }
    
    frames
}

/// Parse a single stack frame line.
fn parse_stack_frame(line: &str) -> Option<StackFrame> {
    let trimmed = line.trim();
    
    // Try more specific formats first
    
    // Try Python format: 'File "main.py", line 10, in function_name'
    if let Some(frame) = parse_python_frame(trimmed) {
        return Some(frame);
    }
    
    // Try Node.js format: "at functionName (file.js:10:5)"
    if let Some(frame) = parse_nodejs_frame(trimmed) {
        return Some(frame);
    }
    
    // Try Go format: "main.functionName()"
    if let Some(frame) = parse_go_frame(trimmed) {
        return Some(frame);
    }
    
    // Try Rust format: "at src/main.rs:10:5" (more general, so try last)
    if let Some(frame) = parse_rust_frame(trimmed) {
        return Some(frame);
    }
    
    None
}

/// Parse Rust stack frame.
fn parse_rust_frame(line: &str) -> Option<StackFrame> {
    // Format: "at src/main.rs:10:5" or just "src/main.rs:10:5"
    let re = regex::Regex::new(r"(?:at\s+)?([^:]+):(\d+)(?::(\d+))?").ok()?;
    let caps = re.captures(line)?;
    
    let file = caps.get(1)?.as_str().to_string();
    let line = caps.get(2)?.as_str().parse().ok()?;
    let column = caps.get(3).and_then(|m| m.as_str().parse().ok());
    
    let is_user_code = !file.starts_with('/') && !file.contains("rustc/") && !file.contains(".cargo/");
    
    Some(StackFrame {
        function: String::new(),
        file: Some(file),
        line: Some(line),
        column,
        module: None,
        raw: line.to_string(),
        is_user_code,
    })
}

/// Parse Python stack frame.
fn parse_python_frame(line: &str) -> Option<StackFrame> {
    // Format: 'File "main.py", line 10, in function_name'
    if !line.starts_with("File ") {
        return None;
    }
    
    let re = regex::Regex::new(r#"File "([^"]+)", line (\d+), in (\w+)"#).ok()?;
    let caps = re.captures(line)?;
    
    let file = caps.get(1)?.as_str().to_string();
    let line = caps.get(2)?.as_str().parse().ok()?;
    let function = caps.get(3)?.as_str().to_string();
    
    let is_user_code = !file.contains("site-packages/") && !file.contains("/usr/lib/");
    
    Some(StackFrame {
        function,
        file: Some(file),
        line: Some(line),
        column: None,
        module: None,
        raw: line.to_string(),
        is_user_code,
    })
}

/// Parse Node.js stack frame.
fn parse_nodejs_frame(line: &str) -> Option<StackFrame> {
    // Format: "at functionName (file.js:10:5)" or "at file.js:10:5"
    if !line.starts_with("at ") {
        return None;
    }
    
    let re = regex::Regex::new(r#"at (?:([\w.]+) )?\(?([^:()]+):(\d+):(\d+)\)?"#).ok()?;
    let caps = re.captures(line)?;
    
    let function = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
    let file = caps.get(2)?.as_str().to_string();
    let line_num = caps.get(3)?.as_str().parse().ok()?;
    let column = caps.get(4).and_then(|m| m.as_str().parse().ok());
    
    let is_user_code = !file.contains("node_modules/") && !file.starts_with("internal/");
    
    Some(StackFrame {
        function,
        file: Some(file),
        line: Some(line_num),
        column,
        module: None,
        raw: line.to_string(),
        is_user_code,
    })
}

/// Parse Go stack frame.
fn parse_go_frame(line: &str) -> Option<StackFrame> {
    // Format: "main.functionName()" or "main.functionName(0x...)"
    let re = regex::Regex::new(r"([\w./]+)\([^)]*\)").ok()?;
    let caps = re.captures(line)?;
    
    let full_name = caps.get(1)?.as_str();
    let (module, function) = if let Some(pos) = full_name.rfind('.') {
        (Some(full_name[..pos].to_string()), full_name[pos+1..].to_string())
    } else {
        (None, full_name.to_string())
    };
    
    Some(StackFrame {
        function,
        file: None,
        line: None,
        column: None,
        module,
        raw: line.to_string(),
        is_user_code: true,
    })
}

/// Analyze an error and provide suggestions.
pub fn analyze_error(error: &str, config: &DebugConfig) -> ParsedError {
    let (error_type, message) = extract_error_type_and_message(error);
    let stack_frames = parse_stack_trace(error, config);
    let suggestions = generate_suggestions(&error_type, &message, &stack_frames);
    let context = extract_error_context(&stack_frames, config);
    
    ParsedError {
        error_type,
        message,
        stack_frames,
        suggestions,
        context,
    }
}

/// Extract error type and message from error string.
fn extract_error_type_and_message(error: &str) -> (String, String) {
    // Try to find error type pattern
    let patterns = [
        // Rust: "error[E0308]: mismatched types"
        (r"error\[E(\d+)\]: (.+)", "Rust"),
        // Python: "TypeError: unsupported operand"
        (r"(\w+Error): (.+)", "Python"),
        // JavaScript: "TypeError: Cannot read property"
        (r"(\w+Error): (.+)", "JavaScript"),
        // Generic: "error: message"
        (r"error: (.+)", "Generic"),
    ];
    
    for (pattern, _) in &patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(error) {
                if caps.len() >= 3 {
                    return (caps[1].to_string(), caps[2].to_string());
                } else if caps.len() >= 2 {
                    return ("Error".to_string(), caps[1].to_string());
                }
            }
        }
    }
    
    // Fallback: use first line
    let first_line = error.lines().next().unwrap_or("");
    ("Error".to_string(), first_line.to_string())
}

/// Generate suggestions for fixing an error.
fn generate_suggestions(error_type: &str, message: &str, frames: &[StackFrame]) -> Vec<ErrorSuggestion> {
    let mut suggestions = Vec::new();
    
    // Common error patterns and suggestions
    let error_patterns = [
        ("E0308", "mismatched types", "Check type annotations and ensure types match"),
        ("E0425", "cannot find", "Check spelling, import the item, or verify it exists"),
        ("E0433", "failed to resolve", "Add missing import or check module path"),
        ("E0599", "no method named", "Check method name or verify trait is in scope"),
        ("TypeError", "", "Check that values are of expected types"),
        ("ReferenceError", "", "Verify that the variable or function is defined"),
        ("KeyError", "", "Check that the key exists in the dictionary"),
        ("IndexError", "", "Verify the index is within bounds"),
        ("NullPointerException", "", "Add null check or use optional chaining"),
        ("DivisionByZero", "", "Add check for zero before division"),
    ];
    
    for (code, keyword, suggestion) in error_patterns {
        if error_type.contains(code) || message.contains(keyword) {
            suggestions.push(ErrorSuggestion {
                description: suggestion.to_string(),
                confidence: 80,
                file: frames.first().and_then(|f| f.file.clone()),
                line: frames.first().and_then(|f| f.line),
                code_snippet: None,
            });
        }
    }
    
    // Add generic suggestion based on first user frame
    if let Some(first_user_frame) = frames.iter().find(|f| f.is_user_code) {
        if suggestions.is_empty() {
            suggestions.push(ErrorSuggestion {
                description: format!("Check the code at {}:{}", 
                    first_user_frame.file.as_deref().unwrap_or("unknown"),
                    first_user_frame.line.unwrap_or(0)
                ),
                confidence: 50,
                file: first_user_frame.file.clone(),
                line: first_user_frame.line,
                code_snippet: None,
            });
        }
    }
    
    suggestions
}

/// Extract error context from stack frames.
fn extract_error_context(frames: &[StackFrame], config: &DebugConfig) -> Option<ErrorContext> {
    let first_user_frame = frames.iter().find(|f| f.is_user_code)?;
    let file = first_user_frame.file.as_ref()?;
    let line = first_user_frame.line?;
    
    let file_path = config.project_root.join(file);
    let content = std::fs::read_to_string(&file_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    
    let start = line.saturating_sub(config.context_lines + 1);
    let end = (line + config.context_lines).min(lines.len());
    
    let before: Vec<String> = lines[start..line.saturating_sub(1)]
        .iter()
        .map(|s| s.to_string())
        .collect();
    
    let error_line = lines.get(line - 1).unwrap_or(&"").to_string();
    
    let after: Vec<String> = lines[line..end]
        .iter()
        .map(|s| s.to_string())
        .collect();
    
    Some(ErrorContext {
        file: file.clone(),
        line,
        before,
        error_line,
        after,
    })
}

/// Diagnostic result for a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiagnostic {
    /// File path
    pub file: String,
    /// Diagnostic items
    pub items: Vec<DiagnosticItem>,
}

/// A single diagnostic item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticItem {
    /// Severity level
    pub severity: DiagnosticSeverity,
    /// Message
    pub message: String,
    /// Line number
    pub line: usize,
    /// Column number
    pub column: Option<usize>,
    /// Related information
    pub related: Vec<String>,
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Run diagnostics on a project.
pub fn run_diagnostics(project_root: &Path) -> Vec<FileDiagnostic> {
    let mut diagnostics = Vec::new();
    
    // Detect project type and run appropriate linter
    if project_root.join("Cargo.toml").exists() {
        diagnostics.extend(run_cargo_check(project_root));
    }
    
    if project_root.join("package.json").exists() {
        diagnostics.extend(run_npm_lint(project_root));
    }
    
    diagnostics
}

/// Run cargo check and parse output.
fn run_cargo_check(project_root: &Path) -> Vec<FileDiagnostic> {
    let output = Command::new("cargo")
        .args(["check", "--message-format=short"])
        .current_dir(project_root)
        .output()
        .ok();
    
    let mut diagnostics = Vec::new();
    let mut current_file: Option<String> = None;
    let mut current_items: Vec<DiagnosticItem> = Vec::new();
    
    if let Some(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            // Parse cargo check output format: "src/main.rs:10:5: error[E0308]: ..."
            if let Some(diag) = parse_cargo_diagnostic(line) {
                if let Some(ref file) = current_file {
                    if file != &diag.0 {
                        diagnostics.push(FileDiagnostic {
                            file: file.clone(),
                            items: std::mem::take(&mut current_items),
                        });
                    }
                }
                current_file = Some(diag.0);
                current_items.push(diag.1);
            }
        }
        
        if let Some(file) = current_file {
            diagnostics.push(FileDiagnostic { file, items: current_items });
        }
    }
    
    diagnostics
}

/// Parse a cargo diagnostic line.
fn parse_cargo_diagnostic(line: &str) -> Option<(String, DiagnosticItem)> {
    let re = regex::Regex::new(r"([^:]+):(\d+):(\d+): (error|warning): (.+)").ok()?;
    let caps = re.captures(line)?;
    
    let file = caps.get(1)?.as_str().to_string();
    let line_num = caps.get(2)?.as_str().parse().ok()?;
    let column = caps.get(3)?.as_str().parse().ok();
    let severity_str = caps.get(4)?.as_str();
    let message = caps.get(5)?.as_str().to_string();
    
    let severity = match severity_str {
        "error" => DiagnosticSeverity::Error,
        "warning" => DiagnosticSeverity::Warning,
        _ => DiagnosticSeverity::Info,
    };
    
    Some((file, DiagnosticItem {
        severity,
        message,
        line: line_num,
        column,
        related: vec![],
    }))
}

/// Run npm lint and parse output.
fn run_npm_lint(project_root: &Path) -> Vec<FileDiagnostic> {
    let output = Command::new("npm")
        .args(["run", "lint", "--silent"])
        .current_dir(project_root)
        .output()
        .ok();
    
    // Basic parsing - would need to be expanded for specific linters
    let diagnostics = Vec::new();
    
    if let Some(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr);
        
        // Simple error detection
        for line in combined.lines() {
            if line.contains("error") || line.contains("Error") {
                // Would need more sophisticated parsing
            }
        }
    }
    
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rust_stack_frame() {
        let line = "at src/main.rs:10:5";
        let frame = parse_stack_frame(line).unwrap();
        
        assert_eq!(frame.file, Some("src/main.rs".to_string()));
        assert_eq!(frame.line, Some(10));
        assert_eq!(frame.column, Some(5));
    }

    #[test]
    fn parses_python_stack_frame() {
        let line = r#"File "main.py", line 10, in main"#;
        let frame = parse_stack_frame(line).unwrap();
        
        assert_eq!(frame.file, Some("main.py".to_string()));
        assert_eq!(frame.line, Some(10));
        assert_eq!(frame.function, "main");
    }

    #[test]
    fn parses_nodejs_stack_frame() {
        let line = "at functionName (file.js:10:5)";
        let frame = parse_stack_frame(line).unwrap();
        
        assert_eq!(frame.function, "functionName");
        assert_eq!(frame.file, Some("file.js".to_string()));
        assert_eq!(frame.line, Some(10));
    }

    #[test]
    fn identifies_user_code() {
        let frame = parse_stack_frame("at src/main.rs:10:5").unwrap();
        assert!(frame.is_user_code);
        
        let frame = parse_stack_frame("at /usr/lib/rustlib/src/rust/library/core/src/mod.rs:10").unwrap();
        assert!(!frame.is_user_code);
    }

    #[test]
    fn extracts_error_type() {
        let error = "TypeError: unsupported operand type(s)";
        let (error_type, message) = extract_error_type_and_message(error);
        
        assert_eq!(error_type, "TypeError");
        assert_eq!(message, "unsupported operand type(s)");
    }

    #[test]
    fn generates_suggestions_for_common_errors() {
        let error = "error[E0308]: mismatched types";
        let (error_type, message) = extract_error_type_and_message(error);
        let suggestions = generate_suggestions(&error_type, &message, &[]);
        
        assert!(!suggestions.is_empty());
        assert!(suggestions[0].description.contains("type"));
    }
}
