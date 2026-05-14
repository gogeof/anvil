//! Git worktree isolation for parallel agent sessions.
//!
//! This module provides functionality to create and manage git worktrees
//! for isolated development environments, allowing multiple agent instances
//! to work on different branches simultaneously without conflicts.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Default directory for claw-managed worktrees.
const CLAW_WORKTREE_DIR: &str = ".anvil/worktrees";

/// Errors that can occur when working with git worktrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeError {
    /// The current directory is not inside a git repository.
    NotAGitRepository,
    /// A worktree with the given name already exists.
    WorktreeAlreadyExists(String),
    /// The specified worktree was not found.
    WorktreeNotFound(String),
    /// A git command failed with the given error message.
    GitError(String),
}

impl std::fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAGitRepository => write!(f, "not a git repository"),
            Self::WorktreeAlreadyExists(name) => {
                write!(f, "worktree '{name}' already exists")
            }
            Self::WorktreeNotFound(name) => {
                write!(f, "worktree '{name}' not found")
            }
            Self::GitError(msg) => write!(f, "git error: {msg}"),
        }
    }
}

impl std::error::Error for WorktreeError {}

/// Result of entering a worktree context.
#[derive(Debug, Clone)]
pub struct WorktreeContext {
    /// The name of the worktree.
    pub name: String,
    /// The absolute path to the worktree directory.
    pub path: PathBuf,
    /// The original working directory before entering the worktree.
    pub original_cwd: PathBuf,
    /// The branch name in the worktree.
    pub branch: Option<String>,
}

impl WorktreeContext {
    /// Create a new worktree context.
    fn new(name: String, path: PathBuf, original_cwd: PathBuf, branch: Option<String>) -> Self {
        Self {
            name,
            path,
            original_cwd,
            branch,
        }
    }
}

/// Check if the given directory is inside a git repository.
///
/// # Errors
///
/// Returns `WorktreeError::NotAGitRepository` if not inside a git repo.
pub fn ensure_git_repo(cwd: &Path) -> Result<(), WorktreeError> {
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .output()
        .map_err(|e| WorktreeError::GitError(e.to_string()))?;

    if !output.status.success() {
        return Err(WorktreeError::NotAGitRepository);
    }

    Ok(())
}

/// Get the current git branch name.
///
/// Returns `None` if not on a branch (detached HEAD) or if not in a git repo.
#[must_use]
pub fn get_current_branch(cwd: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8(output.stdout).ok()?;
    let trimmed = branch.trim();
    if trimmed.is_empty() || trimmed == "HEAD" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Get the path to the git repository root (main worktree).
///
/// Returns `None` if not in a git repository.
#[must_use]
pub fn get_git_root(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Get the expected path for a named worktree.
///
/// The path is `<git_root>/.anvil/worktrees/<name>`.
#[must_use]
pub fn worktree_path(git_root: &Path, name: &str) -> PathBuf {
    git_root.join(CLAW_WORKTREE_DIR).join(name)
}

/// Check if a worktree with the given name exists.
///
/// Returns `true` if the worktree directory exists and is a valid git worktree.
#[must_use]
pub fn worktree_exists(git_root: &Path, name: &str) -> bool {
    let path = worktree_path(git_root, name);
    path.exists() && path.join(".git").exists()
}

/// List all claw-managed worktrees.
///
/// # Errors
///
/// Returns `WorktreeError` if unable to list worktrees.
pub fn list_worktrees(git_root: &Path) -> Result<Vec<String>, WorktreeError> {
    let worktree_dir = git_root.join(CLAW_WORKTREE_DIR);
    if !worktree_dir.exists() {
        return Ok(Vec::new());
    }

    let mut result = Vec::new();
    let entries = std::fs::read_dir(&worktree_dir)
        .map_err(|e| WorktreeError::GitError(format!("failed to read worktree dir: {e}")))?;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.path().join(".git").exists() {
            result.push(name);
        }
    }

    Ok(result)
}

/// Create a new git worktree at `.anvil/worktrees/<name>`.
///
/// If the worktree already exists, returns `WorktreeError::WorktreeAlreadyExists`.
///
/// # Arguments
///
/// * `git_root` - The root of the git repository
/// * `name` - The name for the worktree
/// * `branch` - The branch to check out in the worktree (uses current branch if None)
///
/// # Errors
///
/// Returns `WorktreeError` if the worktree cannot be created.
pub fn create_worktree(
    git_root: &Path,
    name: &str,
    branch: Option<&str>,
) -> Result<PathBuf, WorktreeError> {
    let worktree_path = worktree_path(git_root, name);

    if worktree_path.exists() {
        return Err(WorktreeError::WorktreeAlreadyExists(name.to_string()));
    }

    // Create parent directory
    let parent = worktree_path
        .parent()
        .ok_or_else(|| WorktreeError::GitError("invalid worktree path".to_string()))?;
    std::fs::create_dir_all(parent).map_err(|e| {
        WorktreeError::GitError(format!("failed to create worktree directory: {e}"))
    })?;

    // Determine branch to use
    let branch_name = branch.map_or_else(
        || get_current_branch(git_root).unwrap_or_else(|| "main".to_string()),
        str::to_string,
    );

    // Create the worktree
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            "--track",
            "-b",
            &format!("claw-{name}"),
            worktree_path
                .to_str()
                .ok_or_else(|| WorktreeError::GitError("invalid path encoding".to_string()))?,
            &branch_name,
        ])
        .current_dir(git_root)
        .output()
        .map_err(|e| WorktreeError::GitError(e.to_string()))?;

    if !output.status.success() {
        let _stderr = String::from_utf8(output.stderr).unwrap_or_default();
        // Try without creating a new branch if the branch already exists
        let output2 = Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap_or_default(),
                &branch_name,
            ])
            .current_dir(git_root)
            .output()
            .map_err(|e| WorktreeError::GitError(e.to_string()))?;

        if !output2.status.success() {
            let stderr2 = String::from_utf8(output2.stderr).unwrap_or_default();
            return Err(WorktreeError::GitError(format!(
                "failed to create worktree: {stderr2}"
            )));
        }
    }

    Ok(worktree_path)
}

/// Remove a worktree.
///
/// # Errors
///
/// Returns `WorktreeError` if the worktree cannot be removed.
pub fn remove_worktree(git_root: &Path, name: &str) -> Result<(), WorktreeError> {
    let worktree_path = worktree_path(git_root, name);

    if !worktree_path.exists() {
        return Err(WorktreeError::WorktreeNotFound(name.to_string()));
    }

    // Remove using git worktree remove
    let output = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(
            worktree_path
                .to_str()
                .ok_or_else(|| WorktreeError::GitError("invalid path encoding".to_string()))?,
        )
        .current_dir(git_root)
        .output()
        .map_err(|e| WorktreeError::GitError(e.to_string()))?;

    if !output.status.success() {
        let _stderr = String::from_utf8(output.stderr).unwrap_or_default();
        // Try force-remove, if that fails, just delete the directory
        let _ = std::fs::remove_dir_all(&worktree_path);
        // Still try to prune the worktree
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(git_root)
            .output();
    }

    Ok(())
}

/// Enter a worktree context, creating it if necessary.
///
/// This function:
/// 1. Verifies we're in a git repository
/// 2. Creates the worktree if it doesn't exist
/// 3. Returns a `WorktreeContext` with the worktree path
///
/// The caller is responsible for changing the current directory.
///
/// # Arguments
///
/// * `cwd` - The current working directory
/// * `name` - The name for the worktree
///
/// # Errors
///
/// Returns `WorktreeError` if the worktree cannot be created or entered.
pub fn enter_worktree(cwd: &Path, name: &str) -> Result<WorktreeContext, WorktreeError> {
    ensure_git_repo(cwd)?;

    let git_root = get_git_root(cwd).ok_or(WorktreeError::NotAGitRepository)?;
    let branch = get_current_branch(cwd);
    let original_cwd = cwd.to_path_buf();

    let worktree_path = if worktree_exists(&git_root, name) {
        worktree_path(&git_root, name)
    } else {
        create_worktree(&git_root, name, branch.as_deref())?
    };

    Ok(WorktreeContext::new(
        name.to_string(),
        worktree_path,
        original_cwd,
        branch,
    ))
}

/// Generate a random worktree name.
///
/// Uses a simple alphanumeric suffix to create unique names.
#[must_use]
pub fn generate_worktree_name() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("wt-{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-worktree-{label}-{nanos}"))
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env_lock()
    }

    fn ensure_valid_cwd() {
        if std::env::current_dir().is_err() {
            std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"))
                .expect("test cwd should be recoverable");
        }
    }

    fn git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|_| panic!("git {args:?} should run"))
            .status;
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn ensure_git_repo_fails_for_non_git_directory() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("non-git");
        fs::create_dir_all(&root).expect("create dir");

        let result = ensure_git_repo(&root);
        assert_eq!(result, Err(WorktreeError::NotAGitRepository));

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn ensure_git_repo_succeeds_for_git_directory() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("git-dir");
        fs::create_dir_all(&root).expect("create dir");
        git(&root, &["init", "--quiet", "--initial-branch=main"]);

        let result = ensure_git_repo(&root);
        assert!(result.is_ok());

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn get_current_branch_returns_none_for_non_git() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("no-branch");
        fs::create_dir_all(&root).expect("create dir");

        let result = get_current_branch(&root);
        assert!(result.is_none());

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn get_current_branch_returns_branch_name() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("has-branch");
        fs::create_dir_all(&root).expect("create dir");
        git(&root, &["init", "--quiet", "--initial-branch=test-branch"]);
        git(&root, &["config", "user.email", "tests@example.com"]);
        git(&root, &["config", "user.name", "Worktree Tests"]);
        fs::write(root.join("init.txt"), "init\n").expect("write init");
        git(&root, &["add", "init.txt"]);
        git(&root, &["commit", "-m", "initial", "--quiet"]);

        let result = get_current_branch(&root);
        assert_eq!(result, Some("test-branch".to_string()));

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn get_git_root_returns_none_for_non_git() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("no-git-root");
        fs::create_dir_all(&root).expect("create dir");

        let result = get_git_root(&root);
        assert!(result.is_none());

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn get_git_root_returns_root_for_git_repo() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("git-root");
        fs::create_dir_all(&root).expect("create dir");
        git(&root, &["init", "--quiet", "--initial-branch=main"]);

        let result = get_git_root(&root);
        assert_eq!(
            result.as_ref().map(|p| p.canonicalize().ok()),
            Some(root.canonicalize().ok())
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn worktree_path_joins_correctly() {
        let git_root = PathBuf::from("/repo");
        let result = worktree_path(&git_root, "my-worktree");
        assert_eq!(result, PathBuf::from("/repo/.anvil/worktrees/my-worktree"));
    }

    #[test]
    fn list_worktrees_returns_empty_for_no_worktrees() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("no-worktrees");
        fs::create_dir_all(&root).expect("create dir");
        git(&root, &["init", "--quiet", "--initial-branch=main"]);

        let result = list_worktrees(&root).expect("should list");
        assert!(result.is_empty());

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn create_and_remove_worktree() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("create-worktree");
        fs::create_dir_all(&root).expect("create dir");
        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        git(&root, &["config", "user.email", "tests@example.com"]);
        git(&root, &["config", "user.name", "Worktree Tests"]);
        fs::write(root.join("init.txt"), "init\n").expect("write init");
        git(&root, &["add", "init.txt"]);
        git(&root, &["commit", "-m", "initial", "--quiet"]);

        // Create worktree
        let wt_path =
            create_worktree(&root, "test-wt", Some("main")).expect("should create worktree");
        assert!(wt_path.exists());
        assert!(worktree_exists(&root, "test-wt"));

        // List should include it
        let list = list_worktrees(&root).expect("should list");
        assert!(list.contains(&"test-wt".to_string()));

        // Remove worktree
        remove_worktree(&root, "test-wt").expect("should remove");
        assert!(!worktree_exists(&root, "test-wt"));

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn generate_worktree_name_produces_unique_names() {
        let name1 = generate_worktree_name();
        let name2 = generate_worktree_name();
        assert!(name1.starts_with("wt-"));
        assert_ne!(name1, name2);
    }
}
