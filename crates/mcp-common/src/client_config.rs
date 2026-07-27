//! Client connection configuration for MCP services
//!
//! This module provides a unified configuration structure for connecting
//! to MCP servers via SSE or Streamable HTTP protocols.

use std::collections::HashMap;
use std::time::Duration;

/// Configuration for MCP client connections
///
/// This struct provides a protocol-agnostic way to configure connections
/// to MCP servers. It can be used with both SSE and Streamable HTTP transports.
///
/// # Example
///
/// ```rust
/// use mcp_common::McpClientConfig;
/// use std::time::Duration;
///
/// let config = McpClientConfig::new("http://localhost:8080/mcp")
///     .with_header("Authorization", "Bearer token123")
///     .with_connect_timeout(Duration::from_secs(30));
/// ```
#[derive(Clone, Debug, Default)]
pub struct McpClientConfig {
    /// Target URL for the MCP server
    pub url: String,
    /// HTTP headers to include in requests
    pub headers: HashMap<String, String>,
    /// Connection timeout duration
    pub connect_timeout: Option<Duration>,
    /// Read timeout duration
    pub read_timeout: Option<Duration>,
}

impl McpClientConfig {
    /// Create a new configuration with the given URL
    ///
    /// # Arguments
    /// * `url` - The MCP server URL (e.g., "http://localhost:8080/mcp")
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: HashMap::new(),
            connect_timeout: None,
            read_timeout: None,
        }
    }

    /// Add a header to the configuration
    ///
    /// # Arguments
    /// * `key` - Header name
    /// * `value` - Header value
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Add multiple headers from a HashMap
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers.extend(headers);
        self
    }

    /// Set the Authorization header with a Bearer token
    ///
    /// # Arguments
    /// * `token` - The bearer token (without "Bearer " prefix)
    pub fn with_bearer_auth(self, token: impl Into<String>) -> Self {
        self.with_header("Authorization", format!("Bearer {}", token.into()))
    }

    /// Set the connection timeout
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for connection establishment
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Set the read timeout
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for response data
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = Some(timeout);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_config() {
        let config = McpClientConfig::new("http://localhost:8080");
        assert_eq!(config.url, "http://localhost:8080");
        assert!(config.headers.is_empty());
    }

    #[test]
    fn test_with_header() {
        let config = McpClientConfig::new("http://localhost:8080").with_header("X-Custom", "value");
        assert_eq!(config.headers.get("X-Custom"), Some(&"value".to_string()));
    }

    #[test]
    fn test_with_bearer_auth() {
        let config = McpClientConfig::new("http://localhost:8080").with_bearer_auth("mytoken");
        assert_eq!(
            config.headers.get("Authorization"),
            Some(&"Bearer mytoken".to_string())
        );
    }

    #[test]
    fn test_builder_chain() {
        let config = McpClientConfig::new("http://localhost:8080")
            .with_header("X-Api-Key", "key123")
            .with_connect_timeout(Duration::from_secs(30))
            .with_read_timeout(Duration::from_secs(60));

        assert_eq!(config.url, "http://localhost:8080");
        assert_eq!(config.headers.get("X-Api-Key"), Some(&"key123".to_string()));
        assert_eq!(config.connect_timeout, Some(Duration::from_secs(30)));
        assert_eq!(config.read_timeout, Some(Duration::from_secs(60)));
    }
}
