//! MCP 配置解析
//!
//! 解析 JSON 配置文件，支持多种服务配置格式

use anyhow::{Result, bail};
use serde::Deserialize;
use std::collections::HashMap;

use super::args::ConvertArgs;

/// 解析后的配置源
#[derive(Debug, Clone)]
pub enum McpConfigSource {
    /// 直接 URL 模式（命令行参数）
    DirectUrl { url: String },
    /// 远程服务配置（JSON 配置）
    RemoteService {
        name: String,
        url: String,
        protocol: Option<crate::client::protocol::McpProtocol>,
        headers: HashMap<String, String>,
        timeout: Option<u64>,
    },
    /// 本地命令配置（JSON 配置）
    LocalCommand {
        name: String,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
}

/// MCP 配置格式
#[derive(Deserialize, Debug)]
struct McpConfig {
    #[serde(rename = "mcpServers")]
    mcp_servers: HashMap<String, McpServerInnerConfig>,
}

/// MCP 服务配置（支持 Command 和 Url 两种类型）
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum McpServerInnerConfig {
    Command(StdioConfig),
    Url(UrlConfig),
}

/// stdio 配置（本地命令）
#[derive(Deserialize, Debug, Clone)]
struct StdioConfig {
    command: String,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
}

/// URL 配置（远程服务）
#[derive(Deserialize, Debug, Clone)]
struct UrlConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        rename = "baseUrl",
        alias = "baseurl",
        alias = "base_url"
    )]
    base_url: Option<String>,
    #[serde(default, rename = "type", alias = "Type")]
    r#type: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, alias = "authToken", alias = "auth_token")]
    pub auth_token: Option<String>,
    pub timeout: Option<u64>,
}

impl UrlConfig {
    fn get_url(&self) -> Option<&str> {
        self.url.as_deref().or(self.base_url.as_deref())
    }
}

/// 解析 convert 命令的配置
pub fn parse_convert_config(args: &ConvertArgs) -> Result<McpConfigSource> {
    // 优先级：url > config > config_file
    if let Some(ref url) = args.url {
        return Ok(McpConfigSource::DirectUrl { url: url.clone() });
    }

    // 读取 JSON 配置
    let json_str = if let Some(ref config) = args.config {
        config.clone()
    } else if let Some(ref path) = args.config_file {
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("读取配置文件失败: {}", e))?
    } else {
        bail!("必须提供 URL、--config 或 --config-file 参数之一");
    };

    // 解析 JSON 配置
    let mcp_config: McpConfig = serde_json::from_str(&json_str).map_err(|e| {
        anyhow::anyhow!(
            "配置解析失败: {}。配置必须是标准 MCP 格式，包含 mcpServers 字段",
            e
        )
    })?;

    let servers = mcp_config.mcp_servers;

    if servers.is_empty() {
        bail!("配置中没有找到任何 MCP 服务");
    }

    // 选择服务
    let (name, inner_config) = if let Some(ref name) = args.name {
        // 用户指定了服务名称，必须严格匹配
        let config = servers.get(name).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "服务 '{}' 不存在。可用服务: {:?}",
                name,
                servers.keys().collect::<Vec<_>>()
            )
        })?;
        (name.clone(), config)
    } else if servers.len() == 1 {
        // 单服务且未指定名称，自动使用
        servers.into_iter().next().unwrap()
    } else {
        // 多服务且未指定名称
        bail!(
            "配置包含多个服务 {:?}，请使用 --name 指定要使用的服务",
            servers.keys().collect::<Vec<_>>()
        );
    };

    // 根据配置类型返回
    match inner_config {
        McpServerInnerConfig::Command(stdio) => Ok(McpConfigSource::LocalCommand {
            name,
            command: stdio.command,
            args: stdio.args.unwrap_or_default(),
            env: stdio.env.unwrap_or_default(),
        }),
        McpServerInnerConfig::Url(url_config) => {
            let url = url_config
                .get_url()
                .ok_or_else(|| anyhow::anyhow!("URL 配置缺少 url 或 baseUrl 字段"))?
                .to_string();

            // 解析协议类型
            let protocol = url_config.r#type.as_ref().and_then(|t| match t.as_str() {
                "sse" => Some(crate::client::protocol::McpProtocol::Sse),
                "http" | "stream" => Some(crate::client::protocol::McpProtocol::Stream),
                _ => None,
            });

            // 合并 headers：JSON 配置中的 auth_token -> Authorization
            let mut headers = url_config.headers.clone().unwrap_or_default();
            if let Some(auth_token) = &url_config.auth_token {
                headers.insert("Authorization".to_string(), auth_token.clone());
            }

            Ok(McpConfigSource::RemoteService {
                name,
                url,
                protocol,
                headers,
                timeout: url_config.timeout,
            })
        }
    }
}

/// 合并 headers：JSON 配置 + 命令行参数（命令行优先）
pub fn merge_headers(
    config_headers: HashMap<String, String>,
    cli_headers: &[(String, String)],
    cli_auth: Option<&String>,
) -> HashMap<String, String> {
    let mut merged = config_headers;

    // 命令行 -H 参数覆盖配置
    for (key, value) in cli_headers {
        merged.insert(key.clone(), value.clone());
    }

    // 命令行 --auth 参数优先级最高
    if let Some(auth_value) = cli_auth {
        merged.insert("Authorization".to_string(), auth_value.clone());
    }

    merged
}
