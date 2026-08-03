//! MCP 配置解析
//!
//! 解析 JSON 配置文件，支持多种服务配置格式

use anyhow::{Result, bail};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

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
        servers
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("配置中没有找到任何 MCP 服务"))?
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
            let protocol =
                url_config
                    .r#type
                    .as_ref()
                    .and_then(|t| match t.to_ascii_lowercase().as_str() {
                        "sse" => Some(crate::client::protocol::McpProtocol::Sse),
                        "http" | "stream" | "streamablehttp" | "streamable-http"
                        | "streamable_http" => Some(crate::client::protocol::McpProtocol::Stream),
                        _ => None,
                    });

            let headers = merge_config_headers_checked(
                url_config.headers.clone().unwrap_or_default(),
                url_config.auth_token.as_deref(),
            )?;

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
    let mut config_entries = config_headers.into_iter().collect::<Vec<_>>();
    config_entries.sort_by(|(left, _), (right, _)| {
        left.to_ascii_lowercase()
            .cmp(&right.to_ascii_lowercase())
            .then_with(|| left.cmp(right))
    });

    let mut merged = HashMap::new();
    for (key, value) in config_entries {
        insert_header_case_insensitive(&mut merged, key, value);
    }
    for (key, value) in cli_headers {
        insert_header_case_insensitive(&mut merged, key.clone(), value.clone());
    }
    if let Some(auth_value) = cli_auth {
        insert_header_case_insensitive(
            &mut merged,
            "Authorization".to_string(),
            auth_value.clone(),
        );
    }
    merged
}

pub(crate) fn merge_headers_checked(
    config_headers: HashMap<String, String>,
    cli_headers: &[(String, String)],
    cli_auth: Option<&String>,
) -> Result<HashMap<String, String>> {
    validate_unique_header_names(
        "configured headers",
        config_headers.keys().map(String::as_str),
    )?;
    validate_unique_header_names(
        "CLI headers",
        cli_headers.iter().map(|(name, _)| name.as_str()),
    )?;
    Ok(merge_headers(config_headers, cli_headers, cli_auth))
}

pub(crate) fn merge_config_headers_checked(
    config_headers: HashMap<String, String>,
    auth_token: Option<&str>,
) -> Result<HashMap<String, String>> {
    validate_unique_header_names(
        "configured headers",
        config_headers.keys().map(String::as_str),
    )?;
    let mut merged = merge_headers(config_headers, &[], None);
    if let Some(token) = auth_token {
        insert_header_case_insensitive(
            &mut merged,
            "Authorization".to_string(),
            normalize_authorization(token),
        );
    }
    Ok(merged)
}

fn validate_unique_header_names<'a>(
    source: &str,
    names: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let mut seen = HashSet::new();
    for name in names {
        let normalized = name.to_ascii_lowercase();
        if !seen.insert(normalized) {
            bail!("{source} contains duplicate header name ignoring ASCII case: {name}");
        }
    }
    Ok(())
}

fn insert_header_case_insensitive(
    headers: &mut HashMap<String, String>,
    key: String,
    value: String,
) {
    if let Some(existing) = headers
        .keys()
        .find(|existing| existing.eq_ignore_ascii_case(&key))
        .cloned()
    {
        headers.remove(&existing);
    }
    headers.insert(key, value);
}

/// 规范化 Authorization header 值，确保 Bearer token 带 `"Bearer "` 前缀。
///
/// `auth_token` 配置项通常只写裸 token（如 `"mytoken"`），而 MCP 鉴权服务一般要求
/// `Authorization: Bearer <token>`。统一在此补前缀，使「协议探测」与「实际连接」使用
/// 完全一致的 Authorization 值，避免探测因缺前缀收到 401/403 而误判协议。
///
/// 已带 `"Bearer "` 前缀的值原样返回。
///
/// 注意：CLI `--auth` 不走此函数 —— 它是用户提供的完整 header 值（如 `"Bearer xxx"`、
/// `"Basic xxx"`），由调用方原样使用。
pub fn normalize_authorization(value: &str) -> String {
    if value.starts_with("Bearer ") {
        value.to_string()
    } else {
        format!("Bearer {}", value)
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    #[test]
    fn auth_token_is_the_only_source_that_adds_bearer() {
        let mut configured = HashMap::new();
        configured.insert("X-Test".to_string(), "configured".to_string());

        let merged =
            merge_config_headers_checked(configured, Some("secret")).expect("valid headers");

        assert_eq!(
            merged.get("Authorization").map(String::as_str),
            Some("Bearer secret")
        );
    }

    #[test]
    fn complete_authorization_values_are_preserved_and_override_case_insensitively() {
        let mut configured = HashMap::new();
        configured.insert("authorization".to_string(), "Bearer old".to_string());
        let cli_headers = vec![(
            "AUTHORIZATION".to_string(),
            "ApiKey from-header".to_string(),
        )];
        let cli_auth = "Basic final".to_string();

        let merged = merge_headers_checked(configured, &cli_headers, Some(&cli_auth))
            .expect("valid headers");

        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged.get("Authorization").map(String::as_str),
            Some("Basic final")
        );
    }

    #[test]
    fn duplicate_header_names_in_one_source_fail_fast() {
        let cli_headers = vec![
            ("Authorization".to_string(), "Bearer one".to_string()),
            ("authorization".to_string(), "Bearer two".to_string()),
        ];

        let error = merge_headers_checked(HashMap::new(), &cli_headers, None)
            .expect_err("case-insensitive duplicates must fail");

        assert!(error.to_string().contains("duplicate header name"));
    }
}
