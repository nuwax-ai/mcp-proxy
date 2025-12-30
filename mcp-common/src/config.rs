//! MCP 服务配置

use std::collections::HashMap;
use crate::ToolFilter;

/// MCP 服务配置
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct McpServiceConfig {
    /// 服务名称
    pub name: String,
    /// 启动命令
    pub command: String,
    /// 命令参数
    pub args: Option<Vec<String>>,
    /// 环境变量
    pub env: Option<HashMap<String, String>>,
    /// 工具过滤配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_filter: Option<ToolFilter>,
}

impl McpServiceConfig {
    /// 创建新配置
    pub fn new(name: String, command: String) -> Self {
        Self {
            name,
            command,
            args: None,
            env: None,
            tool_filter: None,
        }
    }

    /// 设置参数
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = Some(args);
        self
    }

    /// 设置环境变量
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = Some(env);
        self
    }

    /// 设置工具过滤器
    pub fn with_tool_filter(mut self, filter: ToolFilter) -> Self {
        self.tool_filter = Some(filter);
        self
    }
}
