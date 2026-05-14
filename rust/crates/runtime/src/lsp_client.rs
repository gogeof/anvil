#![allow(clippy::should_implement_trait, clippy::must_use_candidate)]
//! LSP (Language Server Protocol) client for IDE-level code intelligence.
//!
//! This module provides a registry and process manager for Language Server Protocol
//! servers, enabling features like go-to-definition, find references, hover info,
//! and diagnostics.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{timeout, Duration};

// ============================================================================
// LSP Action Types
// ============================================================================

/// Supported LSP actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspAction {
    Diagnostics,
    Hover,
    Definition,
    References,
    Completion,
    Symbols,
    Format,
}

impl LspAction {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "diagnostics" => Some(Self::Diagnostics),
            "hover" => Some(Self::Hover),
            "definition" | "goto_definition" => Some(Self::Definition),
            "references" | "find_references" => Some(Self::References),
            "completion" | "completions" => Some(Self::Completion),
            "symbols" | "document_symbols" => Some(Self::Symbols),
            "format" | "formatting" => Some(Self::Format),
            _ => None,
        }
    }
}

// ============================================================================
// LSP Result Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub path: String,
    pub line: u32,
    pub character: u32,
    pub severity: String,
    pub message: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspLocation {
    pub path: String,
    pub line: u32,
    pub character: u32,
    pub end_line: Option<u32>,
    pub end_character: Option<u32>,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHoverResult {
    pub content: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCompletionItem {
    pub label: String,
    pub kind: Option<String>,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspSymbol {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspServerStatus {
    Connected,
    Disconnected,
    Starting,
    Error,
}

impl std::fmt::Display for LspServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected => write!(f, "connected"),
            Self::Disconnected => write!(f, "disconnected"),
            Self::Starting => write!(f, "starting"),
            Self::Error => write!(f, "error"),
        }
    }
}

// ============================================================================
// LSP JSON-RPC Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum LspId {
    Number(i64),
    String(String),
    Null,
}

#[derive(Debug, Clone, Serialize)]
pub struct LspRequest<T = JsonValue> {
    pub jsonrpc: &'static str,
    pub id: LspId,
    pub method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<T>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LspResponse<T = JsonValue> {
    pub jsonrpc: String,
    pub id: LspId,
    pub result: Option<T>,
    #[serde(default)]
    pub error: Option<LspError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LspError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<JsonValue>,
}

// LSP Initialize types
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspInitializeParams {
    pub process_id: Option<u32>,
    pub root_uri: Option<String>,
    pub capabilities: LspClientCapabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct LspClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_document: Option<LspTextDocumentClientCapabilities>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LspTextDocumentClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<LspGenericCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<LspGenericCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover: Option<LspGenericCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion: Option<LspGenericCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_symbol: Option<LspGenericCapability>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LspGenericCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
}

impl Default for LspClientCapabilities {
    fn default() -> Self {
        Self {
            text_document: Some(LspTextDocumentClientCapabilities {
                definition: Some(LspGenericCapability {
                    dynamic_registration: Some(false),
                }),
                references: Some(LspGenericCapability {
                    dynamic_registration: Some(false),
                }),
                hover: Some(LspGenericCapability {
                    dynamic_registration: Some(false),
                }),
                completion: Some(LspGenericCapability {
                    dynamic_registration: Some(false),
                }),
                document_symbol: Some(LspGenericCapability {
                    dynamic_registration: Some(false),
                }),
            }),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspInitializeResult {
    pub capabilities: LspServerCapabilities,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LspServerCapabilities {
    #[serde(default)]
    pub definition_provider: Option<bool>,
    #[serde(default)]
    pub references_provider: Option<bool>,
    #[serde(default)]
    pub hover_provider: Option<bool>,
    #[serde(default)]
    pub completion_provider: Option<LspCompletionOptions>,
    #[serde(default)]
    pub document_symbol_provider: Option<bool>,
    #[serde(default)]
    pub text_document_sync: Option<LspTextDocumentSyncKind>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LspCompletionOptions {
    #[serde(default)]
    pub trigger_characters: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum LspTextDocumentSyncKind {
    Number(i32),
    Object(JsonValue),
}

// Text Document Identifier
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspTextDocumentIdentifier {
    pub uri: String,
}

// Position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

// Text Document Position Params
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspTextDocumentPositionParams {
    pub text_document: LspTextDocumentIdentifier,
    pub position: LspPosition,
}

// Location (for definition/references results)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspLocationResult {
    pub uri: String,
    pub range: LspRange,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

// Hover result
#[derive(Debug, Clone, Deserialize)]
pub struct LspHover {
    #[serde(default)]
    pub contents: LspHoverContents,
    #[serde(default)]
    pub range: Option<LspRange>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum LspHoverContents {
    String(String),
    MarkupContent(LspMarkupContent),
    Array(Vec<LspMarkedString>),
}

impl Default for LspHoverContents {
    fn default() -> Self {
        Self::String(String::new())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LspMarkupContent {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LspMarkedString {
    pub language: String,
    pub value: String,
}

// Completion result
#[derive(Debug, Clone, Deserialize)]
pub struct LspCompletionList {
    #[serde(default)]
    pub items: Vec<LspCompletionItemResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LspCompletionItemResult {
    pub label: String,
    #[serde(default)]
    pub kind: Option<i32>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub insert_text: Option<String>,
}

// Document Symbol result
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspSymbolInformation {
    pub name: String,
    pub kind: i32,
    pub location: LspLocationResult,
    #[serde(default)]
    pub container_name: Option<String>,
}

// ============================================================================
// LSP Server Configuration
// ============================================================================

/// Configuration for an LSP server.
#[derive(Debug, Clone)]
pub struct LspServerConfig {
    /// The command to spawn the LSP server.
    pub command: String,
    /// Arguments to pass to the command.
    pub args: Vec<String>,
    /// Environment variables for the process.
    pub env: HashMap<String, String>,
}

impl LspServerConfig {
    /// Create a new LSP server config.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
        }
    }

    /// Add arguments to the command.
    #[must_use]
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Add an environment variable.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}

/// Get the default LSP server config for a language.
pub fn default_lsp_config(language: &str) -> Option<LspServerConfig> {
    match language {
        "rust" => Some(LspServerConfig::new("rust-analyzer").args(["--stdio"])),
        "typescript" | "javascript" => {
            Some(LspServerConfig::new("typescript-language-server").args(["--stdio"]))
        }
        "python" => Some(LspServerConfig::new("pyright-langserver").args(["--stdio"])),
        "go" => Some(LspServerConfig::new("gopls").args(["--stdio", "serve"])),
        "java" => Some(LspServerConfig::new("jdtls")),
        "c" | "cpp" => Some(LspServerConfig::new("clangd")),
        "ruby" => Some(LspServerConfig::new("solargraph").args(["stdio"])),
        "lua" => Some(LspServerConfig::new("lua-language-server")),
        _ => None,
    }
}

// ============================================================================
// LSP Process
// ============================================================================

/// An LSP server process with stdin/stdout communication.
#[derive(Debug)]
pub struct LspProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl LspProcess {
    /// Spawn an LSP server process.
    pub fn spawn(config: &LspServerConfig, root_path: &Path) -> io::Result<Self> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .current_dir(root_path);

        for (key, value) in &config.env {
            command.env(key, value);
        }

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("LSP process missing stdin pipe"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("LSP process missing stdout pipe"))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    fn take_id(&mut self) -> LspId {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        LspId::Number(id)
    }

    async fn write_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        let header = format!("Content-Length: {}\r\n\r\n", payload.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(payload).await?;
        self.stdin.flush().await
    }

    async fn read_frame(&mut self) -> io::Result<Vec<u8>> {
        let mut content_length = None;

        loop {
            let mut line = String::new();
            let bytes_read = self.stdout.read_line(&mut line).await?;
            if bytes_read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "LSP stream closed while reading headers",
                ));
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            let header = line.trim_end_matches(['\r', '\n']);
            if let Some((name, value)) = header.split_once(':') {
                if name.trim().eq_ignore_ascii_case("Content-Length") {
                    let parsed = value
                        .trim()
                        .parse::<usize>()
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                    content_length = Some(parsed);
                }
            }
        }

        let content_length = content_length.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
        })?;

        let mut payload = vec![0_u8; content_length];
        self.stdout.read_exact(&mut payload).await?;
        Ok(payload)
    }

    async fn send_request<T: Serialize>(&mut self, request: &LspRequest<T>) -> io::Result<()> {
        let body = serde_json::to_vec(request)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.write_frame(&body).await
    }

    async fn read_response<T: for<'de> Deserialize<'de>>(&mut self) -> io::Result<LspResponse<T>> {
        let payload = self.read_frame().await?;
        serde_json::from_slice(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn request<TParams: Serialize, TResult: for<'de> Deserialize<'de>>(
        &mut self,
        method: &'static str,
        params: Option<TParams>,
    ) -> io::Result<LspResponse<TResult>> {
        let id = self.take_id();
        let request = LspRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };
        self.send_request(&request).await?;

        let response = self.read_response().await?;
        Ok(response)
    }

    /// Send the initialize handshake.
    pub async fn initialize(
        &mut self,
        root_uri: Option<&str>,
    ) -> io::Result<LspServerCapabilities> {
        let params = LspInitializeParams {
            process_id: Some(std::process::id()),
            root_uri: root_uri.map(String::from),
            capabilities: LspClientCapabilities::default(),
        };

        let response: LspResponse<LspInitializeResult> = timeout(
            Duration::from_secs(30),
            self.request("initialize", Some(params)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "initialize timed out"))??;

        if let Some(error) = response.error {
            return Err(io::Error::other(format!(
                "LSP initialize error: {} ({})",
                error.message, error.code
            )));
        }

        let result = response.result.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "missing initialize result")
        })?;

        // Send initialized notification
        let notification = LspRequest::<()> {
            jsonrpc: "2.0",
            id: LspId::Null,
            method: "initialized",
            params: None,
        };
        self.send_request(&notification).await?;

        Ok(result.capabilities)
    }

    /// Request go-to-definition.
    pub async fn goto_definition(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> io::Result<Vec<LspLocationResult>> {
        let params = LspTextDocumentPositionParams {
            text_document: LspTextDocumentIdentifier {
                uri: uri.to_string(),
            },
            position: LspPosition { line, character },
        };

        let response = timeout(
            Duration::from_secs(10),
            self.request("textDocument/definition", Some(params)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "definition timed out"))??;

        if let Some(error) = response.error {
            return Err(io::Error::other(format!(
                "LSP definition error: {}",
                error.message
            )));
        }

        // Result can be a single location, array of locations, or null
        let result = response.result.unwrap_or(JsonValue::Null);
        let locations = Self::parse_locations(&result);
        Ok(locations)
    }

    /// Request find references.
    pub async fn find_references(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> io::Result<Vec<LspLocationResult>> {
        #[derive(Debug, Clone, Serialize)]
        #[serde(rename_all = "camelCase")]
        struct ReferencesParams {
            text_document: LspTextDocumentIdentifier,
            position: LspPosition,
            context: ReferenceContext,
        }

        #[derive(Debug, Clone, Serialize)]
        struct ReferenceContext {
            include_declaration: bool,
        }

        let params = ReferencesParams {
            text_document: LspTextDocumentIdentifier {
                uri: uri.to_string(),
            },
            position: LspPosition { line, character },
            context: ReferenceContext {
                include_declaration: true,
            },
        };

        let response = timeout(
            Duration::from_secs(10),
            self.request("textDocument/references", Some(params)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "references timed out"))??;

        if let Some(error) = response.error {
            return Err(io::Error::other(format!(
                "LSP references error: {}",
                error.message
            )));
        }

        let result = response.result.unwrap_or(JsonValue::Null);
        let locations = Self::parse_locations(&result);
        Ok(locations)
    }

    /// Request hover info.
    pub async fn hover(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> io::Result<Option<LspHover>> {
        let params = LspTextDocumentPositionParams {
            text_document: LspTextDocumentIdentifier {
                uri: uri.to_string(),
            },
            position: LspPosition { line, character },
        };

        let response = timeout(
            Duration::from_secs(10),
            self.request("textDocument/hover", Some(params)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "hover timed out"))??;

        if let Some(error) = response.error {
            return Err(io::Error::other(format!(
                "LSP hover error: {}",
                error.message
            )));
        }

        let result = response.result;
        if let Some(value) = result {
            let hover: LspHover = serde_json::from_value(value)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            Ok(Some(hover))
        } else {
            Ok(None)
        }
    }

    /// Request completion.
    pub async fn completion(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> io::Result<Vec<LspCompletionItemResult>> {
        let params = LspTextDocumentPositionParams {
            text_document: LspTextDocumentIdentifier {
                uri: uri.to_string(),
            },
            position: LspPosition { line, character },
        };

        let response = timeout(
            Duration::from_secs(10),
            self.request("textDocument/completion", Some(params)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "completion timed out"))??;

        if let Some(error) = response.error {
            return Err(io::Error::other(format!(
                "LSP completion error: {}",
                error.message
            )));
        }

        let result = response.result.unwrap_or(JsonValue::Null);
        let items = Self::parse_completions(&result);
        Ok(items)
    }

    /// Request document symbols.
    pub async fn document_symbols(&mut self, uri: &str) -> io::Result<Vec<LspSymbolInformation>> {
        let params = LspTextDocumentIdentifier {
            uri: uri.to_string(),
        };

        let response = timeout(
            Duration::from_secs(10),
            self.request("textDocument/documentSymbol", Some(params)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "document symbols timed out"))??;

        if let Some(error) = response.error {
            return Err(io::Error::other(format!(
                "LSP document symbols error: {}",
                error.message
            )));
        }

        let result = response.result.unwrap_or(JsonValue::Null);
        let symbols = Self::parse_symbols(&result);
        Ok(symbols)
    }

    fn parse_locations(value: &JsonValue) -> Vec<LspLocationResult> {
        if value.is_null() {
            return Vec::new();
        }
        if let Some(loc) = Self::parse_single_location(value) {
            return vec![loc];
        }
        if let Some(arr) = value.as_array() {
            return arr.iter().filter_map(Self::parse_single_location).collect();
        }
        // Handle LocationLink (from definition)
        if let Some(obj) = value.as_object() {
            if let Some(target_uri) = obj.get("targetUri") {
                if let Some(target_range) = obj.get("targetRange") {
                    if let (Some(uri), Some(range)) = (
                        target_uri.as_str(),
                        serde_json::from_value::<LspRange>(target_range.clone()).ok(),
                    ) {
                        return vec![LspLocationResult {
                            uri: uri.to_string(),
                            range,
                        }];
                    }
                }
            }
        }
        Vec::new()
    }

    fn parse_single_location(value: &JsonValue) -> Option<LspLocationResult> {
        serde_json::from_value(value.clone()).ok()
    }

    fn parse_completions(value: &JsonValue) -> Vec<LspCompletionItemResult> {
        if value.is_null() {
            return Vec::new();
        }
        // Can be CompletionList or array of CompletionItem
        if let Some(obj) = value.as_object() {
            if let Some(items) = obj.get("items") {
                if let Some(arr) = items.as_array() {
                    return arr
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();
                }
            }
        }
        if let Some(arr) = value.as_array() {
            return arr
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
        }
        Vec::new()
    }

    fn parse_symbols(value: &JsonValue) -> Vec<LspSymbolInformation> {
        if value.is_null() {
            return Vec::new();
        }
        if let Some(arr) = value.as_array() {
            return arr
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
        }
        Vec::new()
    }

    /// Shutdown the LSP process.
    pub async fn shutdown(&mut self) -> io::Result<()> {
        let _ = self.request::<(), JsonValue>("shutdown", None).await;
        let notification = LspRequest::<()> {
            jsonrpc: "2.0",
            id: LspId::Null,
            method: "exit",
            params: None,
        };
        let _ = self.send_request(&notification).await;
        let _ = self.child.wait().await;
        Ok(())
    }

    /// Check if the process has exited.
    pub fn has_exited(&mut self) -> io::Result<bool> {
        match self.child.try_wait()? {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
}

// ============================================================================
// LSP Server State
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerState {
    pub language: String,
    pub status: LspServerStatus,
    pub root_path: Option<String>,
    pub capabilities: Vec<String>,
    pub diagnostics: Vec<LspDiagnostic>,
}

// ============================================================================
// LSP Registry
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct LspRegistry {
    inner: Arc<Mutex<RegistryInner>>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    servers: HashMap<String, LspServerState>,
    processes: HashMap<String, LspProcess>,
}

impl LspRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start an LSP server for a language.
    pub async fn start_server(&self, language: &str, root_path: &Path) -> Result<(), String> {
        let config = default_lsp_config(language)
            .ok_or_else(|| format!("No LSP server configured for language: {language}"))?;

        // Check if already running and mark as starting
        {
            let mut inner = self.inner.lock().expect("lsp registry lock poisoned");

            // Check if already running
            if let Some(state) = inner.servers.get(language) {
                if state.status == LspServerStatus::Connected {
                    return Ok(());
                }
            }

            // Mark as starting
            inner.servers.insert(
                language.to_string(),
                LspServerState {
                    language: language.to_string(),
                    status: LspServerStatus::Starting,
                    root_path: Some(root_path.to_string_lossy().to_string()),
                    capabilities: Vec::new(),
                    diagnostics: Vec::new(),
                },
            );
        }

        // Spawn process
        let process = LspProcess::spawn(&config, root_path)
            .map_err(|e| format!("Failed to spawn LSP server: {e}"))?;

        let root_uri = format!("file://{}", root_path.to_string_lossy());

        // Run initialization in a blocking task
        let language = language.to_string();
        let inner = self.inner.clone();

        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            let result = rt.block_on(async {
                let mut proc = process;
                let capabilities = proc.initialize(Some(&root_uri)).await?;
                Ok::<_, io::Error>((proc, capabilities))
            });

            let mut inner = inner.lock().expect("lsp registry lock poisoned");

            match result {
                Ok((proc, capabilities)) => {
                    let caps_vec = Self::extract_capabilities(&capabilities);
                    inner.processes.insert(language.clone(), proc);
                    inner.servers.insert(
                        language.clone(),
                        LspServerState {
                            language: language.clone(),
                            status: LspServerStatus::Connected,
                            root_path: Some(root_uri),
                            capabilities: caps_vec,
                            diagnostics: Vec::new(),
                        },
                    );
                }
                Err(e) => {
                    inner.servers.insert(
                        language.clone(),
                        LspServerState {
                            language: language.clone(),
                            status: LspServerStatus::Error,
                            root_path: None,
                            capabilities: Vec::new(),
                            diagnostics: Vec::new(),
                        },
                    );
                    return Err(format!("LSP initialization failed: {e}"));
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    fn extract_capabilities(caps: &LspServerCapabilities) -> Vec<String> {
        let mut result = Vec::new();
        if caps.definition_provider.unwrap_or(false) {
            result.push("definition".to_string());
        }
        if caps.references_provider.unwrap_or(false) {
            result.push("references".to_string());
        }
        if caps.hover_provider.unwrap_or(false) {
            result.push("hover".to_string());
        }
        if caps.completion_provider.is_some() {
            result.push("completion".to_string());
        }
        if caps.document_symbol_provider.unwrap_or(false) {
            result.push("documentSymbol".to_string());
        }
        result
    }

    pub fn register(
        &self,
        language: &str,
        status: LspServerStatus,
        root_path: Option<&str>,
        capabilities: Vec<String>,
    ) {
        let mut inner = self.inner.lock().expect("lsp registry lock poisoned");
        inner.servers.insert(
            language.to_owned(),
            LspServerState {
                language: language.to_owned(),
                status,
                root_path: root_path.map(str::to_owned),
                capabilities,
                diagnostics: Vec::new(),
            },
        );
    }

    pub fn get(&self, language: &str) -> Option<LspServerState> {
        let inner = self.inner.lock().expect("lsp registry lock poisoned");
        inner.servers.get(language).cloned()
    }

    /// Find the appropriate server for a file path based on extension.
    pub fn find_server_for_path(&self, path: &str) -> Option<LspServerState> {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let language = match ext {
            "rs" => "rust",
            "ts" | "tsx" => "typescript",
            "js" | "jsx" => "javascript",
            "py" => "python",
            "go" => "go",
            "java" => "java",
            "c" | "h" => "c",
            "cpp" | "hpp" | "cc" => "cpp",
            "rb" => "ruby",
            "lua" => "lua",
            _ => return None,
        };

        self.get(language)
    }

    /// List all registered servers.
    pub fn list_servers(&self) -> Vec<LspServerState> {
        let inner = self.inner.lock().expect("lsp registry lock poisoned");
        inner.servers.values().cloned().collect()
    }

    /// Add diagnostics to a server.
    pub fn add_diagnostics(
        &self,
        language: &str,
        diagnostics: Vec<LspDiagnostic>,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().expect("lsp registry lock poisoned");
        let server = inner
            .servers
            .get_mut(language)
            .ok_or_else(|| format!("LSP server not found for language: {language}"))?;
        server.diagnostics.extend(diagnostics);
        Ok(())
    }

    /// Get diagnostics for a specific file path.
    pub fn get_diagnostics(&self, path: &str) -> Vec<LspDiagnostic> {
        let inner = self.inner.lock().expect("lsp registry lock poisoned");
        inner
            .servers
            .values()
            .flat_map(|s| &s.diagnostics)
            .filter(|d| d.path == path)
            .cloned()
            .collect()
    }

    /// Clear diagnostics for a language server.
    pub fn clear_diagnostics(&self, language: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().expect("lsp registry lock poisoned");
        let server = inner
            .servers
            .get_mut(language)
            .ok_or_else(|| format!("LSP server not found for language: {language}"))?;
        server.diagnostics.clear();
        Ok(())
    }

    /// Disconnect a server.
    pub fn disconnect(&self, language: &str) -> Option<LspServerState> {
        let mut inner = self.inner.lock().expect("lsp registry lock poisoned");
        inner.processes.remove(language);
        inner.servers.remove(language)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        let inner = self.inner.lock().expect("lsp registry lock poisoned");
        inner.servers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Execute an LSP action and return the result.
    #[allow(clippy::await_holding_lock, clippy::too_many_lines)]
    pub async fn execute_action(
        &self,
        action: LspAction,
        path: &str,
        line: Option<u32>,
        character: Option<u32>,
    ) -> Result<JsonValue, String> {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let language = match ext {
            "rs" => "rust",
            "ts" | "tsx" => "typescript",
            "js" | "jsx" => "javascript",
            "py" => "python",
            "go" => "go",
            "java" => "java",
            "c" | "h" => "c",
            "cpp" | "hpp" | "cc" => "cpp",
            "rb" => "ruby",
            "lua" => "lua",
            _ => return Err(format!("No LSP server for file extension: {ext}")),
        };

        let mut inner = self.inner.lock().expect("lsp registry lock poisoned");

        let process = inner
            .processes
            .get_mut(language)
            .ok_or_else(|| format!("LSP server not started for language: {language}"))?;

        // Convert path to URI
        let abs_path = std::fs::canonicalize(path)
            .map_err(|e| format!("Failed to resolve path {path}: {e}"))?;
        let uri = format!("file://{}", abs_path.to_string_lossy());

        let result = match action {
            LspAction::Diagnostics => {
                // Diagnostics are typically pushed from the server
                // Return cached diagnostics for this file
                let diags = inner
                    .servers
                    .get(language)
                    .map(|s| s.diagnostics.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|d| d.path == path)
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "action": "diagnostics",
                    "path": path,
                    "diagnostics": diags
                })
            }
            LspAction::Definition => {
                let line = line.ok_or("line is required for definition")?;
                let character = character.ok_or("character is required for definition")?;

                let locations = process
                    .goto_definition(&uri, line, character)
                    .await
                    .map_err(|e| e.to_string())?;

                let results: Vec<_> = locations
                    .iter()
                    .map(|loc| {
                        let path = loc.uri.strip_prefix("file://").unwrap_or(&loc.uri);
                        LspLocation {
                            path: path.to_string(),
                            line: loc.range.start.line,
                            character: loc.range.start.character,
                            end_line: Some(loc.range.end.line),
                            end_character: Some(loc.range.end.character),
                            preview: None,
                        }
                    })
                    .collect();

                serde_json::json!({
                    "action": "definition",
                    "path": path,
                    "line": line,
                    "character": character,
                    "locations": results
                })
            }
            LspAction::References => {
                let line = line.ok_or("line is required for references")?;
                let character = character.ok_or("character is required for references")?;

                let locations = process
                    .find_references(&uri, line, character)
                    .await
                    .map_err(|e| e.to_string())?;

                let results: Vec<_> = locations
                    .iter()
                    .map(|loc| {
                        let path = loc.uri.strip_prefix("file://").unwrap_or(&loc.uri);
                        LspLocation {
                            path: path.to_string(),
                            line: loc.range.start.line,
                            character: loc.range.start.character,
                            end_line: Some(loc.range.end.line),
                            end_character: Some(loc.range.end.character),
                            preview: None,
                        }
                    })
                    .collect();

                serde_json::json!({
                    "action": "references",
                    "path": path,
                    "line": line,
                    "character": character,
                    "locations": results,
                    "count": results.len()
                })
            }
            LspAction::Hover => {
                let line = line.ok_or("line is required for hover")?;
                let character = character.ok_or("character is required for hover")?;

                let hover_result = process
                    .hover(&uri, line, character)
                    .await
                    .map_err(|e| e.to_string())?;

                if let Some(hover) = hover_result {
                    let content = match hover.contents {
                        LspHoverContents::String(s) => s,
                        LspHoverContents::MarkupContent(m) => {
                            if m.kind == "markdown" {
                                format!("```{}\n{}\n```", m.kind, m.value)
                            } else {
                                m.value
                            }
                        }
                        LspHoverContents::Array(arr) => arr
                            .iter()
                            .map(|m| format!("```{}\n{}\n```", m.language, m.value))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                    };

                    serde_json::json!({
                        "action": "hover",
                        "path": path,
                        "line": line,
                        "character": character,
                        "result": LspHoverResult {
                            content,
                            language: None,
                        }
                    })
                } else {
                    serde_json::json!({
                        "action": "hover",
                        "path": path,
                        "line": line,
                        "character": character,
                        "result": null
                    })
                }
            }
            LspAction::Completion => {
                let line = line.ok_or("line is required for completion")?;
                let character = character.ok_or("character is required for completion")?;

                let items = process
                    .completion(&uri, line, character)
                    .await
                    .map_err(|e| e.to_string())?;

                let results: Vec<_> = items
                    .iter()
                    .map(|item| LspCompletionItem {
                        label: item.label.clone(),
                        kind: item.kind.map(completion_kind_to_string),
                        detail: item.detail.clone(),
                        insert_text: item.insert_text.clone(),
                    })
                    .collect();

                serde_json::json!({
                    "action": "completion",
                    "path": path,
                    "line": line,
                    "character": character,
                    "items": results,
                    "count": results.len()
                })
            }
            LspAction::Symbols => {
                let symbols = process
                    .document_symbols(&uri)
                    .await
                    .map_err(|e| e.to_string())?;

                let results: Vec<_> = symbols
                    .iter()
                    .map(|sym| {
                        let path = sym
                            .location
                            .uri
                            .strip_prefix("file://")
                            .unwrap_or(&sym.location.uri);
                        LspSymbol {
                            name: sym.name.clone(),
                            kind: symbol_kind_to_string(sym.kind),
                            path: path.to_string(),
                            line: sym.location.range.start.line,
                            character: sym.location.range.start.character,
                        }
                    })
                    .collect();

                serde_json::json!({
                    "action": "symbols",
                    "path": path,
                    "symbols": results,
                    "count": results.len()
                })
            }
            LspAction::Format => {
                // Formatting not implemented yet
                serde_json::json!({
                    "action": "format",
                    "path": path,
                    "error": "formatting not implemented"
                })
            }
        };

        Ok(result)
    }

    /// Dispatch an LSP action and return a structured result.
    /// This is a synchronous wrapper that uses the tokio runtime.
    pub fn dispatch(
        &self,
        action: &str,
        path: Option<&str>,
        line: Option<u32>,
        character: Option<u32>,
        _query: Option<&str>,
    ) -> Result<JsonValue, String> {
        let lsp_action =
            LspAction::from_str(action).ok_or_else(|| format!("unknown LSP action: {action}"))?;

        // For diagnostics, we can check existing cached diagnostics
        if lsp_action == LspAction::Diagnostics {
            if let Some(path) = path {
                let diags = self.get_diagnostics(path);
                return Ok(serde_json::json!({
                    "action": "diagnostics",
                    "path": path,
                    "diagnostics": diags,
                    "count": diags.len()
                }));
            }
            // All diagnostics across all servers
            let inner = self.inner.lock().expect("lsp registry lock poisoned");
            let all_diags: Vec<_> = inner
                .servers
                .values()
                .flat_map(|s| &s.diagnostics)
                .collect();
            return Ok(serde_json::json!({
                "action": "diagnostics",
                "diagnostics": all_diags,
                "count": all_diags.len()
            }));
        }

        // For other actions, we need a path
        let path = path.ok_or("path is required for this LSP action")?;

        // Check if we have a connected server for this file
        let server = self
            .find_server_for_path(path)
            .ok_or_else(|| format!("no LSP server available for path: {path}"))?;

        if server.status != LspServerStatus::Connected {
            return Err(format!(
                "LSP server for '{}' is not connected (status: {})",
                server.language, server.status
            ));
        }

        // Try to execute the action synchronously via tokio
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| "no tokio runtime available".to_string())?;

        rt.block_on(self.execute_action(lsp_action, path, line, character))
    }
}

fn completion_kind_to_string(kind: i32) -> String {
    match kind {
        1 => "text",
        2 => "method",
        3 => "function",
        4 => "constructor",
        5 => "field",
        6 => "variable",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        11 => "unit",
        12 => "value",
        13 => "enum",
        14 => "keyword",
        15 => "snippet",
        16 => "color",
        17 => "file",
        18 => "reference",
        19 => "folder",
        20 => "enumMember",
        21 => "constant",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "typeParameter",
        _ => "unknown",
    }
    .to_string()
}

fn symbol_kind_to_string(kind: i32) -> String {
    match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enumMember",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "typeParameter",
        _ => "unknown",
    }
    .to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_and_retrieves_server() {
        let registry = LspRegistry::new();
        registry.register(
            "rust",
            LspServerStatus::Connected,
            Some("/workspace"),
            vec!["hover".into(), "completion".into()],
        );

        let server = registry.get("rust").expect("should exist");
        assert_eq!(server.language, "rust");
        assert_eq!(server.status, LspServerStatus::Connected);
        assert_eq!(server.capabilities.len(), 2);
    }

    #[test]
    fn finds_server_by_file_extension() {
        let registry = LspRegistry::new();
        registry.register("rust", LspServerStatus::Connected, None, vec![]);
        registry.register("typescript", LspServerStatus::Connected, None, vec![]);

        let rs_server = registry.find_server_for_path("src/main.rs").unwrap();
        assert_eq!(rs_server.language, "rust");

        let ts_server = registry.find_server_for_path("src/index.ts").unwrap();
        assert_eq!(ts_server.language, "typescript");

        assert!(registry.find_server_for_path("data.csv").is_none());
    }

    #[test]
    fn manages_diagnostics() {
        let registry = LspRegistry::new();
        registry.register("rust", LspServerStatus::Connected, None, vec![]);

        registry
            .add_diagnostics(
                "rust",
                vec![LspDiagnostic {
                    path: "src/main.rs".into(),
                    line: 10,
                    character: 5,
                    severity: "error".into(),
                    message: "mismatched types".into(),
                    source: Some("rust-analyzer".into()),
                }],
            )
            .unwrap();

        let diags = registry.get_diagnostics("src/main.rs");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "mismatched types");

        registry.clear_diagnostics("rust").unwrap();
        assert!(registry.get_diagnostics("src/main.rs").is_empty());
    }

    #[test]
    fn dispatches_diagnostics_action() {
        let registry = LspRegistry::new();
        registry.register("rust", LspServerStatus::Connected, None, vec![]);
        registry
            .add_diagnostics(
                "rust",
                vec![LspDiagnostic {
                    path: "src/lib.rs".into(),
                    line: 1,
                    character: 0,
                    severity: "warning".into(),
                    message: "unused import".into(),
                    source: None,
                }],
            )
            .unwrap();

        let result = registry
            .dispatch("diagnostics", Some("src/lib.rs"), None, None, None)
            .unwrap();
        assert_eq!(result["count"], 1);
    }

    #[test]
    fn dispatches_hover_action_requires_connected() {
        let registry = LspRegistry::new();
        registry.register("rust", LspServerStatus::Connected, None, vec![]);

        // This will fail because we don't have a real process running
        let result = registry.dispatch("hover", Some("src/main.rs"), Some(10), Some(5), None);
        // Should fail because there's no actual LSP process
        assert!(result.is_err() || result.unwrap()["error"].is_string());
    }

    #[test]
    fn rejects_action_on_disconnected_server() {
        let registry = LspRegistry::new();
        registry.register("rust", LspServerStatus::Disconnected, None, vec![]);

        assert!(registry
            .dispatch("hover", Some("src/main.rs"), Some(1), Some(0), None)
            .is_err());
    }

    #[test]
    fn rejects_unknown_action() {
        let registry = LspRegistry::new();
        assert!(registry
            .dispatch("unknown_action", Some("file.rs"), None, None, None)
            .is_err());
    }

    #[test]
    fn disconnects_server() {
        let registry = LspRegistry::new();
        registry.register("rust", LspServerStatus::Connected, None, vec![]);
        assert_eq!(registry.len(), 1);

        let removed = registry.disconnect("rust");
        assert!(removed.is_some());
        assert!(registry.is_empty());
    }

    #[test]
    fn lsp_action_from_str_all_aliases() {
        let cases = [
            ("diagnostics", Some(LspAction::Diagnostics)),
            ("hover", Some(LspAction::Hover)),
            ("definition", Some(LspAction::Definition)),
            ("goto_definition", Some(LspAction::Definition)),
            ("references", Some(LspAction::References)),
            ("find_references", Some(LspAction::References)),
            ("completion", Some(LspAction::Completion)),
            ("completions", Some(LspAction::Completion)),
            ("symbols", Some(LspAction::Symbols)),
            ("document_symbols", Some(LspAction::Symbols)),
            ("format", Some(LspAction::Format)),
            ("formatting", Some(LspAction::Format)),
            ("unknown", None),
        ];

        for (input, expected) in cases {
            assert_eq!(
                LspAction::from_str(input),
                expected,
                "unexpected action resolution for {input}"
            );
        }
    }

    #[test]
    fn lsp_server_status_display_all_variants() {
        assert_eq!(LspServerStatus::Connected.to_string(), "connected");
        assert_eq!(LspServerStatus::Disconnected.to_string(), "disconnected");
        assert_eq!(LspServerStatus::Starting.to_string(), "starting");
        assert_eq!(LspServerStatus::Error.to_string(), "error");
    }

    #[test]
    fn default_lsp_config_for_languages() {
        assert!(default_lsp_config("rust").is_some());
        assert!(default_lsp_config("typescript").is_some());
        assert!(default_lsp_config("python").is_some());
        assert!(default_lsp_config("go").is_some());
        assert!(default_lsp_config("java").is_some());
        assert!(default_lsp_config("c").is_some());
        assert!(default_lsp_config("cpp").is_some());
        assert!(default_lsp_config("ruby").is_some());
        assert!(default_lsp_config("lua").is_some());
        assert!(default_lsp_config("unknown").is_none());
    }

    #[test]
    fn dispatch_diagnostics_without_path_aggregates() {
        let registry = LspRegistry::new();
        registry.register("rust", LspServerStatus::Connected, None, vec![]);
        registry.register("python", LspServerStatus::Connected, None, vec![]);
        registry
            .add_diagnostics(
                "rust",
                vec![LspDiagnostic {
                    path: "src/lib.rs".into(),
                    line: 1,
                    character: 0,
                    severity: "warning".into(),
                    message: "unused import".into(),
                    source: Some("rust-analyzer".into()),
                }],
            )
            .expect("rust diagnostics should add");
        registry
            .add_diagnostics(
                "python",
                vec![LspDiagnostic {
                    path: "script.py".into(),
                    line: 2,
                    character: 4,
                    severity: "error".into(),
                    message: "undefined name".into(),
                    source: Some("pyright".into()),
                }],
            )
            .expect("python diagnostics should add");

        let result = registry
            .dispatch("diagnostics", None, None, None, None)
            .expect("aggregate diagnostics should work");

        assert_eq!(result["action"], "diagnostics");
        assert_eq!(result["count"], 2);
    }

    #[test]
    fn dispatch_non_diagnostics_requires_path() {
        let registry = LspRegistry::new();

        let result = registry.dispatch("hover", None, Some(1), Some(0), None);

        assert_eq!(
            result.expect_err("path should be required"),
            "path is required for this LSP action"
        );
    }

    #[test]
    fn dispatch_no_server_for_path_errors() {
        let registry = LspRegistry::new();

        let result = registry.dispatch("hover", Some("notes.md"), Some(1), Some(0), None);

        let error = result.expect_err("missing server should fail");
        assert!(error.contains("no LSP server available for path: notes.md"));
    }

    #[test]
    fn dispatch_disconnected_server_error_payload() {
        let registry = LspRegistry::new();
        registry.register("typescript", LspServerStatus::Disconnected, None, vec![]);

        let result = registry.dispatch("hover", Some("src/index.ts"), Some(3), Some(2), None);

        let error = result.expect_err("disconnected server should fail");
        assert!(error.contains("typescript"));
        assert!(error.contains("disconnected"));
    }

    #[test]
    fn list_servers_with_multiple() {
        let registry = LspRegistry::new();
        registry.register("rust", LspServerStatus::Connected, None, vec![]);
        registry.register("typescript", LspServerStatus::Starting, None, vec![]);
        registry.register("python", LspServerStatus::Error, None, vec![]);

        let servers = registry.list_servers();

        assert_eq!(servers.len(), 3);
        assert!(servers.iter().any(|s| s.language == "rust"));
        assert!(servers.iter().any(|s| s.language == "typescript"));
        assert!(servers.iter().any(|s| s.language == "python"));
    }

    #[test]
    fn get_missing_server_returns_none() {
        let registry = LspRegistry::new();
        let server = registry.get("missing");
        assert!(server.is_none());
    }

    #[test]
    fn add_diagnostics_missing_language_errors() {
        let registry = LspRegistry::new();

        let result = registry.add_diagnostics("missing", vec![]);

        let error = result.expect_err("missing language should fail");
        assert!(error.contains("LSP server not found for language: missing"));
    }

    #[test]
    fn clear_diagnostics_missing_language_errors() {
        let registry = LspRegistry::new();

        let result = registry.clear_diagnostics("missing");

        let error = result.expect_err("missing language should fail");
        assert!(error.contains("LSP server not found for language: missing"));
    }
}
