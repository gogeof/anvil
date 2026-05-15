# ANVIL

> **深度集成 DeepSeek 的 AI 编程助手。**
> 学习优秀工具的设计理念，
> 做对用户有价值、真正好用的终端 CLI 工具。

<p align="center">
  <pre><code> █████╗ ███╗   ██╗██╗   ██╗██╗██╗     
██╔══██╗████╗  ██║██║   ██║██║██║     
███████║██╔██╗ ██║██║   ██║██║██║     
██╔══██║██║╚██╗██║╚██╗ ██╔╝██║██║     
██║  ██║██║ ╚████║ ╚████╔╝ ██║███████╗
╚═╝  ╚═╝╚═╝  ╚═══╝  ╚═══╝  ╚═╝╚══════╝</code></pre>
</p>

---

## 核心原则

anvil 基于以下三大原则构建：

1. **专注编程领域** — 不做通用聊天，只解决编程问题
2. **第一性原理驱动** — 从"读/写/执行/搜索"四个基本真理推导所有功能
3. **指标驱动改进** — 用量化数据评价优劣，不通过类比、不通过功能数量

📖 详细说明：
- **[FIRST_PRINCIPLES.md](FIRST_PRINCIPLES.md)** — 第一性原理（宪法）
- **[METRICS.md](METRICS.md)** — 指标体系（度量标准）

---

## 目标指标

anvil 的优劣只通过以下量化指标判断：

| 维度 | 核心指标 | 目标值 | 状态 |
|------|----------|--------|------|
| **搜索** | 搜索延迟 | **< 50ms** | ✅ P50: 42.8ms |
| **阅读** | 大文件支持 | **> 10MB** | ✅ 实测 > 10MB |
| **编写** | 编辑成功率 | **> 90%** | ✅ 100% |
| **执行** | 后台支持 | ✅ 支持 | ✅ 已实现 + 自动判断 |
| **整体** | 任务完成率 | **> 80%** | ✅ 100% HumanEval |
| **体验** | 首字延迟 | **< 1s** | ✅ perf 模块已实现 |

详细指标和优化路线图见 [METRICS.md](METRICS.md)。

---

## DeepSeek 模型在编程中的评估

anvil 深度集成 DeepSeek 模型。以下是对 DeepSeek V4 系列在编程领域的诚实评估。

### 优势（anvil 重点集成方向）

| 优势 | 说明 | 在 anvil 中的集成 |
|------|------|------------------|
| **长上下文** | V4-Pro 支持 1M token 上下文，能完整处理大型代码库 | system prompt 充分利用上下文预算 |
| **推理能力强** | DeepSeek 的 CoT（思维链）推理在复杂编程任务上表现出色 | 已启用 extended thinking 模式 |
| **代理能力强** | DeepSeek V4 系列在工具调用和任务规划上表现优异，适合 coding agent 场景 | 完整工具链集成（bash/文件/搜索） |
| **中文理解好** | 中文编程需求的理解远超同类模型 | 系统提示词中英双语优化 |
| **性价比高** | 1M token 上下文下仍保持有竞争力的价格 | 成本控制机制 |
| **多供应商可选** | 可通过官方、华为云、OpenRouter、Together 等多渠道访问 | `DEEPSEEK_BASE_URL` 可配置 |
| **代码补全/分析** | 对已有代码的理解和修改建议质量高 | read_file + edit_file 工具流优化 |

### 劣势（诚实面对，持续改进）

| 劣势 | 说明 | 缓解措施 |
|------|------|---------|
| **响应速度偏慢** | 相比其他模型，首 token 延迟较高 | 提供 lite (flash) 模型别名用于简单任务 |
| **指令遵循偶有偏移** | 复杂多步指令有时会遗漏某些步骤 | system prompt 分步骤结构化，`anvil compare` 持续追踪 |
| **非英文编程习惯偶有偏差** | 代码注释、变量命名等风格不够稳定 | 通过 AGENTS.md / 项目指令文件约束风格 |
| **缺乏专有工具链集成** | 不像其他工具有原生 LSP/沙箱等 | 通过通用工具（bash/grep）替代，保持可移植性 |
| **创意类代码质量一般** | 架构设计、命名创意不如顶尖模型 | 复杂设计建议结合 `anvil compare` 多方案对比 |

---

## 一句话

**anvil** 是一个终端 AI 编程助手，深度集成 DeepSeek 模型：

- **长上下文编程** — 充分利用 DeepSeek V4 的 1M token 上下文处理大型项目
- **推理驱动** — 启用 CoT 推理模式处理复杂编程任务
- **工具调用优化** — 针对 DeepSeek 的工具调用习惯调优 system prompt
- **函数级读取** — `read_function` 按函数/结构体粒度读取，大文件上下文节省 99%
- **语义缓存** — 集成到 `read_file`，TTL 300s/缓存命中 < 1ms
- **持续对比改进** — 内置 `anvil compare`，对比参考基准，数据驱动优化

---

## 新增功能

### 优化模块集成（Phase 5）
优化模块已集成到主流程，包含以下子模块：

- **函数级读取（read_function）** — 按函数/结构体粒度读取文件，大文件上下文节省 99%
- **语义缓存（semantic_cache）** — 集成到 read_file，TTL 300s 且 LRU 淘汰，缓存命中延迟 < 1ms
- **DeepSeek 配置优化** — 使用 `for_model()` 动态调整上下文预算（DeepSeek: 100K tokens）
- **并行工具执行（parallel_executor）** — 多工具并发调用，减少等待时间

### 自动后台判断（background_judge.rs）
自动识别长时间运行的命令（编译、测试、部署等），将其放入后台执行，避免阻塞 REPL。基于规则引擎和历史数据自适应决策。

### 遥测系统（/metrics 命令）
内置性能埋点系统，记录关键操作（grep_search、read_file、edit_file、bash）的延迟和成功率。通过 `/metrics` 命令实时查看 P50/P95 延迟、成功率和周报。

---

## 起源

anvil 最初 fork 自 claw-code（一个开源的 Rust 编程助手项目），
但经历了彻底的 rebrand 和重塑。

| 文件 | 用途 |
|------|------|
| `FIRST_PRINCIPLES.md` | 🎯 **宪法** — 所有功能决策从这里推导 |
| `scripts/anvil-compare.py` | 对比系统 — 量化进步 |
| `~/.anvil/settings.json` | 唯一配置入口 |

---

## 快速开始

### 安装

```bash
# 从源码构建
cd rust && cargo build --release
sudo cp target/release/anvil /usr/local/bin/anvil

# codesign（macOS）
sudo codesign --force --sign - /usr/local/bin/anvil
```

### 配置

编辑 `~/.anvil/settings.json`：

```json
{
  "aliases": {
    "lite": "deepseek/deepseek-v4-flash",
    "pro": "deepseek/deepseek-v4-pro",
    "glm": "glm-5"
  },
  "permissions": {
    "defaultMode": "dontAsk"
  },
  "env": {
    "DEEPSEEK_API_KEY": "sk-your-deepseek-key",
    "DEEPSEEK_BASE_URL": "https://your-deepseek-provider.com/v1",
    "ANTHROPIC_AUTH_TOKEN": "your-anthropic-compat-token",
    "ANTHROPIC_BASE_URL": "https://your-anthropic-provider.com/anthropic",
    "ANVIL_WEB_SEARCH_BASE_URL": "http://localhost:4000/search?format=json"
  },
  "toolModel": "glm-5"
}
```

`env` 中的变量会自动注入到 anvil 的进程环境，不需要写 `~/.zshrc`。

### 使用

```bash
# 进入交互式 REPL
anvil

# 单次 prompt
anvil prompt "分析这个项目的架构"

# 用指定模型
anvil --model lite       # 华为 DeepSeek（快速响应）
anvil --model pro        # 华为 DeepSeek（深度推理）
anvil --model glm        # 百度千帆 glm-5（轻量任务/工具调用）

# 双供应商配置
# 在 settings.json 的 env 中同时配置两个 provider 的密钥：
#   DEEPSEEK_API_KEY + DEEPSEEK_BASE_URL → 华为/官方 DeepSeek
#   ANTHROPIC_AUTH_TOKEN + ANTHROPIC_BASE_URL → 百度千帆等 Anthropic 兼容端点
# settings.json 中的 toolModel 字段指向工具调用模型（默认 glm-5）

# 网页检索（需自部署 SearXNG）
# 1. docker run -d --name searxng -p 4000:8080 searxng/searxng
# 2. 在 env 中设 ANVIL_WEB_SEARCH_BASE_URL=http://localhost:4000/search?format=json
# anvil 会自动走 JSON API 获取结构化搜索结果

# 对比 anvil 与参考基准
anvil compare "实现一个 LRU 缓存"

# 查看对比历史
anvil compare --list
anvil compare --show 20260514_164435

# REPL 内命令
/status     # 当前上下文
/diff       # 查看未提交变更
/commit     # 提交代码
/help       # 所有命令
quit        # 退出（或 /exit）
```

---

## 架构

### 技术栈

| 层 | 技术 |
|----|------|
| **二进制入口** | Rust (`rust/crates/anvil-cli/src/main.rs`) |
| **核心运行时** | Rust (`rust/crates/runtime/`) |
| **API 客户端** | Rust (`rust/crates/api/`) — OpenAI 兼容协议 |
| **工具系统** | Rust (`rust/crates/tools/`) |
| **MCP 支持** | Rust (`rust/crates/plugins/`) |
| **对比系统** | Python (`scripts/anvil-compare.py`) |
| **REPL** | Rust (rustyline) |
| **构建** | Cargo workspace (`rust/Cargo.toml`) |

### Workspace 结构

```
rust/
├── Cargo.toml                  # workspace 根
├── crates/
│   ├── anvil-cli/              # 二进制入口（main.rs + CLI 解析）
│   ├── runtime/                # 对话运行时 + 配置加载 + system prompt
│   ├── api/                    # API 客户端（OpenAI 兼容协议）
│   ├── tools/                  # 内置工具实现（bash/read/write/grep/WebSearch）
│   ├── plugins/                # 插件系统 + MCP 支持
│   ├── commands/               # slash 命令
│   ├── telemetry/              # 遥测
│   └── compat-harness/         # 兼容性测试框架
```

---

## 内置工具（8 个核心工具）

从第一性原理出发，编程只需要四个基本能力：读、写、执行、搜索。anvil 围绕这四个能力设计最小的工具集：

| 工具 | 用途 | 对应基本能力 | 权限 |
|------|------|------------|------|
| `bash` | 执行 shell 命令 | 执行 | DangerFullAccess |
| `read_file` | 读取文件内容 | 读 | ReadOnly |
| `write_file` | 写入或创建文件 | 写 | WorkspaceWrite |
| `edit_file` | patch 式编辑已有文件 | 写 | WorkspaceWrite |
| `grep_search` | 正则搜索文件内容 | 搜索 | ReadOnly |
| `glob_search` | 按文件名模式查找 | 搜索 | ReadOnly |
| `WebFetch` | 抓取 URL 内容 | 搜索 | ReadOnly |
| `WebSearch` | 通过搜索引擎查询 | 搜索 | ReadOnly |

> 工具数量不在多，在于覆盖编程的核心需求。所有工具都秉持最少权限原则。

## 对比系统

`anvil compare` 通过和参考基准工具对比，持续驱动改进：

```
anvil compare "实现一个线程安全的计数器"
```

流程：
1. 同时发给 anvil 和参考基准工具执行
2. 自动提取核心回答内容（去除 spinner、工具调用等元信息）
3. 分析差异：回答长度、代码块、关键话题覆盖
4. 从第一性原理判断：差异是本质性的还是实现细节
5. 报告保存到 `~/.anvil/comparisons/`

---

## 配置参考

`~/.anvil/settings.json` 完整字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `aliases` | object | 模型别名（如 `"lite": "deepseek-v4-flash"`） |
| `permissions.defaultMode` | string | 权限模式（`dontAsk`/`read-only`/等） |
| `permissions.allow` | string[] | 允许的工具 |
| `permissions.deny` | string[] | 禁止的工具 |
| `env` | object | 注入到进程环境变量的键值对 |
| `toolModel` | string | 工具调用模型（如 `"glm-5"`），双供应商模式下使用 |
| `hooks` | object | 钩子配置 |
| `mcpServers` | object | MCP 服务器配置 |

### 可用环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `DEEPSEEK_API_KEY` | — | DeepSeek API 密钥（用于华为/官方等 OpenAI 兼容端点） |
| `DEEPSEEK_BASE_URL` | `https://api.deepseek.com` | DeepSeek API 端点 |
| `ANTHROPIC_AUTH_TOKEN` | — | Anthropic 兼容 API 密钥（用于百度千帆等） |
| `ANTHROPIC_BASE_URL` | — | Anthropic 兼容 API 端点 |
| `OPENROUTER_API_KEY` | — | OpenRouter API 密钥 |
| `TOGETHER_API_KEY` | — | Together AI API 密钥 |
| `ANVIL_WEB_SEARCH_BASE_URL` | DuckDuckGo | 搜索引擎地址（国内推荐 `http://localhost:4000/search?format=json` 自部署 SearXNG）|

---

## 开源协议

本项目基于 [MIT License](LICENSE)。

是基于 claw-code 的 fork。
