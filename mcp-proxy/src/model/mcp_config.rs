use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{
    McpProtocol,
    mcp_router_model::{McpJsonServerParameters, McpServerConfig},
};

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
    //mcp协议（后端协议，用于连接到远程MCP服务）
    #[serde(default = "default_mcp_protocol", rename = "mcpProtocol")]
    pub mcp_protocol: McpProtocol,
    //客户端协议（用于暴露给客户端的API接口类型）
    //如果不指定，则与 mcp_protocol 相同
    #[serde(default = "default_mcp_protocol", rename = "clientProtocol")]
    pub client_protocol: McpProtocol,
    // 解析后的服务器配置（可选）
    #[serde(skip_serializing, skip_deserializing)]
    pub server_config: Option<McpServerConfig>,
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
            client_protocol: mcp_protocol.clone(),
            mcp_protocol,
            server_config: None,
        }
    }

    pub fn new_with_protocols(
        mcp_id: String,
        mcp_json_config: Option<String>,
        mcp_type: McpType,
        client_protocol: McpProtocol,
        backend_protocol: McpProtocol,
    ) -> Self {
        Self {
            mcp_id,
            mcp_json_config,
            mcp_type,
            client_protocol,
            mcp_protocol: backend_protocol,
            server_config: None,
        }
    }

    pub fn from_json(json: &str) -> Result<Self> {
        let config: McpConfig = serde_json::from_str(json)?;
        Ok(config)
    }

    /// 从 JSON 字符串创建并解析服务器配置
    pub fn from_json_with_server(
        mcp_id: String,
        mcp_json_config: String,
        mcp_type: McpType,
        mcp_protocol: McpProtocol,
    ) -> Result<Self> {
        let mcp_json_server_parameters =
            crate::model::McpJsonServerParameters::from(mcp_json_config.clone());
        let server_config = mcp_json_server_parameters
            .try_get_first_mcp_server()
            .context("Failed to parse MCP server config")?;

        Ok(Self {
            mcp_id,
            mcp_json_config: Some(mcp_json_config),
            mcp_type,
            client_protocol: mcp_protocol.clone(),
            mcp_protocol,
            server_config: Some(server_config),
        })
    }
}
