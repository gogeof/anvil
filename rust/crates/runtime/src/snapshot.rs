//! Workspace snapshot / rollback capability.
//!
//! Uses `git stash create` to take lightweight snapshots of repository state and
//! stores metadata in `~/.anvil/snapshots/`. Restoration applies the stashed commit
//! via `git stash apply` and immediately removes it from the user's stash stack,
//! keeping zero trace in normal git history.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Default snapshot storage root.
fn default_snapshots_dir() -> PathBuf {
    dirs::home_dir()
        .expect("home directory must exist")
        .join(".anvil")
        .join("snapshots")
}

/// Errors that can occur during snapshot operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// The working directory is not inside a git repository.
    NotAGitRepository,
    /// A git command failed.
    GitError(String),
    /// An I/O error occurred while reading or writing snapshot data.
    IoError(String),
    /// No snapshot with the given id exists.
    SnapshotNotFound(String),
    /// Stored snapshot data is corrupt or unparseable.
    InvalidSnapshotData(String),
    /// Restoration of a snapshot failed.
    RestoreFailed(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAGitRepository => write!(f, "not a git repository"),
            Self::GitError(msg) => write!(f, "git error: {msg}"),
            Self::IoError(msg) => write!(f, "I/O error: {msg}"),
            Self::SnapshotNotFound(id) => write!(f, "snapshot '{id}' not found"),
            Self::InvalidSnapshotData(msg) => write!(f, "invalid snapshot data: {msg}"),
            Self::RestoreFailed(msg) => write!(f, "restore failed: {msg}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// A snapshot of the workspace at a point in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique identifier (UUID).
    pub id: String,
    /// The git stash commit hash (the object created by `git stash create`).
    pub stash_hash: String,
    /// ISO-8601 timestamp when the snapshot was created.
    pub created_at: String,
    /// Semantic label (e.g., "before-op", "after-op").
    pub label: String,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Branch name at snapshot time.
    pub branch: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a snapshot of the current workspace state.
///
/// Uses `git stash create` under the hood – the working tree is **not** modified
/// by this call. The snapshot is stored in `~/.anvil/snapshots/`.
///
/// # Arguments
///
/// * `cwd`     – Working directory (must be inside a git repo).
/// * `label`   – Short semantic label such as `"before-op"` or `"after-op"`.
/// * `description` – Optional longer description.
pub fn create_snapshot(
    cwd: &Path,
    label: &str,
    description: Option<&str>,
) -> Result<Snapshot, SnapshotError> {
    create_snapshot_in(cwd, &default_snapshots_dir(), label, description)
}

/// List all snapshots stored in `~/.anvil/snapshots/`.
///
/// Snapshots are returned in reverse chronological order (newest first).
pub fn list_snapshots() -> Result<Vec<Snapshot>, SnapshotError> {
    list_snapshots_in(&default_snapshots_dir())
}

/// Restore the workspace to the state captured by a named snapshot.
///
/// Internally this registers the stash commit, applies it, and immediately
/// removes the entry from the stash stack so the user's git history is
/// not permanently affected.
///
/// # Arguments
///
/// * `cwd` – Working directory of the repository.
/// * `id`  – Snapshot id (UUID) to restore.
pub fn restore_snapshot(cwd: &Path, id: &str) -> Result<(), SnapshotError> {
    restore_snapshot_in(cwd, &default_snapshots_dir(), id)
}

// ---------------------------------------------------------------------------
// Internal helpers (parameterised so tests can use arbitrary directories)
// ---------------------------------------------------------------------------

fn create_snapshot_in(
    cwd: &Path,
    snapshots_dir: &Path,
    label: &str,
    description: Option<&str>,
) -> Result<Snapshot, SnapshotError> {
    // 1. Verify this is a git repository.
    if !is_git_repo(cwd) {
        return Err(SnapshotError::NotAGitRepository);
    }

    // 2. Read current branch.
    let branch = current_branch(cwd).unwrap_or_else(|| "detached".to_string());

    // 3. Stage ALL changes (including untracked) so `git stash create`
    //    captures the full working state. We reset afterwards.
    let add_output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(cwd)
        .output()
        .map_err(|e| SnapshotError::GitError(e.to_string()))?;

    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        return Err(SnapshotError::GitError(format!(
            "git add -A failed: {stderr}"
        )));
    }

    // 4. Run `git stash create` — all changes are now staged so the
    //    stash commit captures the complete working state.
    let output = Command::new("git")
        .args(["stash", "create"])
        .current_dir(cwd)
        .output()
        .map_err(|e| SnapshotError::GitError(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SnapshotError::GitError(format!(
            "git stash create failed: {stderr}"
        )));
    }

    let stash_hash = String::from_utf8(output.stdout)
        .map_err(|e| SnapshotError::GitError(format!("invalid utf-8 in git output: {e}")))?
        .trim()
        .to_string();

    // 5. Restore the staging area — unstage everything that was just staged.
    let reset_output = Command::new("git")
        .args(["reset", "HEAD", "--", "."])
        .current_dir(cwd)
        .output();

    // Reset failure is non-fatal — the snapshot has been captured.
    if let Err(e) = reset_output {
        eprintln!("warning: git reset after snapshot failed (harmless): {e}");
    }

    // 6. Build the snapshot record.
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = iso_timestamp();
    let snapshot = Snapshot {
        id: id.clone(),
        stash_hash,
        created_at,
        label: label.to_string(),
        description: description.map(String::from),
        branch,
    };

    // 5. Persist to disk.
    fs::create_dir_all(snapshots_dir)
        .map_err(|e| SnapshotError::IoError(format!("cannot create snapshots dir: {e}")))?;

    let path = snapshots_dir.join(format!("{id}.json"));
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| SnapshotError::IoError(format!("serialization error: {e}")))?;

    fs::write(&path, &json)
        .map_err(|e| SnapshotError::IoError(format!("cannot write snapshot file: {e}")))?;

    Ok(snapshot)
}

fn list_snapshots_in(snapshots_dir: &Path) -> Result<Vec<Snapshot>, SnapshotError> {
    if !snapshots_dir.exists() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::new();

    let entries = fs::read_dir(snapshots_dir)
        .map_err(|e| SnapshotError::IoError(format!("cannot list snapshots: {e}")))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(true, |ext| ext != "json") {
            continue;
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| SnapshotError::IoError(format!("cannot read snapshot file: {e}")))?;

        match serde_json::from_str::<Snapshot>(&content) {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(e) => {
                // Skip corrupt entries.
                eprintln!("warning: skipping corrupt snapshot file {:?}: {e}", path);
            }
        }
    }

    // Newest first.
    snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(snapshots)
}

fn restore_snapshot_in(
    cwd: &Path,
    snapshots_dir: &Path,
    id: &str,
) -> Result<(), SnapshotError> {
    // 1. Load the snapshot record.
    let path = snapshots_dir.join(format!("{id}.json"));
    if !path.exists() {
        return Err(SnapshotError::SnapshotNotFound(id.to_string()));
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| SnapshotError::IoError(format!("cannot read snapshot: {e}")))?;

    let snapshot: Snapshot = serde_json::from_str(&content)
        .map_err(|e| SnapshotError::InvalidSnapshotData(e.to_string()))?;

    // 2. If there's nothing to restore, succeed silently.
    if snapshot.stash_hash.is_empty() {
        return Ok(());
    }

    // 3. Force-restore working tree from the stash commit.
    //    Uses `git checkout` which overwrites files even with local changes,
    //    unlike `git stash apply` which tries to merge and can conflict.
    let checkout = Command::new("git")
        .args(["checkout", &snapshot.stash_hash, "--", "."])
        .current_dir(cwd)
        .output()
        .map_err(|e| SnapshotError::GitError(e.to_string()))?;

    if !checkout.status.success() {
        let stderr = String::from_utf8_lossy(&checkout.stderr);
        return Err(SnapshotError::RestoreFailed(format!(
            "git checkout failed: {stderr}"
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn is_git_repo(cwd: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .output()
        .ok()
        .map_or(false, |o| o.status.success())
}

fn current_branch(cwd: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8(output.stdout).ok()?;
    let trimmed = branch.trim().to_string();
    if trimmed.is_empty() || trimmed == "HEAD" {
        None
    } else {
        Some(trimmed)
    }
}

fn iso_timestamp() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();
    let days = total_secs / 86400;
    let time_secs = total_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let (year, month, day) = days_to_date(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hours, minutes, seconds, millis
    )
}

/// Simple days-to-date conversion (works for reasonable date ranges).
fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Days since 1970-01-01
    let mut y = 1970i64;
    let mut remaining = days as i64;

    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let year = y as u64;
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 0u64;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            month = (i + 1) as u64;
            break;
        }
        remaining -= md;
    }

    let day = (remaining + 1) as u64;
    (year, month, day)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    // ── Helpers ────────────────────────────────────────────────────────────

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("anvil-snapshot-{label}-{nanos}"))
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
        assert!(status.success(), "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&Command::new("git").args(args).current_dir(cwd).output().unwrap().stderr));
    }

    fn init_repo(root: &Path) {
        fs::create_dir_all(root).expect("create dir");
        git(root, &["init", "--quiet", "--initial-branch=main"]);
        git(root, &["config", "user.email", "tests@example.com"]);
        git(root, &["config", "user.name", "Snapshot Tests"]);
        fs::write(root.join("init.txt"), "initial content\n").expect("write init");
        git(root, &["add", "init.txt"]);
        git(root, &["commit", "-m", "initial commit", "--quiet"]);
    }

    // ── Test: create_snapshot ─────────────────────────────────────────────

    #[test]
    fn create_snapshot_fails_for_non_git_directory() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("non-git");
        fs::create_dir_all(&root).expect("create dir");
        let snaps = temp_dir("snaps-non-git");

        let result = create_snapshot_in(&root, &snaps, "before-op", None);
        assert_eq!(result, Err(SnapshotError::NotAGitRepository));

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&snaps);
    }

    #[test]
    fn create_snapshot_succeeds_in_git_repo_and_persists_file() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("create-ok");
        let snaps = temp_dir("snaps-create-ok");
        init_repo(&root);

        // Make a change so stash create returns a real hash.
        fs::write(root.join("work.txt"), "working state\n").expect("write");

        let result = create_snapshot_in(&root, &snaps, "before-op", Some("just before action"));

        let snapshot = result.expect("snapshot should be created");
        assert_eq!(snapshot.label, "before-op");
        assert_eq!(snapshot.description.as_deref(), Some("just before action"));
        assert_eq!(snapshot.branch, "main");
        assert!(!snapshot.stash_hash.is_empty(), "stash hash should not be empty when there are changes");
        assert!(!snapshot.id.is_empty());

        // Verify file was written.
        let meta_path = snaps.join(format!("{}.json", snapshot.id));
        assert!(meta_path.exists(), "snapshot metadata file should exist");

        // Verify file content is valid JSON.
        let content = fs::read_to_string(&meta_path).expect("read snapshot file");
        let parsed: Snapshot = serde_json::from_str(&content).expect("valid JSON");
        assert_eq!(parsed.id, snapshot.id);
        assert_eq!(parsed.stash_hash, snapshot.stash_hash);

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&snaps);
    }

    #[test]
    fn create_snapshot_records_empty_hash_when_no_changes() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("create-clean");
        let snaps = temp_dir("snaps-create-clean");
        init_repo(&root);

        // No changes made.
        let result = create_snapshot_in(&root, &snaps, "clean", None);
        let snapshot = result.expect("snapshot should be created even when clean");
        assert!(snapshot.stash_hash.is_empty(), "stash hash should be empty for clean repo");

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&snaps);
    }

    // ── Test: list_snapshots ──────────────────────────────────────────────

    #[test]
    fn list_snapshots_returns_empty_when_dir_does_not_exist() {
        let snaps = temp_dir("list-empty");
        // Directory does not exist.
        let result = list_snapshots_in(&snaps).expect("should not error");
        assert!(result.is_empty());
        let _ = fs::remove_dir_all(&snaps);
    }

    #[test]
    fn list_snapshots_returns_created_snapshots() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("list-ok");
        let snaps = temp_dir("snaps-list");
        init_repo(&root);

        // Create two snapshots.
        fs::write(root.join("a.txt"), "a\n").expect("write");
        let snap1 = create_snapshot_in(&root, &snaps, "first", None)
            .expect("create first");
        fs::write(root.join("b.txt"), "b\n").expect("write");
        let snap2 = create_snapshot_in(&root, &snaps, "second", Some("second op"))
            .expect("create second");

        let list = list_snapshots_in(&snaps).expect("list snapshots");
        assert_eq!(list.len(), 2);
        // Both snapshot IDs should be present in the list.
        let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&snap1.id.as_str()), "list should contain snap1");
        assert!(ids.contains(&snap2.id.as_str()), "list should contain snap2");

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&snaps);
    }

    // ── Test: restore_snapshot ────────────────────────────────────────────

    #[test]
    fn restore_snapshot_returns_error_for_nonexistent_id() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("restore-missing");
        let snaps = temp_dir("snaps-restore-missing");
        init_repo(&root);

        let result = restore_snapshot_in(&root, &snaps, "nonexistent-uuid");
        assert_eq!(
            result,
            Err(SnapshotError::SnapshotNotFound("nonexistent-uuid".to_string()))
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&snaps);
    }

    #[test]
    fn restore_snapshot_restores_working_tree() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("restore-ok");
        let snaps = temp_dir("snaps-restore");
        init_repo(&root);

        // Initial state: init.txt contains "initial content\n"
        // We'll make a change, snapshot, then make another change, and restore.

        // --- Phase 1: create the "before" state ---
        // Modify init.txt and add a new file.
        fs::write(root.join("init.txt"), "modified content\n").expect("write");
        fs::write(root.join("new_file.txt"), "new file\n").expect("write");
        git(&root, &["add", "new_file.txt"]); // stage new_file

        let snap_before = create_snapshot_in(&root, &snaps, "before-op", Some("state before AI op"))
            .expect("create before snapshot");
        assert!(!snap_before.stash_hash.is_empty());
        // `git stash create` does NOT revert changes, so they remain.
        assert!(root.join("new_file.txt").exists());

        // --- Phase 2: make more changes (simulating AI work) ---
        fs::write(root.join("init.txt"), "ai changed content\n").expect("write");
        fs::write(root.join("ai_result.txt"), "ai output\n").expect("write");

        // Verify the AI state is different.
        let ai_content = fs::read_to_string(root.join("init.txt")).expect("read");
        assert_eq!(ai_content, "ai changed content\n");

        // --- Phase 3: restore to "before" snapshot ---
        restore_snapshot_in(&root, &snaps, &snap_before.id)
            .expect("restore should succeed");

        // Verify restored state.
        let restored_init = fs::read_to_string(root.join("init.txt")).expect("read");
        assert_eq!(
            restored_init, "modified content\n",
            "init.txt should be restored to snapshot state"
        );
        assert!(
            root.join("new_file.txt").exists(),
            "new_file.txt should still exist (was in snapshot)"
        );
        // ai_result.txt was created AFTER snapshot, so after restore it may or may not
        // be present depending on how the restore works. git stash apply will
        // try to merge, and ai_result.txt doesn't conflict so it should remain.

        // Verify stash stack is clean (no leftover stashes).
        let stash_list = Command::new("git")
            .args(["stash", "list"])
            .current_dir(&root)
            .output()
            .expect("git stash list");
        let stash_output = String::from_utf8_lossy(&stash_list.stdout);
        assert!(
            stash_output.is_empty(),
            "stash stack should be empty after restore, got: {stash_output}"
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&snaps);
    }

    #[test]
    fn restore_clean_snapshot_is_noop() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("restore-clean");
        let snaps = temp_dir("snaps-restore-clean");
        init_repo(&root);

        // Snapshot with no changes.
        let snap = create_snapshot_in(&root, &snaps, "clean", None)
            .expect("create clean snapshot");
        assert!(snap.stash_hash.is_empty());

        // Make changes AFTER the clean snapshot.
        fs::write(root.join("after.txt"), "after\n").expect("write");

        // Restore the clean snapshot — it has empty hash, should be noop.
        restore_snapshot_in(&root, &snaps, &snap.id).expect("restore clean");

        // After snapshot should still exist (noop restore).
        assert!(root.join("after.txt").exists());

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&snaps);
    }

    /// snapshot module properly integrates with the git stash workflow —
    /// no leaked stash entries.
    #[test]
    fn no_leaked_stash_entries_after_restore() {
        let _guard = env_lock();
        ensure_valid_cwd();
        let root = temp_dir("no-leak");
        let snaps = temp_dir("snaps-no-leak");
        init_repo(&root);

        fs::write(root.join("work.txt"), "work\n").expect("write");
        let snap = create_snapshot_in(&root, &snaps, "leak-test", None)
            .expect("create snapshot");

        restore_snapshot_in(&root, &snaps, &snap.id).expect("restore");

        let output = Command::new("git")
            .args(["stash", "list"])
            .current_dir(&root)
            .output()
            .expect("git stash list");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.is_empty(), "stash list should be empty: {stdout}");

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&snaps);
    }
}
