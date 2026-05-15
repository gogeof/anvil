//! 并行工具执行器
//!
//! 功能：分析工具依赖，并行执行无依赖的工具

use std::collections::{HashMap, HashSet};
use serde_json::Value;

/// 工具调用
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub dependencies: Vec<String>, // 依赖的工具调用 ID
}

/// 并行执行器
pub struct ParallelExecutor {
    /// 工具调用队列
    calls: Vec<ToolCall>,
}

impl ParallelExecutor {
    pub fn new() -> Self {
        Self { calls: Vec::new() }
    }
    
    /// 添加工具调用
    pub fn add_call(&mut self, call: ToolCall) {
        self.calls.push(call);
    }
    
    /// 分析依赖关系
    pub fn analyze_dependencies(&self) -> HashMap<String, Vec<String>> {
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        
        // 规则：
        // 1. read_file, grep_search, glob_search 之间无依赖，可并行
        // 2. write_file, edit_file 依赖之前的 read_file
        // 3. bash 命令之间可能有依赖（保守处理）
        
        let mut read_ops: Vec<String> = Vec::new();
        let mut write_ops: Vec<String> = Vec::new();
        
        for call in &self.calls {
            match call.name.as_str() {
                "read_file" | "grep_search" | "glob_search" => {
                    read_ops.push(call.id.clone());
                }
                "write_file" | "edit_file" => {
                    // 写操作依赖之前的读操作
                    deps.insert(call.id.clone(), read_ops.clone());
                    write_ops.push(call.id.clone());
                }
                "bash" => {
                    // bash 命令依赖之前的写操作
                    deps.insert(call.id.clone(), write_ops.clone());
                }
                _ => {}
            }
        }
        
        deps
    }
    
    /// 获取可并行执行的批次
    pub fn get_parallel_batches(&self) -> Vec<Vec<&ToolCall>> {
        let deps = self.analyze_dependencies();
        let mut batches: Vec<Vec<&ToolCall>> = Vec::new();
        let mut executed: HashSet<String> = HashSet::new();
        
        let mut remaining: Vec<&ToolCall> = self.calls.iter().collect();
        
        while !remaining.is_empty() {
            // 找出当前可执行的调用（依赖已满足）
            let mut batch: Vec<&ToolCall> = Vec::new();
            let mut next_remaining: Vec<&ToolCall> = Vec::new();
            
            for call in remaining {
                let call_deps = deps.get(&call.id).cloned().unwrap_or_default();
                let all_deps_satisfied = call_deps.iter().all(|dep| executed.contains(dep));
                
                if all_deps_satisfied {
                    batch.push(call);
                } else {
                    next_remaining.push(call);
                }
            }
            
            if !batch.is_empty() {
                batches.push(batch.clone());
                for call in batch {
                    executed.insert(call.id.clone());
                }
            }
            
            remaining = next_remaining;
        }
        
        batches
    }
}

impl Default for ParallelExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parallel_batches() {
        let mut executor = ParallelExecutor::new();
        
        // 添加多个读取操作（可并行）
        executor.add_call(ToolCall {
            id: "1".to_string(),
            name: "read_file".to_string(),
            input: serde_json::json!({"path": "a.rs"}),
            dependencies: vec![],
        });
        
        executor.add_call(ToolCall {
            id: "2".to_string(),
            name: "read_file".to_string(),
            input: serde_json::json!({"path": "b.rs"}),
            dependencies: vec![],
        });
        
        // 添加写操作（依赖前面的读）
        executor.add_call(ToolCall {
            id: "3".to_string(),
            name: "edit_file".to_string(),
            input: serde_json::json!({"path": "a.rs"}),
            dependencies: vec![],
        });
        
        let batches = executor.get_parallel_batches();
        
        // 第一批应该包含两个 read_file
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 2); // 两个读操作并行
        assert_eq!(batches[1].len(), 1); // 一个写操作
    }
}
