//! 函数级读取工具
//!
//! 功能：只读取指定函数，而非整个文件

use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::smart_summary::{SmartSummaryEngine, Signature};

/// 函数读取输入
#[derive(Debug, Deserialize)]
pub struct ReadFunctionInput {
    pub path: String,
    pub function_name: String,
}

/// 函数读取结果
#[derive(Debug, Serialize)]
pub struct ReadFunctionResult {
    pub path: String,
    pub function_name: String,
    pub found: bool,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
    pub code: Option<String>,
    pub message: String,
}

/// 读取指定函数
pub fn read_function(input: ReadFunctionInput) -> Result<ReadFunctionResult, String> {
    let path = Path::new(&input.path);
    let func_name = input.function_name.clone();
    
    // 使用智能摘要引擎提取签名
    let engine = SmartSummaryEngine::new();
    let summary = engine.summarize_file(path, 0)?;
    
    // 查找指定函数
    let signature = summary.signatures.iter()
        .find(|sig| sig.name == func_name);
    
    match signature {
        Some(sig) => {
            // 读取函数代码
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read file: {}", e))?;
            
            let lines: Vec<&str> = content.lines()
                .skip(sig.line_start - 1)
                .take(sig.line_end - sig.line_start + 1)
                .collect();
            
            let code = lines.join("\n");
            let name = input.function_name.clone();
            
            Ok(ReadFunctionResult {
                path: input.path,
                function_name: input.function_name,
                found: true,
                line_start: Some(sig.line_start),
                line_end: Some(sig.line_end),
                code: Some(code),
                message: format!("Found function '{}' at lines {}-{}", 
                    name, sig.line_start, sig.line_end),
            })
        }
        None => {
            // 列出所有可用函数
            let available: Vec<String> = summary.signatures.iter()
                .map(|sig| sig.name.clone())
                .collect();
            let name = input.function_name.clone();
            
            Ok(ReadFunctionResult {
                path: input.path,
                function_name: input.function_name,
                found: false,
                line_start: None,
                line_end: None,
                code: None,
                message: format!("Function '{}' not found. Available functions: {:?}", 
                    name, available),
            })
        }
    }
}
