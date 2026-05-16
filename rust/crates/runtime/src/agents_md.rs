//! AGENTS.md 解析：支持 YAML front matter + 模板变量渲染。
//!
//! # 格式
//! `AGENTS.md` 文件可以可选地以 `---` 包围的 YAML front matter 开头：
//! ```markdown
//! ---
//! sandbox: true
//! max_turns: 50
//! model: gpt-4
//! hooks:
//!   - on_start: echo "started"
//! ---
//! 这里是 AGENTS.md 的 prompt 正文，支持 {{project.name}} 等模板变量。
//! ```
//!
//! # 配置项
//! - `sandbox` (bool) — 是否启用沙箱执行
//! - `max_turns` (int) — 会话的最大轮次上限
//! - `model` (string) — 推荐使用的模型名称
//! - `hooks` (object) — 生命周期钩子配置
//! - `tags` (Vec<String>) — 标签元数据

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Hooks 配置，支持在 AGENTS.md 中声明生命周期回调。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AgentsHooksConfig {
    /// 会话启动时执行的命令或脚本路径
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_start: Option<String>,
    /// 工具调用前执行的命令或脚本路径
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_tool: Option<String>,
    /// 工具调用后执行的命令或脚本路径
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_tool: Option<String>,
    /// 会话结束时执行的命令或脚本路径
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_end: Option<String>,
}

/// AGENTS.md 中 YAML front matter 的完整配置结构。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AgentsMdFrontMatter {
    /// 是否启用沙箱执行环境
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<bool>,
    /// 会话最大轮次上限
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// 推荐使用的模型标识符
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 生命周期钩子配置
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<AgentsHooksConfig>,
    /// 标签/分类信息
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// 任意额外字段（保留扩展性）
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

/// AGENTS.md 解析后的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAgentsMd {
    /// 解析出的 YAML front matter 配置（如果存在）
    pub front_matter: Option<AgentsMdFrontMatter>,
    /// 移除了 front matter 后的纯 markdown body（模板变量未渲染）
    pub body: String,
    /// 完整的原始内容
    pub raw: String,
}

/// 模板渲染使用的上下文变量。
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    pub variables: HashMap<String, String>,
}

impl TemplateContext {
    /// 创建一个空的模板上下文。
    #[must_use]
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
        }
    }

    /// 添加一个变量。
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.variables.insert(key.into(), value.into());
    }

    /// 批量设置变量。
    pub fn extend(&mut self, pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) {
        for (key, value) in pairs {
            self.set(key, value);
        }
    }
}

/// 解析 AGENTS.md 内容，提取 YAML front matter 和 body。
///
/// front matter 位于文件开头的 `---` 与 `---` 之间。
pub fn parse_agents_md(content: &str) -> ParsedAgentsMd {
    let trimmed = content.trim_start();

    // 检查是否以 `---` 开头（YAML front matter 标记）
    if let Some(rest) = trimmed.strip_prefix("---") {
        // 找到 closing `---`
        if let Some(end_pos) = find_front_matter_end(rest) {
            let yaml_text = &rest[..end_pos];
            let body_start = end_pos + 3; // skip the closing `---`
            let body = rest[body_start..].trim_start().to_string();

            let front_matter: Option<AgentsMdFrontMatter> =
                serde_yaml::from_str(yaml_text).ok();

            return ParsedAgentsMd {
                front_matter,
                body,
                raw: content.to_string(),
            };
        }
    }

    // 没有有效的 front matter，返回纯 body
    ParsedAgentsMd {
        front_matter: None,
        body: trimmed.to_string(),
        raw: content.to_string(),
    }
}

/// 在 YAML 正文中查找 closing `---` 的位置。
fn find_front_matter_end(yaml_body: &str) -> Option<usize> {
    let mut pos = 0;
    for line in yaml_body.lines() {
        let line_len = line.len();
        let line_trimmed = line.trim();
        if line_trimmed == "---" {
            return Some(pos);
        }
        // +1 for the newline character
        pos += line_len + 1;
        if pos > yaml_body.len() {
            return None;
        }
    }
    None
}

/// 渲染模板字符串：将 `{{key}}` 替换为 context 中对应的值。
///
/// 支持点号分隔的嵌套键，如 `{{project.name}}` 会查找 `project.name` 键。
/// 未找到的变量保留原样。
#[must_use]
pub fn render_template(template: &str, context: &TemplateContext) -> String {
    let mut result = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(start) = remaining.find("{{") {
        // 追加 `{{` 之前的内容
        result.push_str(&remaining[..start]);
        let after_open = &remaining[start + 2..];

        if let Some(end) = after_open.find("}}") {
            let key = after_open[..end].trim();

            // 从 context 中查找值
            let value = context.variables.get(key).map_or_else(
                || {
                    // 尝试点号分隔的嵌套查找
                    resolve_nested_key(key, &context.variables)
                },
                |v| Some(v.clone()),
            );

            match value {
                Some(val) => result.push_str(&val),
                None => {
                    // 未找到，保留原样
                    result.push_str(&format!("{{{{{}}}}}", key));
                }
            }

            remaining = &after_open[end + 2..];
        } else {
            // 没有 closing `}}`，追加剩余内容
            result.push_str(&remaining[start..]);
            break;
        }
    }

    result.push_str(remaining);
    result
}

/// 解析点号分隔的嵌套键，例如 `project.name`。
fn resolve_nested_key(key: &str, variables: &HashMap<String, String>) -> Option<String> {
    // 直接作为整体键查找
    if let Some(val) = variables.get(key) {
        return Some(val.clone());
    }

    // 尝试点号分隔
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    if parts.len() == 2 {
        let prefix = parts[0];
        let suffix = parts[1];

        // 先查找完整前缀键，如果是 JSON 对象则进一步查找
        if let Some(val) = variables.get(prefix) {
            // 尝试将值作为 JSON 解析并获取嵌套字段
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(val) {
                return resolve_json_path(&json_val, suffix);
            }
        }
    }

    None
}

/// 在 JSON Value 中沿点号路径查找。
fn resolve_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<String> {
    let parts: Vec<&str> = path.splitn(2, '.').collect();
    let key = parts[0];

    match value.get(key) {
        Some(sub_val) => {
            if parts.len() == 1 {
                // 叶子节点
                match sub_val {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Array(arr) => {
                        Some(arr.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "))
                    }
                    serde_json::Value::Object(_) => Some(sub_val.to_string()),
                    serde_json::Value::Null => None,
                }
            } else {
                // 继续递归
                resolve_json_path(sub_val, parts[1])
            }
        }
        None => None,
    }
}

/// 解析并渲染 AGENTS.md 文件。
///
/// 1. 读取文件内容
/// 2. 解析 YAML front matter
/// 3. 用提供的 context 渲染模板变量
/// 4. 返回解析后的结构
pub fn load_and_parse_agents_md(
    path: &Path,
    context: &TemplateContext,
) -> std::io::Result<ParsedAgentsMd> {
    let content = std::fs::read_to_string(path)?;
    let mut parsed = parse_agents_md(&content);

    // 渲染 body 中的模板变量
    parsed.body = render_template(&parsed.body, context);

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_front_matter() {
        let content = r#"---
sandbox: true
max_turns: 50
model: gpt-4
---
# Project Rules

Follow these rules."#;

        let parsed = parse_agents_md(content);
        assert!(parsed.front_matter.is_some());
        let fm = parsed.front_matter.unwrap();
        assert_eq!(fm.sandbox, Some(true));
        assert_eq!(fm.max_turns, Some(50));
        assert_eq!(fm.model, Some("gpt-4".to_string()));
        assert!(parsed.body.contains("# Project Rules"));
    }

    #[test]
    fn parses_front_matter_with_hooks() {
        let content = r#"---
hooks:
  on_start: echo "started"
  on_end: echo "ended"
tags:
  - test
  - ci
---
Body content"#;

        let parsed = parse_agents_md(content);
        assert!(parsed.front_matter.is_some());
        let fm = parsed.front_matter.unwrap();
        assert_eq!(
            fm.hooks.as_ref().and_then(|h| h.on_start.as_deref()),
            Some(r#"echo "started""#)
        );
        assert_eq!(fm.tags, vec!["test", "ci"]);
        assert_eq!(parsed.body, "Body content");
    }

    #[test]
    fn returns_no_front_matter_without_delimiters() {
        let content = "# Just a regular markdown file\n\nNo front matter here.";
        let parsed = parse_agents_md(content);
        assert!(parsed.front_matter.is_none());
        assert_eq!(parsed.body, content.trim_start());
    }

    #[test]
    fn returns_no_front_matter_for_invalid_yaml() {
        let content = r#"---
invalid: [unclosed
---
Body"#;
        let parsed = parse_agents_md(content);
        assert!(parsed.front_matter.is_none());
        // body 仍应正确提取
        assert_eq!(parsed.body, "Body");
    }

    #[test]
    fn empty_front_matter() {
        let content = r#"---
---
Body"#;
        let parsed = parse_agents_md(content);
        assert!(parsed.front_matter.is_some());
        // 空 front matter 应为默认值
        let fm = parsed.front_matter.unwrap();
        assert_eq!(fm.sandbox, None);
        assert_eq!(fm.max_turns, None);
    }

    #[test]
    fn renders_simple_template_variables() {
        let template = "Hello, {{name}}! Today is {{date}}.";
        let mut context = TemplateContext::new();
        context.set("name", "World");
        context.set("date", "2026-05-16");

        let result = render_template(template, &context);
        assert_eq!(result, "Hello, World! Today is 2026-05-16.");
    }

    #[test]
    fn renders_nested_template_variables() {
        let template = "Project: {{project.name}}, Branch: {{git.branch}}";
        let mut context = TemplateContext::new();
        context.set("project.name", "MyApp");
        context.set("git.branch", "main");

        let result = render_template(template, &context);
        assert_eq!(result, "Project: MyApp, Branch: main");
    }

    #[test]
    fn leaves_unresolved_variables_unchanged() {
        let template = "Hello, {{unknown}}!";
        let context = TemplateContext::new();
        let result = render_template(template, &context);
        assert_eq!(result, "Hello, {{unknown}}!");
    }

    #[test]
    fn renders_multiple_occurrences() {
        let template = "{{x}} + {{x}} = {{y}}";
        let mut context = TemplateContext::new();
        context.set("x", "1");
        context.set("y", "2");
        let result = render_template(template, &context);
        assert_eq!(result, "1 + 1 = 2");
    }

    #[test]
    fn renders_empty_template() {
        assert_eq!(render_template("", &TemplateContext::new()), "");
    }

    #[test]
    fn renders_template_with_no_variables() {
        let result = render_template("Plain text without variables.", &TemplateContext::new());
        assert_eq!(result, "Plain text without variables.");
    }

    #[test]
    fn renders_front_matter_and_body_with_template() {
        let content = r#"---
sandbox: true
---
Project: {{project.name}}"#;

        let parsed = parse_agents_md(content);
        let mut context = TemplateContext::new();
        context.set("project.name", "Anvil");

        let rendered_body = render_template(&parsed.body, &context);
        assert_eq!(rendered_body, "Project: Anvil");
    }

    #[test]
    fn load_and_parse_agents_md_from_file() {
        let dir = std::env::temp_dir().join(format!("agents-md-test-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("AGENTS.md");
        std::fs::write(&path, r#"---
max_turns: 100
---
# {{project.name}} Rules

Be careful."#).unwrap();

        let mut context = TemplateContext::new();
        context.set("project.name", "TestProject");

        let parsed = load_and_parse_agents_md(&path, &context).unwrap();
        assert_eq!(parsed.front_matter.as_ref().and_then(|f| f.max_turns), Some(100));
        assert!(parsed.body.contains("TestProject Rules"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_json_path_nested_object() {
        let json: serde_json::Value = serde_json::from_str(r#"{"name": "Anvil", "version": "1.0"}"#).unwrap();
        let result = resolve_json_path(&json, "name");
        assert_eq!(result, Some("Anvil".to_string()));
    }

    #[test]
    fn resolve_json_path_missing_key() {
        let json: serde_json::Value = serde_json::from_str(r#"{"name": "Anvil"}"#).unwrap();
        let result = resolve_json_path(&json, "missing");
        assert_eq!(result, None);
    }

    #[test]
    fn front_matter_with_extra_fields() {
        let content = r#"---
sandbox: true
custom_field: value123
priority: 5
---
Body"#;

        let parsed = parse_agents_md(content);
        assert!(parsed.front_matter.is_some());
        let fm = parsed.front_matter.unwrap();
        assert_eq!(fm.sandbox, Some(true));
        // extra 字段应包含未映射的字段
        assert!(fm.extra.contains_key("custom_field"));
        assert!(fm.extra.contains_key("priority"));
    }

    #[test]
    fn dedupe_instruction_files_works_with_front_matter() {
        // 通过从普通内容中剥离 front matter 来测试去重不影响 body
        let content1 = r#"---
sandbox: true
---
same body"#;
        let content2 = r#"---
sandbox: false
---
same body"#;

        let parsed1 = parse_agents_md(content1);
        let parsed2 = parse_agents_md(content2);

        // 两个文件的 body 相同，但 front matter 不同
        assert_eq!(parsed1.body, parsed2.body);
        assert_ne!(parsed1.front_matter, parsed2.front_matter);
    }
}
