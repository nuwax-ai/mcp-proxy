//! Configuration types for Streamable HTTP proxy

use serde::{Deserialize, Serialize};

/// Streamable HTTP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamableConfig {
    /// Bind address for the HTTP server
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    /// Enable stateful mode (session management)
    #[serde(default = "default_stateful_mode")]
    pub stateful_mode: bool,

    /// Quiet mode (suppress startup messages)
    #[serde(default)]
    pub quiet: bool,
}

impl Default for StreamableConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_bind_addr(),
            stateful_mode: default_stateful_mode(),
            quiet: false,
        }
    }
}

fn default_bind_addr() -> String {
    "127.0.0.1:3000".to_string()
}

fn default_stateful_mode() -> bool {
    true // Enable stateful mode by default for this module
}
