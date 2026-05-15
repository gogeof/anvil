//! Git Diff Visualization - Git diff 可视化工具
//!
//! 功能：生成彩色的 git diff 输出，支持多种格式

use std::path::Path;
use std::process::Command;
use serde::{Deserialize, Serialize};

/// Diff 结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub file_path: String,
    pub success: bool,
    pub has_changes: bool,
    pub additions: usize,
    pub deletions: usize,
    pub diff_output: String,
    pub error: Option<String>,
}

/// Git Diff 引擎
pub struct GitDiffEngine {
    repo_root: Option<std::path::PathBuf>,
}

impl GitDiffEngine {
    pub fn new() -> Self {
        // 尝试找到 git 仓库根目录
        let repo_root = Self::find_git_root();
        Self { repo_root }
    }
    
    pub fn with_root(path: &Path) -> Self {
        Self {
            repo_root: Some(path.to_path_buf()),
        }
    }
    
    /// 查找 git 仓库根目录
    fn find_git_root() -> Option<std::path::PathBuf> {
        Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .to_string()
                    .into()
            })
            .map(|s| std::path::PathBuf::from(s))
    }
    
    /// 获取工作区变更（未暂存的变更）
    pub fn diff_workdir(&self, path: Option<&Path>) -> DiffResult {
        let file_path = path
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "all files".to_string());
        
        let mut args = vec!["diff", "--color=always"];
        if let Some(p) = path {
            args.push("--");
            args.push(p.to_str().unwrap_or("."));
        }
        
        let result = Command::new("git")
            .args(&args)
            .current_dir(self.repo_root.as_deref().unwrap_or(Path::new(".")))
            .output();
        
        match result {
            Ok(output) => {
                let diff = String::from_utf8_lossy(&output.stdout);
                let (additions, deletions) = self.count_changes(&diff);
                
                DiffResult {
                    file_path,
                    success: true,
                    has_changes: !diff.trim().is_empty(),
                    additions,
                    deletions,
                    diff_output: diff.to_string(),
                    error: None,
                }
            }
            Err(e) => DiffResult {
                file_path,
                success: false,
                has_changes: false,
                additions: 0,
                deletions: 0,
                diff_output: String::new(),
                error: Some(e.to_string()),
            },
        }
    }
    
    /// 获取暂存区变更（已 git add 的变更）
    pub fn diff_cached(&self, path: Option<&Path>) -> DiffResult {
        let file_path = path
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "all staged files".to_string());
        
        let mut args = vec!["diff", "--cached", "--color=always"];
        if let Some(p) = path {
            args.push("--");
            args.push(p.to_str().unwrap_or("."));
        }
        
        let result = Command::new("git")
            .args(&args)
            .current_dir(self.repo_root.as_deref().unwrap_or(Path::new(".")))
            .output();
        
        match result {
            Ok(output) => {
                let diff = String::from_utf8_lossy(&output.stdout);
                let (additions, deletions) = self.count_changes(&diff);
                
                DiffResult {
                    file_path,
                    success: true,
                    has_changes: !diff.trim().is_empty(),
                    additions,
                    deletions,
                    diff_output: diff.to_string(),
                    error: None,
                }
            }
            Err(e) => DiffResult {
                file_path,
                success: false,
                has_changes: false,
                additions: 0,
                deletions: 0,
                diff_output: String::new(),
                error: Some(e.to_string()),
            },
        }
    }
    
    /// 获取所有变更（工作区 + 暂存区）
    pub fn diff_all(&self) -> DiffResult {
        let result = Command::new("git")
            .args(["diff", "HEAD", "--color=always"])
            .current_dir(self.repo_root.as_deref().unwrap_or(Path::new(".")))
            .output();
        
        match result {
            Ok(output) => {
                let diff = String::from_utf8_lossy(&output.stdout);
                let (additions, deletions) = self.count_changes(&diff);
                
                DiffResult {
                    file_path: "all changes".to_string(),
                    success: true,
                    has_changes: !diff.trim().is_empty(),
                    additions,
                    deletions,
                    diff_output: diff.to_string(),
                    error: None,
                }
            }
            Err(e) => DiffResult {
                file_path: "all changes".to_string(),
                success: false,
                has_changes: false,
                additions: 0,
                deletions: 0,
                diff_output: String::new(),
                error: Some(e.to_string()),
            },
        }
    }
    
    /// 获取两个 commit 之间的差异
    pub fn diff_commits(&self, from: &str, to: &str, path: Option<&Path>) -> DiffResult {
        let file_path = path
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}..{}", from, to));
        
        let mut args = vec!["diff", "--color=always", &format!("{}..{}", from, to)];
        if let Some(p) = path {
            args.push("--");
            args.push(p.to_str().unwrap_or("."));
        }
        
        let result = Command::new("git")
            .args(&args)
            .current_dir(self.repo_root.as_deref().unwrap_or(Path::new(".")))
            .output();
        
        match result {
            Ok(output) => {
                let diff = String::from_utf8_lossy(&output.stdout);
                let (additions, deletions) = self.count_changes(&diff);
                
                DiffResult {
                    file_path,
                    success: true,
                    has_changes: !diff.trim().is_empty(),
                    additions,
                    deletions,
                    diff_output: diff.to_string(),
                    error: None,
                }
            }
            Err(e) => DiffResult {
                file_path,
                success: false,
                has_changes: false,
                additions: 0,
                deletions: 0,
                diff_output: String::new(),
                error: Some(e.to_string()),
            },
        }
    }
    
    /// 统计变更行数
    fn count_changes(&self, diff: &str) -> (usize, usize) {
        let mut additions = 0;
        let mut deletions = 0;
        
        for line in diff.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                additions += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                deletions += 1;
            }
        }
        
        (additions, deletions)
    }
    
    /// 生成统计摘要
    pub fn stat(&self) -> String {
        let result = Command::new("git")
            .args(["diff", "--stat"])
            .current_dir(self.repo_root.as_deref().unwrap_or(Path::new(".")))
            .output();
        
        match result {
            Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
            Err(_) => String::new(),
        }
    }
}

impl Default for GitDiffEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_count_changes() {
        let engine = GitDiffEngine::new();
        let diff = r#"+line 1
-line 2
+line 3
 line 4
-line 5
"#;
        let (additions, deletions) = engine.count_changes(diff);
        assert_eq!(additions, 2);
        assert_eq!(deletions, 2);
    }
}
