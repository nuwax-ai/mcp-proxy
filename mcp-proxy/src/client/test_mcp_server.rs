//! 用于集成测试的最小 MCP 服务器
//!
//! 支持 echo 和 counter 两个简单工具，用于测试连接、重连和工具调用功能。
//!
//! 运行方式:
//! ```bash
//! cargo run --bin test-mcp-server
//! ```

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParam, CallToolResult, Content, Implementation, JsonObject,
        ListToolsResult, PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    transport::stdio,
    RoleServer,
};
use std::sync::Arc;
use tokio::sync::Mutex;

/// 测试用 MCP 服务器
///
/// 提供简单工具:
/// - `echo`: 回显输入消息
/// - `increment`: 递增计数器并返回当前值
/// - `reset`: 重置计数器
/// - `get_counter`: 获取当前计数器值
#[derive(Clone)]
pub struct TestMcpServer {
    counter: Arc<Mutex<i32>>,
}

impl TestMcpServer {
    /// 创建新的测试服务器实例
    pub fn new() -> Self {
        Self {
            counter: Arc::new(Mutex::new(0)),
        }
    }

    /// 创建一个简单的空 schema
    fn empty_schema() -> Arc<JsonObject> {
        let mut schema = JsonObject::new();
        schema.insert("type".to_string(), serde_json::json!("object"));
        schema.insert("properties".to_string(), serde_json::json!({}));
        Arc::new(schema)
    }

    /// 创建 echo 工具的 schema
    fn echo_schema() -> Arc<JsonObject> {
        let mut schema = JsonObject::new();
        schema.insert("type".to_string(), serde_json::json!("object"));
        schema.insert(
            "properties".to_string(),
            serde_json::json!({
                "message": {
                    "type": "string",
                    "description": "Message to echo back"
                }
            }),
        );
        schema.insert("required".to_string(), serde_json::json!(["message"]));
        Arc::new(schema)
    }

    /// 获取工具定义列表
    fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new("echo", "Echo back the input message", Self::echo_schema()),
            Tool::new(
                "increment",
                "Increment the counter by 1 and return new value",
                Self::empty_schema(),
            ),
            Tool::new("reset", "Reset the counter to 0", Self::empty_schema()),
            Tool::new(
                "get_counter",
                "Get current counter value without changing it",
                Self::empty_schema(),
            ),
        ]
    }

    /// 处理 echo 工具调用
    async fn handle_echo(&self, args: &serde_json::Value) -> CallToolResult {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(no message)");
        CallToolResult::success(vec![Content::text(format!("Echo: {}", message))])
    }

    /// 处理 increment 工具调用
    async fn handle_increment(&self) -> CallToolResult {
        let mut counter = self.counter.lock().await;
        *counter += 1;
        CallToolResult::success(vec![Content::text(format!("Counter: {}", *counter))])
    }

    /// 处理 reset 工具调用
    async fn handle_reset(&self) -> CallToolResult {
        let mut counter = self.counter.lock().await;
        *counter = 0;
        CallToolResult::success(vec![Content::text("Counter reset to 0".to_string())])
    }

    /// 处理 get_counter 工具调用
    async fn handle_get_counter(&self) -> CallToolResult {
        let counter = self.counter.lock().await;
        CallToolResult::success(vec![Content::text(format!("Counter: {}", *counter))])
    }
}

impl Default for TestMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerHandler for TestMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "test-mcp-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                title: Some("Test MCP Server".to_string()),
                website_url: None,
                icons: None,
            },
            instructions: Some(
                "A minimal MCP server for integration testing. \
                 Provides echo, increment, reset, and get_counter tools."
                    .to_string(),
            ),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = request
            .arguments
            .as_ref()
            .map(|v| serde_json::Value::Object(v.clone()))
            .unwrap_or(serde_json::Value::Object(Default::default()));

        // 使用 &str 来进行匹配
        let tool_name: &str = &request.name;
        let result = match tool_name {
            "echo" => self.handle_echo(&args).await,
            "increment" => self.handle_increment().await,
            "reset" => self.handle_reset().await,
            "get_counter" => self.handle_get_counter().await,
            _ => CallToolResult::error(vec![Content::text(format!(
                "Unknown tool: {}",
                request.name
            ))]),
        };

        Ok(result)
    }
}

/// 独立运行时作为 stdio MCP 服务器
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志（输出到 stderr，避免干扰 stdio 通信）
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    tracing::info!("Test MCP Server starting...");

    let server = TestMcpServer::new();
    let transport = stdio();

    tracing::info!("Serving on stdio...");
    let running = server.serve(transport).await?;
    running.waiting().await?;

    tracing::info!("Test MCP Server stopped.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_counter_increment() {
        let server = TestMcpServer::new();

        // 第一次递增
        let result = server.handle_increment().await;
        assert!(!result.is_error.unwrap_or(false));

        // 第二次递增
        let result = server.handle_increment().await;
        assert!(!result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_echo() {
        let server = TestMcpServer::new();
        let args = serde_json::json!({"message": "hello"});
        let result = server.handle_echo(&args).await;
        assert!(!result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_reset() {
        let server = TestMcpServer::new();

        // 先递增几次
        server.handle_increment().await;
        server.handle_increment().await;

        // 重置
        let result = server.handle_reset().await;
        assert!(!result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_get_tools() {
        let tools = TestMcpServer::get_tools();
        assert_eq!(tools.len(), 4);
        assert!(tools.iter().any(|t| &*t.name == "echo"));
        assert!(tools.iter().any(|t| &*t.name == "increment"));
        assert!(tools.iter().any(|t| &*t.name == "reset"));
        assert!(tools.iter().any(|t| &*t.name == "get_counter"));
    }
}
