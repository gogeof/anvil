//! Test runner tool for automatic test detection and execution.
//!
//! Detects project type, selects appropriate test framework,
//! runs tests, and parses results into structured output.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::io;
use serde::{Deserialize, Serialize};

/// Detected project type with its test framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Rust,
    Python,
    NodeJs,
    Go,
    Java,
    DotNet,
    Unknown,
}

impl ProjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectType::Rust => "rust",
            ProjectType::Python => "python",
            ProjectType::NodeJs => "nodejs",
            ProjectType::Go => "go",
            ProjectType::Java => "java",
            ProjectType::DotNet => "dotnet",
            ProjectType::Unknown => "unknown",
        }
    }
}

/// Test runner configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunnerConfig {
    /// Project root directory
    pub project_root: PathBuf,
    /// Detected project type
    pub project_type: ProjectType,
    /// Test command to run
    pub test_command: Vec<String>,
    /// Extra environment variables
    pub env: HashMap<String, String>,
    /// Timeout in seconds (0 = no timeout)
    pub timeout_secs: u64,
    /// Run only specific test pattern
    pub test_filter: Option<String>,
    /// Enable verbose output
    pub verbose: bool,
}

impl Default for TestRunnerConfig {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            project_type: ProjectType::Unknown,
            test_command: vec![],
            env: HashMap::new(),
            timeout_secs: 300,
            test_filter: None,
            verbose: false,
        }
    }
}

/// Result of running tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunResult {
    /// Whether all tests passed
    pub success: bool,
    /// Total number of tests
    pub total_tests: usize,
    /// Number of passed tests
    pub passed: usize,
    /// Number of failed tests
    pub failed: usize,
    /// Number of skipped tests
    pub skipped: usize,
    /// Time taken in seconds
    pub duration_secs: f64,
    /// Raw test output
    pub output: String,
    /// Parsed test results
    pub test_cases: Vec<TestCaseResult>,
    /// Detected project type
    pub project_type: ProjectType,
    /// Command that was run
    pub command: String,
}

/// Result of a single test case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCaseResult {
    /// Test name
    pub name: String,
    /// Module or suite name
    pub module: Option<String>,
    /// Test status
    pub status: TestStatus,
    /// Duration in seconds
    pub duration_secs: Option<f64>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// Line number of failure (if applicable)
    pub failure_line: Option<usize>,
}

/// Status of a single test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
    Ignored,
    Pending,
}

/// Detect project type from directory contents.
pub fn detect_project_type(dir: &Path) -> ProjectType {
    // Check for Cargo.toml (Rust)
    if dir.join("Cargo.toml").exists() {
        return ProjectType::Rust;
    }
    
    // Check for pyproject.toml, setup.py, requirements.txt (Python)
    if dir.join("pyproject.toml").exists() 
        || dir.join("setup.py").exists()
        || dir.join("requirements.txt").exists()
        || dir.join("Pipfile").exists()
    {
        return ProjectType::Python;
    }
    
    // Check for package.json (Node.js)
    if dir.join("package.json").exists() {
        return ProjectType::NodeJs;
    }
    
    // Check for go.mod (Go)
    if dir.join("go.mod").exists() {
        return ProjectType::Go;
    }
    
    // Check for pom.xml or build.gradle (Java)
    if dir.join("pom.xml").exists() || dir.join("build.gradle").exists() {
        return ProjectType::Java;
    }
    
    // Check for .csproj or .sln (.NET)
    if dir.join("*.csproj").exists() 
        || dir.read_dir()
            .map(|mut entries| {
                entries.any(|e| e.ok().and_then(|e| e.file_name().into_string().ok())
                    .map(|n| n.ends_with(".csproj") || n.ends_with(".sln"))
                    .unwrap_or(false))
            })
            .unwrap_or(false)
    {
        return ProjectType::DotNet;
    }
    
    ProjectType::Unknown
}

/// Get the test command for a project type.
pub fn get_test_command(project_type: ProjectType, filter: Option<&str>, verbose: bool) -> Vec<String> {
    match project_type {
        ProjectType::Rust => {
            let mut cmd = vec!["cargo".to_string(), "test".to_string()];
            if verbose {
                cmd.push("--verbose".to_string());
            }
            if let Some(f) = filter {
                cmd.push("--".to_string());
                cmd.push(f.to_string());
            }
            cmd
        }
        ProjectType::Python => {
            // Try pytest first, fallback to unittest
            let mut cmd = vec!["python".to_string(), "-m".to_string(), "pytest".to_string()];
            if verbose {
                cmd.push("-v".to_string());
            }
            if let Some(f) = filter {
                cmd.push("-k".to_string());
                cmd.push(f.to_string());
            }
            cmd
        }
        ProjectType::NodeJs => {
            let mut cmd = vec!["npm".to_string(), "test".to_string()];
            if verbose {
                cmd.push("--verbose".to_string());
            }
            cmd
        }
        ProjectType::Go => {
            let mut cmd = vec!["go".to_string(), "test".to_string(), "./...".to_string()];
            if verbose {
                cmd.push("-v".to_string());
            }
            if let Some(f) = filter {
                cmd.push("-run".to_string());
                cmd.push(f.to_string());
            }
            cmd
        }
        ProjectType::Java => {
            // Maven or Gradle
            if PathBuf::from("pom.xml").exists() {
                let mut cmd = vec!["mvn".to_string(), "test".to_string()];
                if let Some(f) = filter {
                    cmd.push("-Dtest=".to_string() + f);
                }
                cmd
            } else {
                vec!["./gradlew".to_string(), "test".to_string()]
            }
        }
        ProjectType::DotNet => {
            vec!["dotnet".to_string(), "test".to_string()]
        }
        ProjectType::Unknown => {
            vec!["echo".to_string(), "No test framework detected".to_string()]
        }
    }
}

/// Run tests in the given directory.
pub fn run_tests(config: &TestRunnerConfig) -> io::Result<TestRunResult> {
    let start = std::time::Instant::now();
    
    let (program, args) = if config.test_command.is_empty() {
        let cmd = get_test_command(config.project_type, config.test_filter.as_deref(), config.verbose);
        (cmd[0].clone(), cmd[1..].to_vec())
    } else {
        (config.test_command[0].clone(), config.test_command[1..].to_vec())
    };
    
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .current_dir(&config.project_root);
    
    // Add environment variables
    for (key, value) in &config.env {
        cmd.env(key, value);
    }
    
    let output = cmd.output()?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined_output = format!("{}\n{}", stdout, stderr);
    
    let (total, passed, failed, skipped, test_cases) = parse_test_output(
        config.project_type, 
        &combined_output
    );
    
    let duration = start.elapsed().as_secs_f64();
    
    Ok(TestRunResult {
        success: output.status.success() && failed == 0,
        total_tests: total,
        passed,
        failed,
        skipped,
        duration_secs: duration,
        output: combined_output,
        test_cases,
        project_type: config.project_type,
        command: format!("{} {}", program, args.join(" ")),
    })
}

/// Parse test output based on project type.
fn parse_test_output(project_type: ProjectType, output: &str) -> (usize, usize, usize, usize, Vec<TestCaseResult>) {
    match project_type {
        ProjectType::Rust => parse_cargo_test_output(output),
        ProjectType::Python => parse_pytest_output(output),
        ProjectType::Go => parse_go_test_output(output),
        _ => {
            // Fallback: count test results generically
            let passed = output.matches("passed").count();
            let failed = output.matches("failed").count();
            let total = passed + failed;
            (total, passed, failed, 0, vec![])
        }
    }
}

/// Parse cargo test output.
fn parse_cargo_test_output(output: &str) -> (usize, usize, usize, usize, Vec<TestCaseResult>) {
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut test_cases = Vec::new();
    
    for line in output.lines() {
        let trimmed = line.trim();
        
        // Match lines like: "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured"
        if trimmed.starts_with("test result:") {
            // Parse summary line
            let parts: Vec<&str> = trimmed.split(';').collect();
            for part in parts {
                let part = part.trim();
                if let Some(num) = extract_number_before(part, "passed") {
                    passed += num;
                }
                if let Some(num) = extract_number_before(part, "failed") {
                    failed += num;
                }
                if let Some(num) = extract_number_before(part, "ignored") {
                    skipped += num;
                }
            }
        }
        
        // Match individual test lines like: "test module::test_name ... ok"
        if trimmed.starts_with("test ") && trimmed.contains("... ") {
            let parts: Vec<&str> = trimmed.splitn(2, "... ").collect();
            if parts.len() == 2 {
                let name = parts[0].strip_prefix("test ").unwrap_or(parts[0]).trim();
                let status_str = parts[1].trim();
                
                let status = if status_str == "ok" {
                    TestStatus::Passed
                } else if status_str == "FAILED" || status_str.starts_with("FAILED") {
                    TestStatus::Failed
                } else if status_str == "ignored" {
                    TestStatus::Ignored
                } else {
                    TestStatus::Skipped
                };
                
                // Parse module from name
                let (module, test_name) = if let Some(pos) = name.rfind("::") {
                    (Some(name[..pos].to_string()), name[pos+2..].to_string())
                } else {
                    (None, name.to_string())
                };
                
                test_cases.push(TestCaseResult {
                    name: test_name,
                    module,
                    status,
                    duration_secs: None,
                    error: None,
                    failure_line: None,
                });
                total += 1;
            }
        }
    }
    
    // Fallback if we didn't parse a summary line
    if total == 0 && passed == 0 && failed == 0 {
        passed = test_cases.iter().filter(|t| t.status == TestStatus::Passed).count();
        failed = test_cases.iter().filter(|t| t.status == TestStatus::Failed).count();
        skipped = test_cases.iter().filter(|t| t.status == TestStatus::Ignored).count();
        total = test_cases.len();
    }
    
    (total, passed, failed, skipped, test_cases)
}

/// Parse pytest output.
fn parse_pytest_output(output: &str) -> (usize, usize, usize, usize, Vec<TestCaseResult>) {
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut test_cases = Vec::new();
    
    for line in output.lines() {
        let trimmed = line.trim();
        
        // Match summary line like: "5 passed, 2 failed, 1 skipped"
        if trimmed.contains(" passed") || trimmed.contains(" failed") {
            if let Some(num) = extract_number_before(trimmed, "passed") {
                passed = num;
            }
            if let Some(num) = extract_number_before(trimmed, "failed") {
                failed = num;
            }
            if let Some(num) = extract_number_before(trimmed, "skipped") {
                skipped = num;
            }
            total = passed + failed + skipped;
        }
        
        // Match test lines like: "test_module.py::test_name PASSED"
        if trimmed.contains("::") && (trimmed.ends_with("PASSED") || trimmed.ends_with("FAILED") || trimmed.ends_with("SKIPPED")) {
            let parts: Vec<&str> = trimmed.rsplitn(2, ' ').collect();
            if parts.len() == 2 {
                let full_name = parts[1];
                let status_str = parts[0];
                
                let status = match status_str {
                    "PASSED" => TestStatus::Passed,
                    "FAILED" => TestStatus::Failed,
                    "SKIPPED" => TestStatus::Skipped,
                    _ => TestStatus::Pending,
                };
                
                // Parse module and test name
                let (module, test_name) = if let Some(pos) = full_name.find("::") {
                    (Some(full_name[..pos].to_string()), full_name[pos+2..].to_string())
                } else {
                    (None, full_name.to_string())
                };
                
                test_cases.push(TestCaseResult {
                    name: test_name,
                    module,
                    status,
                    duration_secs: None,
                    error: None,
                    failure_line: None,
                });
            }
        }
    }
    
    (total, passed, failed, skipped, test_cases)
}

/// Parse go test output.
fn parse_go_test_output(output: &str) -> (usize, usize, usize, usize, Vec<TestCaseResult>) {
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut test_cases = Vec::new();
    
    for line in output.lines() {
        let trimmed = line.trim();
        
        // Match lines like: "PASS TestName 0.00s" or "FAIL TestName 0.00s"
        if trimmed.starts_with("PASS") || trimmed.starts_with("FAIL") || trimmed.starts_with("SKIP") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let status = match parts[0] {
                    "PASS" => TestStatus::Passed,
                    "FAIL" => TestStatus::Failed,
                    "SKIP" => TestStatus::Skipped,
                    _ => TestStatus::Pending,
                };
                
                let name = parts[1].to_string();
                let duration = parts.last()
                    .and_then(|s| s.strip_suffix('s'))
                    .and_then(|s| s.parse::<f64>().ok());
                
                test_cases.push(TestCaseResult {
                    name,
                    module: None,
                    status,
                    duration_secs: duration,
                    error: None,
                    failure_line: None,
                });
                
                match status {
                    TestStatus::Passed => passed += 1,
                    TestStatus::Failed => failed += 1,
                    TestStatus::Skipped => skipped += 1,
                    _ => {}
                }
                total += 1;
            }
        }
    }
    
    (total, passed, failed, skipped, test_cases)
}

/// Extract a number that appears before a keyword.
fn extract_number_before(text: &str, keyword: &str) -> Option<usize> {
    let pos = text.find(keyword)?;
    let before = &text[..pos];
    before
        .trim()
        .split_whitespace()
        .last()
        .and_then(|s| s.parse::<usize>().ok())
}

/// Quick test detection and run.
pub fn quick_test(dir: &Path, filter: Option<&str>) -> io::Result<TestRunResult> {
    let project_type = detect_project_type(dir);
    let config = TestRunnerConfig {
        project_root: dir.to_path_buf(),
        project_type,
        test_command: get_test_command(project_type, filter, true),
        test_filter: filter.map(|s| s.to_string()),
        verbose: true,
        ..Default::default()
    };
    run_tests(&config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("test-runner-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()))
    }

    #[test]
    fn detects_rust_project() {
        let tmp = temp_dir();
        fs::create_dir_all(&tmp).ok();
        fs::write(tmp.join("Cargo.toml"), "[package]\nname = \"test\"").ok();
        
        assert_eq!(detect_project_type(&tmp), ProjectType::Rust);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn detects_python_project() {
        let tmp = temp_dir();
        fs::create_dir_all(&tmp).ok();
        fs::write(tmp.join("pyproject.toml"), "[project]\nname = \"test\"").ok();
        
        assert_eq!(detect_project_type(&tmp), ProjectType::Python);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn detects_nodejs_project() {
        let tmp = temp_dir();
        fs::create_dir_all(&tmp).ok();
        fs::write(tmp.join("package.json"), "{\"name\": \"test\"}").ok();
        
        assert_eq!(detect_project_type(&tmp), ProjectType::NodeJs);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn detects_go_project() {
        let tmp = temp_dir();
        fs::create_dir_all(&tmp).ok();
        fs::write(tmp.join("go.mod"), "module test\ngo 1.21").ok();
        
        assert_eq!(detect_project_type(&tmp), ProjectType::Go);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn gets_correct_test_command() {
        assert!(get_test_command(ProjectType::Rust, None, false)[0] == "cargo");
        assert!(get_test_command(ProjectType::Python, None, false)[0] == "python");
        assert!(get_test_command(ProjectType::Go, None, false)[0] == "go");
    }

    #[test]
    fn parses_cargo_test_output() {
        let output = r#"
test result: ok. 5 passed; 2 failed; 1 ignored; 0 measured
test module::test_one ... ok
test module::test_two ... FAILED
test module::test_three ... ignored
"#;
        let (total, passed, failed, skipped, cases) = parse_cargo_test_output(output);
        
        assert_eq!(total, 3);
        assert_eq!(passed, 5);
        assert_eq!(failed, 2);
        assert_eq!(skipped, 1);
        assert_eq!(cases.len(), 3);
    }

    #[test]
    fn parses_pytest_output() {
        let output = r#"
test_module.py::test_one PASSED
test_module.py::test_two FAILED
test_module.py::test_three SKIPPED
5 passed, 2 failed, 1 skipped
"#;
        let (total, passed, failed, skipped, cases) = parse_pytest_output(output);
        
        assert_eq!(total, 8);
        assert_eq!(passed, 5);
        assert_eq!(failed, 2);
        assert_eq!(skipped, 1);
        assert_eq!(cases.len(), 3);
    }

    #[test]
    fn extracts_numbers() {
        assert_eq!(extract_number_before("5 passed", "passed"), Some(5));
        assert_eq!(extract_number_before("10 failed", "failed"), Some(10));
    }
}
