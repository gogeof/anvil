# Changelog

> 本文件记录 anvil 的版本特性和变更历史。
> 每次新增特性时在此文件中追加记录。

---

## 0.1.0 — 2026-05-24

### 新增特性

#### 工作区回滚 (Workspace Rollback)
- **文件:** `rust/crates/runtime/src/snapshot.rs`
- **核心能力:**
  - `create_snapshot()` — 使用 `git add -A` + `git stash create` 捕获完整工作区状态，不修改工作树
  - `list_snapshots()` — 列出 `~/.anvil/snapshots/` 中所有历史快照
  - `restore_snapshot()` — 使用 `git checkout <hash> -- .` 强制恢复到快照状态，无冲突风险
- **设计原则:** 不污染用户 git 历史（不操作 stash 栈），快照元数据以 JSON 格式独立存储

#### 会话上下文预算管理 (Context Budget)
- **文件:** `rust/crates/runtime/src/context_budget.rs`
- **核心能力:**
  - `estimate_context_size()` — 基于 `estimate_session_tokens()` 估算当前会话 token 用量
  - `should_compact()` — 当前使用率超过阈值（默认 80%）时触发告警
  - `get_compaction_strategy()` — 三级策略：`None` / `SummaryOldest` / `Rewind`
  - `format_budget_summary()` — 格式化输出如 `450K/1M (45%)`
- **默认配置:** max_tokens=1,000,000（DeepSeek V4 上下文窗口），warning_threshold=0.8

#### context_budget 集成到运行时
- **变更文件:** `rust/crates/runtime/src/conversation.rs`
- **内容:**
  - `AutoCompactionEvent` 新增 `budget_summary: Option<String>` 字段
  - `maybe_auto_compact()` 在触发压缩时同步估算并输出上下文预算信息

---

## 版本规范

- 版本号格式: `MAJOR.MINOR.PATCH`
- 新增特性 → MINOR 递增
- Bug 修复 → PATCH 递增
- 破坏性变更 → MAJOR 递增
