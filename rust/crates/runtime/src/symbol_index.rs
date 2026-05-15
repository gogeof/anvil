//! Project symbol indexing for fast code navigation.
//!
//! Provides lightweight symbol extraction using regex patterns,
//! supporting multiple languages without heavy dependencies like tree-sitter.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A symbol extracted from source code.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Symbol {
    /// Symbol name (function, class, struct, etc.)
    pub name: String,
    /// Symbol kind (function, struct, class, trait, etc.)
    pub kind: SymbolKind,
    /// File path where the symbol is defined
    pub file: PathBuf,
    /// Line number (1-indexed)
    pub line: u32,
    /// Column number (1-indexed, approximate)
    pub column: u32,
    /// Parent scope (e.g., module name, class name)
    pub scope: Option<String>,
    /// Documentation comment (if any)
    pub doc: Option<String>,
}

/// Kind of symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    EnumVariant,
    Trait,
    Impl,
    Module,
    TypeAlias,
    Constant,
    Static,
    Macro,
    Class,
    Interface,
    Namespace,
    Property,
    Field,
    Variable,
    Import,
    Unknown,
}

impl SymbolKind {
    /// Returns a display name for the symbol kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::EnumVariant => "variant",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Module => "module",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Constant => "const",
            SymbolKind::Static => "static",
            SymbolKind::Macro => "macro",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Namespace => "namespace",
            SymbolKind::Property => "property",
            SymbolKind::Field => "field",
            SymbolKind::Variable => "variable",
            SymbolKind::Import => "import",
            SymbolKind::Unknown => "unknown",
        }
    }
}

/// Language-specific symbol extraction patterns.
#[derive(Debug, Clone)]
pub struct LanguagePatterns {
    /// File extensions for this language
    pub extensions: Vec<&'static str>,
    /// Pattern to extract symbols (name, kind, line content)
    pub patterns: Vec<SymbolPattern>,
    /// Comment prefix for doc extraction
    pub doc_comment_prefix: &'static str,
}

/// A pattern for extracting a specific symbol kind.
#[derive(Debug, Clone)]
pub struct SymbolPattern {
    pub kind: SymbolKind,
    pub regex: &'static str,
    pub name_group: usize,
}

/// Symbol index for a project.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SymbolIndex {
    /// All symbols indexed
    pub symbols: Vec<Symbol>,
    /// Name -> symbol indices mapping (for fast lookup)
    #[serde(skip)]
    pub name_index: HashMap<String, Vec<usize>>,
    /// File -> symbol indices mapping
    #[serde(skip)]
    pub file_index: HashMap<PathBuf, Vec<usize>>,
    /// Index build time
    pub indexed_at: Option<String>,
    /// Total files scanned
    pub files_scanned: usize,
    /// Total time spent indexing (ms)
    pub index_time_ms: u64,
}

impl SymbolIndex {
    /// Create an empty symbol index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a symbol index for a directory.
    pub fn build(root: &Path, max_files: usize) -> io::Result<Self> {
        let start = Instant::now();
        let mut index = Self::new();
        let mut files_scanned = 0;

        // Walk the directory tree
        let walker = ignore::WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .ignore(true)
            .max_depth(Some(10))
            .build();

        for entry in walker.flatten() {
            if files_scanned >= max_files {
                break;
            }
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            
            if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                if let Some(patterns) = get_patterns_for_extension(ext) {
                    if let Ok(symbols) = extract_symbols_from_file(path, &patterns) {
                        for symbol in symbols {
                            let idx = index.symbols.len();
                            index.name_index
                                .entry(symbol.name.clone())
                                .or_default()
                                .push(idx);
                            index.file_index
                                .entry(symbol.file.clone())
                                .or_default()
                                .push(idx);
                            index.symbols.push(symbol);
                        }
                    }
                    files_scanned += 1;
                }
            }
        }

        index.files_scanned = files_scanned;
        index.index_time_ms = start.elapsed().as_millis() as u64;
        index.indexed_at = Some(chrono_lite_now());
        
        Ok(index)
    }

    /// Look up symbols by name (exact match or prefix).
    pub fn lookup(&self, query: &str) -> Vec<&Symbol> {
        let mut results = Vec::new();
        
        // Exact match
        if let Some(indices) = self.name_index.get(query) {
            for &idx in indices {
                if let Some(symbol) = self.symbols.get(idx) {
                    results.push(symbol);
                }
            }
        }
        
        // Prefix match (for autocomplete)
        for (name, indices) in &self.name_index {
            if name.starts_with(query) && name != query {
                for &idx in indices {
                    if let Some(symbol) = self.symbols.get(idx) {
                        results.push(symbol);
                    }
                }
            }
        }
        
        results
    }

    /// Get all symbols in a file.
    pub fn symbols_in_file(&self, path: &Path) -> Vec<&Symbol> {
        self.file_index
            .get(path)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&idx| self.symbols.get(idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Search symbols by fuzzy matching.
    pub fn fuzzy_search(&self, query: &str, limit: usize) -> Vec<&Symbol> {
        let query_lower = query.to_lowercase();
        let mut scored: Vec<(i32, &Symbol)> = Vec::new();

        for symbol in &self.symbols {
            let name_lower = symbol.name.to_lowercase();
            let score = fuzzy_score(&query_lower, &name_lower);
            if score > 0 {
                scored.push((score, symbol));
            }
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(limit);
        scored.into_iter().map(|(_, s)| s).collect()
    }
}

/// Calculate fuzzy match score (higher is better).
fn fuzzy_score(query: &str, target: &str) -> i32 {
    if target.contains(query) {
        // Substring match - highest score
        return 100 - (target.len() - query.len()) as i32;
    }
    
    let mut score = 0i32;
    let mut query_chars = query.chars().peekable();
    
    for ch in target.chars() {
        if let Some(&qch) = query_chars.peek() {
            if ch.to_ascii_lowercase() == qch.to_ascii_lowercase() {
                score += 10;
                query_chars.next();
            } else if ch.is_uppercase() && query_chars.peek().map(|&c| c.is_lowercase()).unwrap_or(false) {
                // Bonus for matching uppercase (camelCase awareness)
            }
        }
    }
    
    // All query chars matched?
    if query_chars.peek().is_none() {
        score
    } else {
        0
    }
}

/// Extract symbols from a file using language-specific patterns.
pub fn extract_symbols_from_file(path: &Path, patterns: &LanguagePatterns) -> io::Result<Vec<Symbol>> {
    let content = fs::read_to_string(path)?;
    let mut symbols = Vec::new();
    
    // Pre-compile regex patterns
    let compiled: Vec<(SymbolKind, regex::Regex, usize)> = patterns
        .patterns
        .iter()
        .filter_map(|p| {
            regex::Regex::new(p.regex).ok().map(|r| (p.kind, r, p.name_group))
        })
        .collect();

    let mut current_scope: Option<String> = None;
    let mut pending_doc: Option<String> = None;
    
    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num as u32 + 1;
        let trimmed = line.trim();
        
        // Check for doc comments
        if trimmed.starts_with(patterns.doc_comment_prefix) {
            let doc = trimmed.trim_start_matches(patterns.doc_comment_prefix).trim();
            if doc.starts_with('/') {
                // Skip triple-slash continuation for now
            } else {
                pending_doc = Some(pending_doc.take().unwrap_or_default() + " " + doc);
            }
            continue;
        }
        
        // Check for scope changes (impl blocks, modules)
        if let Some(caps) = regex::Regex::new(r"^(pub\s+)?impl\s+(?:(\w+)\s+for\s+)?(\w+)")
            .ok()
            .and_then(|r| r.captures(trimmed))
        {
            if let Some(trait_name) = caps.get(2) {
                current_scope = Some(format!("impl {} for {}", trait_name.as_str(), caps.get(3).unwrap().as_str()));
            } else {
                current_scope = Some(caps.get(3).unwrap().as_str().to_string());
            }
        }
        
        // Try each symbol pattern
        for (kind, regex, name_group) in &compiled {
            if let Some(caps) = regex.captures(trimmed) {
                if let Some(name_match) = caps.get(*name_group) {
                    let name = name_match.as_str().to_string();
                    let column = trimmed.find(&name).map(|p| p as u32 + 1).unwrap_or(1);
                    
                    symbols.push(Symbol {
                        name,
                        kind: *kind,
                        file: path.to_path_buf(),
                        line: line_num,
                        column,
                        scope: current_scope.clone(),
                        doc: pending_doc.take(),
                    });
                    
                    break; // Only match one pattern per line
                }
            }
        }
        
        // Clear pending doc if not used
        if !trimmed.starts_with("//") && !trimmed.starts_with("/*") && !trimmed.starts_with("#") {
            pending_doc = None;
        }
    }
    
    Ok(symbols)
}

/// Get language patterns for a file extension.
pub fn get_patterns_for_extension(ext: &str) -> Option<LanguagePatterns> {
    match ext {
        "rs" => Some(rust_patterns()),
        "py" => Some(python_patterns()),
        "js" | "ts" | "jsx" | "tsx" => Some(javascript_patterns()),
        "go" => Some(go_patterns()),
        "java" | "kt" => Some(java_patterns()),
        "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" => Some(cpp_patterns()),
        _ => None,
    }
}

/// Rust symbol patterns.
fn rust_patterns() -> LanguagePatterns {
    LanguagePatterns {
        extensions: vec!["rs"],
        doc_comment_prefix: "///",
        patterns: vec![
            // pub fn name(
            SymbolPattern {
                kind: SymbolKind::Function,
                regex: r"^(?:pub\s+)?(?:async\s+)?fn\s+(\w+)",
                name_group: 1,
            },
            // pub struct Name
            SymbolPattern {
                kind: SymbolKind::Struct,
                regex: r"^(?:pub\s+)?struct\s+(\w+)",
                name_group: 1,
            },
            // pub enum Name
            SymbolPattern {
                kind: SymbolKind::Enum,
                regex: r"^(?:pub\s+)?enum\s+(\w+)",
                name_group: 1,
            },
            // pub trait Name
            SymbolPattern {
                kind: SymbolKind::Trait,
                regex: r"^(?:pub\s+)?trait\s+(\w+)",
                name_group: 1,
            },
            // pub type Name =
            SymbolPattern {
                kind: SymbolKind::TypeAlias,
                regex: r"^(?:pub\s+)?type\s+(\w+)",
                name_group: 1,
            },
            // pub const NAME:
            SymbolPattern {
                kind: SymbolKind::Constant,
                regex: r"^(?:pub\s+)?const\s+(\w+)",
                name_group: 1,
            },
            // pub static NAME:
            SymbolPattern {
                kind: SymbolKind::Static,
                regex: r"^(?:pub\s+)?static\s+(\w+)",
                name_group: 1,
            },
            // macro_rules! name
            SymbolPattern {
                kind: SymbolKind::Macro,
                regex: r"^macro_rules!\s+(\w+)",
                name_group: 1,
            },
            // mod name {
            SymbolPattern {
                kind: SymbolKind::Module,
                regex: r"^(?:pub\s+)?mod\s+(\w+)",
                name_group: 1,
            },
        ],
    }
}

/// Python symbol patterns.
fn python_patterns() -> LanguagePatterns {
    LanguagePatterns {
        extensions: vec!["py"],
        doc_comment_prefix: "#",
        patterns: vec![
            // def name(
            SymbolPattern {
                kind: SymbolKind::Function,
                regex: r"^(?:async\s+)?def\s+(\w+)",
                name_group: 1,
            },
            // class Name:
            SymbolPattern {
                kind: SymbolKind::Class,
                regex: r"^class\s+(\w+)",
                name_group: 1,
            },
            // VAR = value (module-level)
            SymbolPattern {
                kind: SymbolKind::Variable,
                regex: r"^([A-Z_][A-Z0-9_]*)\s*=",
                name_group: 1,
            },
        ],
    }
}

/// JavaScript/TypeScript symbol patterns.
fn javascript_patterns() -> LanguagePatterns {
    LanguagePatterns {
        extensions: vec!["js", "ts", "jsx", "tsx"],
        doc_comment_prefix: "//",
        patterns: vec![
            // function name(
            SymbolPattern {
                kind: SymbolKind::Function,
                regex: r"^(?:export\s+)?(?:async\s+)?function\s+(\w+)",
                name_group: 1,
            },
            // const name = (
            SymbolPattern {
                kind: SymbolKind::Function,
                regex: r"^(?:export\s+)?(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?(?:\([^)]*\)|[\w]+)\s*=>",
                name_group: 1,
            },
            // class Name {
            SymbolPattern {
                kind: SymbolKind::Class,
                regex: r"^(?:export\s+)?class\s+(\w+)",
                name_group: 1,
            },
            // interface Name {
            SymbolPattern {
                kind: SymbolKind::Interface,
                regex: r"^(?:export\s+)?interface\s+(\w+)",
                name_group: 1,
            },
            // type Name =
            SymbolPattern {
                kind: SymbolKind::TypeAlias,
                regex: r"^(?:export\s+)?type\s+(\w+)",
                name_group: 1,
            },
            // const/let/var NAME
            SymbolPattern {
                kind: SymbolKind::Variable,
                regex: r"^(?:export\s+)?(?:const|let|var)\s+(\w+)",
                name_group: 1,
            },
        ],
    }
}

/// Go symbol patterns.
fn go_patterns() -> LanguagePatterns {
    LanguagePatterns {
        extensions: vec!["go"],
        doc_comment_prefix: "//",
        patterns: vec![
            // func Name(
            SymbolPattern {
                kind: SymbolKind::Function,
                regex: r"^func\s+(?:\([^)]+\)\s+)?(\w+)",
                name_group: 1,
            },
            // type Name struct
            SymbolPattern {
                kind: SymbolKind::Struct,
                regex: r"^type\s+(\w+)\s+struct",
                name_group: 1,
            },
            // type Name interface
            SymbolPattern {
                kind: SymbolKind::Interface,
                regex: r"^type\s+(\w+)\s+interface",
                name_group: 1,
            },
            // type Name
            SymbolPattern {
                kind: SymbolKind::TypeAlias,
                regex: r"^type\s+(\w+)\s+(?!struct|interface)",
                name_group: 1,
            },
            // const NAME
            SymbolPattern {
                kind: SymbolKind::Constant,
                regex: r"^const\s+(\w+)",
                name_group: 1,
            },
            // var NAME
            SymbolPattern {
                kind: SymbolKind::Variable,
                regex: r"^var\s+(\w+)",
                name_group: 1,
            },
        ],
    }
}

/// Java/Kotlin symbol patterns.
fn java_patterns() -> LanguagePatterns {
    LanguagePatterns {
        extensions: vec!["java", "kt"],
        doc_comment_prefix: "//",
        patterns: vec![
            // class Name {
            SymbolPattern {
                kind: SymbolKind::Class,
                regex: r"^(?:public|private|protected)?\s*(?:abstract|final|static)?\s*class\s+(\w+)",
                name_group: 1,
            },
            // interface Name {
            SymbolPattern {
                kind: SymbolKind::Interface,
                regex: r"^(?:public|private|protected)?\s*interface\s+(\w+)",
                name_group: 1,
            },
            // void name(
            SymbolPattern {
                kind: SymbolKind::Method,
                regex: r"^(?:public|private|protected)?\s*(?:static|final|abstract|synchronized)?\s*\w+\s+(\w+)\s*\(",
                name_group: 1,
            },
            // fun name(
            SymbolPattern {
                kind: SymbolKind::Function,
                regex: r"^fun\s+(\w+)",
                name_group: 1,
            },
        ],
    }
}

/// C/C++ symbol patterns.
fn cpp_patterns() -> LanguagePatterns {
    LanguagePatterns {
        extensions: vec!["c", "cpp", "cc", "cxx", "h", "hpp"],
        doc_comment_prefix: "//",
        patterns: vec![
            // class Name {
            SymbolPattern {
                kind: SymbolKind::Class,
                regex: r"^class\s+(\w+)",
                name_group: 1,
            },
            // struct Name {
            SymbolPattern {
                kind: SymbolKind::Struct,
                regex: r"^struct\s+(\w+)",
                name_group: 1,
            },
            // void name(
            SymbolPattern {
                kind: SymbolKind::Function,
                regex: r"^(?:static|inline|virtual)?\s*\w+\s+(\w+)\s*\(",
                name_group: 1,
            },
            // #define NAME
            SymbolPattern {
                kind: SymbolKind::Constant,
                regex: r"^#define\s+(\w+)",
                name_group: 1,
            },
            // enum Name {
            SymbolPattern {
                kind: SymbolKind::Enum,
                regex: r"^enum\s+(?:class\s+)?(\w+)",
                name_group: 1,
            },
        ],
    }
}

/// Simple current time string (avoid chrono dependency).
fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("symbol-index-test-{}", std::process::id()))
    }

    #[test]
    fn extracts_rust_function() {
        let code = r#"
pub fn hello_world() {
    println!("Hello");
}

async fn fetch_data() -> Result<()> {
    Ok(())
}
"#;
        let patterns = rust_patterns();
        let tmp = temp_dir();
        fs::create_dir_all(&tmp).ok();
        let file = tmp.join("test.rs");
        fs::write(&file, code).ok();
        
        let symbols = extract_symbols_from_file(&file, &patterns).unwrap();
        
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "hello_world");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[1].name, "fetch_data");
    }

    #[test]
    fn extracts_rust_struct_and_enum() {
        let code = r#"
pub struct User {
    name: String,
    age: u32,
}

enum Status {
    Active,
    Inactive,
}
"#;
        let patterns = rust_patterns();
        let tmp = temp_dir();
        fs::create_dir_all(&tmp).ok();
        let file = tmp.join("test.rs");
        fs::write(&file, code).ok();
        
        let symbols = extract_symbols_from_file(&file, &patterns).unwrap();
        
        assert!(symbols.iter().any(|s| s.name == "User" && s.kind == SymbolKind::Struct));
        assert!(symbols.iter().any(|s| s.name == "Status" && s.kind == SymbolKind::Enum));
    }

    #[test]
    fn extracts_python_class_and_function() {
        let code = r#"
class Calculator:
    def add(self, a, b):
        return a + b

async def fetch_url(url):
    pass
"#;
        let patterns = python_patterns();
        let tmp = temp_dir();
        fs::create_dir_all(&tmp).ok();
        let file = tmp.join("test.py");
        fs::write(&file, code).ok();
        
        let symbols = extract_symbols_from_file(&file, &patterns).unwrap();
        
        assert!(symbols.iter().any(|s| s.name == "Calculator" && s.kind == SymbolKind::Class));
        assert!(symbols.iter().any(|s| s.name == "add" && s.kind == SymbolKind::Function));
        assert!(symbols.iter().any(|s| s.name == "fetch_url" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn fuzzy_search_works() {
        let mut index = SymbolIndex::new();
        index.symbols.push(Symbol {
            name: "hello_world".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("test.rs"),
            line: 1,
            column: 1,
            scope: None,
            doc: None,
        });
        index.symbols.push(Symbol {
            name: "hello_user".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("test.rs"),
            line: 5,
            column: 1,
            scope: None,
            doc: None,
        });
        index.symbols.push(Symbol {
            name: "goodbye".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("test.rs"),
            line: 10,
            column: 1,
            scope: None,
            doc: None,
        });
        
        // Rebuild indices
        for (idx, sym) in index.symbols.iter().enumerate() {
            index.name_index.entry(sym.name.clone()).or_default().push(idx);
        }
        
        let results = index.fuzzy_search("hel", 10);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn gets_patterns_for_extensions() {
        assert!(get_patterns_for_extension("rs").is_some());
        assert!(get_patterns_for_extension("py").is_some());
        assert!(get_patterns_for_extension("js").is_some());
        assert!(get_patterns_for_extension("ts").is_some());
        assert!(get_patterns_for_extension("go").is_some());
        assert!(get_patterns_for_extension("java").is_some());
        assert!(get_patterns_for_extension("unknown").is_none());
    }
}
