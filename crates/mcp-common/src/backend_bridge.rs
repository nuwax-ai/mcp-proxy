//! Backend Bridge trait for cross-protocol MCP proxy
//!
//! This trait provides a protocol-agnostic interface for backend MCP connections,
//! enabling `mcp-sse-proxy` to communicate with any backend implementation
//! (e.g., `mcp-streamable-proxy`) without direct dependencies.
//!
//! All method parameters and return values use `serde_json::Value` to avoid
//! coupling to specific rmcp version types.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

/// Protocol-agnostic backend bridge for MCP proxy
///
/// Implementations of this trait wrap a concrete MCP client (e.g., rmcp 1.4.0's
/// `ProxyHandler`) and expose its functionality through a version-independent
/// JSON-based interface.
///
/// # Design
///
/// This trait uses `Pin<Box<dyn Future>>` for async methods instead of `async_trait`
/// to maintain object safety (`dyn BackendBridge`) without additional macro dependencies.
pub trait BackendBridge: Send + Sync {
    /// Get the MCP service identifier (used for logging and tracking)
    fn mcp_id(&self) -> &str;

    /// Get the backend's ServerInfo as JSON
    ///
    /// Used for cross-rmcp-version bridging: the implementation serializes its
    /// native `ServerInfo` to JSON, and the consumer deserializes it into its
    /// own rmcp version's `ServerInfo` type.
    fn get_server_info_json(&self) -> Value;

    /// Check if the backend connection is available (fast, synchronous check)
    fn is_backend_available(&self) -> bool;

    /// Check if the MCP server is ready (async, sends a validation request)
    fn is_mcp_server_ready(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;

    /// Check if the backend connection is terminated (async)
    fn is_terminated_async(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;

    /// Call an MCP method on the backend via JSON bridge
    ///
    /// # Arguments
    /// * `method` - MCP method name (e.g., "tools/list", "tools/call",
    ///   "resources/list", "prompts/get", "completion/complete")
    /// * `params` - JSON-serialized method parameters
    ///
    /// # Returns
    /// * `Ok(Value)` - JSON-serialized result from the backend
    /// * `Err(String)` - Error description
    fn call_peer_method(
        &self,
        method: &str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send + '_>>;
}
