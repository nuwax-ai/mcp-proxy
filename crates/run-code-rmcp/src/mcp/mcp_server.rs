use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;

use crate::model::{CodeExecutor, LanguageScript};

/// 代码执行请求参数
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CodeRunRequest {
    #[schemars(description = "要执行的代码")]
    pub code: String,

    #[schemars(description = "可选的执行参数")]
    pub params: Option<serde_json::Value>,
}

/// 代码执行工具服务
#[derive(Debug, Clone, Default)]
pub struct CodeRunnerService;

impl CodeRunnerService {
    /// 三种语言工具共用的执行与结果包装逻辑
    async fn execute_tool(
        &self,
        language: LanguageScript,
        request: CodeRunRequest,
    ) -> Result<CallToolResult, McpError> {
        match CodeExecutor::execute_with_params_compat(&request.code, language, request.params)
            .await
        {
            Ok(result) => {
                let payload = if result.success {
                    json!({
                        "result": result.result,
                        "logs": result.logs,
                        "success": true
                    })
                } else {
                    json!({
                        "success": false,
                        "error": result.error,
                        "logs": result.logs
                    })
                };
                let content = ContentBlock::json(payload)?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(err) => {
                let content = ContentBlock::json(json!({
                    "success": false,
                    "error": err.to_string(),
                    "logs": []
                }))?;
                Ok(CallToolResult::success(vec![content]))
            }
        }
    }
}

#[tool_router]
impl CodeRunnerService {
    #[tool(description = "执行JavaScript代码并返回结果")]
    async fn run_javascript(
        &self,
        request: Parameters<CodeRunRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.execute_tool(LanguageScript::Js, request.0).await
    }

    #[tool(description = "执行TypeScript代码并返回结果")]
    async fn run_typescript(
        &self,
        request: Parameters<CodeRunRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.execute_tool(LanguageScript::Ts, request.0).await
    }

    #[tool(description = "执行Python代码并返回结果")]
    async fn run_python(
        &self,
        request: Parameters<CodeRunRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.execute_tool(LanguageScript::Python, request.0).await
    }
}

#[tool_handler]
impl ServerHandler for CodeRunnerService {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("一个支持执行JavaScript、TypeScript和Python代码的服务")
    }
}
