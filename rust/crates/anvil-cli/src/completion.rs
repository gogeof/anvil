//! Terminal input completion system for the anvil REPL.
//!
//! Provides context-aware completion for:
//! - Slash commands (`/help`, `/status`, `/model`, etc.)
//! - Model names (known aliases from `MODEL_REGISTRY`)
//! - Session IDs (via callback)
//! - Permission modes
//! - File paths
//! - Command arguments (contextual)

use std::io;

/// Known model aliases for completion.
///
/// Mirrors the aliases from `api::providers::MODEL_REGISTRY` and
/// the hard-coded arms of `validate_model_syntax` in `main.rs`.
pub const KNOWN_MODEL_ALIASES: &[&str] = &[
    "pro",
    "lite",
    "opus",
    "sonnet",
    "haiku",
    "deepseek-chat",
    "deepseek-reasoner",
    "glm",
    "grok",
    "grok-3",
    "grok-mini",
    "grok-3-mini",
    "grok-2",
    "kimi",
    "openrouter",
    "together",
];

/// All registered slash command names (non-stub) used for prefix matching.
pub const SLASH_COMMAND_NAMES: &[&str] = &[
    "help",
    "status",
    "model",
    "permissions",
    "compact",
    "clear",
    "cost",
    "context",
    "resume",
    "config",
    "mcp",
    "memory",
    "init",
    "diff",
    "version",
    "export",
    "session",
    "plugin",
    "plugins",
    "marketplace",
    "agents",
    "skills",
    "skill",
    "doctor",
    "history",
    "stats",
    "metrics",
    "bughunter",
    "review",
    "commit",
    "pr",
    "issue",
    "ultraplan",
    "plan",
    "teleport",
    "debug-tool-call",
    "rewind",
    "tasks",
    "bug",
    "batch",
    "add-dir",
];

/// Known permission modes for completion.
pub const PERMISSION_MODES: &[&str] = &[
    "read-only",
    "workspace-write",
    "danger-full-access",
];

/// Additional workflow completion shortcuts.
pub const WORKFLOW_SHORTCUTS: &[&str] = &[
    "/clear --confirm",
    "/session list",
    "/session fork ",
    "/session switch ",
    "/session delete ",
    "/mcp list",
    "/mcp show ",
    "/mcp help",
    "/plugin list",
    "/plugins list",
    "/agents help",
    "/skills help",
    "/config env",
    "/config hooks",
    "/config model",
    "/config plugins",
    "/history 20",
];

/// Try to complete a file path prefix in the current working directory.
///
/// Returns (start_offset, completions) where start_offset is the byte
/// position where the prefix begins (for replacing the right part of the input).
pub fn complete_file_path(prefix: &str) -> io::Result<Vec<String>> {
    let path = std::path::Path::new(prefix);
    let (search_dir, partial_name) = if prefix.ends_with('/') {
        // User typed "foo/bar/" — list contents of foo/bar/
        let search_dir = path;
        let partial_name = "";
        (search_dir.to_path_buf(), partial_name.to_string())
    } else {
        // User typed "foo/ba" — look in parent dir (or '.') for "ba*"
        let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        (parent.to_path_buf(), file_name)
    };

    let resolved_dir = if search_dir.is_absolute() {
        search_dir.clone()
    } else {
        std::env::current_dir().unwrap_or_default().join(&search_dir)
    };

    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&resolved_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with(&partial_name) {
                    let mut candidate = if search_dir_is_current(&search_dir, prefix) {
                        name.to_string()
                    } else {
                        // Preserve the directory prefix the user typed
                        let dir_prefix = if prefix.contains('/') {
                            let last_slash = prefix.rfind('/').unwrap_or(0);
                            &prefix[..=last_slash]
                        } else {
                            ""
                        };
                        format!("{dir_prefix}{name}")
                    };

                    // Append trailing '/' for directories
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        candidate.push('/');
                    }

                    candidates.push(candidate);
                }
            }
        }
    }

    candidates.sort();
    Ok(candidates)
}

/// Check if the search_dir is effectively the current directory,
/// based on how the user typed the prefix.
fn search_dir_is_current(search_dir: &std::path::Path, prefix: &str) -> bool {
    if prefix.is_empty() || prefix == "." || prefix == "./" {
        return true;
    }
    // If prefix doesn't contain any path separator, the parent is "."
    // which means we're completing in cwd.
    !prefix.contains('/')
}

/// Build completions for the `/model <partial>` context.
pub fn complete_model_name(partial: &str) -> Vec<String> {
    let lower = partial.to_ascii_lowercase();
    KNOWN_MODEL_ALIASES
        .iter()
        .filter(|name| name.starts_with(&lower))
        .map(|name| format!("/model {name}"))
        .collect()
}

/// Build completions for `/permissions <partial>` context.
pub fn complete_permission_mode(partial: &str) -> Vec<String> {
    let lower = partial.to_ascii_lowercase();
    PERMISSION_MODES
        .iter()
        .filter(|mode| mode.starts_with(&lower))
        .map(|mode| format!("/permissions {mode}"))
        .collect()
}

/// Build completions for `/config <partial>` context.
pub fn complete_config_section(partial: &str) -> Vec<String> {
    let sections = ["env", "hooks", "model", "plugins"];
    let lower = partial.to_ascii_lowercase();
    sections
        .iter()
        .filter(|s| s.starts_with(&lower))
        .map(|s| format!("/config {s}"))
        .collect()
}

/// Build bash-style completions from a list of candidates.
pub fn bash_completions(partial: &str, candidates: &[String]) -> Vec<String> {
    candidates
        .iter()
        .filter(|c| c.starts_with(partial))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_known_model_aliases() {
        assert!(KNOWN_MODEL_ALIASES.contains(&"pro"));
        assert!(KNOWN_MODEL_ALIASES.contains(&"lite"));
        assert!(KNOWN_MODEL_ALIASES.contains(&"opus"));
        assert!(KNOWN_MODEL_ALIASES.contains(&"grok"));
    }

    #[test]
    fn completes_model_names_from_prefix() {
        let completions = complete_model_name("op");
        assert_eq!(completions, vec!["/model opus"]);
    }

    #[test]
    fn completes_permission_modes_from_prefix() {
        let completions = complete_permission_mode("read");
        assert_eq!(completions, vec!["/permissions read-only"]);
    }

    #[test]
    fn completes_config_sections_from_prefix() {
        let completions = complete_config_section("h");
        let expected: Vec<String> = ["hooks"].iter().map(|s| format!("/config {s}")).collect();
        assert_eq!(completions, expected);
    }

    #[test]
    fn file_path_completion_returns_candidates() {
        // Completing in current directory — should find at least Cargo.toml or similar
        let results = complete_file_path("").unwrap_or_default();
        // Should always have some entries in the project directory
        assert!(!results.is_empty(), "expected at least some files in cwd");
        // Results starting with '.' or regular names are fine
    }

    #[test]
    fn file_path_completion_for_absolute_works() {
        let results = complete_file_path("/").unwrap_or_default();
        // Root should have some directories
        assert!(!results.is_empty(), "expected entries in root dir");
    }

    #[test]
    fn bash_completions_filters_candidates() {
        let candidates = vec![
            "/help".to_string(),
            "/hello".to_string(),
            "/history".to_string(),
            "/status".to_string(),
        ];
        let matches = bash_completions("/he", &candidates);
        assert_eq!(matches.len(), 3);
        assert!(matches.contains(&"/help".to_string()));
        assert!(matches.contains(&"/hello".to_string()));
        assert!(matches.contains(&"/history".to_string()));
    }

    #[test]
    fn bash_completions_returns_empty_for_no_match() {
        let candidates = vec!["/help".to_string()];
        let matches = bash_completions("/x", &candidates);
        assert!(matches.is_empty());
    }
}
