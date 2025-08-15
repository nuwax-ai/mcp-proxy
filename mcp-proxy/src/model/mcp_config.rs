use std::str::FromStr;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::McpProtocol;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    //mcp_id
    #[serde(rename = "mcpId")]
    pub mcp_id: String,
    //mcp_json_config,可能没有
    #[serde(rename = "mcpJsonConfig")]
    pub mcp_json_config: Option<String>,
    //mcp类型，默认为持续运行
    #[serde(default = "default_mcp_type", rename = "mcpType")]
    pub mcp_type: McpType,
    //mcp协议
    #[serde(default = "default_mcp_protocol", rename = "mcpProtocol")]
    pub mcp_protocol: McpProtocol,
}

fn default_mcp_protocol() -> McpProtocol {
    McpProtocol::Sse
}

fn default_mcp_type() -> McpType {
    McpType::OneShot
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum McpType {
    // 持续运行
    Persistent,
    // 一次性任务
    #[default]
    OneShot,
}

impl FromStr for McpType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "persistent" => Ok(McpType::Persistent),
            "oneShot" => Ok(McpType::OneShot),
            _ => Err(anyhow::anyhow!("无效的 MCP 类型: {}", s)),
        }
    }
}

impl McpConfig {
    pub fn new(
        mcp_id: String,
        mcp_json_config: Option<String>,
        mcp_type: McpType,
        mcp_protocol: McpProtocol,
    ) -> Self {
        Self {
            mcp_id,
            mcp_json_config,
            mcp_type,
            mcp_protocol,
        }
    }

    pub fn from_json(json: &str) -> Result<Self> {
        let config: McpConfig = serde_json::from_str(json)?;
        Ok(config)
    }
}
