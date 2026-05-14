# ANVIL

> **从第一性原理出发的 AI 编程助手。**
> 不绑定任何模型供应商，所有配置统一在 `~/.anvil/` 下，
> 通过 `anvil compare` 用数据驱动持续迭代。

<p align="center">
  <pre><code> █████╗ ███╗   ██╗██╗   ██╗██╗██╗     
██╔══██╗████╗  ██║██║   ██║██║██║     
███████║██╔██╗ ██║██║   ██║██║██║     
██╔══██║██║╚██╗██║╚██╗ ██╔╝██║██║     
██║  ██║██║ ╚████║ ╚████╔╝ ██║███████╗
╚═╝  ╚═╝╚═╝  ╚═══╝  ╚═══╝  ╚═╝╚══════╝</code></pre>
</p>

---

## 一句话

**anvil** 是一个终端 AI 编程助手，像 Claude Code 一样工作在命令行 REPL 中，
但从第一性原理重新设计：
- **模型自由** — 支持任何 OpenAI 兼容 API（DeepSeek、OpenRouter、Together 等）
- **配置统一** — 全部在 `~/.anvil/settings.json`，不依赖 shell profile
- **持续迭代** — 内置 `anvil compare`，和参考基准工具对比，量化进步
- **供应商中立** — 不锁定在 Anthropic/OpenAI 任何一家

---

## 起源

anvil 最初 fork 自 claw-code（Claude Code 的 Rust 开源实现），
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
    "lite": "deepseek-v4-flash",
    "pro": "deepseek-v4-pro"
  },
  "permissions": {
    "defaultMode": "dontAsk"
  },
  "env": {
    "DEEPSEEK_API_KEY": "sk-your-key",
    "DEEPSEEK_BASE_URL": "https://your-provider.com/v1",
    "ANVIL_WEB_SEARCH_BASE_URL": "https://cn.bing.com/search"
  }
}
```

`env` 中的变量会自动注入到 anvil 的进程环境，不需要写 `~/.zshrc`。

### 使用

```bash
# 进入交互式 REPL
anvil

# 单次 prompt
anvil prompt "写一个快速排序"

# 用指定模型
anvil --model lite
anvil --model pro

# 对比 anvil 与参考基准
anvil compare "实现一个 LRU 缓存"

# 查看对比历史
anvil compare --list
anvil compare --show 20260514_164435

# 查看配置
anvil config

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
| **二进制入口** | Rust (`rust/crates/rusty-claude-cli/src/main.rs`) |
| **核心运行时** | Rust (`rust/crates/runtime/`) |
| **API 客户端** | Rust (`rust/crates/api/`) — OpenAI 兼容协议 |
| **工具系统** | Rust (`rust/crates/tools/`) |
| **MCP 支持** | Rust (`rust/crates/plugins/`) |
| **对比系统** | Python (`scripts/anvil-compare.py`) |
| **REPL** | Rust (rustyline) |
| **构建** | Cargo workspace (`rust/Cargo.toml`) |

###  Workspace 结构

```
rust/
├── Cargo.toml                  # workspace 根
├── crates/
│   ├── rusty-claude-cli/       # 二进制入口（main.rs + CLI 解析）
│   ├── runtime/                # 对话运行时 + 配置加载 + system prompt
│   ├── api/                    # API 客户端（OpenAI 兼容协议）
│   ├── tools/                  # 内置工具实现（bash/read/write/grep/WebSearch）
│   ├── plugins/                # 插件系统 + MCP 支持
│   ├── commands/               # slash 命令
│   ├── telemetry/              # 遥测
│   └── compat-harness/         # 兼容性测试框架
```

### 数据流

```
用户输入
  ↓
REPL (rustyline)
  ↓
CLI 解析 → CliAction::Prompt / Repl / Compare / ...
  ↓
运行时 (ConversationRuntime)
  ├── System Prompt 构建器
  ├── API 客户端 → 供应商 API (OpenAI 兼容)
  ├── 工具调度器 → bash / 文件 / 搜索 / Web
  └── 配置加载器 → ~/.anvil/settings.json
```

---

## 第一性原理（摘要）

anvil 的每个设计都从四个不可再分的基本真理推导：

| 基本真理 | 推导出的实现 |
|----------|------------|
| 编程 = 读、写、执行、搜索 | `read_file`, `write_file`, `edit_file`, `bash`, `WebSearch` |
| AI 模型只是引擎 | OpenAI 兼容协议 + 可配置的 provider/endpoint |
| 配置属于用户 | `~/.anvil/settings.json`，不依赖 shell 或外部文件 |
| 进步需要度量 | `anvil compare` 内置对比框架 |

完整内容见 [`FIRST_PRINCIPLES.md`](./FIRST_PRINCIPLES.md)。

---

## 内置工具

| 工具 | 用途 | 权限 |
|------|------|------|
| `bash` | 执行 shell 命令 | DangerFullAccess |
| `read_file` | 读取文件 | ReadOnly |
| `write_file` | 写入文件 | WorkspaceWrite |
| `edit_file` | patch 式编辑 | WorkspaceWrite |
| `grep_search` | 正则搜索文件内容 | ReadOnly |
| `glob_search` | 按文件名模式查找 | ReadOnly |
| `WebFetch` | 抓取 URL 内容 | ReadOnly |
| `WebSearch` | 搜索引擎搜索 | ReadOnly |
| `TodoWrite` | 会话任务管理 | WorkspaceWrite |
| `Config` | 查看配置 | ReadOnly |

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
| `hooks` | object | 钩子配置 |
| `mcpServers` | object | MCP 服务器配置 |

### 可用环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `DEEPSEEK_API_KEY` | — | DeepSeek API 密钥 |
| `DEEPSEEK_BASE_URL` | `https://api.deepseek.com` | DeepSeek API 端点 |
| `OPENROUTER_API_KEY` | — | OpenRouter API 密钥 |
| `TOGETHER_API_KEY` | — | Together AI API 密钥 |
| `ANVIL_WEB_SEARCH_BASE_URL` | DuckDuckGo | 搜索引擎地址（国内推荐 `cn.bing.com`） |

---

## 对比系统

`anvil compare` 从第一性原理出发度量进步：

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

## 更新方向

基于第一性原理决策树的未来路线：

### 短期（数据驱动）

- [ ] **优化 system prompt** — 让 anvil 对简单任务回答更精炼（对比数据显示当前回答偏长）
- [ ] **减少工具调用开销** — 简单问答场景不该触发文件写入和 bash 执行
- [ ] **更多 provider 支持** — 通过 `~/.anvil/settings.json` 配置更多供应商

### 中期（能力增强）

- [ ] **会话管理优化** — session 生命周期管理
- [ ] **MCP 工具生态** — 通过 MCP 协议接入更多工具
- [ ] **对比自动化** — 定期批量对比生成趋势报告

### 长期（由用户需求驱动）

> 不做 roadmap。方向由 `anvil compare` 的数据和用户反馈决定。
> 如果有新工具出现比参考基准更好，anvil 就和更好的工具比。

---

## 与参考基准工具的关系

anvil 不和任何特定工具对标。

参考基准工具的定位是 **度量尺**：
- 告诉我们"当前最好的体验能做到什么程度"
- 帮助我们找到差距
- 但不意味着 anvil 要用同样的方式去填补差距

anvil 的方式始终是：**从最根本的需求出发，走最直接的路径**。

---

## 开源协议

本项目基于 [MIT License](LICENSE)。

是基于 [claw-code](https://github.com/ultraworkers/claw-code) 的 fork，原始项目同样使用 MIT License。
