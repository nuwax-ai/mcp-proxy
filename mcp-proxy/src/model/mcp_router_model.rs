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

//mcp的URL配置（用于Streamable/SSE协议）
#[derive(Debug, Deserialize, Clone)]
pub struct McpServerUrlConfig {
    pub url: String,

    // 认证配置
    pub auth_token: Option<String>,
    pub headers: Option<HashMap<String, String>>,

    // 连接配置
    pub timeout_secs: Option<u64>,
    pub connect_timeout_secs: Option<u64>,

    // 重试配置
    pub max_retries: Option<usize>,
    pub retry_min_backoff_ms: Option<u64>,
    pub retry_max_backoff_ms: Option<u64>,
}

impl Default for McpServerUrlConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            auth_token: None,
            headers: None,
            timeout_secs: Some(30),
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
    Sse,
    Stream,
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
            let stream_path = format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/proxy/{mcp_id}/stream");

            return Some(Self {
                mcp_id: mcp_id.clone(),
                base_path: GLOBAL_STREAM_MCP_ROUTES_PREFIX.to_string(),
                mcp_protocol_path: McpProtocolPath::StreamPath(StreamMcpRouterPath { stream_path }),
                mcp_protocol: McpProtocol::Stream,
                last_accessed: Instant::now(),
            });
        }

        // 不匹配任何已知路径模式
        None
    }

    pub fn new(mcp_id: String, mcp_protocol: McpProtocol) -> Self {
        match mcp_protocol {
            McpProtocol::Sse => {
                //使用全局变量的前缀定义: sse 和 stream
                // 创建McpRouterPath结构
                let sse_mcp_router_path = McpRouterPath::from_mcp_id_for_sse(mcp_id.clone());

                Self {
                    mcp_id: mcp_id.clone(),
                    base_path: format!("{GLOBAL_SSE_MCP_ROUTES_PREFIX}/proxy/{mcp_id}"),
                    mcp_protocol_path: McpProtocolPath::SsePath(sse_mcp_router_path),
                    mcp_protocol: McpProtocol::Sse,
                    last_accessed: Instant::now(),
                }
            }
            McpProtocol::Stream => {
                let stream_path: String =
                    format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/proxy/{mcp_id}/stream");
                Self {
                    mcp_id: mcp_id.clone(),
                    base_path: format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/proxy/{mcp_id}"),
                    mcp_protocol_path: McpProtocolPath::StreamPath(StreamMcpRouterPath {
                        stream_path,
                    }),
                    mcp_protocol: McpProtocol::Stream,
                    last_accessed: Instant::now(),
                }
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
        match serde_json::from_str::<McpJsonServerParameters>(&s) {
            Ok(mcp_json_server_parameters) => mcp_json_server_parameters,
            Err(e) => {
                error!("mcp_json_server_parameters 解析失败: {e:?}");
                McpJsonServerParameters {
                    mcp_servers: HashMap::new(),
                }
            }
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
                    url_config.url,
                    "https://aip.baidubce.com/mcp/image_recognition/sse?Authorization=Bearer%20bce-v3/ALTAK-zX2w0VFXauTMxEf5BypEl/1835f7e1886946688b132e9187392d9fee8f3c06"
                );
            }
            McpServerConfig::Command(_) => {
                panic!("Expected URL config, got command config");
            }
        }

        Ok(())
    }
}
