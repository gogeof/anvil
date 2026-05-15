//! Incremental search engine for real-time file content searching.
//!
//! Provides an `IncrementalSearchEngine` that indexes files in a directory
//! and supports fast substring-based search as the user types. This is
//! designed for interactive "search-as-you-type" experiences.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

/// A single search result from the incremental search engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Absolute path to the matching file.
    pub file_path: PathBuf,
    /// 1-based line number of the match.
    pub line_number: usize,
    /// The text content of the matching line.
    pub line_content: String,
    /// Byte offset of the match within the line (0-based).
    pub match_start: usize,
    /// Byte offset of the end of the match within the line.
    pub match_end: usize,
}

/// In-memory representation of an indexed file.
#[derive(Debug, Clone)]
struct FileIndex {
    /// Absolute path of the file.
    path: PathBuf,
    /// Raw file content.
    content: String,
    /// Lines extracted from the content (for fast line-relative search).
    lines: Vec<String>,
}

/// Configuration for the incremental search engine.
#[derive(Debug, Clone)]
pub struct IncrementalSearchConfig {
    /// Whether to follow symlinks when indexing directories.
    pub follow_symlinks: bool,
    /// Maximum file size (in bytes) to index. Files larger than this are skipped.
    pub max_file_size: u64,
    /// Glob-like patterns of files/directories to exclude (checked as suffixes or
    /// path components). See [`IncrementalSearchEngine::add_ignore_pattern`].
    pub ignore_patterns: Vec<String>,
    /// File extensions to include. If empty, all text-like extensions are included.
    pub included_extensions: Vec<String>,
}

impl Default for IncrementalSearchConfig {
    fn default() -> Self {
        IncrementalSearchConfig {
            follow_symlinks: false,
            max_file_size: 1_048_576, // 1 MB
            ignore_patterns: vec![
                ".git".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                ".anvil".to_string(),
                ".venv".to_string(),
                "__pycache__".to_string(),
                ".DS_Store".to_string(),
            ],
            included_extensions: vec![
                "rs".to_string(),
                "toml".to_string(),
                "md".to_string(),
                "py".to_string(),
                "js".to_string(),
                "ts".to_string(),
                "tsx".to_string(),
                "jsx".to_string(),
                "json".to_string(),
                "yaml".to_string(),
                "yml".to_string(),
                "sh".to_string(),
                "bash".to_string(),
                "css".to_string(),
                "html".to_string(),
                "go".to_string(),
                "java".to_string(),
                "c".to_string(),
                "cpp".to_string(),
                "h".to_string(),
                "hpp".to_string(),
                "sql".to_string(),
                "rb".to_string(),
                "php".to_string(),
                "swift".to_string(),
                "kt".to_string(),
                "scala".to_string(),
                "zig".to_string(),
                "vue".to_string(),
                "svelte".to_string(),
                "xml".to_string(),
                "gradle".to_string(),
                "lock".to_string(),
            ],
        }
    }
}

/// An in-memory incremental search engine.
///
/// Indexes file contents from a directory tree and supports fast substring
/// searches that can be called on every keystroke.
#[derive(Debug)]
pub struct IncrementalSearchEngine {
    /// Map from canonical file path to indexed file data.
    files: RwLock<HashMap<PathBuf, FileIndex>>,
    /// Configuration for indexing behaviour.
    config: IncrementalSearchConfig,
}

impl IncrementalSearchEngine {
    /// Create a new empty incremental search engine with default configuration.
    pub fn new() -> Self {
        IncrementalSearchEngine {
            files: RwLock::new(HashMap::new()),
            config: IncrementalSearchConfig::default(),
        }
    }

    /// Create a new incremental search engine with a custom configuration.
    pub fn with_config(config: IncrementalSearchConfig) -> Self {
        IncrementalSearchEngine {
            files: RwLock::new(HashMap::new()),
            config,
        }
    }

    /// Add a glob-like pattern to the ignore list.
    ///
    /// Patterns are matched as path suffixes or individual path components.
    /// For example, `"build"` will ignore any file whose path contains `build`
    /// as a component or ends with `/build`.
    pub fn add_ignore_pattern(&mut self, pattern: &str) {
        self.config.ignore_patterns.push(pattern.to_string());
    }

    /// Replace the full set of included file extensions.
    pub fn set_included_extensions(&mut self, extensions: Vec<String>) {
        self.config.included_extensions = extensions;
    }

    /// Recursively index all supported files under `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` does not exist or cannot be read.
    pub fn index_directory(&self, path: &Path) -> std::io::Result<()> {
        if !path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("path does not exist: {}", path.display()),
            ));
        }

        let canonical_root = path.canonicalize()?;
        let mut walk = WalkDir::new(&canonical_root);
        if self.config.follow_symlinks {
            walk = walk.follow_links(true);
        }

        for entry in walk.into_iter().filter_map(|e| e.ok()) {
            let entry_path = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Skip directories (we only index files)
            if metadata.is_dir() {
                continue;
            }

            // Skip files that exceed the size limit
            if metadata.len() > self.config.max_file_size {
                continue;
            }

            // Skip ignored paths
            if self.is_ignored(entry_path) {
                continue;
            }

            // Check extension whitelist
            let ext = entry_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !self.config.included_extensions.contains(&ext.to_string()) {
                continue;
            }

            // Read and index the file
            let content = match std::fs::read_to_string(entry_path) {
                Ok(c) => c,
                Err(_) => continue, // skip binary or unreadable files
            };

            let canonical_path = match entry_path.canonicalize() {
                Ok(p) => p,
                Err(_) => entry_path.to_path_buf(),
            };

            self.index_content(canonical_path, content);
        }

        Ok(())
    }

    /// Index a single file by reading it from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or is not valid UTF-8.
    pub fn index_file(&self, path: &Path) -> std::io::Result<()> {
        let content = std::fs::read_to_string(path)?;
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.index_content(canonical, content);
        Ok(())
    }

    /// Update or insert a file's content in the index.
    ///
    /// If the file was previously indexed, its entry is replaced.
    pub fn update_file(&self, path: &Path, content: &str) -> std::io::Result<()> {
        let canonical = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        self.index_content(canonical, content.to_string());
        Ok(())
    }

    /// Remove a file from the index.
    pub fn remove_file(&self, path: &Path) {
        let mut files = self.files.write().expect("lock poisoned");
        files.remove(path);
    }

    /// Search the indexed content for a query string.
    ///
    /// Returns up to `limit` results, ordered by file path then line number.
    /// The search is case-insensitive and uses simple substring matching,
    /// making it suitable for real-time "search-as-you-type" scenarios.
    ///
    /// When `query` is empty, an empty result set is returned.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();
        let files = self.files.read().expect("lock poisoned");
        let mut results = Vec::new();

        for file_index in files.values() {
            for (line_idx, line) in file_index.lines.iter().enumerate() {
                let line_lower = line.to_lowercase();
                if let Some(pos) = line_lower.find(&query_lower) {
                    results.push(SearchResult {
                        file_path: file_index.path.clone(),
                        line_number: line_idx + 1, // 1-based
                        line_content: line.clone(),
                        match_start: pos,
                        match_end: pos + query.len(),
                    });
                }
            }
        }

        // Sort by file path then line number
        results.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line_number.cmp(&b.line_number)));

        results.truncate(limit);
        results
    }

    /// Return the total number of indexed files.
    pub fn file_count(&self) -> usize {
        let files = self.files.read().expect("lock poisoned");
        files.len()
    }

    /// Clear all indexed content.
    pub fn clear(&self) {
        let mut files = self.files.write().expect("lock poisoned");
        files.clear();
    }

    // ── private helpers ──────────────────────────────────────────────

    /// Internal: store content under a canonical path.
    fn index_content(&self, path: PathBuf, content: String) {
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let file_index = FileIndex {
            path: path.clone(),
            content,
            lines,
        };
        let mut files = self.files.write().expect("lock poisoned");
        files.insert(path, file_index);
    }

    /// Check whether a path should be ignored according to configured patterns.
    fn is_ignored(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        for pattern in &self.config.ignore_patterns {
            if path_str.contains(pattern) {
                return true;
            }
        }
        false
    }
}

impl Default for IncrementalSearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();

        // Create a few files for testing
        fs::write(dir.path().join("hello.txt"), "hello world\nthis is a test\nHELLO AGAIN").unwrap();
        fs::write(dir.path().join("foo.rs"), "fn main() {\n    println!(\"hello\");\n}").unwrap();
        fs::write(dir.path().join("bar.md"), "# Hello\n\nThis is markdown").unwrap();
        fs::write(dir.path().join("exclude.me"), "should not be indexed").unwrap();

        dir
    }

    #[test]
    fn test_new_engine_is_empty() {
        let engine = IncrementalSearchEngine::new();
        assert_eq!(engine.file_count(), 0);
        assert!(engine.search("anything", 10).is_empty());
    }

    #[test]
    fn test_index_directory() {
        let dir = setup_test_dir();
        let engine = IncrementalSearchEngine::new();

        // By default "exclude.me" is not in included_extensions, so it will be skipped.
        // hello.txt also won't be indexed because .txt is not in default extensions.
        // Let's verify: hello.txt has .txt, foo.rs has .rs, bar.md has .md
        engine.index_directory(dir.path()).unwrap();

        // foo.rs and bar.md should be indexed (hello.txt has .txt which is not in default extensions)
        assert_eq!(engine.file_count(), 2);

        let results = engine.search("hello", 10);
        assert!(!results.is_empty());

        // Results should include both foo.rs and bar.md
        let file_names: Vec<&str> = results
            .iter()
            .map(|r| r.file_path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(file_names.contains(&"foo.rs"));
    }

    #[test]
    fn test_search_case_insensitive() {
        let dir = setup_test_dir();
        let engine = IncrementalSearchEngine::new();

        // Index .txt files by adding extension
        let mut config = IncrementalSearchConfig::default();
        config.included_extensions.push("txt".to_string());
        let engine = IncrementalSearchEngine::with_config(config);
        engine.index_directory(dir.path()).unwrap();

        let results = engine.search("hello", 10);
        assert!(!results.is_empty());

        // Should match "hello" in hello.txt (lowercase) and also lines containing "HELLO"
        let line_contents: Vec<&str> = results
            .iter()
            .map(|r| r.line_content.as_str())
            .collect();
        assert!(line_contents.contains(&"hello world"));
        assert!(line_contents.contains(&"HELLO AGAIN"));
    }

    #[test]
    fn test_update_file() {
        let dir = setup_test_dir();
        let engine = IncrementalSearchEngine::new();

        // Index a specific file by extending the default config
        let mut config = IncrementalSearchConfig::default();
        config.included_extensions.push("txt".to_string());
        let engine = IncrementalSearchEngine::with_config(config);
        engine.index_file(&dir.path().join("hello.txt")).unwrap();

        assert_eq!(engine.file_count(), 1);
        let results = engine.search("world", 10);
        assert_eq!(results.len(), 1);

        // Update the file content
        engine
            .update_file(&dir.path().join("hello.txt"), "updated content\nnew line here")
            .unwrap();

        assert_eq!(engine.file_count(), 1);
        let results = engine.search("world", 10);
        assert!(results.is_empty());
        let results = engine.search("updated", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_limit() {
        let engine = IncrementalSearchEngine::new();
        engine
            .update_file(Path::new("test.rs"), "apple\nbanana\napple pie\ncherry")
            .unwrap();

        let results = engine.search("apple", 10);
        assert_eq!(results.len(), 2);

        let results = engine.search("apple", 1);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_remove_file() {
        let engine = IncrementalSearchEngine::new();
        engine
            .update_file(Path::new("keep.rs"), "content here")
            .unwrap();
        engine
            .update_file(Path::new("remove.rs"), "remove this")
            .unwrap();
        assert_eq!(engine.file_count(), 2);

        engine.remove_file(Path::new("remove.rs"));
        assert_eq!(engine.file_count(), 1);
    }

    #[test]
    fn test_clear() {
        let engine = IncrementalSearchEngine::new();
        engine
            .update_file(Path::new("a.rs"), "alpha")
            .unwrap();
        engine
            .update_file(Path::new("b.rs"), "beta")
            .unwrap();
        assert_eq!(engine.file_count(), 2);

        engine.clear();
        assert_eq!(engine.file_count(), 0);
    }

    #[test]
    fn test_empty_query_returns_empty() {
        let engine = IncrementalSearchEngine::new();
        engine
            .update_file(Path::new("test.rs"), "something")
            .unwrap();
        let results = engine.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_match_positions() {
        let engine = IncrementalSearchEngine::new();
        engine
            .update_file(Path::new("test.rs"), "hello world")
            .unwrap();

        let results = engine.search("world", 10);
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.line_content, "hello world");
        assert_eq!(r.line_number, 1);
        assert!(r.match_start < r.match_end);
    }

    #[test]
    fn test_index_ignores_git_dir() {
        let dir = TempDir::new().unwrap();
        let git_dir = dir.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("config"), "some git config").unwrap();
        fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").unwrap();

        let engine = IncrementalSearchEngine::new();
        engine.index_directory(dir.path()).unwrap();

        // .git/config should be ignored, only main.rs indexed
        assert_eq!(engine.file_count(), 1);
    }

    #[test]
    fn test_result_file_paths() {
        let engine = IncrementalSearchEngine::new();
        engine
            .update_file(Path::new("/tmp/test_file.rs"), "search me")
            .unwrap();

        let results = engine.search("search", 10);
        assert_eq!(results.len(), 1);
        assert!(results[0].file_path.to_string_lossy().ends_with("test_file.rs"));
    }

    #[test]
    fn test_multiple_matches_same_line() {
        let engine = IncrementalSearchEngine::new();
        engine
            .update_file(Path::new("dup.rs"), "foo foo foo bar")
            .unwrap();

        // Substring search will match "foo" once per line since we use find()
        // which returns the first occurrence
        let results = engine.search("foo", 10);
        assert_eq!(results.len(), 1);
    }
}
