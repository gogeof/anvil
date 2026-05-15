//! Smart Summary Engine - 智能摘要引擎
//!
//! 功能：分析大文件，提取关键部分（函数签名、类定义、import）

use std::path::Path;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// 函数/方法签名
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub name: String,
    pub kind: String, // "function", "struct", "class", "interface", "enum"
    pub line_start: usize,
    pub line_end: usize,
    pub params: Option<String>,
    pub return_type: Option<String>,
}

/// 文件摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSummary {
    pub file_path: String,
    pub total_lines: usize,
    pub summary_lines: usize,
    pub signatures: Vec<Signature>,
    pub imports: Vec<String>,
    pub key_sections: Vec<Section>,
}

/// 关键代码段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub kind: String, // "signature", "import", "constant", "main"
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
}

/// 智能摘要引擎
pub struct SmartSummaryEngine {
    /// 语言检测器
    language_map: HashMap<&'static str, &'static str>,
}

impl Default for SmartSummaryEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SmartSummaryEngine {
    pub fn new() -> Self {
        let mut language_map = HashMap::new();
        language_map.insert("rs", "rust");
        language_map.insert("py", "python");
        language_map.insert("js", "javascript");
        language_map.insert("ts", "typescript");
        language_map.insert("go", "go");
        language_map.insert("java", "java");
        language_map.insert("c", "c");
        language_map.insert("cpp", "cpp");
        language_map.insert("h", "c");
        language_map.insert("hpp", "cpp");
        
        Self { language_map }
    }
    
    /// 从文件路径检测语言
    pub fn detect_language(&self, path: &Path) -> Option<&str> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| self.language_map.get(ext))
            .copied()
    }
    
    /// 生成文件摘要
    pub fn summarize_file(&self, path: &Path, max_lines: usize) -> Result<FileSummary, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read file: {}", e))?;
        
        let language = self.detect_language(path).unwrap_or("text");
        let total_lines = content.lines().count();
        
        let signatures = self.extract_signatures(&content, language);
        let imports = self.extract_imports(&content, language);
        
        // 构建关键段落
        let mut key_sections = Vec::new();
        
        // 添加 imports
        if !imports.is_empty() {
            let import_lines: Vec<&str> = content.lines()
                .filter(|line| {
                    language == "rust" && line.trim().starts_with("use ") ||
                    language == "python" && (line.trim().starts_with("import ") || line.trim().starts_with("from ")) ||
                    (language == "javascript" || language == "typescript") && 
                        (line.trim().starts_with("import ") || line.trim().starts_with("require("))
                })
                .collect();
            
            if !import_lines.is_empty() {
                key_sections.push(Section {
                    kind: "import".to_string(),
                    line_start: 1,
                    line_end: import_lines.len(),
                    content: import_lines.join("\n"),
                });
            }
        }
        
        // 添加签名
        for sig in &signatures {
            let lines: Vec<&str> = content.lines()
                .skip(sig.line_start - 1)
                .take(sig.line_end - sig.line_start + 1)
                .collect();
            
            key_sections.push(Section {
                kind: "signature".to_string(),
                line_start: sig.line_start,
                line_end: sig.line_end,
                content: lines.join("\n"),
            });
        }
        
        // 计算摘要行数
        let summary_lines: usize = key_sections.iter()
            .map(|s| s.line_end - s.line_start + 1)
            .sum();
        
        Ok(FileSummary {
            file_path: path.to_string_lossy().to_string(),
            total_lines,
            summary_lines,
            signatures,
            imports,
            key_sections,
        })
    }
    
    /// 提取函数/类签名
    pub fn extract_signatures(&self, content: &str, language: &str) -> Vec<Signature> {
        let mut signatures = Vec::new();
        
        match language {
            "rust" => self.extract_rust_signatures(content, &mut signatures),
            "python" => self.extract_python_signatures(content, &mut signatures),
            "javascript" | "typescript" => self.extract_js_signatures(content, &mut signatures),
            "go" => self.extract_go_signatures(content, &mut signatures),
            _ => {}
        }
        
        signatures
    }
    
    /// 提取 Rust 签名
    fn extract_rust_signatures(&self, content: &str, signatures: &mut Vec<Signature>) {
        let lines: Vec<&str> = content.lines().collect();
        
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            
            // fn name(
            if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") || trimmed.starts_with("async fn ") {
                if let Some(name) = self.extract_rust_fn_name(trimmed) {
                    let line_end = self.find_block_end(&lines, i);
                    signatures.push(Signature {
                        name,
                        kind: "function".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
            // struct Name
            else if trimmed.starts_with("struct ") || trimmed.starts_with("pub struct ") {
                if let Some(name) = trimmed.split_whitespace().nth(1) {
                    let name = name.trim_end_matches('<').trim_end_matches('{').to_string();
                    let line_end = self.find_block_end(&lines, i);
                    signatures.push(Signature {
                        name,
                        kind: "struct".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
            // enum Name
            else if trimmed.starts_with("enum ") || trimmed.starts_with("pub enum ") {
                if let Some(name) = trimmed.split_whitespace().nth(1) {
                    let name = name.trim_end_matches('{').to_string();
                    let line_end = self.find_block_end(&lines, i);
                    signatures.push(Signature {
                        name,
                        kind: "enum".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
        }
    }
    
    /// 提取 Python 签名
    fn extract_python_signatures(&self, content: &str, signatures: &mut Vec<Signature>) {
        let lines: Vec<&str> = content.lines().collect();
        
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            
            // def name(
            if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
                if let Some(name) = self.extract_python_fn_name(trimmed) {
                    let line_end = self.find_python_block_end(&lines, i, line.chars().take_while(|c| c.is_whitespace()).count());
                    signatures.push(Signature {
                        name,
                        kind: "function".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
            // class Name
            else if trimmed.starts_with("class ") {
                if let Some(name) = trimmed.split_whitespace().nth(1) {
                    let name = name.split('(').next().unwrap_or(name).split(':').next().unwrap_or(name).to_string();
                    let line_end = self.find_python_block_end(&lines, i, line.chars().take_while(|c| c.is_whitespace()).count());
                    signatures.push(Signature {
                        name,
                        kind: "class".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
        }
    }
    
    /// 提取 JavaScript/TypeScript 签名
    fn extract_js_signatures(&self, content: &str, signatures: &mut Vec<Signature>) {
        let lines: Vec<&str> = content.lines().collect();
        
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            
            // function name(
            if trimmed.starts_with("function ") {
                if let Some(name) = trimmed.split_whitespace().nth(1) {
                    let name = name.split('(').next().unwrap_or(name).to_string();
                    let line_end = self.find_block_end(&lines, i);
                    signatures.push(Signature {
                        name,
                        kind: "function".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
            // const name = ( or const name = async (
            else if trimmed.starts_with("const ") && trimmed.contains("=>") {
                if let Some(name) = trimmed.split('=').next() {
                    let name = name.replace("const", "").trim().trim_end_matches(':').to_string();
                    let line_end = self.find_block_end(&lines, i);
                    signatures.push(Signature {
                        name,
                        kind: "function".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
            // class Name
            else if trimmed.starts_with("class ") {
                if let Some(name) = trimmed.split_whitespace().nth(1) {
                    let name = name.split('{').next().unwrap_or(name).split('<').next().unwrap_or(name).to_string();
                    let line_end = self.find_block_end(&lines, i);
                    signatures.push(Signature {
                        name,
                        kind: "class".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
        }
    }
    
    /// 提取 Go 签名
    fn extract_go_signatures(&self, content: &str, signatures: &mut Vec<Signature>) {
        let lines: Vec<&str> = content.lines().collect();
        
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            
            // func name(
            if trimmed.starts_with("func ") {
                if let Some(name) = self.extract_go_fn_name(trimmed) {
                    let line_end = self.find_block_end(&lines, i);
                    signatures.push(Signature {
                        name,
                        kind: "function".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
            // type Name struct
            else if trimmed.starts_with("type ") && trimmed.contains("struct") {
                if let Some(name) = trimmed.split_whitespace().nth(1) {
                    let line_end = self.find_block_end(&lines, i);
                    signatures.push(Signature {
                        name: name.to_string(),
                        kind: "struct".to_string(),
                        line_start: i + 1,
                        line_end: line_end + 1,
                        params: None,
                        return_type: None,
                    });
                }
            }
        }
    }
    
    /// 提取 import 语句
    pub fn extract_imports(&self, content: &str, language: &str) -> Vec<String> {
        let mut imports = Vec::new();
        
        for line in content.lines() {
            let trimmed = line.trim();
            
            match language {
                "rust" => {
                    if trimmed.starts_with("use ") {
                        imports.push(trimmed.to_string());
                    }
                }
                "python" => {
                    if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                        imports.push(trimmed.to_string());
                    }
                }
                "javascript" | "typescript" => {
                    if trimmed.starts_with("import ") || trimmed.starts_with("require(") || trimmed.starts_with("export ") {
                        imports.push(trimmed.to_string());
                    }
                }
                "go" => {
                    if trimmed.starts_with("import ") || trimmed.starts_with('"') {
                        imports.push(trimmed.to_string());
                    }
                }
                _ => {}
            }
        }
        
        imports
    }
    
    // Helper methods
    fn extract_rust_fn_name(&self, line: &str) -> Option<String> {
        let line = line.replace("pub ", "").replace("async ", "").replace("fn ", "");
        let name = line.split('(').next()?;
        Some(name.trim().to_string())
    }
    
    fn extract_python_fn_name(&self, line: &str) -> Option<String> {
        let line = line.replace("async ", "").replace("def ", "");
        let name = line.split('(').next()?;
        Some(name.trim().to_string())
    }
    
    fn extract_go_fn_name(&self, line: &str) -> Option<String> {
        let line = line.replace("func ", "");
        // Handle methods: (receiver) name
        let line = if line.starts_with('(') {
            line.split(')').nth(1)?.trim()
        } else {
            &line
        };
        let name = line.split('(').next()?;
        Some(name.trim().to_string())
    }
    
    fn find_block_end(&self, lines: &[&str], start: usize) -> usize {
        let mut brace_count = 0;
        let mut found_open = false;
        
        for (i, line) in lines.iter().enumerate().skip(start) {
            for c in line.chars() {
                match c {
                    '{' => {
                        brace_count += 1;
                        found_open = true;
                    }
                    '}' => {
                        brace_count -= 1;
                        if found_open && brace_count == 0 {
                            return i;
                        }
                    }
                    _ => {}
                }
            }
        }
        
        start
    }
    
    fn find_python_block_end(&self, lines: &[&str], start: usize, base_indent: usize) -> usize {
        for (i, line) in lines.iter().enumerate().skip(start + 1) {
            let line_indent = line.chars().take_while(|c| c.is_whitespace()).count();
            if !line.trim().is_empty() && line_indent <= base_indent {
                return i - 1;
            }
        }
        lines.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_rust_signatures() {
        let engine = SmartSummaryEngine::new();
        let code = r#"
use std::collections::HashMap;

pub struct Config {
    name: String,
}

fn main() {
    println!("Hello");
}

pub async fn fetch_data(url: &str) -> Result<String, Error> {
    // ...
}
"#;
        let sigs = engine.extract_signatures(code, "rust");
        assert_eq!(sigs.len(), 3);
        assert_eq!(sigs[0].name, "Config");
        assert_eq!(sigs[1].name, "main");
        assert_eq!(sigs[2].name, "fetch_data");
    }
    
    #[test]
    fn test_extract_python_signatures() {
        let engine = SmartSummaryEngine::new();
        let code = r#"
import os

class DataProcessor:
    def __init__(self):
        pass
    
    def process(self, data):
        return data

async def fetch(url):
    pass
"#;
        let sigs = engine.extract_signatures(code, "python");
        assert_eq!(sigs.len(), 3);
        assert_eq!(sigs[0].name, "DataProcessor");
        assert_eq!(sigs[1].name, "process");
        assert_eq!(sigs[2].name, "fetch");
    }
}
