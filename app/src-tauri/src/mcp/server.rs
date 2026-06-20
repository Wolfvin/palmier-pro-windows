//! MCP HTTP server.
//!
//! Port of `Sources/PalmierPro/Agent/MCP/MCPHTTPServer.swift` and
//! `MCPService.swift`. Built on `axum` + `tokio` instead of `Network.NWListener`,
//! but the wire contract is identical:
//!
//! - Binds to IPv4 loopback `127.0.0.1:19789` only — never reachable from the
//!   LAN. (Swift: `params.requiredLocalEndpoint = .hostPort(host: "127.0.0.1",
//!   port: endpointPort)`.)
//! - `GET /.well-known/oauth-protected-resource` → RFC 9728 metadata blob
//!   pointing at `http://127.0.0.1:<port>`.
//! - `GET /mcp` and `GET /` → SSE keep-alive stream, initial event
//!   `: connected\n\n` (Swift: `: connected\n\n`).
//! - `POST /mcp` and `POST /` → JSON-RPC 2.0 request/response (Streamable
//!   HTTP transport).
//! - Any other path → `404 Not Found`.
//!
//! The JSON-RPC dispatcher supports the methods the macOS build exposes:
//! `initialize`, `notifications/initialized` (acknowledged silently),
//! `tools/list`, `tools/call`, `resources/list`, `resources/read`. Unknown
//! methods return a `-32601 Method not found` error.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_core::Stream;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::error;

use crate::tlog_err;
use crate::tlog_info;

use super::instructions::SERVER_INSTRUCTIONS;
use super::protocol::{
    error_codes, CallToolResult, InitializeResult, JsonRpcErrorResponse, JsonRpcMessage,
    JsonRpcResponse, MCP_PROTOCOL_VERSION, RpcError, ServerCapabilities, ServerInfo,
    ToolsCapability, ResourcesCapability,
};
use super::resources;
use super::tools;

/// Default MCP server port. Matches `MCPService.port` in the Swift source.
pub const MCP_DEFAULT_PORT: u16 = 19789;

/// Server name advertised in `initialize` results. Matches the Swift
/// `Server(name: "palmier-pro", version: "1.0.0", ...)`.
pub const SERVER_NAME: &str = "palmier-pro";
pub const SERVER_VERSION: &str = "1.0.0";

/// Configuration for [`McpServer`]. Mirrors the role of `MCPService`'s
/// `Self.port` + `Self.isEnabledPreference`.
#[derive(Clone, Copy, Debug)]
pub struct McpServerConfig {
    /// TCP port to bind on `127.0.0.1`. Defaults to [`MCP_DEFAULT_PORT`].
    pub port: u16,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self { port: MCP_DEFAULT_PORT }
    }
}

/// Running MCP HTTP server. Holds the join handle of the accept loop and a
/// shutdown signal sender. Dropping this value stops the server.
pub struct McpServer {
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<tokio::task::JoinHandle<()>>,
    /// The address the server actually bound to. Useful for tests that pass
    /// `port = 0` to let the OS pick a free port.
    pub local_addr: SocketAddr,
}

impl McpServer {
    /// Starts the MCP HTTP server on `127.0.0.1:<config.port>`.
    ///
    /// Returns immediately; the server runs on a background tokio task. Call
    /// [`Self::shutdown`] or drop the value to stop it.
    pub async fn start(config: McpServerConfig) -> Result<Self, std::io::Error> {
        let addr: SocketAddr = ([127, 0, 0, 1], config.port).into();
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(err) => {
                tlog_err!("mcp", "failed to bind listener", err);
                return Err(err);
            }
        };
        let local_addr = listener.local_addr().map_err(|err| {
            tlog_err!("mcp", "failed to read local_addr", err);
            err
        })?;

        let app = build_router(local_addr.port());
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let join_handle = tokio::spawn(async move {
            // `axum::serve`'s graceful shutdown waits for in-flight requests
            // to finish before returning. We wire it to our oneshot so that
            // `shutdown()` cancels the accept loop cleanly.
            let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            if let Err(err) = serve.await {
                tlog_err!("mcp", "accept loop exited with error", err);
            }
        });

        tlog_info!(
            "mcp",
            "http server started",
            format!("listening on http://127.0.0.1:{}/mcp", local_addr.port())
        );

        Ok(Self {
            shutdown_tx: Some(shutdown_tx),
            join_handle: Some(join_handle),
            local_addr,
        })
    }

    /// Stops the server gracefully. Safe to call multiple times.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        // Best-effort shutdown if the caller forgot to await [`Self::shutdown`].
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Builds the axum router for the MCP server. `port` is the actual bound port
/// (which may differ from the configured one when `port = 0`); it's used to
/// build the RFC 9728 `resource` URL in the OAuth metadata endpoint.
fn build_router(port: u16) -> Router {
    Router::new()
        .route("/.well-known/oauth-protected-resource", get(oauth_protected_resource))
        .route("/mcp", get(mcp_get_sse).post(mcp_post_jsonrpc))
        .route("/", get(mcp_get_sse).post(mcp_post_jsonrpc))
        .with_state(port)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /.well-known/oauth-protected-resource` — RFC 9728 metadata blob.
/// Mirrors the Swift handler which emits
/// `{"resource":"http://127.0.0.1:<port>"}`.
async fn oauth_protected_resource(State(port): State<u16>) -> Response {
    let body = json!({ "resource": format!("http://127.0.0.1:{port}") });
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    (StatusCode::OK, headers, body.to_string()).into_response()
}

/// `GET /mcp` / `GET /` — Server-Sent Events keep-alive stream.
///
/// Mirrors the Swift behavior of emitting `: connected\n\n` immediately and
/// then holding the connection open. We use axum's `Sse` with a one-shot
/// initial event and `KeepAlive` to keep the connection warm.
async fn mcp_get_sse() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = async_stream::stream! {
        // Initial event, matching Swift's `: connected\n\n` preamble.
        yield Ok(Event::default().comment("connected"));
        // Hold the connection open indefinitely. The client (Claude / Cursor)
        // opens a POST in parallel for the actual JSON-RPC traffic; this GET
        // stream just proves the server is alive.
        //
        // We don't yield further events — `KeepAlive` below injects
        // `: keep-alive\n\n` comments periodically so intermediaries don't
        // close the connection.
        std::future::pending::<()>().await;
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// `POST /mcp` / `POST /` — JSON-RPC 2.0 request/response.
///
/// Parses the body as a single JSON-RPC message, dispatches it, and returns
/// the response as `application/json`. Notifications (messages without an
/// `id`) get a `202 Accepted` with no body, matching the Streamable HTTP
/// transport.
async fn mcp_post_jsonrpc(State(port): State<u16>, body: String) -> Response {
    let message: JsonRpcMessage = match serde_json::from_str(&body) {
        Ok(m) => m,
        Err(err) => {
            tlog_err!("mcp", "failed to parse JSON-RPC body", err);
            let err_resp = JsonRpcErrorResponse {
                jsonrpc: super::protocol::JSONRPC_VERSION,
                id: Value::Null,
                error: RpcError::new(error_codes::PARSE_ERROR, "Parse error"),
            };
            return json_response(StatusCode::OK, &err_resp);
        }
    };

    // Notifications (no `id`) are acknowledged with 202 + empty body.
    let request = match message {
        JsonRpcMessage::Notification(n) => {
            tlog_info!("mcp", "received notification", format!("method={}", n.method));
            return (StatusCode::ACCEPTED, "").into_response();
        }
        JsonRpcMessage::Request(r) => r,
    };

    let id = request.id.clone().unwrap_or(Value::Null);
    let result = dispatch_method(&request.method, &request.params, port).await;

    let response_body = match result {
        Ok(value) => serde_json::to_value(JsonRpcResponse {
            jsonrpc: super::protocol::JSONRPC_VERSION,
            id: id.clone(),
            result: value,
        })
        .unwrap_or_else(|_| {
            json!({"jsonrpc": super::protocol::JSONRPC_VERSION, "id": id, "error": {"code": error_codes::INTERNAL_ERROR, "message": "Failed to serialize response"}})
        }),
        Err(rpc_error) => serde_json::to_value(JsonRpcErrorResponse {
            jsonrpc: super::protocol::JSONRPC_VERSION,
            id: id.clone(),
            error: rpc_error,
        })
        .unwrap_or_else(|_| {
            json!({"jsonrpc": super::protocol::JSONRPC_VERSION, "id": id, "error": {"code": error_codes::INTERNAL_ERROR, "message": "Failed to serialize error"}})
        }),
    };
    json_response(StatusCode::OK, &response_body)
}

/// Dispatches a single JSON-RPC method to its handler.
///
/// `port` is currently unused at the dispatch layer but threaded through so
/// that future methods that need to emit absolute URLs (e.g. `logging/setLevel`
/// breadcrumbs) can use it without a refactor.
async fn dispatch_method(
    method: &str,
    params: &Value,
    _port: u16,
) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(serde_json::to_value(initialize_result()).map_err(|err| {
            error!(?err, "failed to serialize initialize result");
            RpcError::new(error_codes::INTERNAL_ERROR, "Internal error")
        })?),
        "initialized" | "notifications/initialized" => {
            // Acknowledged silently — the macOS build also swallows this.
            Ok(Value::Null)
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            let tools_list: Vec<_> = tools::all_tools()
                .into_iter()
                .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
                .collect();
            Ok(json!({ "tools": tools_list }))
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| RpcError::new(error_codes::INVALID_PARAMS, "Missing 'name' in tools/call params"))?;
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Object(Default::default()));
            let result: CallToolResult = tools::dispatch_call(name, &arguments).await;
            Ok(serde_json::to_value(result).map_err(|err| {
                error!(?err, "failed to serialize CallToolResult");
                RpcError::new(error_codes::INTERNAL_ERROR, "Internal error")
            })?)
        }
        "resources/list" => {
            let list = resources::list_resources();
            Ok(serde_json::to_value(list).map_err(|err| {
                error!(?err, "failed to serialize ListResourcesResult");
                RpcError::new(error_codes::INTERNAL_ERROR, "Internal error")
            })?)
        }
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| RpcError::new(error_codes::INVALID_PARAMS, "Missing 'uri' in resources/read params"))?;
            let result = resources::read_resource(uri);
            Ok(serde_json::to_value(result).map_err(|err| {
                error!(?err, "failed to serialize ReadResourceResult");
                RpcError::new(error_codes::INTERNAL_ERROR, "Internal error")
            })?)
        }
        other => {
            tlog_err!("mcp", "method not found", format!("method={other}"));
            Err(RpcError::new(error_codes::METHOD_NOT_FOUND, format!("Method not found: {other}")))
        }
    }
}

/// Builds the static `initialize` result returned to MCP clients.
fn initialize_result() -> InitializeResult {
    InitializeResult {
        protocol_version: MCP_PROTOCOL_VERSION,
        capabilities: ServerCapabilities {
            tools: ToolsCapability { list_changed: false },
            resources: ResourcesCapability {
                subscribe: false,
                list_changed: false,
            },
        },
        server_info: ServerInfo {
            name: SERVER_NAME.into(),
            version: SERVER_VERSION.into(),
        },
        instructions: SERVER_INSTRUCTIONS.to_string(),
    }
}

/// Helper that serializes a value to JSON and wraps it in a `200 OK` response
/// with `Content-Type: application/json`.
fn json_response<T: serde::Serialize>(status: StatusCode, body: &T) -> Response {
    let bytes = match serde_json::to_vec(body) {
        Ok(b) => b,
        Err(err) => {
            tlog_err!("mcp", "failed to serialize JSON response", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    (status, headers, bytes).into_response()
}
