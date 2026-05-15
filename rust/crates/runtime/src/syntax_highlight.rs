//! Syntax highlighting and code structure analysis.
//!
//! Provides syntax-aware code highlighting and structure extraction
//! using the syntect library for multiple programming languages.

use std::path::Path;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use serde::{Deserialize, Serialize};

/// Global syntax set for parsing (lazy loaded)
static SYNTAX_SET: std::sync::OnceLock<SyntaxSet> = std::sync::OnceLock::new();
static THEME_SET: std::sync::OnceLock<ThemeSet> = std::sync::OnceLock::new();

fn get_syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(|| SyntaxSet::load_defaults_newlines())
}

fn get_theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Supported programming languages for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Java,
    C,
    Cpp,
    Ruby,
    Php,
    Swift,
    Kotlin,
    Scala,
    Html,
    Css,
    Json,
    Yaml,
    Toml,
    Markdown,
    Shell,
    Sql,
    Unknown,
}

impl Language {
    /// Get the file extensions for this language.
    pub fn extensions(&self) -> &[&'static str] {
        match self {
            Language::Rust => &["rs"],
            Language::Python => &["py", "pyw", "pyi"],
            Language::JavaScript => &["js", "mjs", "cjs"],
            Language::TypeScript => &["ts", "tsx"],
            Language::Go => &["go"],
            Language::Java => &["java"],
            Language::C => &["c", "h"],
            Language::Cpp => &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
            Language::Ruby => &["rb", "rake", "gemspec"],
            Language::Php => &["php"],
            Language::Swift => &["swift"],
            Language::Kotlin => &["kt", "kts"],
            Language::Scala => &["scala", "sc"],
            Language::Html => &["html", "htm"],
            Language::Css => &["css", "scss", "sass"],
            Language::Json => &["json"],
            Language::Yaml => &["yaml", "yml"],
            Language::Toml => &["toml"],
            Language::Markdown => &["md", "markdown"],
            Language::Shell => &["sh", "bash", "zsh", "fish"],
            Language::Sql => &["sql"],
            Language::Unknown => &[],
        }
    }

    /// Get the syntect syntax name for this language.
    pub fn syntax_name(&self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Go => "Go",
            Language::Java => "Java",
            Language::C => "C",
            Language::Cpp => "C++",
            Language::Ruby => "Ruby",
            Language::Php => "PHP",
            Language::Swift => "Swift",
            Language::Kotlin => "Kotlin",
            Language::Scala => "Scala",
            Language::Html => "HTML",
            Language::Css => "CSS",
            Language::Json => "JSON",
            Language::Yaml => "YAML",
            Language::Toml => "TOML",
            Language::Markdown => "Markdown",
            Language::Shell => "Bash",
            Language::Sql => "SQL",
            Language::Unknown => "Plain Text",
        }
    }

    /// Check if this language is one of the core 5+ supported languages.
    pub fn is_core_supported(&self) -> bool {
        matches!(
            self,
            Language::Rust
                | Language::Python
                | Language::JavaScript
                | Language::TypeScript
                | Language::Go
                | Language::Java
        )
    }
}

/// Detect language from file path based on extension.
pub fn detect_language(file_path: &str) -> Option<Language> {
    let path = Path::new(file_path);
    let extension = path.extension()?.to_str()?.to_lowercase();
    
    // Check each language's extensions
    for lang in [
        Language::Rust,
        Language::Python,
        Language::JavaScript,
        Language::TypeScript,
        Language::Go,
        Language::Java,
        Language::C,
        Language::Cpp,
        Language::Ruby,
        Language::Php,
        Language::Swift,
        Language::Kotlin,
        Language::Scala,
        Language::Html,
        Language::Css,
        Language::Json,
        Language::Yaml,
        Language::Toml,
        Language::Markdown,
        Language::Shell,
        Language::Sql,
    ] {
        if lang.extensions().contains(&extension.as_str()) {
            return Some(lang);
        }
    }
    
    None
}

/// Detect language from file content (heuristic).
pub fn detect_language_from_content(content: &str) -> Option<Language> {
    let content = content.trim();
    
    // Rust: fn main, use std, etc.
    if content.starts_with("fn ") || content.contains("fn main") || content.starts_with("use std") {
        return Some(Language::Rust);
    }
    
    // Python: def, import, class
    if content.starts_with("def ") || content.starts_with("import ") || content.starts_with("class ") {
        return Some(Language::Python);
    }
    
    // JavaScript: const, let, var, function
    if content.starts_with("const ") || content.starts_with("let ") || content.starts_with("var ") || content.starts_with("function ") {
        return Some(Language::JavaScript);
    }
    
    // Go: package main, func main
    if content.starts_with("package ") || content.contains("func main") {
        return Some(Language::Go);
    }
    
    // Java: public class, import java
    if content.starts_with("package ") && content.contains(";") || content.starts_with("public class ") || content.starts_with("import java.") {
        return Some(Language::Java);
    }
    
    None
}

/// A code block representing a structural element.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    /// Block type (function, class, struct, etc.)
    pub block_type: CodeBlockType,
    /// Block name
    pub name: String,
    /// Start line (1-indexed)
    pub start_line: usize,
    /// End line (1-indexed)
    pub end_line: usize,
    /// Nested children
    pub children: Vec<CodeBlock>,
}

/// Type of code block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeBlockType {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Interface,
    Module,
    Trait,
    Variable,
    Constant,
    Import,
    Comment,
    Other,
}

/// Highlight code with syntax coloring.
///
/// Returns ANSI-colored code string.
pub fn highlight_code(code: &str, language: Language) -> String {
    let ss = get_syntax_set();
    let ts = get_theme_set();
    
    let syntax = ss.find_syntax_by_name(language.syntax_name())
        .or_else(|| Some(ss.find_syntax_plain_text()))
        .unwrap();
    
    let theme: &Theme = ts.themes.get("base16-ocean.dark")
        .or_else(|| ts.themes.values().next())
        .unwrap();
    
    let mut h = HighlightLines::new(syntax, theme);
    let mut result = String::new();
    
    for line in LinesWithEndings::from(code) {
        let ranges: Vec<(syntect::highlighting::Style, &str)> = h.highlight_line(line, ss).unwrap_or_default();
        for (style, text) in ranges {
            // Convert to ANSI escape codes
            let fg = style.foreground;
            result.push_str(&format!(
                "\x1b[38;2;{};{};{}m{}\x1b[0m",
                fg.r, fg.g, fg.b, text
            ));
        }
    }
    
    result
}

/// Highlight code with a specific theme.
pub fn highlight_code_with_theme(code: &str, language: Language, theme_name: &str) -> String {
    let ss = get_syntax_set();
    let ts = get_theme_set();
    
    let syntax = ss.find_syntax_by_name(language.syntax_name())
        .or_else(|| Some(ss.find_syntax_plain_text()))
        .unwrap();
    
    let theme: &Theme = ts.themes.get(theme_name)
        .or_else(|| ts.themes.values().next())
        .unwrap();
    
    let mut h = HighlightLines::new(syntax, theme);
    let mut result = String::new();
    
    for line in LinesWithEndings::from(code) {
        let ranges: Vec<(syntect::highlighting::Style, &str)> = h.highlight_line(line, ss).unwrap_or_default();
        for (style, text) in ranges {
            let fg = style.foreground;
            result.push_str(&format!(
                "\x1b[38;2;{};{};{}m{}\x1b[0m",
                fg.r, fg.g, fg.b, text
            ));
        }
    }
    
    result
}

/// Get code structure (functions, classes, etc.).
///
/// Uses regex-based parsing for fast structure extraction.
pub fn get_code_structure(code: &str, language: Language) -> Vec<CodeBlock> {
    match language {
        Language::Rust => parse_rust_structure(code),
        Language::Python => parse_python_structure(code),
        Language::JavaScript | Language::TypeScript => parse_js_structure(code),
        Language::Go => parse_go_structure(code),
        Language::Java => parse_java_structure(code),
        _ => parse_generic_structure(code),
    }
}

/// Parse Rust code structure.
fn parse_rust_structure(code: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;
        
        // Match fn name
        if let Some(name) = extract_rust_fn_name(trimmed) {
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Function,
                name,
                start_line: line_num,
                end_line: line_num, // Will be updated
                children: vec![],
            });
        }
        // Match struct
        else if trimmed.starts_with("struct ") {
            let name = trimmed["struct ".len()..].split(|c| c == '<' || c == '{' || c == ' ')
                .next()
                .unwrap_or("unknown")
                .trim()
                .to_string();
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Struct,
                name,
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
        // Match enum
        else if trimmed.starts_with("enum ") {
            let name = trimmed["enum ".len()..].split(|c| c == '<' || c == '{' || c == ' ')
                .next()
                .unwrap_or("unknown")
                .trim()
                .to_string();
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Enum,
                name,
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
        // Match trait
        else if trimmed.starts_with("trait ") {
            let name = trimmed["trait ".len()..].split(|c| c == '<' || c == '{' || c == ':')
                .next()
                .unwrap_or("unknown")
                .trim()
                .to_string();
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Trait,
                name,
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
    }
    
    blocks
}

/// Extract Rust function name from line.
fn extract_rust_fn_name(line: &str) -> Option<String> {
    if !line.starts_with("fn ") && !line.starts_with("pub fn ") && !line.starts_with("async fn ") {
        return None;
    }
    
    // Remove pub, async, etc.
    let line = line.trim_start_matches("pub ")
        .trim_start_matches("async ")
        .trim_start_matches("fn ");
    
    let name = line.split('(').next()?.trim();
    Some(name.to_string())
}

/// Parse Python code structure.
fn parse_python_structure(code: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;
        
        // Match def name
        if trimmed.starts_with("def ") {
            let rest = &trimmed["def ".len()..];
            let name = rest.split('(').next().unwrap_or("unknown").trim();
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Function,
                name: name.to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
        // Match class
        else if trimmed.starts_with("class ") {
            let rest = &trimmed["class ".len()..];
            let name = rest.split('(').next().unwrap_or("unknown")
                .trim()
                .trim_end_matches(':');
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Class,
                name: name.to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
    }
    
    blocks
}

/// Parse JavaScript/TypeScript code structure.
fn parse_js_structure(code: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;
        
        // Match function name
        if trimmed.starts_with("function ") {
            let rest = &trimmed["function ".len()..];
            let name = rest.split('(').next().unwrap_or("unknown").trim();
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Function,
                name: name.to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
        // Match const/let/var name = function
        else if trimmed.starts_with("const ") || trimmed.starts_with("let ") || trimmed.starts_with("var ") {
            if trimmed.contains("= function") || trimmed.contains("=>") {
                let name = trimmed.split('=').next().unwrap_or("unknown")
                    .trim()
                    .trim_start_matches("const ")
                    .trim_start_matches("let ")
                    .trim_start_matches("var ")
                    .trim();
                blocks.push(CodeBlock {
                    block_type: CodeBlockType::Function,
                    name: name.to_string(),
                    start_line: line_num,
                    end_line: line_num,
                    children: vec![],
                });
            }
        }
        // Match class
        else if trimmed.starts_with("class ") {
            let rest = &trimmed["class ".len()..];
            let name = rest.split(|c| c == ' ' || c == '{' || c == 'e')
                .filter(|s| !s.is_empty() && *s != "extends")
                .next()
                .unwrap_or("unknown");
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Class,
                name: name.to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
    }
    
    blocks
}

/// Parse Go code structure.
fn parse_go_structure(code: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;
        
        // Match func name
        if trimmed.starts_with("func ") {
            let rest = &trimmed["func ".len()..];
            // Handle methods: func (r *Receiver) Name()
            let name = if rest.starts_with('(') {
                // Method
                rest.split(')')
                    .nth(1)
                    .and_then(|s| s.trim().split('(').next())
                    .unwrap_or("unknown")
            } else {
                // Function
                rest.split('(').next().unwrap_or("unknown")
            };
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Function,
                name: name.trim().to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
        // Match type struct
        else if trimmed.starts_with("type ") && trimmed.contains(" struct") {
            let rest = &trimmed["type ".len()..];
            let name = rest.split(' ').next().unwrap_or("unknown");
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Struct,
                name: name.to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
        // Match type interface
        else if trimmed.starts_with("type ") && trimmed.contains(" interface") {
            let rest = &trimmed["type ".len()..];
            let name = rest.split(' ').next().unwrap_or("unknown");
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Interface,
                name: name.to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
    }
    
    blocks
}

/// Parse Java code structure.
fn parse_java_structure(code: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;
        
        // Match class
        if trimmed.contains(" class ") {
            let name = trimmed.split(" class ")
                .nth(1)
                .and_then(|s| s.split(|c| c == ' ' || c == '{').next())
                .unwrap_or("unknown");
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Class,
                name: name.to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
        // Match interface
        else if trimmed.contains(" interface ") {
            let name = trimmed.split(" interface ")
                .nth(1)
                .and_then(|s| s.split(|c| c == ' ' || c == '{').next())
                .unwrap_or("unknown");
            blocks.push(CodeBlock {
                block_type: CodeBlockType::Interface,
                name: name.to_string(),
                start_line: line_num,
                end_line: line_num,
                children: vec![],
            });
        }
        // Match method (simplified)
        else if trimmed.contains("(") && trimmed.contains(")") && trimmed.ends_with("{") {
            // Extract method name (simplified)
            let name = trimmed.split('(')
                .next()
                .and_then(|s| s.split_whitespace().last())
                .unwrap_or("unknown");
            
            if !name.is_empty() && name != "if" && name != "for" && name != "while" && name != "switch" {
                blocks.push(CodeBlock {
                    block_type: CodeBlockType::Method,
                    name: name.to_string(),
                    start_line: line_num,
                    end_line: line_num,
                    children: vec![],
                });
            }
        }
    }
    
    blocks
}

/// Parse generic code structure (fallback).
fn parse_generic_structure(code: &str) -> Vec<CodeBlock> {
    // Return empty for unknown languages
    let _ = code;
    vec![]
}

/// List available themes.
pub fn list_themes() -> Vec<&'static str> {
    let ts = get_theme_set();
    ts.themes.keys().map(|s| s.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_from_extension() {
        assert_eq!(detect_language("main.rs"), Some(Language::Rust));
        assert_eq!(detect_language("lib.rs"), Some(Language::Rust));
    }

    #[test]
    fn detects_python_from_extension() {
        assert_eq!(detect_language("main.py"), Some(Language::Python));
        assert_eq!(detect_language("script.pyw"), Some(Language::Python));
    }

    #[test]
    fn detects_javascript_from_extension() {
        assert_eq!(detect_language("index.js"), Some(Language::JavaScript));
        assert_eq!(detect_language("module.mjs"), Some(Language::JavaScript));
    }

    #[test]
    fn detects_go_from_extension() {
        assert_eq!(detect_language("main.go"), Some(Language::Go));
    }

    #[test]
    fn detects_java_from_extension() {
        assert_eq!(detect_language("Main.java"), Some(Language::Java));
    }

    #[test]
    fn detects_rust_from_content() {
        let code = "fn main() { println!(\"Hello\"); }";
        assert_eq!(detect_language_from_content(code), Some(Language::Rust));
    }

    #[test]
    fn detects_python_from_content() {
        let code = "def main():\n    print('Hello')";
        assert_eq!(detect_language_from_content(code), Some(Language::Python));
    }

    #[test]
    fn highlights_rust_code() {
        let code = "fn main() { let x = 1; }";
        let highlighted = highlight_code(code, Language::Rust);
        assert!(highlighted.contains("\x1b[")); // Contains ANSI codes
    }

    #[test]
    fn parses_rust_structure() {
        let code = r#"
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    println!("Hello");
}
"#;
        let blocks = get_code_structure(code, Language::Rust);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].block_type, CodeBlockType::Struct);
        assert_eq!(blocks[0].name, "Point");
        assert_eq!(blocks[1].block_type, CodeBlockType::Function);
        assert_eq!(blocks[1].name, "main");
    }

    #[test]
    fn parses_python_structure() {
        let code = r#"
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

def main():
    print("Hello")
"#;
        let blocks = get_code_structure(code, Language::Python);
        // 3 blocks: class Point, def __init__, def main
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].block_type, CodeBlockType::Class);
        assert_eq!(blocks[0].name, "Point");
    }

    #[test]
    fn parses_go_structure() {
        let code = r#"
package main

type Point struct {
    X int
    Y int
}

func main() {
    println("Hello")
}
"#;
        let blocks = get_code_structure(code, Language::Go);
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn lists_available_themes() {
        let themes = list_themes();
        assert!(!themes.is_empty());
    }
}
