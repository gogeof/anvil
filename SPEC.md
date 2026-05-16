# ANVIL SPEC — 正式行为规范

> **版本 1.0 — 2026-05-16**
>
> 本文档使用 RFC 2119 关键词（MUST / MUST NOT / SHOULD / SHOULD NOT / MAY）
> 定义 Anvil 的正式行为规范。所有实现和测试必须以此为依据。
>
> 本文档与 [FIRST_PRINCIPLES.md](FIRST_PRINCIPLES.md)（宪法）、
> [METRICS.md](METRICS.md)（度量标准）共同构成 Anvil 的三层规范体系。

---

## 目录

1. [通用规范](#1-通用规范)
2. [核心操作规范](#2-核心操作规范)
3. [工具调用规范](#3-工具调用规范)
4. [权限与安全规范](#4-权限与安全规范)
5. [错误处理规范](#5-错误处理规范)
6. [性能指标规范](#6-性能指标规范)
7. [可验证性要求](#7-可验证性要求)

---

## 1. 通用规范

### 1.1 命名与标识

- **1.1.1** Anvil MUST 在自我介绍时使用名称 "Anvil"。
- **1.1.2** Anvil MUST NOT 声称自己是其他 AI 助手（如 Claude、ChatGPT）。
- **1.1.3** Anvil SHOULD 在回答中清晰区分"我（Anvil）"和"用户"。

### 1.2 第一性原理约束

- **1.2.1** Anvil MUST 只提供编程相关的能力。非编程任务的拒绝率 SHOULD > 80%。
- **1.2.2** Anvil 的每个工具 MUST 对应"读/写/执行/搜索"四个基本能力之一。
- **1.2.3** Anvil SHOULD NOT 引入不直接对应四个基本能力的子系统（如 LSP 守护进程、
  向量数据库、Docker 沙箱），除非有数据证明其必要性。

### 1.3 度量驱动

- **1.3.1** 所有功能优化 MUST 定义量化指标、测量基线、设定目标值。
- **1.3.2** Anvil MUST 在关键路径上埋点记录延迟和成功率（见第 6 节）。
- **1.3.3** 度量数据 MUST 存储在 `~/.anvil/metrics/` 目录。

---

## 2. 核心操作规范

### 2.1 读取（Read）

#### 2.1.1 `read_file`

- **2.1.1-1** 输入 MUST 包含 `path`（字符串）参数。
- **2.1.1-2** 输入 MAY 包含 `offset`（整数，>=0）和 `limit`（整数，>=1）参数。
- **2.1.1-3** 返回 MUST 包含 `content`、`startLine`、`numLines` 字段。
- **2.1.1-4** 当文件不存在时 MUST 返回错误，而非返回空内容。
- **2.1.1-5** 当 `offset` 超过文件行数时 MUST 返回空内容（startLine = offset + 1）。
- **2.1.1-6** 支持的单个文件大小 SHOULD >= 10MB。
- **2.1.1-7** 语义缓存 MUST 实现 LRU 淘汰和 TTL（默认 300 秒），缓存命中延迟 MUST < 1ms。
- **2.1.1-8** MUST NOT 修改被读取的文件。

#### 2.1.2 `read_function`

- **2.1.2-1** 输入 MUST 包含 `path` 和 `function_name` 参数。
- **2.1.2-2** 返回 MUST 只包含指定函数/结构体的代码，而非整个文件。
- **2.1.2-3** 大文件（>1000 行）的场景下 SHOULD 节省 >95% 的上下文。

### 2.2 写入（Write）

#### 2.2.1 `write_file`

- **2.2.1-1** 输入 MUST 包含 `path` 和 `content` 参数。
- **2.2.1-2** 当路径指向新文件时 MUST 创建文件及其父目录。
- **2.2.1-3** 当路径指向已有文件时 MUST 覆盖文件内容。
- **2.2.1-4** 返回 MUST 包含 `type` 字段（值为 `"create"` 或 `"update"`）。
- **2.2.1-5** 当为更新操作时，返回 SHOULD 包含 `originalFile` 原内容快照。
- **2.2.1-6** MUST NOT 写 `~` 开头的路径（除非在 WorkspaceWrite 模式下路径被规范化）。

#### 2.2.2 `edit_file`

- **2.2.2-1** 输入 MUST 包含 `path`、`old_string`、`new_string` 参数。
- **2.2.2-2** 输入 MAY 包含 `replace_all`（布尔值）参数。
- **2.2.2-3** `old_string` MUST 在文件中精确匹配至少一次，否则返回错误。
- **2.2.2-4** `old_string` 与 `new_string` MUST 不相同，否则返回错误。
- **2.2.2-5** `replace_all = false`（默认）时 MUST 只替换第一个匹配。
- **2.2.2-6** `replace_all = true` 时 MUST 替换所有匹配。
- **2.2.2-7** 返回 MUST 包含 `replaceAll` 和 `type` 字段。
- **2.2.2-8** edit_file 的一次成功率 SHOULD > 90%（目标：> 95%）。
- **2.2.2-9** MUST 支持模糊匹配（空格、缩进差异），模糊匹配率 SHOULD > 50%。

### 2.3 执行（Execute）

#### 2.3.1 `bash`

- **2.3.1-1** 输入 MUST 包含 `command`（字符串）参数。
- **2.3.1-2** 输入 MAY 包含 `timeout`（整数，>=1，单位为秒）、`description`（字符串）、
  `run_in_background`（布尔值）参数。
- **2.3.1-3** 返回 MUST 包含 `stdout`、`stderr`、`interrupted`、`returnCodeInterpretation` 字段。
- **2.3.1-4** 当命令超时时 MUST 返回 `interrupted = true` 并在 stderr 中说明超时原因。
- **2.3.1-5** 后台执行（`run_in_background = true`）时 MUST 立即返回，不等待命令完成。
- **2.3.1-6** 后台命令的返回 MUST 包含 `backgroundTaskId` 字段。
- **2.3.1-7** 自动后台判断（background_judge）MUST 基于规则引擎和历史数据，
  自动将长时间运行的命令放入后台。
- **2.3.1-8** bash 工具 MUST 在命令执行前进行分支预检（branch preflight）：
  - 当检测到分支落后于 `main` 时，MUST 返回 `preflight_blocked:branch_divergence`
    并附带结构化的事件数据。
- **2.3.1-9** 分类为只读的命令（cat/head/tail/grep/ls 等）SHOULD 在
  `WorkspaceWrite` 权限级别允许执行，无需提升到 `DangerFullAccess`。

#### 2.3.2 `PowerShell`

- **2.3.2-1** 输入 MUST 包含 `command` 参数。
- **2.3.2-2** MUST 检测并优先使用 `pwsh`，回退到 `powershell`。
- **2.3.2-3** 当两者均不可用时 MUST 返回"PowerShell executable not found"错误。
- **2.3.2-4** 支持与 bash 相同的权限分类逻辑（只读命令降低权限要求）。

#### 2.3.3 `REPL`

- **2.3.3-1** 输入 MUST 包含 `code` 和 `language` 参数。
- **2.3.3-2** 支持的语言 MUST 包含 `python`、`javascript`/`node`、`bash`/`sh`。
- **2.3.3-3** 输入 MAY 包含 `timeout_ms` 参数。
- **2.3.3-4** 当代码为空时 MUST 返回错误。
- **2.3.3-5** 当语言不支持时 MUST 返回错误。

### 2.4 搜索（Search）

#### 2.4.1 `grep_search`

- **2.4.1-1** 输入 MUST 包含 `pattern`（字符串，正则表达式）参数。
- **2.4.1-2** 输入 MAY 包含 `path`、`glob`、`output_mode`、`-n`、`-i`、`-A`、`-B`、`-C`、
  `context`、`head_limit`、`offset`、`multiline` 参数。
- **2.4.1-3** 当 `pattern` 是无效正则表达式时 MUST 返回错误。
- **2.4.1-4** 搜索延迟 MUST < 50ms（P50）。
- **2.4.1-5** 搜索准确率（返回结果中包含期望内容的比例）SHOULD > 95%。

#### 2.4.2 `glob_search`

- **2.4.2-1** 输入 MUST 包含 `pattern`（glob 模式字符串）参数。
- **2.4.2-2** 输入 MAY 包含 `path`（搜索根目录）参数。
- **2.4.2-3** 当 `pattern` 是无效 glob 时 MUST 返回错误。
- **2.4.2-4** 返回 MUST 包含 `filenames` 和 `numFiles` 字段。

#### 2.4.3 `WebSearch`

- **2.4.3-1** 输入 MUST 包含 `query`（字符串，minLength: 2）参数。
- **2.4.3-2** 输入 MAY 包含 `allowed_domains`、`blocked_domains`（字符串数组）、
  `max_results`（整数，1-20，默认 8）参数。
- **2.4.3-3** 返回 MUST 包含 `query`、`results`（搜索结果数组）、`durationSeconds` 字段。
- **2.4.3-4** MUST 支持通过 `ANVIL_WEB_SEARCH_BASE_URL` 环境变量配置搜索引擎端点。
- **2.4.3-5** 当配置了 JSON API 端点（URL 包含 `format=json`）时，MUST 直接解析 JSON 响应。
- **2.4.3-6** 未配置搜索引擎时，默认回退到 DuckDuckGo HTML 搜索。

#### 2.4.4 `WebFetch`

- **2.4.4-1** 输入 MUST 包含 `url`（URI 格式字符串）和 `prompt`（字符串）参数。
- **2.4.4-2** 当 `url` 不是有效 URI 时 MUST 返回错误。
- **2.4.4-3** HTTP 非本地 URL（非 localhost/127.0.0.1/::1）MUST 自动升级到 HTTPS。
- **2.4.4-4** 返回 MUST 包含 `bytes`、`code`、`codeText`、`result`、`url` 字段。
- **2.4.4-5** 返回的内容 MUST 根据 `prompt` 进行摘要（如标题提取、内容总结）。

---

## 3. 工具调用规范

### 3.1 工具注册

- **3.1.1** 每个工具 MUST 在 `ToolSpec` 中定义 `name`、`description`、`input_schema`、
  `required_permission`。
- **3.1.2** 工具名称 MUST 唯一，插件工具和运行时工具 MUST NOT 与内建工具名称冲突。
- **3.1.3** 工具输入 schema MUST 使用 JSON Schema 格式。

### 3.2 权限层级

- **3.2.1** Anvil 定义四种权限模式，按权限递增：`ReadOnly` < `WorkspaceWrite` <
  `DangerFullAccess` < `Allow`。
- **3.2.2** 每个工具 MUST 声明其 `required_permission`：
  - ReadOnly：`read_file`、`read_function`、`grep_search`、`glob_search`、`WebFetch`、`WebSearch`
  - WorkspaceWrite：`write_file`、`edit_file`、`TodoWrite`、`Config`
  - DangerFullAccess：`bash`、`Agent`、`TaskCreate`、`WorkerCreate` 等
- **3.2.3** `PermissionEnforcer` MUST 在执行每个工具前检查权限。
- **3.2.4** 权限不足时 MUST 返回明确的 `Denied` 结果，包含工具名、当前模式、所需模式及原因。
- **3.2.5** bash 命令的权限 MUST 动态分类：
  - 只读命令（如 cat/grep/ls）且路径在工作区内 → 仅需 `WorkspaceWrite`
  - 其他命令 → 需要 `DangerFullAccess`

### 3.3 工具执行流

- **3.3.1** 工具执行流 MUST 为：权限检查 → 输入解析 → 执行 → 返回结构化结果。
- **3.3.2** 工具执行 SHOULD 通过遥测系统记录延迟和成功率。
- **3.3.3** 工具返回数据 MUST 是 JSON 格式（`serde_json::Value`）。
- **3.3.4** 工具执行 MUST 是同步的（后台操作除外）。

### 3.4 工具发现

- **3.4.1** `ToolSearch` 工具 MUST 支持按关键词搜索可用工具。
- **3.4.2** `ToolSearch` SHOULD 支持 `select:ToolName1,ToolName2` 精确选择语法。
- **3.4.3** 内建核心工具（bash/read_file/write_file/edit_file/grep_search/glob_search）
  MUST 始终在 API 定义中可见；其他工具 MAY 通过 `ToolSearch` 按需发现。

### 3.5 供应商中性

- **3.5.1** Anvil MUST 通过 OpenAI 兼容协议与模型供应商通信。
- **3.5.2** API key、端点 URL、模型名 MUST 通过 `~/.anvil/settings.json` 配置。
- **3.5.3** 模型别名（如 lite/pro/smart）MUST 可配置，不硬编码特定模型名。
- **3.5.4** 发生模型故障时 SHOULD 支持供应商回退链（provider fallback chain）。

---

## 4. 权限与安全规范

### 4.1 权限模式

- **4.1.1** `ReadOnly` 模式 MUST 禁止所有写操作（write_file/edit_file 等）和破坏性 bash 命令。
- **4.1.2** `WorkspaceWrite` 模式 MUST 只允许在工作区目录内的写操作。
- **4.1.3** `DangerFullAccess` 模式 MUST 允许所有操作，包括工作区外的写操作。
- **4.1.4** `Allow` 模式 MUST 允许所有操作且不进行安全检查。
- **4.1.5** 默认模式从 `~/.anvil/settings.json` 的 `permissions.defaultMode` 读取。

### 4.2 工作区边界

- **4.2.1** `WorkspaceWrite` 模式下，文件写入 MUST 限制在当前工作目录内。
- **4.2.2** 工作区边界检查 MUST 通过路径前缀匹配实现（`is_within_workspace`）。
- **4.2.3** 相对路径 MUST 先规范化为绝对路径再进行边界检查。
- **4.2.4** `../` 路径遍历 MUST 被检测并拒绝（当导致路径超出工作区边界时）。

### 4.3 bash 安全

- **4.3.1** `ReadOnly` 模式下，bash 命令 MUST 被限制在只读命令白名单中。
- **4.3.2** 只读命令 MUST NOT 包含 `-i` 标志、`--in-place` 标志、`>` 或 `>>` 重定向。
- **4.3.3** 包含危险路径（指向工作区外的绝对路径、`../..` 遍历）的命令
  MUST 提升权限要求到 `DangerFullAccess`。
- **4.3.4** 后台进程（run_in_background）SHOULD 可管理（查看/停止/监控）。

### 4.4 沙箱

- **4.4.1** Anvil MAY 支持可选的沙箱隔离（当前默认关闭）。
- **4.4.2** 当沙箱启用时，SHOULD 支持命名空间隔离、网络隔离、文件系统限制。
- **4.4.3** 沙箱状态 MUST 在执行结果中通过 `sandboxStatus` 字段报告。

### 4.5 用户主权

- **4.5.1** 所有用户配置 MUST 存储在 `~/.anvil/` 目录（或 `ANVIL_CONFIG_HOME` 指定目录）。
- **4.5.2** 配置 MUST 是 JSON 格式，人类可读可写。
- **4.5.3** API 密钥 MUST 通过 `env` 字段在 settings.json 中配置，
  MUST NOT 从 shell profile（~/.zshrc 等）自动读取。

---

## 5. 错误处理规范

### 5.1 错误分类

- **5.1.1** 所有工具错误 MUST 分为以下类别之一：
  - **输入验证错误**：参数缺失、类型错误、值超出范围
  - **权限错误**：权限不足
  - **运行时错误**：文件不存在、命令失败、网络超时
  - **系统错误**：配置损坏、IO 错误、内部状态不一致
- **5.1.2** 错误信息 MUST 对人类可读且包含足够的上下文用于诊断。
- **5.1.3** 错误信息 MUST NOT 包含敏感信息（API key、令牌等）。

### 5.2 错误恢复

- **5.2.1** 工具执行失败后主动恢复率 SHOULD > 70%。
- **5.2.2** 可重试的错误（网络超时、供应商故障）SHOULD 支持自动重试。
- **5.2.3** 供应商故障 SHOULD 支持回退链（primary → fallback1 → fallback2）。
- **5.2.4** 幂等的操作（read_file/grep_search 等）SHOULD 在失败后安全重试。

### 5.3 超时处理

- **5.3.1** bash 命令超时后 MUST 优雅终止子进程（先 kill，再 wait）。
- **5.3.2** 超时命令 MUST 返回 `interrupted = true`，并在 stderr 中包含超时原因。
- **5.3.3** WebFetch/WebSearch 的 HTTP 请求 MUST 设置 20 秒超时。
- **5.3.4** Sleep 工具 MUST 有最大睡眠时长限制（300 秒）。

### 5.4 日志与追踪

- **5.4.1** 错误事件 SHOULD 通过 LaneEvent 系统记录结构化事件数据。
- **5.4.2** 遥测系统 MUST 记录每次工具操作的延迟、成功/失败状态、上下文信息。
- **5.4.3** 工具执行错误 SHOULD 包含 failureClass 分类（编译失败、测试失败、工具运行时错误等）。

### 5.5 Green Contract（绿契）

- **5.5.1** Green Contract 定义了代码质量等级：`TargetedTests` < `Package` < `Workspace` < `MergeReady`。
- **5.5.2** 每个操作 SHOULD 通过 Green Contract 评估当前代码状态是否满足要求。
- **5.5.3** 等级不足时 SHOULD 阻止后续操作并给出明确的阻碍原因。

---

## 6. 性能指标规范

### 6.1 通用性能指标

- **6.1.1** Anvil MUST 在关键工具操作上埋点记录 P50/P95/P99 延迟和成功率。
- **6.1.2** 度量数据 MUST 按日期存储为 JSONL 格式（`metrics-YYYY-MM-DD.jsonl`）。
- **6.1.3** 关键操作 MUST 包含 `grep_search`、`read_file`、`edit_file`、`bash`。
- **6.1.4** 度量指标 MUST 通过 `/metrics` 命令实时可查。

### 6.2 搜索性能

| 指标 | 目标值 | 测量方法 |
|------|--------|----------|
| 搜索延迟（P50） | MUST < 50ms | 埋点统计 P50 |
| 搜索延迟（P95） | SHOULD < 150ms | 埋点统计 P95 |
| 搜索准确率 | SHOULD > 95% | 测试集验证 |
| 搜索内存占用 | MUST < 100MB | `/usr/bin/time -v` |
| 大项目支持 | SHOULD > 500K LOC | 在大型仓库中测试 |

### 6.3 读取性能

| 指标 | 目标值 | 测量方法 |
|------|--------|----------|
| 读取延迟 | SHOULD < 30ms | 埋点统计 |
| 缓存命中延迟 | MUST < 1ms | 计时测量 |
| 大文件支持 | MUST > 10MB | 压力测试 |
| 函数级读取上下文节省 | SHOULD > 95% | 集成测试 |

### 6.4 写入性能

| 指标 | 目标值 | 测量方法 |
|------|--------|----------|
| 编辑一次成功率 | MUST > 90% | 测试集验证 |
| 模糊匹配率 | SHOULD > 50% | 测试集验证 |
| 并发冲突率 | SHOULD < 5% | 并发编辑测试 |

### 6.5 执行性能

| 指标 | 目标值 | 测量方法 |
|------|--------|----------|
| 命令成功率 | SHOULD > 85% | 回归测试 + 埋点 |
| 超时处理 | MUST 100% 优雅终止 | 压力测试 |
| 后台执行 | MUST 支持长时间命令不阻塞 REPL | 功能验证 |

### 6.6 整体性能

| 指标 | 目标值 | 测量方法 |
|------|--------|----------|
| 首字延迟（TTFB） | SHOULD < 1s | 埋点统计 |
| 任务完成率 | SHOULD > 80% | HumanEval + 真实场景测试 |
| 交互轮次 | SHOULD < 10 轮 | 埋点统计 |
| 错误恢复率 | SHOULD > 70% | 故障注入测试 |

---

## 7. 可验证性要求

### 7.1 测试规范

- **7.1.1** 每个核心工具 MUST 有对应的单元测试覆盖以下场景：
  - 正常输入 → 成功输出
  - 无效输入 → 适当错误
  - 边界条件（空文件、超大文件、并发操作）
- **7.1.2** 权限系统 MUST 有测试覆盖每种权限模式下每个工具的预期行为。
- **7.1.3** 测试 SHOULD 使用临时目录和隔离环境，不依赖外部状态。

### 7.2 基准测试

- **7.2.1** 基准测试套件 MUST 覆盖 search/write/execute/humaneval 四个维度。
- **7.2.2** 基准测试 MUST 自动运行并生成可比较的数值结果。
- **7.2.3** 基准测试结果 SHOULD 提供 P50/P95 延迟和成功率。
- **7.2.4** 基准测试数据集 MUST 版本化管理在 `tests/benchmarks/` 目录。

### 7.3 对比验证

- **7.3.1** `anvil compare` MUST 支持将 Anvil 与参考基准工具进行对比。
- **7.3.2** 对比 MUST 覆盖任务完成情况、完成时间、代码质量、交互轮次。
- **7.3.3** 对比结果 MUST 持久化到 `~/.anvil/comparisons/` 目录。
- **7.3.4** 对比 SHOULD 定期运行，追踪性能变化趋势。

### 7.4 指标验证

- **7.4.1** 所有埋点数据 MUST 可导出为可读格式。
- **7.4.2** 周报 MUST 包含每个关键操作的 P50/P95 延迟趋势和成功率变化。
- **7.4.3** 指标退化（如延迟增加 >20%）SHOULD 触发告警。

---

## 附录 A：RFC 2119 关键词解释

| 关键词 | 含义 |
|--------|------|
| **MUST** | 该要求是绝对的，实现必须满足 |
| **MUST NOT** | 该禁止是绝对的，实现绝对不能允许 |
| **SHOULD** | 在特定情况下可能存在合理理由忽略该要求 |
| **SHOULD NOT** | 在特定情况下可能存在合理理由允许该行为 |
| **MAY** | 该要求是可选的，实现可以自由选择 |

## 附录 B：MUST 要求清单

以下是所有 MUST 要求的汇总清单，用于合规性检查：

| # | 要求 | 分类 |
|---|------|------|
| 1.1.1 | 自我介绍时使用名称 "Anvil" | 通用 |
| 1.1.2 | 不得声称自己是其他 AI 助手 | 通用 |
| 1.2.1 | 只提供编程相关能力 | 通用 |
| 1.2.2 | 每个工具对应四个基本能力之一 | 通用 |
| 1.3.1 | 所有优化定义量化指标 | 通用 |
| 1.3.2 | 关键路径埋点 | 通用 |
| 1.3.3 | 度量数据存储在 ~/.anvil/metrics/ | 通用 |
| 2.1.1-1 | read_file 输入包含 path | 读取 |
| 2.1.1-4 | read_file 文件不存在返回错误 | 读取 |
| 2.1.1-5 | read_file offset 超范围返回空 | 读取 |
| 2.1.1-7 | 语义缓存 LRU + TTL | 读取 |
| 2.1.1-8 | 不修改被读取文件 | 读取 |
| 2.1.2-1 | read_function 包含 path 和 function_name | 读取 |
| 2.2.1-1 | write_file 包含 path 和 content | 写入 |
| 2.2.1-2 | 新文件创建父目录 | 写入 |
| 2.2.1-3 | 已有文件覆盖 | 写入 |
| 2.2.1-4 | 返回 type 字段 | 写入 |
| 2.2.1-6 | 不写 ~ 开头的路径 | 写入 |
| 2.2.2-1 | edit_file 包含 path/old_string/new_string | 写入 |
| 2.2.2-3 | old_string 至少匹配一次 | 写入 |
| 2.2.2-4 | old_string != new_string | 写入 |
| 2.2.2-5 | replace_all=false 替换第一个 | 写入 |
| 2.2.2-6 | replace_all=true 替换所有 | 写入 |
| 2.2.2-7 | 返回 replaceAll 和 type | 写入 |
| 2.3.1-1 | bash 包含 command | 执行 |
| 2.3.1-3 | bash 返回 stdout/stderr/interrupted/returnCodeInterpretation | 执行 |
| 2.3.1-4 | 超时返回 interrupted=true | 执行 |
| 2.3.1-5 | 后台执行立即返回 | 执行 |
| 2.3.1-6 | 后台命令返回 backgroundTaskId | 执行 |
| 2.3.1-7 | 自动后台判断 | 执行 |
| 2.3.2-1 | PowerShell 包含 command | 执行 |
| 2.3.2-2 | 优先使用 pwsh | 执行 |
| 2.3.2-3 | 不可用时返回错误 | 执行 |
| 2.3.3-1 | REPL 包含 code 和 language | 执行 |
| 2.3.3-2 | 支持 python/javascript/bash | 执行 |
| 2.3.3-4 | 空代码返回错误 | 执行 |
| 2.3.3-5 | 不支持的语言返回错误 | 执行 |
| 2.4.1-1 | grep_search 包含 pattern | 搜索 |
| 2.4.1-3 | 无效正则返回错误 | 搜索 |
| 2.4.1-4 | 搜索延迟 < 50ms (P50) | 搜索 |
| 2.4.2-1 | glob_search 包含 pattern | 搜索 |
| 2.4.2-3 | 无效 glob 返回错误 | 搜索 |
| 2.4.2-4 | 返回 filenames 和 numFiles | 搜索 |
| 2.4.3-1 | WebSearch 包含 query | 搜索 |
| 2.4.3-3 | WebSearch 返回 query/results/durationSeconds | 搜索 |
| 2.4.3-4 | WebSearch 支持 ANVIL_WEB_SEARCH_BASE_URL | 搜索 |
| 2.4.3-5 | 支持 JSON API 解析 | 搜索 |
| 2.4.4-1 | WebFetch 包含 url 和 prompt | 搜索 |
| 2.4.4-2 | 无效 url 返回错误 | 搜索 |
| 2.4.4-3 | 非本地 URL 升级到 HTTPS | 搜索 |
| 2.4.4-4 | 返回 bytes/code/codeText/result/url | 搜索 |
| 3.1.1 | 工具定义 name/description/input_schema/required_permission | 工具 |
| 3.1.2 | 工具名称唯一 | 工具 |
| 3.2.2 | 每个工具声明 required_permission | 工具 |
| 3.2.3 | PermissionEnforcer 执行前检查 | 工具 |
| 3.2.4 | 权限不足返回 Denied | 工具 |
| 3.3.1 | 工具执行流：权限→输入→执行→返回 | 工具 |
| 3.3.3 | 返回 JSON 格式 | 工具 |
| 3.4.1 | ToolSearch 支持关键词搜索 | 工具 |
| 3.4.3 | 核心工具始终可见 | 工具 |
| 3.5.1 | OpenAI 兼容协议 | 工具 |
| 3.5.2 | API key/端点/模型通过 settings.json 配置 | 工具 |
| 3.5.3 | 模型别名可配置 | 工具 |
| 4.1.1 | ReadOnly 禁止写操作 | 安全 |
| 4.1.2 | WorkspaceWrite 只允许工作区内写 | 安全 |
| 4.1.3 | DangerFullAccess 允许所有操作 | 安全 |
| 4.1.4 | Allow 允许所有操作 | 安全 |
| 4.2.1 | WorkspaceWrite 限制在工作目录 | 安全 |
| 4.2.2 | 路径前缀匹配 | 安全 |
| 4.2.3 | 相对路径规范化 | 安全 |
| 4.2.4 | ../ 遍历检测 | 安全 |
| 4.3.1 | ReadOnly bash 限制白名单 | 安全 |
| 4.3.2 | 只读命令排除 -i/--in-place/重定向 | 安全 |
| 4.3.3 | 危险路径提升权限 | 安全 |
| 4.5.1 | 配置存储在 ~/.anvil/ | 安全 |
| 4.5.2 | JSON 格式可读可写 | 安全 |
| 4.5.3 | API key 通过 settings.json 配置 | 安全 |
| 5.1.1 | 错误分类 | 错误处理 |
| 5.1.2 | 错误信息可读 | 错误处理 |
| 5.1.3 | 不包含敏感信息 | 错误处理 |
| 5.3.1 | 超时优雅终止 | 错误处理 |
| 5.3.2 | 超时返回 interrupted=true | 错误处理 |
| 5.3.3 | HTTP 请求 20 秒超时 | 错误处理 |
| 5.3.4 | Sleep 最大 300 秒 | 错误处理 |
| 5.4.2 | 遥测记录延迟/成功状态/上下文 | 错误处理 |
| 6.1.1 | 关键操作埋点 P50/P95/P99 | 性能 |
| 6.1.2 | 度量数据按日期存储 JSONL | 性能 |
| 6.1.3 | 关键操作：grep_search/read_file/edit_file/bash | 性能 |
| 6.1.4 | 度量指标通过 /metrics 可查 | 性能 |
| 7.1.1 | 核心工具有单元测试 | 可验证性 |
| 7.1.2 | 权限系统有测试 | 可验证性 |
| 7.2.1 | 基准测试覆盖四个维度 | 可验证性 |
| 7.2.2 | 基准测试自动运行 | 可验证性 |
| 7.3.1 | anvil compare 支持对比 | 可验证性 |
| 7.3.2 | 对比覆盖完成情况/时间/质量/轮次 | 可验证性 |
| 7.3.3 | 对比结果持久化 | 可验证性 |
| 7.4.1 | 埋点数据可导出 | 可验证性 |
| 7.4.2 | 周报包含 P50/P95 趋势 | 可验证性 |

---

> 本文档是 Anvil 项目的行为规范基准。
> 所有实现和测试必须与此规范保持一致。
> 规范如有更新，必须更新版本号和日期。
