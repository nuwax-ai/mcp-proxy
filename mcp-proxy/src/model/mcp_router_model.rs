use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

use log::{debug, error, info};
use serde::{Deserialize, Serialize};

use anyhow::Result;

use super::mcp_config::McpType;

// 统一定义 mcp服务的路由前缀, 分 sse 和 stream 两种;如果是mcp协议的透明代理,则是: /mcp/sse/proxy开头,或者 /mcp/stream/proxy开头
pub static GLOBAL_SSE_MCP_ROUTES_PREFIX: &str = "/mcp/sse";
pub static GLOBAL_STREAM_MCP_ROUTES_PREFIX: &str = "/mcp/stream";

#[derive(Deserialize, Debug)]
pub struct AddRouteParams {
    //mcp的json配置
    pub mcp_json_config: String,
    //mcp类型，默认为持续运行
    pub mcp_type: Option<McpType>,
}

/// Settings for the SSE server
pub struct SseServerSettings {
    pub bind_addr: SocketAddr,
    pub keep_alive: Option<Duration>,
}
//mcp的配置，支持命令行和URL两种方式
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum McpServerConfig {
    Command(McpServerCommandConfig),
    Url(McpServerUrlConfig),
}

//mcp的命令行配置
#[derive(Debug, Deserialize, Clone)]
pub struct McpServerCommandConfig {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
}

/// MCP URL 协议类型枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum McpUrlProtocolType {
    /// Stdio 协议（本地命令启动）
    #[serde(rename = "stdio")]
    Stdio,
    /// Server-Sent Events 协议
    #[serde(rename = "sse")]
    Sse,
    /// Streamable HTTP 协议（别名 http）
    #[serde(rename = "http")]
    Http,
    /// Streamable HTTP 协议（别名 stream）
    #[serde(rename = "stream")]
    Stream,
}

impl std::str::FromStr for McpUrlProtocolType {
    type Err = String;

    fn from_str(type_str: &str) -> Result<Self, Self::Err> {
        match type_str {
            "sse" => Ok(McpUrlProtocolType::Sse),
            "http" | "stream" => Ok(McpUrlProtocolType::Stream),
            _ => Err(format!("Unsupported protocol type: {}", type_str)),
        }
    }
}

impl McpUrlProtocolType {
    /// 判断是否为 Streamable HTTP 协议（包括 http 和 stream）
    pub fn is_streamable(&self) -> bool {
        matches!(self, McpUrlProtocolType::Http | McpUrlProtocolType::Stream)
    }

    /// 获取对应的 McpProtocol 枚举
    pub fn to_mcp_protocol(&self) -> super::McpProtocol {
        match self {
            McpUrlProtocolType::Stdio => super::McpProtocol::Stdio,
            McpUrlProtocolType::Sse => super::McpProtocol::Sse,
            McpUrlProtocolType::Http | McpUrlProtocolType::Stream => super::McpProtocol::Stream,
        }
    }
}

//mcp的URL配置（用于Streamable/SSE协议）
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct McpServerUrlConfig {
    // 支持 url 字段，如果不存在则尝试使用 baseUrl
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default, rename = "baseUrl")]
    base_url: Option<String>,

    // 协议类型（可选，字符串格式）
    #[serde(default, rename = "type")]
    pub r#type: Option<String>,
    pub disabled: Option<bool>,
    pub timeout: Option<u64>,

    // 认证配置
    pub auth_token: Option<String>,
    pub headers: Option<HashMap<String, String>>,

    // 连接配置
    pub connect_timeout_secs: Option<u64>,

    // 重试配置
    pub max_retries: Option<usize>,
    pub retry_min_backoff_ms: Option<u64>,
    pub retry_max_backoff_ms: Option<u64>,
}

// 添加一个公共方法来获取实际的URL（优先使用url，其次baseUrl）
impl McpServerUrlConfig {
    /// 获取实际的URL（优先使用url，其次baseUrl）
    pub fn get_url(&self) -> &str {
        self.url
            .as_deref()
            .or_else(|| self.base_url.as_deref())
            .expect("至少需要提供 url 或 baseUrl 字段")
    }

    /// 获取实际的URL的可变引用
    pub fn get_url_mut(&mut self) -> &mut String {
        if self.url.is_none() && self.base_url.is_some() {
            self.url = self.base_url.take();
        }
        self.url.as_mut().expect("至少需要提供 url 或 baseUrl 字段")
    }

    /// 检查是否提供了URL字段
    pub fn has_url(&self) -> bool {
        self.url.is_some() || self.base_url.is_some()
    }
}

impl McpServerUrlConfig {
    /// 获取协议类型，如果未指定或不是 "sse"，则返回 None（需要自动检测）
    pub fn get_protocol_type(&self) -> Option<McpUrlProtocolType> {
        self.r#type
            .as_ref()
            .and_then(|type_str| type_str.parse::<McpUrlProtocolType>().ok())
    }
}

impl Default for McpServerUrlConfig {
    fn default() -> Self {
        Self {
            url: None,
            base_url: None,
            r#type: None,
            disabled: None,
            timeout: None,
            auth_token: None,
            headers: None,
            connect_timeout_secs: Some(5),
            max_retries: Some(3),
            retry_min_backoff_ms: Some(100),
            retry_max_backoff_ms: Some(5000),
        }
    }
}

impl TryFrom<String> for McpServerConfig {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        info!("mcp_server_config: {s:?}");
        let mcp_json_server_parameters = McpJsonServerParameters::from(s);
        mcp_json_server_parameters.try_get_first_mcp_server()
    }
}
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum McpServerInnerConfig {
    Command(McpServerCommandConfig),
    Url(McpServerUrlConfig),
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpJsonServerParameters {
    #[serde(rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerInnerConfig>,
}

impl McpJsonServerParameters {
    //check里面的hashmap是否只有一个,如果没问题,尝试返回第一个
    pub fn try_get_first_mcp_server(&self) -> Result<McpServerConfig> {
        debug!("mcp_servers: {:?}", &self.mcp_servers);
        if self.mcp_servers.len() == 1 {
            let vals = self.mcp_servers.values().next();
            if let Some(val) = vals {
                match val {
                    McpServerInnerConfig::Command(cmd) => Ok(McpServerConfig::Command(cmd.clone())),
                    McpServerInnerConfig::Url(url) => Ok(McpServerConfig::Url(url.clone())),
                }
            } else {
                error!("mcp_server_config: {:?}", "没有找到对应的mcp_server_config");
                Err(anyhow::anyhow!("没有找到对应的mcp配置"))
            }
        } else {
            error!(
                "mcp_servers 必须恰好只有一个MCP插件,mcp_servers: {:?}",
                &self.mcp_servers
            );
            Err(anyhow::anyhow!("mcp_servers 必须恰好只有一个MCP插件"))
        }
    }
}

/// 灵活的 MCP 配置结构体 - 接受任何字段名作为服务容器
#[derive(Debug, Clone)]
pub struct FlexibleMcpConfig {
    services: HashMap<String, McpServerInnerConfig>,
}

impl FlexibleMcpConfig {
    /// 尝试获取第一个 MCP 服务器配置
    pub fn try_get_first_mcp_server(&self) -> Result<McpServerConfig> {
        debug!("flexible_mcp_config: {:?}", self.services);
        if self.services.len() == 1 {
            let vals = self.services.values().next();
            if let Some(val) = vals {
                match val {
                    McpServerInnerConfig::Command(cmd) => Ok(McpServerConfig::Command(cmd.clone())),
                    McpServerInnerConfig::Url(url) => Ok(McpServerConfig::Url(url.clone())),
                }
            } else {
                error!("flexible_mcp_config: {:?}", "没有找到对应的mcp配置");
                Err(anyhow::anyhow!("没有找到对应的mcp配置"))
            }
        } else {
            error!(
                "MCP 配置必须恰好有一个服务, 当前数量: {:?}",
                self.services.len()
            );
            Err(anyhow::anyhow!("MCP 配置必须恰好有一个服务"))
        }
    }

    /// 获取所有服务配置（用于调试）
    pub fn get_all_services(&self) -> &HashMap<String, McpServerInnerConfig> {
        &self.services
    }

    /// 获取服务名称列表
    pub fn get_service_names(&self) -> Vec<&String> {
        self.services.keys().collect()
    }
}

impl TryFrom<String> for FlexibleMcpConfig {
    type Error = anyhow::Error;

    fn try_from(json_str: String) -> Result<Self> {
        debug!("flexible_mcp_json_server_parameters: {json_str:?}");

        // 首先尝试标准格式 (包含 "mcpServers" 字段)
        if let Ok(standard_config) = serde_json::from_str::<McpJsonServerParameters>(&json_str) {
            return Ok(Self {
                services: standard_config.mcp_servers,
            });
        }

        // 如果标准格式失败，尝试直接解析为 HashMap
        let parsed_value: serde_json::Value =
            serde_json::from_str(&json_str).map_err(|e| anyhow::anyhow!("JSON 解析失败: {}", e))?;

        // 找到第一个包含 McpServerInnerConfig 的字段
        if let serde_json::Value::Object(obj) = parsed_value {
            for (key, value) in obj {
                if let Ok(service_config) =
                    serde_json::from_value::<McpServerInnerConfig>(value.clone())
                {
                    let mut services = HashMap::new();
                    services.insert(key, service_config);
                    return Ok(Self { services });
                }
            }
        }

        Err(anyhow::anyhow!("无法从 JSON 中提取 MCP 服务配置"))
    }
}

//根据生成的 mcp_id 生成对应的 sse path路径和 message path路径
#[derive(Debug, Clone)]
pub struct McpRouterPath {
    //mcp_id
    pub mcp_id: String,
    //base_path
    pub base_path: String,
    //mcp协议,对应不同的路径枚举定义
    pub mcp_protocol_path: McpProtocolPath,
    //mcp协议
    pub mcp_protocol: McpProtocol,
    //最后访问时间
    pub last_accessed: Instant,
}
//定义 mcp协议枚举: sse 和 stream
#[derive(Debug, Clone)]
pub enum McpProtocolPath {
    SsePath(SseMcpRouterPath),
    StreamPath(StreamMcpRouterPath),
}

//定义 mcp 协议枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum McpProtocol {
    Stdio,
    Sse,
    Stream,
}

impl std::str::FromStr for McpProtocol {
    type Err = String;

    fn from_str(type_str: &str) -> Result<Self, Self::Err> {
        match type_str {
            "stdio" => Ok(McpProtocol::Stdio),
            "sse" => Ok(McpProtocol::Sse),
            "http" | "stream" => Ok(McpProtocol::Stream),
            _ => Err(format!(
                "不支持的协议类型: {}, 支持的类型: sse, http, stream, stdio",
                type_str
            )),
        }
    }
}

//sse 协议下,需要有2个path: sse 和 message
#[derive(Debug, Clone)]
pub struct SseMcpRouterPath {
    pub sse_path: String,
    pub message_path: String,
}
//stream 协议下,需要有1个path: stream
#[derive(Debug, Clone)]
pub struct StreamMcpRouterPath {
    pub stream_path: String,
}

impl McpRouterPath {
    //根据 uri 路由前缀,匹配请求的mcp协议是 sse 还是 stream
    pub fn from_uri_prefix_protocol(uri: &str) -> Option<McpProtocol> {
        if uri.starts_with(GLOBAL_SSE_MCP_ROUTES_PREFIX) {
            Some(McpProtocol::Sse)
        } else if uri.starts_with(GLOBAL_STREAM_MCP_ROUTES_PREFIX) {
            Some(McpProtocol::Stream)
        } else {
            None
        }
    }

    //根据 mcp_id,生成对应的 sse path路径和 message path路径
    fn from_mcp_id_for_sse(mcp_id: String) -> SseMcpRouterPath {
        // 创建McpRouterPath结构
        let sse_path = format!("{GLOBAL_SSE_MCP_ROUTES_PREFIX}/proxy/{mcp_id}/sse");
        let message_path = format!("{GLOBAL_SSE_MCP_ROUTES_PREFIX}/proxy/{mcp_id}/message");
        // let message_path = "/message".to_string();
        SseMcpRouterPath {
            sse_path,
            message_path,
        }
    }
    // 辅助函数：从路径中提取MCP ID
    fn extract_mcp_id(
        path_without_prefix: &str,
        prefix_to_strip: &str,
        suffixes: &[&str],
    ) -> Option<String> {
        // 先移除前导的prefix
        let path = path_without_prefix.strip_prefix(prefix_to_strip)?;

        // 移除所有可能的后缀
        let mut mcp_id = path;
        for suffix in suffixes {
            mcp_id = mcp_id.trim_end_matches(suffix);
        }

        // 如果提取后的ID为空，则返回None
        if mcp_id.is_empty() {
            return None;
        }

        Some(mcp_id.to_string())
    }
    //根据 请求的url path ,根据前缀,可以区分 sse 和 stream,然后解析成:  McpRouterPath 结构
    pub fn from_url(path: &str) -> Option<Self> {
        // 检查是否为SSE路径
        if let Some(path_without_prefix) = path.strip_prefix(GLOBAL_SSE_MCP_ROUTES_PREFIX) {
            // 提取MCP ID
            let mcp_id = McpRouterPath::extract_mcp_id(
                path_without_prefix,
                "/proxy/",
                &["/sse", "/message"],
            )?;

            // 创建McpRouterPath结构
            let sse_mcp_router_path = McpRouterPath::from_mcp_id_for_sse(mcp_id.clone());

            return Some(Self {
                mcp_id: mcp_id.clone(),
                base_path: format!("{GLOBAL_SSE_MCP_ROUTES_PREFIX}/proxy/{mcp_id}"),
                mcp_protocol_path: McpProtocolPath::SsePath(sse_mcp_router_path),
                mcp_protocol: McpProtocol::Sse,
                last_accessed: Instant::now(),
            });
        }

        // 检查是否为Stream路径
        if let Some(path_without_prefix) = path.strip_prefix(GLOBAL_STREAM_MCP_ROUTES_PREFIX) {
            // 提取MCP ID
            let mcp_id =
                McpRouterPath::extract_mcp_id(path_without_prefix, "/proxy/", &["/stream"])?;

            // 创建流路径
            let stream_path = format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/proxy/{mcp_id}");

            return Some(Self {
                mcp_id: mcp_id.clone(),
                base_path: format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/proxy/{mcp_id}"),
                mcp_protocol_path: McpProtocolPath::StreamPath(StreamMcpRouterPath { stream_path }),
                mcp_protocol: McpProtocol::Stream,
                last_accessed: Instant::now(),
            });
        }

        // 不匹配任何已知路径模式
        None
    }

    pub fn new(mcp_id: String, mcp_protocol: McpProtocol) -> Result<Self, anyhow::Error> {
        match mcp_protocol {
            McpProtocol::Sse => {
                //使用全局变量的前缀定义: sse 和 stream
                // 创建McpRouterPath结构
                let sse_mcp_router_path = McpRouterPath::from_mcp_id_for_sse(mcp_id.clone());

                Ok(Self {
                    mcp_id: mcp_id.clone(),
                    base_path: format!("{GLOBAL_SSE_MCP_ROUTES_PREFIX}/proxy/{mcp_id}"),
                    mcp_protocol_path: McpProtocolPath::SsePath(sse_mcp_router_path),
                    mcp_protocol: McpProtocol::Sse,
                    last_accessed: Instant::now(),
                })
            }
            McpProtocol::Stream => {
                let stream_path: String =
                    format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/proxy/{mcp_id}");
                Ok(Self {
                    mcp_id: mcp_id.clone(),
                    base_path: format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/proxy/{mcp_id}"),
                    mcp_protocol_path: McpProtocolPath::StreamPath(StreamMcpRouterPath {
                        stream_path,
                    }),
                    mcp_protocol: McpProtocol::Stream,
                    last_accessed: Instant::now(),
                })
            }
            McpProtocol::Stdio => {
                // Stdio 协议不支持通过此方法创建路由路径
                Err(anyhow::anyhow!(
                    "McpRouterPath::new 不支持 Stdio 协议。Stdio 协议仅用于命令行启动的 MCP 服务，不提供 HTTP 路由接口"
                ))
            }
        }
    }

    pub fn check_mcp_path(path: &str) -> bool {
        //检查是否是 mcp 协议的路径，需要/mcp开头，且 /sse, 或者 /message结尾
        let mcp_path_flag = path.starts_with("/mcp");
        let sse_path_flag = path.ends_with("/sse");
        let message_path_flag = path.ends_with("/message");
        if mcp_path_flag && (sse_path_flag || message_path_flag) {
            let base_path = path
                .trim_end_matches("/sse")
                .trim_end_matches("/message")
                .to_string();
            let mcp_id = base_path.strip_prefix("/mcp/").map(|id| id.to_string());
            mcp_id.is_some()
        } else {
            false
        }
    }

    pub fn update_last_accessed(&mut self) {
        self.last_accessed = Instant::now();
    }

    pub fn time_since_last_access(&self) -> Duration {
        self.last_accessed.elapsed()
    }
}

impl From<String> for McpJsonServerParameters {
    fn from(s: String) -> Self {
        debug!("mcp_json_server_parameters: {s:?}");

        // 首先尝试标准格式 (包含 "mcpServers" 字段)
        if let Ok(mcp_json_server_parameters) = serde_json::from_str::<McpJsonServerParameters>(&s)
        {
            return mcp_json_server_parameters;
        }

        // 如果标准格式失败，尝试使用灵活格式
        let flexible_config: FlexibleMcpConfig = s
            .try_into()
            .expect("Failed to convert to FlexibleMcpConfig");
        let services = flexible_config.get_all_services().clone();

        McpJsonServerParameters {
            mcp_servers: services,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdio_server_parameters_from_json() {
        let json = r#"{
            "mcpServers": {
                "baidu-map": {
                    "command": "npx",
                    "args": [
                        "-y",
                        "@baidumap/mcp-server-baidu-map"
                    ],
                    "env": {
                        "BAIDU_MAP_API_KEY": "xxx"
                    }
                }
            }
        }"#;
        let params = McpJsonServerParameters::from(json.to_string());
        let baidu = params
            .mcp_servers
            .get("baidu-map")
            .expect("baidu-map should exist");

        match baidu {
            McpServerInnerConfig::Command(cmd_config) => {
                assert_eq!(cmd_config.command, "npx");
                assert_eq!(
                    cmd_config.args,
                    Some(vec![
                        "-y".to_string(),
                        "@baidumap/mcp-server-baidu-map".to_string()
                    ])
                );
                assert_eq!(
                    cmd_config
                        .env
                        .as_ref()
                        .unwrap()
                        .get("BAIDU_MAP_API_KEY")
                        .unwrap(),
                    "xxx"
                );
            }
            McpServerInnerConfig::Url(_) => {
                panic!("Expected command config, got URL config");
            }
        }
    }

    #[test]
    fn test_stdio_server_parameters_from_mysql_json() -> Result<()> {
        let json = r#"
        {"mcpServers": {"mysql": {"command": "/Users/soddy/go/bin/go-mcp-mysql", "args": ["--host", "192.168.1.12", "--user", "agent_platform_test", "--pass", "SRJG7NdiwKGDkmPs", "--port", "3306", "--db", "agent_platform_test"], "env": {}}}}
        "#;
        let params = McpJsonServerParameters::from(json.to_string());
        println!("params.len: {:?}", params.mcp_servers.len());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Command(cmd_config) => {
                assert_eq!(cmd_config.command, "/Users/soddy/go/bin/go-mcp-mysql");
                assert_eq!(
                    cmd_config.args,
                    Some(vec![
                        "--host".to_string(),
                        "192.168.1.12".to_string(),
                        "--user".to_string(),
                        "agent_platform_test".to_string(),
                        "--pass".to_string(),
                        "SRJG7NdiwKGDkmPs".to_string(),
                        "--port".to_string(),
                        "3306".to_string(),
                        "--db".to_string(),
                        "agent_platform_test".to_string()
                    ])
                );
                assert_eq!(cmd_config.env, Some(HashMap::new()));
            }
            McpServerConfig::Url(_) => {
                panic!("Expected command config, got URL config");
            }
        }

        Ok(())
    }

    #[test]
    fn test_stdio_server_parameters_from_playwright_json() -> Result<()> {
        let json = r#"{
            "mcpServers": {
                "playwright": {
                    "command": "npx",
                    "args": [
                        "@playwright/mcp@latest",
                        "--headless"
                    ]
                }
            }
        }"#;

        let params = McpJsonServerParameters::from(json.to_string());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Command(cmd_config) => {
                assert_eq!(cmd_config.command, "npx");
                assert_eq!(
                    cmd_config.args,
                    Some(vec![
                        "@playwright/mcp@latest".to_string(),
                        "--headless".to_string()
                    ])
                );
                assert_eq!(cmd_config.env, None);
            }
            McpServerConfig::Url(_) => {
                panic!("Expected command config, got URL config");
            }
        }

        Ok(())
    }

    #[test]
    fn test_stdio_server_parameters_from_url_json() -> Result<()> {
        let json = r#"{
            "mcpServers": {
                "ocr_edu": {
                    "url": "https://aip.baidubce.com/mcp/image_recognition/sse?Authorization=Bearer%20bce-v3/ALTAK-zX2w0VFXauTMxEf5BypEl/1835f7e1886946688b132e9187392d9fee8f3c06"
                }
            }
        }"#;

        let params = McpJsonServerParameters::from(json.to_string());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Url(url_config) => {
                assert_eq!(
                    url_config.get_url(),
                    "https://aip.baidubce.com/mcp/image_recognition/sse?Authorization=Bearer%20bce-v3/ALTAK-zX2w0VFXauTMxEf5BypEl/1835f7e1886946688b132e9187392d9fee8f3c06"
                );
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        Ok(())
    }

    #[test]
    fn test_url_config_with_type_field() -> Result<()> {
        let json = r#"{
            "mcpServers": {
                "amap-amap-test": {
                    "url": "https://mcp.amap.com/sse",
                    "disabled": false,
                    "timeout": 60,
                    "type": "sse",
                    "headers": {
                        "Authorization": "Bearer 12121221"
                    }
                }
            }
        }"#;

        let params = McpJsonServerParameters::from(json.to_string());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Url(url_config) => {
                assert_eq!(url_config.get_url(), "https://mcp.amap.com/sse");
                assert_eq!(url_config.disabled, Some(false));
                assert_eq!(url_config.timeout, Some(60));
                assert_eq!(url_config.r#type, Some("sse".to_string()));
                assert_eq!(
                    url_config.get_protocol_type(),
                    Some(McpUrlProtocolType::Sse)
                );
                assert!(
                    url_config
                        .headers
                        .as_ref()
                        .unwrap()
                        .contains_key("Authorization")
                );
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        Ok(())
    }

    #[test]
    fn test_url_config_with_stream_type() -> Result<()> {
        let json = r#"{
            "mcpServers": {
                "streamable-service": {
                    "url": "https://example.com/mcp",
                    "type": "stream"
                }
            }
        }"#;

        let params = McpJsonServerParameters::from(json.to_string());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Url(url_config) => {
                assert_eq!(url_config.get_url(), "https://example.com/mcp");
                assert_eq!(url_config.r#type, Some("stream".to_string()));
                assert_eq!(
                    url_config.get_protocol_type(),
                    Some(McpUrlProtocolType::Stream)
                ); // "stream" 应该解析为 Stream
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        Ok(())
    }

    #[test]
    fn test_url_config_with_http_type() -> Result<()> {
        let json = r#"{
            "mcpServers": {
                "http-service": {
                    "url": "https://example.com/mcp",
                    "type": "http"
                }
            }
        }"#;

        let params = McpJsonServerParameters::from(json.to_string());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Url(url_config) => {
                assert_eq!(url_config.get_url(), "https://example.com/mcp");
                assert_eq!(url_config.r#type, Some("http".to_string()));
                assert_eq!(
                    url_config.get_protocol_type(),
                    Some(McpUrlProtocolType::Stream)
                ); // "http" 应该解析为 Stream
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        Ok(())
    }

    #[test]
    fn test_url_protocol_type_conversion() {
        // 测试 FromStr trait
        assert_eq!(
            "sse".parse::<McpUrlProtocolType>(),
            Ok(McpUrlProtocolType::Sse)
        );
        assert_eq!(
            "http".parse::<McpUrlProtocolType>(),
            Ok(McpUrlProtocolType::Stream)
        );
        assert_eq!(
            "stream".parse::<McpUrlProtocolType>(),
            Ok(McpUrlProtocolType::Stream)
        );
        assert!("stdio".parse::<McpUrlProtocolType>().is_err());

        // 测试 is_streamable 方法
        assert!(McpUrlProtocolType::Http.is_streamable());
        assert!(McpUrlProtocolType::Stream.is_streamable());
        assert!(!McpUrlProtocolType::Sse.is_streamable());
        assert!(!McpUrlProtocolType::Stdio.is_streamable());

        // 测试 to_mcp_protocol 方法
        assert_eq!(
            McpUrlProtocolType::Sse.to_mcp_protocol(),
            super::McpProtocol::Sse
        );
        assert_eq!(
            McpUrlProtocolType::Stdio.to_mcp_protocol(),
            super::McpProtocol::Stdio
        );
        assert_eq!(
            McpUrlProtocolType::Http.to_mcp_protocol(),
            super::McpProtocol::Stream
        );
        assert_eq!(
            McpUrlProtocolType::Stream.to_mcp_protocol(),
            super::McpProtocol::Stream
        );
    }

    #[test]
    fn test_mcp_protocol_from_str() {
        // 测试有效的协议类型
        assert_eq!("stdio".parse::<McpProtocol>(), Ok(McpProtocol::Stdio));
        assert_eq!("sse".parse::<McpProtocol>(), Ok(McpProtocol::Sse));
        assert_eq!("http".parse::<McpProtocol>(), Ok(McpProtocol::Stream));
        assert_eq!("stream".parse::<McpProtocol>(), Ok(McpProtocol::Stream));

        // 测试无效的协议类型
        assert!("invalid".parse::<McpProtocol>().is_err());
        assert!("tcp".parse::<McpProtocol>().is_err());
        assert!("".parse::<McpProtocol>().is_err());
    }

    #[test]
    fn test_url_config_with_base_url() -> Result<()> {
        // 测试使用 baseUrl 字段的配置
        let json = r#"{
            "mcpServers": {
                "aliyun-exchange": {
                    "type": "sse",
                    "description": "阿里云百炼_新浪实时汇率报价",
                    "isActive": true,
                    "name": "阿里云百炼_阿里云百炼_新浪实时汇率报价",
                    "baseUrl": "https://dashscope.aliyuncs.com/api/v1/mcps/mcp-NjZmY2NhZDc5NTQz/sse",
                    "headers": {
                        "Authorization": "Bearer sk-d39046bd64b446d8a19d642e9a2b8967"
                    }
                }
            }
        }"#;

        let params = McpJsonServerParameters::from(json.to_string());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Url(url_config) => {
                assert_eq!(
                    url_config.get_url(),
                    "https://dashscope.aliyuncs.com/api/v1/mcps/mcp-NjZmY2NhZDc5NTQz/sse"
                );
                assert_eq!(url_config.r#type, Some("sse".to_string()));
                assert_eq!(
                    url_config.get_protocol_type(),
                    Some(McpUrlProtocolType::Sse)
                );
                assert!(url_config.has_url());
                assert!(
                    url_config
                        .headers
                        .as_ref()
                        .unwrap()
                        .contains_key("Authorization")
                );
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        Ok(())
    }

    #[test]
    fn test_url_config_with_both_url_and_base_url() -> Result<()> {
        // 测试同时提供 url 和 baseUrl 的配置，url 应该优先使用
        let json = r#"{
            "mcpServers": {
                "test-service": {
                    "url": "https://primary.example.com/mcp",
                    "baseUrl": "https://fallback.example.com/mcp",
                    "type": "sse"
                }
            }
        }"#;

        let params = McpJsonServerParameters::from(json.to_string());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Url(url_config) => {
                // 应该优先使用 url 字段
                assert_eq!(url_config.get_url(), "https://primary.example.com/mcp");
                assert!(url_config.has_url());
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        Ok(())
    }

    #[test]
    fn test_flexible_config_with_custom_field_name() -> Result<()> {
        // 测试使用自定义字段名的灵活配置
        let json = r#"{
            "mcpServers2222": {
                "mcp-NjZmY2NhZDc5NTQz": {
                    "type": "sse",
                    "description": "阿里云百炼_新浪实时汇率报价",
                    "isActive": true,
                    "name": "阿里云百炼_阿里云百炼_新浪实时汇率报价",
                    "baseUrl": "https://dashscope.aliyuncs.com/api/v1/mcps/mcp-NjZmY2NhZDc5NTQz/sse",
                    "headers": {
                        "Authorization": "Bearer sk-d39046bd64b446d8a19d642e9a2b8967"
                    }
                }
            }
        }"#;

        let flexible_config: FlexibleMcpConfig = json.try_into()?;
        let mcp_server_config = flexible_config.try_get_first_mcp_server()?;

        // 验证服务名称列表
        let service_names = flexible_config.get_service_names();
        assert_eq!(service_names.len(), 1);
        assert_eq!(service_names[0], "mcp-NjZmY2NhZDc5NTQz");

        match mcp_server_config {
            McpServerConfig::Url(url_config) => {
                assert_eq!(
                    url_config.get_url(),
                    "https://dashscope.aliyuncs.com/api/v1/mcps/mcp-NjZmY2NhZDc5NTQz/sse"
                );
                assert_eq!(url_config.r#type, Some("sse".to_string()));
                assert_eq!(
                    url_config.get_protocol_type(),
                    Some(McpUrlProtocolType::Sse)
                );
                assert!(url_config.has_url());

                let headers = url_config.headers.as_ref().unwrap();
                assert!(headers.contains_key("Authorization"));
                assert_eq!(
                    headers.get("Authorization").unwrap(),
                    "Bearer sk-d39046bd64b446d8a19d642e9a2b8967"
                );
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        println!("✅ 自定义字段名配置测试通过！");
        Ok(())
    }

    #[test]
    fn test_flexible_config_standard_format() -> Result<()> {
        // 测试灵活配置仍然支持标准格式
        let json = r#"{
            "mcpServers": {
                "test-service": {
                    "url": "https://example.com/mcp",
                    "type": "sse"
                }
            }
        }"#;

        let flexible_config: FlexibleMcpConfig = json.try_into()?;
        let mcp_server_config = flexible_config.try_get_first_mcp_server()?;

        let service_names = flexible_config.get_service_names();
        assert_eq!(service_names.len(), 1);
        assert_eq!(service_names[0], "test-service");

        match mcp_server_config {
            McpServerConfig::Url(url_config) => {
                assert_eq!(url_config.get_url(), "https://example.com/mcp");
                assert_eq!(url_config.r#type, Some("sse".to_string()));
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        println!("✅ 灵活配置标准格式测试通过！");
        Ok(())
    }

    #[test]
    fn test_flexible_config_through_mcp_json_server_parameters() -> Result<()> {
        // 测试通过 McpJsonServerParameters 使用灵活配置
        let json = r#"{
            "myCustomFieldName": {
                "test-service": {
                    "command": "npx",
                    "args": ["-y", "@playwright/mcp@latest"]
                }
            }
        }"#;

        let params = McpJsonServerParameters::from(json.to_string());
        let mcp_server_config = params.try_get_first_mcp_server()?;

        match mcp_server_config {
            McpServerConfig::Command(cmd_config) => {
                assert_eq!(cmd_config.command, "npx");
                assert_eq!(
                    cmd_config.args,
                    Some(vec!["-y".to_string(), "@playwright/mcp@latest".to_string()])
                );
            }
            McpServerConfig::Url(_) => {
                panic!("Expected command config, got URL config");
            }
        }

        println!("✅ 通过 McpJsonServerParameters 使用灵活配置测试通过！");
        Ok(())
    }

    #[test]
    fn test_flexible_config_multiple_fields_error() -> Result<()> {
        // 测试多个字段时的错误处理
        let json = r#"{
            "field1": {
                "service1": {
                    "url": "https://example1.com/mcp",
                    "type": "sse"
                }
            },
            "field2": {
                "service2": {
                    "url": "https://example2.com/mcp",
                    "type": "sse"
                }
            }
        }"#;

        let flexible_config: FlexibleMcpConfig = json.try_into()?;
        let result = flexible_config.try_get_first_mcp_server();

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("必须恰好有一个服务")
        );

        println!("✅ 多字段错误处理测试通过！");
        Ok(())
    }

    #[test]
    fn test_flexible_config_empty_json() -> Result<()> {
        // 测试空 JSON 的错误处理
        let json = r#"{}"#;

        let flexible_config: Result<FlexibleMcpConfig, _> = json.try_into();
        assert!(flexible_config.is_err());
        assert!(
            flexible_config
                .unwrap_err()
                .to_string()
                .contains("无法从 JSON 中提取 MCP 服务配置")
        );

        println!("✅ 空 JSON 错误处理测试通过！");
        Ok(())
    }
}
