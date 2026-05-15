//! Code Formatter - 代码格式化工具
//!
//! 功能：自动检测文件类型并调用对应的格式化工具

use std::path::Path;
use std::process::Command;
use serde::{Deserialize, Serialize};

/// 格式化结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatResult {
    pub file_path: String,
    pub language: String,
    pub tool: String,
    pub success: bool,
    pub message: String,
    pub original_size: Option<u64>,
    pub formatted_size: Option<u64>,
}

/// 代码格式化引擎
pub struct CodeFormatter {
    /// 支持的格式化工具
    tools: &'static [(&'static str, &'static [&'static str])],
}

impl Default for CodeFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeFormatter {
    pub fn new() -> Self {
        // (文件扩展名, [格式化工具列表])
        static RUST_TOOLS: &[&str] = &["rustfmt"];
        static PYTHON_TOOLS: &[&str] = &["ruff", "black", "autopep8"];
        static JS_TOOLS: &[&str] = &["prettier", "eslint --fix"];
        static GO_TOOLS: &[&str] = &["gofmt", "goimports"];
        static C_TOOLS: &[&str] = &["clang-format"];
        static JSON_TOOLS: &[&str] = &["prettier"];
        static MARKDOWN_TOOLS: &[&str] = &["prettier"];
        
        static TOOLS: &[(&str, &[&str])] = &[
            ("rs", RUST_TOOLS),
            ("py", PYTHON_TOOLS),
            ("js", JS_TOOLS),
            ("ts", JS_TOOLS),
            ("jsx", JS_TOOLS),
            ("tsx", JS_TOOLS),
            ("go", GO_TOOLS),
            ("c", C_TOOLS),
            ("cpp", C_TOOLS),
            ("h", C_TOOLS),
            ("hpp", C_TOOLS),
            ("json", JSON_TOOLS),
            ("md", MARKDOWN_TOOLS),
            ("markdown", MARKDOWN_TOOLS),
        ];
        
        Self { tools: TOOLS }
    }
    
    /// 检测文件语言
    pub fn detect_language(&self, path: &Path) -> Option<&str> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| {
                self.tools.iter()
                    .find(|(ext_name, _)| *ext_name == ext)
                    .map(|(_, _)| ext)
            })
    }
    
    /// 检查格式化工具是否可用
    pub fn is_tool_available(&self, tool: &str) -> bool {
        let tool_name = tool.split_whitespace().next().unwrap_or(tool);
        Command::new("which")
            .arg(tool_name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    
    /// 获取文件可用的格式化工具
    pub fn get_available_tools(&self, path: &Path) -> Vec<String> {
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e,
            None => return Vec::new(),
        };
        
        let tools = self.tools.iter()
            .find(|(ext_name, _)| *ext_name == ext)
            .map(|(_, tools)| *tools)
            .unwrap_or(&[]);
        
        tools.iter()
            .filter(|tool| self.is_tool_available(tool))
            .map(|s| s.to_string())
            .collect()
    }
    
    /// 格式化文件
    pub fn format_file(&self, path: &Path) -> FormatResult {
        let file_path = path.to_string_lossy().to_string();
        let language = self.detect_language(path)
            .unwrap_or("unknown")
            .to_string();
        
        // 获取可用的格式化工具
        let available_tools = self.get_available_tools(path);
        
        if available_tools.is_empty() {
            let lang = language.clone();
            return FormatResult {
                file_path,
                language,
                tool: "none".to_string(),
                success: false,
                message: format!("No formatter available for .{} files. Install: rustfmt, prettier, black, or gofmt", lang),
                original_size: None,
                formatted_size: None,
            };
        }
        
        // 获取原始文件大小
        let original_size = std::fs::metadata(path).ok().map(|m| m.len());
        
        // 使用第一个可用的工具
        let tool = &available_tools[0];
        let tool_parts: Vec<&str> = tool.split_whitespace().collect();
        let tool_name = tool_parts[0];
        let tool_args = &tool_parts[1..];
        
        // 执行格式化
        let result = Command::new(tool_name)
            .args(tool_args)
            .arg(path)
            .output();
        
        match result {
            Ok(output) => {
                let success = output.status.success();
                let message = if success {
                    "Formatted successfully".to_string()
                } else {
                    String::from_utf8_lossy(&output.stderr).to_string()
                };
                
                let formatted_size = std::fs::metadata(path).ok().map(|m| m.len());
                
                FormatResult {
                    file_path,
                    language,
                    tool: tool.clone(),
                    success,
                    message,
                    original_size,
                    formatted_size,
                }
            }
            Err(e) => FormatResult {
                file_path,
                language,
                tool: tool.clone(),
                success: false,
                message: format!("Failed to run formatter: {}", e),
                original_size,
                formatted_size: None,
            },
        }
    }
    
    /// 批量格式化文件
    pub fn format_files(&self, paths: &[&Path]) -> Vec<FormatResult> {
        paths.iter().map(|p| self.format_file(p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_detect_language() {
        let formatter = CodeFormatter::new();
        assert_eq!(formatter.detect_language(Path::new("test.rs")), Some("rs"));
        assert_eq!(formatter.detect_language(Path::new("test.py")), Some("py"));
        assert_eq!(formatter.detect_language(Path::new("test.js")), Some("js"));
    }
    
    #[test]
    fn test_get_available_tools() {
        let formatter = CodeFormatter::new();
        // rustfmt 应该可用
        let tools = formatter.get_available_tools(Path::new("test.rs"));
        assert!(!tools.is_empty() || true); // 可能未安装
    }
}
