use anyhow::{Context, Result, bail};
use rmcp::model::{ListToolsResult, ProtocolVersion, ServerInfo};

#[derive(Clone, Debug)]
pub struct DiscoverySnapshot {
    pub server_info: ServerInfo,
    pub tools: ListToolsResult,
}

#[derive(Clone, Debug)]
pub struct FallbackMetadata {
    snapshot: DiscoverySnapshot,
}

impl FallbackMetadata {
    pub fn from_json(initialize: &str, tools: &str) -> Result<Self> {
        let server_info: ServerInfo = serde_json::from_str(initialize)
            .context("invalid Streamable HTTP initialize fallback JSON")?;
        validate_protocol_version(&server_info.protocol_version)?;
        if server_info.capabilities.tools.is_none() {
            bail!("Streamable HTTP initialize fallback must declare capabilities.tools");
        }

        let tools: ListToolsResult =
            serde_json::from_str(tools).context("invalid Streamable HTTP tools fallback JSON")?;
        if tools.next_cursor.is_some() {
            bail!("Streamable HTTP tools fallback must be a complete result without nextCursor");
        }

        Ok(Self {
            snapshot: DiscoverySnapshot { server_info, tools },
        })
    }

    pub fn snapshot(&self) -> DiscoverySnapshot {
        self.snapshot.clone()
    }

    pub fn into_snapshot(self) -> DiscoverySnapshot {
        self.snapshot
    }
}

impl DiscoverySnapshot {
    pub fn initialize_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.server_info)
            .context("failed to serialize Streamable HTTP initialize snapshot")
    }

    pub fn tools_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.tools)
            .context("failed to serialize Streamable HTTP tools snapshot")
    }
}

fn validate_protocol_version(version: &ProtocolVersion) -> Result<()> {
    if ProtocolVersion::KNOWN_VERSIONS.contains(version) {
        return Ok(());
    }
    bail!("unsupported Streamable HTTP MCP protocol version: {version}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INITIALIZE: &str = r#"{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1"}}"#;
    const TOOLS: &str = r#"{"tools":[{"name":"search","inputSchema":{"type":"object"}}]}"#;

    #[test]
    fn parses_complete_fallback() {
        let fallback = FallbackMetadata::from_json(INITIALIZE, TOOLS)
            .expect("valid Streamable HTTP fallback metadata");
        assert_eq!(fallback.snapshot().tools.tools.len(), 1);
    }

    #[test]
    fn rejects_initialize_without_tools_capability() {
        let initialize = r#"{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"test","version":"1"}}"#;
        let error = FallbackMetadata::from_json(initialize, TOOLS)
            .expect_err("missing tools capability must fail");
        assert!(error.to_string().contains("capabilities.tools"));
    }
}
