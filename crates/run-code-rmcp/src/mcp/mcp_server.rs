use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    tool, tool_router,
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

#[tool_router]
impl CodeRunnerService {
    #[tool(description = "执行JavaScript代码并返回结果")]
    async fn run_javascript(
        &self,
        request: Parameters<CodeRunRequest>,
    ) -> Result<CallToolResult, McpError> {
        let request = request.0;
        match CodeExecutor::execute_with_params_compat(
            &request.code,
            LanguageScript::Js,
            request.params,
        )
        .await
        {
            Ok(result) => {
                if result.success {
                    let content = Content::json(json!({
                        "result": result.result,
                        "logs": result.logs,
                        "success": true
                    }))?;
                    Ok(CallToolResult::success(vec![content]))
                } else {
                    let content = Content::json(json!({
                        "success": false,
                        "error": result.error,
                        "logs": result.logs
                    }))?;
                    Ok(CallToolResult::success(vec![content]))
                }
            }
            Err(err) => {
                let content = Content::json(json!({
                    "success": false,
                    "error": err.to_string(),
                    "logs": []
                }))?;
                Ok(CallToolResult::success(vec![content]))
            }
        }
    }

    #[tool(description = "执行TypeScript代码并返回结果")]
    async fn run_typescript(
        &self,
        request: Parameters<CodeRunRequest>,
    ) -> Result<CallToolResult, McpError> {
        let request = request.0;
        match CodeExecutor::execute_with_params_compat(
            &request.code,
            LanguageScript::Ts,
            request.params,
        )
        .await
        {
            Ok(result) => {
                if result.success {
                    let content = Content::json(json!({
                        "result": result.result,
                        "logs": result.logs,
                        "success": true
                    }))?;
                    Ok(CallToolResult::success(vec![content]))
                } else {
                    let content = Content::json(json!({
                        "success": false,
                        "error": result.error,
                        "logs": result.logs
                    }))?;
                    Ok(CallToolResult::success(vec![content]))
                }
            }
            Err(err) => {
                let content = Content::json(json!({
                    "success": false,
                    "error": err.to_string(),
                    "logs": []
                }))?;
                Ok(CallToolResult::success(vec![content]))
            }
        }
    }

    #[rmcp::tool(description = "执行Python代码并返回结果")]
    async fn run_python(
        &self,
        request: Parameters<CodeRunRequest>,
    ) -> Result<CallToolResult, McpError> {
        let request = request.0;
        match CodeExecutor::execute_with_params_compat(
            &request.code,
            LanguageScript::Python,
            request.params,
        )
        .await
        {
            Ok(result) => {
                if result.success {
                    let content = Content::json(json!({
                        "result": result.result,
                        "logs": result.logs,
                        "success": true
                    }))?;
                    Ok(CallToolResult::success(vec![content]))
                } else {
                    let content = Content::json(json!({
                        "success": false,
                        "error": result.error,
                        "logs": result.logs
                    }))?;
                    Ok(CallToolResult::success(vec![content]))
                }
            }
            Err(err) => {
                let content = Content::json(json!({
                    "success": false,
                    "error": err.to_string(),
                    "logs": []
                }))?;
                Ok(CallToolResult::success(vec![content]))
            }
        }
    }
}

impl ServerHandler for CodeRunnerService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some("一个支持执行JavaScript、TypeScript和Python代码的服务".to_string()),
        }
    }
}
