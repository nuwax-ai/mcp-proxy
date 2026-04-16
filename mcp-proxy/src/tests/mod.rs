#[cfg(test)]
pub mod test_utils {
    // Tests utility module for common test setup
}

// Coze MCP integration tests - Streamable HTTP to SSE conversion
#[cfg(test)]
pub mod coze_mcp_test;

// Protocol detection tests - SSE vs Streamable HTTP
#[cfg(test)]
pub mod protocol_detection_test;

// Streamable HTTP configuration parsing and protocol detection tests
#[cfg(test)]
pub mod streamable_http_test;
