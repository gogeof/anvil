# Anvil Phase 1-3 剩余任务

## 项目路径
/Users/limiancai/anvil-main/rust

## 任务概述
完成 anvil 优化计划的剩余 4 个任务：

### 任务 1: 增强语法感知 (Phase 1)
**目标:** 支持 5+ 语言的语法高亮和代码结构识别

**文件路径:** `crates/runtime/src/syntax_highlight.rs`

**要求:**
1. 使用 `syntect` crate 实现 syntax highlighting
2. 支持 Rust, Python, JavaScript, Go, Java 至少 5 种语言
3. 提供函数:
   - `highlight_code(code: &str, language: &str) -> String` - 返回带 ANSI 颜色码的代码
   - `detect_language(file_path: &str) -> Option<String>` - 根据文件扩展名检测语言
   - `get_code_structure(code: &str, language: &str) -> Vec<CodeBlock>` - 返回代码结构（函数、类、变量定义）
4. 添加到 `lib.rs` 的 `pub mod`

### 任务 2: 增强模糊匹配 (Phase 2)
**目标:** 提升 edit_file 一次成功率到 > 90%

**文件路径:** `crates/runtime/src/file_ops.rs`

**当前状态:** 已有 7 种模糊匹配策略

**要求:**
1. 添加基于 Levenshtein 距离的匹配
2. 添加 AST-aware diff（使用 tree-sitter 或简化版本）
3. 改进匹配算法的优先级排序
4. 添加置信度评分机制

### 任务 3: PTY 支持 (Phase 2)
**目标:** 支持交互式命令（vim, htop 等）

**文件路径:** `crates/runtime/src/bash.rs` 或新文件 `crates/runtime/src/pty.rs`

**要求:**
1. 使用 `portable-pty` crate 创建 PTY
2. 实现 `PtySession` 结构体:
   - `spawn(command: &str) -> Result<PtySession>`
   - `read(&mut self) -> Result<String>`
   - `write(&mut self, input: &str) -> Result<()>`
   - `resize(&mut self, rows: u16, cols: u16) -> Result<()>`
3. 集成到现有的 bash 执行流程

### 任务 4: 性能优化 (Phase 3)
**目标:** 首字延迟 < 1s

**要求:**
1. 添加延迟监控埋点
2. 优化启动路径
3. 实现延迟统计报告功能

## 编码规范
- 遵循 Rust 最佳实践
- 保持与现有代码风格一致
- 所有新模块需要添加测试
- 使用 `cargo fmt` 和 `cargo clippy` 检查代码

## 验证命令
```bash
cd /Users/limiancai/anvil-main/rust
cargo test -p runtime
cargo build --release
cargo install --path crates/anvil-cli
```
