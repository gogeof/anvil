//! Symbol index module for fast symbol lookup in code projects.
//!
//! This module provides functionality to scan code projects and build an index
//! of symbols (functions, structs, enums, traits, modules, variables) for
//! accelerated searching.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Symbol kind enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Module,
    Variable,
}

impl SymbolKind {
    /// Returns a string representation of the symbol kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Module => "mod",
            SymbolKind::Variable => "var",
        }
    }
}

/// A symbol definition found in source code.
#[derive(Debug, Clone)]
pub struct Symbol {
    /// The name of the symbol.
    pub name: String,
    /// The kind of symbol (function, struct, etc.).
    pub kind: SymbolKind,
    /// The file where the symbol is defined.
    pub file: PathBuf,
    /// The line number (1-indexed) where the symbol is defined.
    pub line: usize,
    /// The column number (1-indexed) where the symbol name starts.
    pub column: usize,
}

/// Symbol index for fast lookup of symbols in a project.
#[derive(Debug, Default)]
pub struct SymbolIndex {
    /// Symbols indexed by name for O(1) lookup.
    symbols: HashMap<String, Vec<Symbol>>,
    /// Symbols indexed by file for file-based queries.
    by_file: HashMap<PathBuf, Vec<Symbol>>,
}

impl SymbolIndex {
    /// Creates a new empty symbol index.
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            by_file: HashMap::new(),
        }
    }

    /// Scans a project directory and builds a symbol index.
    ///
    /// # Arguments
    ///
    /// * `root` - The root directory of the project to scan.
    ///
    /// # Returns
    ///
    /// A `Result` containing the built `SymbolIndex` or an `io::Error`.
    pub fn scan_project(root: &Path) -> io::Result<Self> {
        let mut index = Self::new();
        index.scan_directory(root)?;
        Ok(index)
    }

    /// Recursively scans a directory for Rust source files.
    fn scan_directory(&mut self, dir: &Path) -> io::Result<()> {
        let entries = fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Skip hidden directories and files
            if path.file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }

            if path.is_dir() {
                // Skip target and other build directories
                if path.file_name().map(|n| n == "target").unwrap_or(false) {
                    continue;
                }
                self.scan_directory(&path)?;
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                self.scan_file(&path)?;
            }
        }

        Ok(())
    }

    /// Scans a single Rust source file for symbols.
    fn scan_file(&mut self, file: &Path) -> io::Result<()> {
        let content = fs::read_to_string(file)?;
        let mut file_symbols = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            let line_num = line_num + 1; // 1-indexed

            // Skip comment lines
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("/*") {
                continue;
            }

            // Extract symbols using simple pattern matching
            if let Some(symbol) = self.extract_symbol(line, line_num, file) {
                file_symbols.push(symbol);
            }
        }

        // Index by file
        if !file_symbols.is_empty() {
            for symbol in &file_symbols {
                self.symbols
                    .entry(symbol.name.clone())
                    .or_default()
                    .push(symbol.clone());
            }
            self.by_file.insert(file.to_path_buf(), file_symbols);
        }

        Ok(())
    }

    /// Extracts a symbol from a line of code.
    fn extract_symbol(&self, line: &str, line_num: usize, file: &Path) -> Option<Symbol> {
        let trimmed = line.trim_start();

        // Match function definitions: fn name
        if let Some(name) = self.extract_keyword_symbol(trimmed, "fn ") {
            return Some(Symbol {
                name,
                kind: SymbolKind::Function,
                file: file.to_path_buf(),
                line: line_num,
                column: line.find("fn ").unwrap() + 3,
            });
        }

        // Match struct definitions: struct Name
        if let Some(name) = self.extract_keyword_symbol(trimmed, "struct ") {
            return Some(Symbol {
                name,
                kind: SymbolKind::Struct,
                file: file.to_path_buf(),
                line: line_num,
                column: line.find("struct ").unwrap() + 7,
            });
        }

        // Match enum definitions: enum Name
        if let Some(name) = self.extract_keyword_symbol(trimmed, "enum ") {
            return Some(Symbol {
                name,
                kind: SymbolKind::Enum,
                file: file.to_path_buf(),
                line: line_num,
                column: line.find("enum ").unwrap() + 5,
            });
        }

        // Match trait definitions: trait Name
        if let Some(name) = self.extract_keyword_symbol(trimmed, "trait ") {
            return Some(Symbol {
                name,
                kind: SymbolKind::Trait,
                file: file.to_path_buf(),
                line: line_num,
                column: line.find("trait ").unwrap() + 6,
            });
        }

        // Match module definitions: mod name
        if let Some(name) = self.extract_keyword_symbol(trimmed, "mod ") {
            return Some(Symbol {
                name,
                kind: SymbolKind::Module,
                file: file.to_path_buf(),
                line: line_num,
                column: line.find("mod ").unwrap() + 4,
            });
        }

        None
    }

    /// Extracts a symbol name following a keyword.
    fn extract_keyword_symbol<'a>(&self, line: &'a str, keyword: &str) -> Option<String> {
        if !line.starts_with(keyword) {
            return None;
        }

        let rest = &line[keyword.len()..];
        let name_end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());

        if name_end == 0 {
            return None;
        }

        Some(rest[..name_end].to_string())
    }

    /// Finds all symbols with the exact given name.
    ///
    /// # Arguments
    ///
    /// * `name` - The exact name to search for.
    ///
    /// # Returns
    ///
    /// A vector of references to matching symbols.
    pub fn find(&self, name: &str) -> Vec<&Symbol> {
        self.symbols
            .get(name)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Performs a fuzzy search for symbols matching the query.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query (case-insensitive substring match).
    ///
    /// # Returns
    ///
    /// A vector of references to matching symbols, sorted by relevance.
    pub fn search(&self, query: &str) -> Vec<&Symbol> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<&Symbol> = Vec::new();

        for symbols in self.symbols.values() {
            for symbol in symbols {
                if symbol.name.to_lowercase().contains(&query_lower) {
                    results.push(symbol);
                }
            }
        }

        // Sort by name for consistent output
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// Returns all symbols in a specific file.
    ///
    /// # Arguments
    ///
    /// * `file` - The file path to query.
    ///
    /// # Returns
    ///
    /// A vector of references to symbols in the file.
    pub fn symbols_in_file(&self, file: &Path) -> Vec<&Symbol> {
        self.by_file
            .get(file)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Returns the total number of indexed symbols.
    pub fn len(&self) -> usize {
        self.symbols.values().map(|v| v.len()).sum()
    }

    /// Returns true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Returns the number of files that have been indexed.
    pub fn file_count(&self) -> usize {
        self.by_file.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_kind_as_str() {
        assert_eq!(SymbolKind::Function.as_str(), "fn");
        assert_eq!(SymbolKind::Struct.as_str(), "struct");
        assert_eq!(SymbolKind::Enum.as_str(), "enum");
        assert_eq!(SymbolKind::Trait.as_str(), "trait");
        assert_eq!(SymbolKind::Module.as_str(), "mod");
        assert_eq!(SymbolKind::Variable.as_str(), "var");
    }

    #[test]
    fn test_extract_keyword_symbol() {
        let index = SymbolIndex::new();

        // Function extraction
        assert_eq!(
            index.extract_keyword_symbol("fn my_function", "fn "),
            Some("my_function".to_string())
        );

        // Struct extraction
        assert_eq!(
            index.extract_keyword_symbol("struct MyStruct", "struct "),
            Some("MyStruct".to_string())
        );

        // With generics
        assert_eq!(
            index.extract_keyword_symbol("struct Point<T>", "struct "),
            Some("Point".to_string())
        );

        // No match
        assert_eq!(index.extract_keyword_symbol("let x = 5", "fn "), None);
    }

    #[test]
    fn test_find_empty() {
        let index = SymbolIndex::new();
        assert!(index.find("nonexistent").is_empty());
    }

    #[test]
    fn test_search_empty() {
        let index = SymbolIndex::new();
        assert!(index.search("query").is_empty());
    }

    #[test]
    fn test_is_empty() {
        let index = SymbolIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(index.file_count(), 0);
    }
}
