# Anvil 改进清单

## 已完成的改进

### ✅ 1. 自动后台执行判断 (2026-05-15)
**实现文件：**
- `crates/runtime/src/background_judge.rs` - 自动判断引擎
- 集成到 `crates/runtime/src/bash.rs`

**功能特性：**
1. **静态规则匹配：**
   - 必定后台：`--watch`, `--serve`, `npm run dev`, `cargo watch`, `vite` 等
   - 必定前台：`ls`, `cat`, `git status`, `vim` 等
   - 支持正则表达式匹配

2. **历史学习系统：**
   - 记录命令执行时间到 `~/.anvil/execution_history.json`
   - 根据历史平均时间判断（阈值：30秒）
   - 最少 3 次执行后开始学习

3. **配置项：**
   - `slow_threshold_ms`: 慢任务阈值（默认 30000ms）
   - `enable_learning`: 是否启用历史学习
   - `min_samples_for_learning`: 最小样本数

**测试验证：**
```bash
cd /Users/limiancai/anvil-main/rust
cargo test -p runtime background_judge  # 6 tests passed
cargo build --release
cargo install --path crates/anvil-cli
```

---

## 待改进项

### 2. 非交互模式输出问题
**现象：** 在后台运行 `anvil "prompt"` 时，进程运行但无输出
**改进方向：**
- 添加 `--no-tty` 模式，禁用终端特性
- 确保输出 flush 到 stdout/stderr
- 添加日志文件输出选项

### 3. 任务文件处理
**现象：** anvil 没有 `-p` 参数读取任务文件
**改进方向：**
- 添加 `--task-file` 或 `-f` 参数
- 支持从文件读取详细任务描述

---

## 验证命令

```bash
anvil --version
anvil status
```
