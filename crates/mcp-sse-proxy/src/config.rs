//! Configuration types for SSE proxy

use serde::{Deserialize, Serialize};

/// SSE server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseConfig {
    /// Bind address for the HTTP server
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    /// Quiet mode (suppress startup messages)
    #[serde(default)]
    pub quiet: bool,
}

impl Default for SseConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_bind_addr(),
            quiet: false,
        }
    }
}

fn default_bind_addr() -> String {
    "127.0.0.1:3001".to_string()
}
