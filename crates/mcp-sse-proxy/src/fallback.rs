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
        let server_info: ServerInfo =
            serde_json::from_str(initialize).context("invalid SSE initialize fallback JSON")?;
        validate_protocol_version(&server_info.protocol_version)?;
        if server_info.capabilities.tools.is_none() {
            bail!("SSE initialize fallback must declare capabilities.tools");
        }

        let tools: ListToolsResult =
            serde_json::from_str(tools).context("invalid SSE tools fallback JSON")?;
        if tools.next_cursor.is_some() {
            bail!("SSE tools fallback must be a complete result without nextCursor");
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
            .context("failed to serialize SSE initialize snapshot")
    }

    pub fn tools_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.tools).context("failed to serialize SSE tools snapshot")
    }
}

fn validate_protocol_version(version: &ProtocolVersion) -> Result<()> {
    if version == &ProtocolVersion::V_2024_11_05
        || version == &ProtocolVersion::V_2025_03_26
        || version == &ProtocolVersion::V_2025_06_18
    {
        return Ok(());
    }
    bail!("unsupported SSE MCP protocol version: {version}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INITIALIZE: &str = r#"{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1"}}"#;
    const TOOLS: &str = r#"{"tools":[{"name":"search","inputSchema":{"type":"object"}}]}"#;

    #[test]
    fn parses_complete_fallback() {
        let fallback =
            FallbackMetadata::from_json(INITIALIZE, TOOLS).expect("valid SSE fallback metadata");
        assert_eq!(fallback.snapshot().tools.tools.len(), 1);
    }

    #[test]
    fn rejects_incomplete_tools_snapshot() {
        let error = FallbackMetadata::from_json(INITIALIZE, r#"{"tools":[],"nextCursor":"next"}"#)
            .expect_err("cursor must be rejected");
        assert!(error.to_string().contains("complete result"));
    }
}
