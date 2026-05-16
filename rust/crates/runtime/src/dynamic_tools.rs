//! Dynamic Tool Registration System
//!
//! Provides a runtime mechanism for registering, unregistering, and executing
//! tools that are not compiled into the binary. This enables plugin systems,
//! MCP server tool mirrors, and user-defined tool extensions.
//!
//! # Security Model
//!
//! - Tool names are sanitized to prevent path traversal / injection
//! - Built-in tool names cannot be overwritten
//! - Registration requires explicit permission level declaration
//! - Name conflicts are detected at registration time

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permissions::PermissionMode;

/// Error returned by the dynamic tool registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynamicToolError {
    /// The tool name conflicts with an already-registered tool.
    NameConflict(String),
    /// The tool name contains invalid characters.
    InvalidName(String),
    /// No tool with the given name is registered.
    NotFound(String),
    /// The tool execution failed with the given reason.
    ExecutionFailed(String),
    /// The tool's permission declaration was rejected.
    PermissionViolation(String),
}

impl std::fmt::Display for DynamicToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameConflict(name) => {
                write!(f, "dynamic tool name conflict: `{name}` already exists")
            }
            Self::InvalidName(name) => {
                write!(f, "invalid dynamic tool name: `{name}`")
            }
            Self::NotFound(name) => write!(f, "dynamic tool not found: `{name}`"),
            Self::ExecutionFailed(reason) => write!(f, "dynamic tool execution failed: {reason}"),
            Self::PermissionViolation(reason) => {
                write!(f, "dynamic tool permission violation: {reason}")
            }
        }
    }
}

impl std::error::Error for DynamicToolError {}

/// Metadata describing a dynamic tool, used for API schemas and introspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicToolDefinition {
    /// Canonical tool name (e.g. "my_plugin__greet").
    pub name: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// JSON Schema describing the expected input.
    pub input_schema: Value,
    /// Minimum permission level required.
    pub required_permission: PermissionMode,
    /// Optional source label (e.g. "plugin:python-mcp", "user:custom").
    pub source: Option<String>,
}

/// The core trait for a dynamically registered tool.
///
/// Implementors provide the tool's identity and execution logic.
/// The registry applies security checks and permission gating around
/// the execution.
pub trait DynamicTool: Send + Sync {
    /// Returns the tool's canonical name.
    fn name(&self) -> &str;

    /// Returns the tool's metadata definition.
    fn definition(&self) -> DynamicToolDefinition;

    /// Execute the tool with the given parsed JSON input.
    ///
    /// Implementors should validate the input against their schema
    /// before performing the operation.
    fn execute(&self, input: &Value) -> Result<String, String>;
}

// Blanket impl to box storage
impl<T: DynamicTool + 'static> From<T> for Box<dyn DynamicTool> {
    fn from(tool: T) -> Self {
        Box::new(tool)
    }
}

/// Pre-defined set of built-in tool names that cannot be overwritten.
fn builtin_tool_names() -> BTreeSet<&'static str> {
    BTreeSet::from([
        "bash",
        "read_file",
        "read_function",
        "write_file",
        "edit_file",
        "glob_search",
        "grep_search",
        "WebFetch",
        "WebSearch",
        "TodoWrite",
        "Skill",
        "Agent",
        "ToolSearch",
        "NotebookEdit",
        "Sleep",
        "SendUserMessage",
        "Config",
        "EnterPlanMode",
        "ExitPlanMode",
        "StructuredOutput",
        "REPL",
        "PowerShell",
        "AskUserQuestion",
        "TaskCreate",
        "RunTaskPacket",
        "TaskGet",
        "TaskList",
        "TaskStop",
        "TaskUpdate",
        "TaskOutput",
        "WorkerCreate",
        "WorkerGet",
        "WorkerObserve",
        "WorkerResolveTrust",
        "WorkerAwaitReady",
        "WorkerSendPrompt",
        "WorkerRestart",
        "WorkerTerminate",
        "WorkerObserveCompletion",
        "TeamCreate",
        "TeamDelete",
        "CronCreate",
        "CronDelete",
        "CronList",
        "LSP",
        "ListMcpResources",
        "ReadMcpResource",
        "McpAuth",
        "RemoteTrigger",
        "MCP",
        "TestingPermission",
        "Browser",
        "Brief",
    ])
}

/// Validate that a tool name is safe and well-formed.
///
/// Rules:
/// - Must not be empty
/// - Must start with an ASCII alphabetic character
/// - Must contain only ASCII alphanumerics, underscores, or hyphens
/// - Must not be a reserved built-in name
fn validate_tool_name(name: &str) -> Result<(), DynamicToolError> {
    if name.is_empty() {
        return Err(DynamicToolError::InvalidName(
            "tool name must not be empty".to_string(),
        ));
    }

    if !name.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        return Err(DynamicToolError::InvalidName(format!(
            "tool name `{name}` must start with an ASCII alphabetic character"
        )));
    }

    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(DynamicToolError::InvalidName(format!(
            "tool name `{name}` contains invalid characters (only a-z, A-Z, 0-9, _, - allowed)"
        )));
    }

    // Prevent names that look like MCP-qualified tool names (containing `__`)
    if name.contains("__") {
        return Err(DynamicToolError::InvalidName(format!(
            "tool name `{name}` resembles an MCP-qualified name (contains `__`); \
             use the MCP tool bridge instead"
        )));
    }

    Ok(())
}

/// Thread-safe registry for dynamically registered tools.
///
/// This is the central coordination point for runtime tool registration.
/// It enforces naming rules, prevents conflicts with built-in tools,
/// and provides a unified interface for execution and introspection.
#[derive(Default)]
pub struct DynamicToolRegistry {
    inner: RwLock<HashMap<String, DynamicToolEntry>>,
}

impl std::fmt::Debug for DynamicToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = self.list_names();
        f.debug_struct("DynamicToolRegistry")
            .field("tool_count", &names.len())
            .field("tools", &names)
            .finish()
    }
}

/// Internal entry storing the tool and its derived metadata.
struct DynamicToolEntry {
    definition: DynamicToolDefinition,
    tool: Arc<dyn DynamicTool>,
}

impl DynamicToolRegistry {
    /// Create a new, empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Register a dynamic tool.
    ///
    /// # Errors
    ///
    /// Returns [`DynamicToolError::InvalidName`] if the name fails validation,
    /// [`DynamicToolError::NameConflict`] if a tool with that name already exists
    /// or if it conflicts with a built-in tool.
    pub fn register(
        &self,
        tool: Box<dyn DynamicTool>,
    ) -> Result<DynamicToolDefinition, DynamicToolError> {
        let definition = tool.definition();

        // Validate name
        validate_tool_name(&definition.name)?;

        // Prevent overwriting built-in tools
        if builtin_tool_names().contains(definition.name.as_str()) {
            return Err(DynamicToolError::NameConflict(format!(
                "`{}` is a built-in tool and cannot be registered dynamically",
                definition.name
            )));
        }

        let mut inner = self.inner.write().map_err(|_| {
            DynamicToolError::PermissionViolation("registry lock poisoned".to_string())
        })?;

        if inner.contains_key(&definition.name) {
            return Err(DynamicToolError::NameConflict(definition.name));
        }

        let entry = DynamicToolEntry {
            definition: definition.clone(),
            tool: tool.into(),
        };

        inner.insert(definition.name.clone(), entry);
        Ok(definition)
    }

    /// Unregister a dynamic tool by name.
    ///
    /// Returns `true` if the tool was removed, `false` if it was not found.
    pub fn unregister(&self, name: &str) -> bool {
        let mut inner = match self.inner.write() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        inner.remove(name).is_some()
    }

    /// Get the definition for a registered tool.
    #[must_use]
    pub fn get_definition(&self, name: &str) -> Option<DynamicToolDefinition> {
        let inner = self.inner.read().ok()?;
        inner.get(name).map(|entry| entry.definition.clone())
    }

    /// Check whether a tool with the given name is registered.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.inner.read().ok().is_some_and(|inner| inner.contains_key(name))
    }

    /// List all registered dynamic tool names.
    #[must_use]
    pub fn list_names(&self) -> Vec<String> {
        let inner = match self.inner.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        let mut names: Vec<String> = inner.keys().cloned().collect();
        names.sort();
        names
    }

    /// List all registered dynamic tool definitions.
    #[must_use]
    pub fn list_definitions(&self) -> Vec<DynamicToolDefinition> {
        let inner = match self.inner.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        let mut defs: Vec<DynamicToolDefinition> =
            inner.values().map(|entry| entry.definition.clone()).collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    /// Execute a registered dynamic tool.
    ///
    /// # Errors
    ///
    /// Returns [`DynamicToolError::NotFound`] if the tool is not registered,
    /// or [`DynamicToolError::ExecutionFailed`] if execution fails.
    pub fn execute(&self, name: &str, input: &Value) -> Result<String, DynamicToolError> {
        let inner = self.inner.read().map_err(|_| {
            DynamicToolError::PermissionViolation("registry lock poisoned".to_string())
        })?;

        let entry = inner
            .get(name)
            .ok_or_else(|| DynamicToolError::NotFound(name.to_string()))?;

        let result = entry.tool.execute(input).map_err(|error| {
            DynamicToolError::ExecutionFailed(format!("`{name}` failed: {error}"))
        })?;

        Ok(result)
    }

    /// Execute a registered dynamic tool with a raw JSON string input.
    ///
    /// Convenience wrapper that parses the input string first.
    ///
    /// # Errors
    ///
    /// Returns [`DynamicToolError::NotFound`] if the tool is not registered,
    /// [`DynamicToolError::ExecutionFailed`] if parsing or execution fails.
    pub fn execute_json(&self, name: &str, input_json: &str) -> Result<String, DynamicToolError> {
        let value: Value = serde_json::from_str(input_json).map_err(|error| {
            DynamicToolError::ExecutionFailed(format!(
                "invalid JSON input for `{name}`: {error}"
            ))
        })?;
        self.execute(name, &value)
    }

    /// Return the number of registered tools.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().map(|guard| guard.len()).unwrap_or(0)
    }

    /// Returns `true` if no tools are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all registered tools.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.write() {
            inner.clear();
        }
    }
}

// ---------------------------------------------------------------------------
// Global singleton accessor
// ---------------------------------------------------------------------------

/// Returns a reference to the global [`DynamicToolRegistry`].
///
/// This is the canonical singleton used across the runtime and CLI.
/// All dynamic tool registration should go through this instance.
#[must_use]
pub fn global_dynamic_tool_registry() -> &'static DynamicToolRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<DynamicToolRegistry> = OnceLock::new();
    REGISTRY.get_or_init(DynamicToolRegistry::new)
}

// ---------------------------------------------------------------------------
// Permissions helper
// ---------------------------------------------------------------------------

/// Provide a permission_spec for a dynamic tool, given a set of allowed
/// tool names. Returns `None` if the tool is not allowed by the set.
#[must_use]
pub fn dynamic_tool_permission_spec(
    registry: &DynamicToolRegistry,
    name: &str,
    allowed_tools: Option<&BTreeSet<String>>,
) -> Option<(String, PermissionMode)> {
    if allowed_tools.is_some_and(|allowed| !allowed.contains(name)) {
        return None;
    }
    let def = registry.get_definition(name)?;
    Some((def.name, def.required_permission))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A simple echo tool for testing.
    struct EchoTool {
        name: String,
        description: Option<String>,
        required_permission: PermissionMode,
    }

    impl EchoTool {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                description: Some(format!("Echo tool for {name}")),
                required_permission: PermissionMode::ReadOnly,
            }
        }
    }

    impl DynamicTool for EchoTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn definition(&self) -> DynamicToolDefinition {
            DynamicToolDefinition {
                name: self.name.clone(),
                description: self.description.clone(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" }
                    },
                    "required": ["text"],
                    "additionalProperties": false
                }),
                required_permission: self.required_permission,
                source: Some("test".to_string()),
            }
        }

        fn execute(&self, input: &Value) -> Result<String, String> {
            let text = input
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Ok(json!({
                "echoed": text,
                "tool": self.name
            })
            .to_string())
        }
    }

    struct FailingTool;

    impl DynamicTool for FailingTool {
        fn name(&self) -> &str {
            "failing"
        }
        fn definition(&self) -> DynamicToolDefinition {
            DynamicToolDefinition {
                name: "failing".to_string(),
                description: Some("Always fails".to_string()),
                input_schema: json!({ "type": "object" }),
                required_permission: PermissionMode::ReadOnly,
                source: None,
            }
        }
        fn execute(&self, _input: &Value) -> Result<String, String> {
            Err("simulated failure".to_string())
        }
    }

    #[test]
    fn registers_and_executes_dynamic_tool() {
        let registry = DynamicToolRegistry::new();
        let tool = EchoTool::new("my_greeter");
        let def = registry
            .register(Box::new(tool))
            .expect("registration should succeed");

        assert_eq!(def.name, "my_greeter");
        assert!(registry.has("my_greeter"));

        let result = registry
            .execute("my_greeter", &json!({"text": "hello"}))
            .expect("execution should succeed");
        assert!(result.contains("hello"));
        assert!(result.contains("my_greeter"));
    }

    #[test]
    fn rejects_empty_name() {
        let registry = DynamicToolRegistry::new();
        struct EmptyName;
        impl DynamicTool for EmptyName {
            fn name(&self) -> &str { "" }
            fn definition(&self) -> DynamicToolDefinition {
                DynamicToolDefinition {
                    name: String::new(),
                    description: None,
                    input_schema: json!({}),
                    required_permission: PermissionMode::ReadOnly,
                    source: None,
                }
            }
            fn execute(&self, _input: &Value) -> Result<String, String> {
                Ok("ok".to_string())
            }
        }
        let err = registry
            .register(Box::new(EmptyName))
            .expect_err("empty name should be rejected");
        assert!(matches!(err, DynamicToolError::InvalidName(_)));
    }

    #[test]
    fn rejects_name_with_special_chars() {
        let registry = DynamicToolRegistry::new();
        struct BadName;
        impl DynamicTool for BadName {
            fn name(&self) -> &str { "bad/tool!" }
            fn definition(&self) -> DynamicToolDefinition {
                DynamicToolDefinition {
                    name: "bad/tool!".to_string(),
                    description: None,
                    input_schema: json!({}),
                    required_permission: PermissionMode::ReadOnly,
                    source: None,
                }
            }
            fn execute(&self, _input: &Value) -> Result<String, String> {
                Ok("ok".to_string())
            }
        }
        let err = registry
            .register(Box::new(BadName))
            .expect_err("special chars should be rejected");
        assert!(matches!(err, DynamicToolError::InvalidName(_)));
        assert!(err.to_string().contains("invalid characters"));
    }

    #[test]
    fn rejects_mcp_style_double_underscore() {
        let registry = DynamicToolRegistry::new();
        struct McpStyle;
        impl DynamicTool for McpStyle {
            fn name(&self) -> &str { "mcp__server__tool" }
            fn definition(&self) -> DynamicToolDefinition {
                DynamicToolDefinition {
                    name: "mcp__server__tool".to_string(),
                    description: None,
                    input_schema: json!({}),
                    required_permission: PermissionMode::ReadOnly,
                    source: None,
                }
            }
            fn execute(&self, _input: &Value) -> Result<String, String> {
                Ok("ok".to_string())
            }
        }
        let err = registry
            .register(Box::new(McpStyle))
            .expect_err("MCP-style double underscore should be rejected");
        assert!(matches!(err, DynamicToolError::InvalidName(_)));
        assert!(err.to_string().contains("MCP-qualified"));
    }

    #[test]
    fn rejects_builtin_name_conflict() {
        let registry = DynamicToolRegistry::new();
        struct OverwriteBash;
        impl DynamicTool for OverwriteBash {
            fn name(&self) -> &str { "bash" }
            fn definition(&self) -> DynamicToolDefinition {
                DynamicToolDefinition {
                    name: "bash".to_string(),
                    description: None,
                    input_schema: json!({}),
                    required_permission: PermissionMode::DangerFullAccess,
                    source: None,
                }
            }
            fn execute(&self, _input: &Value) -> Result<String, String> {
                Ok("override".to_string())
            }
        }
        let err = registry
            .register(Box::new(OverwriteBash))
            .expect_err("builtin name should be rejected");
        assert!(matches!(err, DynamicToolError::NameConflict(_)));
    }

    #[test]
    fn rejects_duplicate_registration() {
        let registry = DynamicToolRegistry::new();
        registry
            .register(Box::new(EchoTool::new("duplicate")))
            .expect("first registration should succeed");
        let err = registry
            .register(Box::new(EchoTool::new("duplicate")))
            .expect_err("duplicate should be rejected");
        assert!(matches!(err, DynamicToolError::NameConflict(_)));
    }

    #[test]
    fn unregister_removes_tool() {
        let registry = DynamicToolRegistry::new();
        registry
            .register(Box::new(EchoTool::new("temp")))
            .expect("registration should succeed");
        assert!(registry.has("temp"));
        assert!(registry.unregister("temp"));
        assert!(!registry.has("temp"));
        assert!(!registry.unregister("temp")); // second unregister returns false
    }

    #[test]
    fn execute_returns_not_found_for_missing_tool() {
        let registry = DynamicToolRegistry::new();
        let err = registry
            .execute("nonexistent", &json!({}))
            .expect_err("missing tool should error");
        assert!(matches!(err, DynamicToolError::NotFound(_)));
    }

    #[test]
    fn execute_returns_execution_failed() {
        let registry = DynamicToolRegistry::new();
        registry
            .register(Box::new(FailingTool))
            .expect("registration should succeed");
        let err = registry
            .execute("failing", &json!({}))
            .expect_err("failing tool should error");
        assert!(matches!(err, DynamicToolError::ExecutionFailed(_)));
        assert!(err.to_string().contains("simulated failure"));
    }

    #[test]
    fn list_names_returns_sorted() {
        let registry = DynamicToolRegistry::new();
        registry
            .register(Box::new(EchoTool::new("z_tool")))
            .expect("reg z");
        registry
            .register(Box::new(EchoTool::new("a_tool")))
            .expect("reg a");
        registry
            .register(Box::new(EchoTool::new("m_tool")))
            .expect("reg m");

        let names = registry.list_names();
        assert_eq!(names, vec!["a_tool", "m_tool", "z_tool"]);
    }

    #[test]
    fn list_definitions_returns_sorted_by_name() {
        let registry = DynamicToolRegistry::new();
        registry
            .register(Box::new(EchoTool::new("beta")))
            .expect("reg beta");
        registry
            .register(Box::new(EchoTool::new("alpha")))
            .expect("reg alpha");

        let defs = registry.list_definitions();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "alpha");
        assert_eq!(defs[1].name, "beta");
    }

    #[test]
    fn execute_json_parses_and_dispatches() {
        let registry = DynamicToolRegistry::new();
        registry
            .register(Box::new(EchoTool::new("json_echo")))
            .expect("reg");
        let result = registry
            .execute_json("json_echo", r#"{"text": "world"}"#)
            .expect("execute_json should succeed");
        assert!(result.contains("world"));
    }

    #[test]
    fn execute_json_rejects_invalid_json() {
        let registry = DynamicToolRegistry::new();
        registry
            .register(Box::new(EchoTool::new("json_echo")))
            .expect("reg");
        let err = registry
            .execute_json("json_echo", "not-json")
            .expect_err("invalid JSON should fail");
        assert!(matches!(err, DynamicToolError::ExecutionFailed(_)));
    }

    #[test]
    fn clear_removes_all_tools() {
        let registry = DynamicToolRegistry::new();
        registry
            .register(Box::new(EchoTool::new("a")))
            .expect("reg a");
        registry
            .register(Box::new(EchoTool::new("b")))
            .expect("reg b");
        assert_eq!(registry.len(), 2);
        registry.clear();
        assert!(registry.is_empty());
    }

    #[test]
    fn global_singleton_is_accessible() {
        let registry = global_dynamic_tool_registry();
        assert!(registry.is_empty());
        // Should be same instance on second call
        let same = global_dynamic_tool_registry();
        assert_eq!(registry.len(), same.len());
    }

    #[test]
    fn validate_tool_name_rejects_various_patterns() {
        assert!(validate_tool_name("").is_err());
        assert!(validate_tool_name("123abc").is_err());
        assert!(validate_tool_name("-leading").is_err());
        assert!(validate_tool_name("has spaces").is_err());
        assert!(validate_tool_name("has.dots").is_err());
        assert!(validate_tool_name("double__underscore").is_err());
        assert!(validate_tool_name("valid_name").is_ok());
        assert!(validate_tool_name("valid-name").is_ok());
        assert!(validate_tool_name("camelCase").is_ok());
    }
}
