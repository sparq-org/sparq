//! Minimal JSON-RPC 2.0 framing for the MCP wire protocol.
//!
//! [OPUS-4.8] (sq-0z43i, gh #909) MCP messages are JSON-RPC 2.0 objects. We model
//! only the subset MCP uses — a request/notification (`method` + optional `params` +
//! optional `id`) and a response (`result` XOR `error`) — over `serde_json::Value`,
//! so no heavy dependency or codegen is needed. The transport (stdio, in `transport`)
//! is line-delimited: one JSON value per line — an object, or, since the MCP 2025-03-26
//! revision requires receiving them, a top-level batch array. [OPUS-5] gh #2497
//!
//! Why hand-rolled rather than the official `rmcp` SDK: assessed 2026-07-28 in
//! `research/mcp-rmcp-sdk-adoption-assessment.md` (sq-95zda, gh #3219). Verdict was to
//! keep this module — `rmcp` would replace it and `transport` (146 lines together) at the
//! cost of an unconditional async runtime, and neither the vendoring-policy nor the
//! cargo-vet precondition held at the time. That record also lists the triggers that
//! should reopen the question; an HTTP/SSE transport is the strongest one. [SONNET-4.6]
//! That trigger fired and was re-assessed 2026-07-29 in
//! `research/mcp-streamable-http-transport-design.md` (sq-2c0f0, gh #3221): the verdict
//! held — a Streamable HTTP transport on the axum stack already in the workspace lock adds
//! no new crate to audit, where `rmcp` would. [OPUS-5]

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON-RPC 2.0 request or notification. A *notification* is a request with no
/// `id` (the client does not expect a response); MCP uses these for lifecycle
/// signals such as `notifications/initialized`.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Must be `"2.0"`. Tolerated-but-ignored if absent (some clients omit it on
    /// notifications); we never reject on it so a benign client is not broken.
    #[serde(default)]
    pub jsonrpc: String,
    /// The correlation id. Absent ⇒ this is a notification (no response is sent).
    /// JSON-RPC permits a string or a number; we keep the raw `Value` and echo it
    /// back verbatim, which is exactly what the spec requires.
    #[serde(default)]
    pub id: Option<Value>,
    /// The RPC method name (`initialize`, `tools/list`, `tools/call`, …).
    pub method: String,
    /// Method parameters; method-specific shape, parsed by the handler.
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// True when this is a notification (no `id`) — no response must be emitted.
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// A JSON-RPC 2.0 response. Exactly one of `result` / `error` is set; `id` echoes
/// the request's id (or JSON `null` for a parse error that could not be correlated).
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// A success response echoing `id` and carrying `result`.
    pub fn ok(id: Value, result: Value) -> Self {
        Response {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error response echoing `id` and carrying a JSON-RPC error object.
    pub fn err(id: Value, error: RpcError) -> Self {
        Response {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// Build an error with no `data` payload.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// The standard JSON-RPC error code for a malformed request object.
pub const INVALID_REQUEST: i64 = -32600;
/// The standard JSON-RPC error code for an unknown method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The standard JSON-RPC error code for invalid method parameters.
pub const INVALID_PARAMS: i64 = -32602;
/// The standard JSON-RPC error code for an internal server error.
pub const INTERNAL_ERROR: i64 = -32603;
/// MCP's implementation-defined error code for "the requested resource does not exist",
/// returned by `resources/read` and `resources/subscribe`. The pod server reuses it for
/// the existence-non-disclosure case too, so an unreadable resource is reported exactly
/// like a nonexistent one. [SONNET-4.6] sq-cmjmr
pub const RESOURCE_NOT_FOUND: i64 = -32002;
