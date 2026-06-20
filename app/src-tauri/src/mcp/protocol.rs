//! JSON-RPC 2.0 envelope types + MCP-specific result shapes.
//!
//! These are intentionally minimal — only what the Streamable HTTP transport
//! needs to round-trip `initialize`, `tools/list`, `tools/call`,
//! `resources/list`, `resources/read`. Notifications (`notifications/*`) are
//! accepted and ignored, matching the Swift behavior (it delegates to the MCP
//! SDK which also ignores unknown notifications on the server side).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC protocol version string. MCP currently pins to `"2.0"`.
pub const JSONRPC_VERSION: &str = "2.0";

/// MCP protocol version advertised by the server in `initialize` results.
/// Matches the version the macOS Swift SDK advertises so existing
/// Claude / Codex / Cursor clients keep working without negotiation changes.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Generic JSON-RPC request envelope. `params` is kept as a raw `Value` so we
/// can dispatch per-method without a one-shot deserialize.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// JSON-RPC notification — same shape as [`JsonRpcRequest`] but `id` is
/// always `None` and we never send a response.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// A request or notification — what we deserialize from the wire.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
}

impl JsonRpcMessage {
    /// Returns the method name regardless of variant.
    pub fn method(&self) -> &str {
        match self {
            JsonRpcMessage::Request(r) => &r.method,
            JsonRpcMessage::Notification(n) => &n.method,
        }
    }
}

/// A successful JSON-RPC response.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub result: Value,
}

/// A JSON-RPC error response.
#[derive(Debug, Serialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub error: RpcError,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Standard JSON-RPC error codes.
///
/// See <https://www.jsonrpc.org/specification#error_object> for the canonical
/// definitions; MCP adds a few domain-specific ones which we mirror here.
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// `initialize` result returned by the server.
///
/// Field names use the snake_case wire format expected by the MCP spec (the
/// Swift MCP SDK also emits snake_case here).
#[derive(Debug, Serialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: &'static str,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
    pub instructions: String,
}

#[derive(Debug, Serialize)]
pub struct ServerCapabilities {
    pub tools: ToolsCapability,
    pub resources: ResourcesCapability,
}

#[derive(Debug, Serialize)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

#[derive(Debug, Serialize)]
pub struct ResourcesCapability {
    pub subscribe: bool,
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// `tools/list` result.
#[derive(Debug, Serialize)]
pub struct ListToolsResult {
    pub tools: Vec<ToolDescriptor>,
}

#[derive(Debug, Serialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// `tools/call` result. `content` is a list of typed blocks; `is_error`
/// distinguishes tool-level errors from protocol errors.
#[derive(Debug, Serialize)]
pub struct CallToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl CallToolResult {
    pub fn text(message: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(message)],
            is_error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(message)],
            is_error: Some(true),
        }
    }
}

/// A single content block inside a `tools/call` result. Matches the MCP spec
/// — `text`, `image`, `audio`, `resource`. We only emit `text` for now; the
/// image variant exists so that future ports of `inspect_media` (which
/// returns sampled frames) can reuse the type without a breaking change.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text { text: String },
    Image { data: String, #[serde(rename = "mimeType")] mime_type: String },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

/// `resources/list` result.
#[derive(Debug, Serialize)]
pub struct ListResourcesResult {
    pub resources: Vec<ResourceDescriptor>,
}

#[derive(Debug, Serialize)]
pub struct ResourceDescriptor {
    pub uri: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

/// `resources/read` result.
#[derive(Debug, Serialize)]
pub struct ReadResourceResult {
    pub contents: Vec<ResourceContent>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename = "resource")]
pub struct ResourceContent {
    pub uri: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub text: String,
}
