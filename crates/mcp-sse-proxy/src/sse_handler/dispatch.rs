use super::*;

/// Unified SSE server handler for direct and bridged backends.
#[derive(Clone, Debug)]
pub enum SseServerHandler {
    /// Standard SSE handler for Stdio/SSE URL backends
    Sse(SseHandler),
    /// Backend session handler for Streamable HTTP backends
    BackendSession(BackendSessionHandler),
}

impl ServerHandler for SseServerHandler {
    fn get_info(&self) -> ServerInfo {
        match self {
            SseServerHandler::Sse(h) => h.get_info(),
            SseServerHandler::BackendSession(h) => h.get_info(),
        }
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.list_tools(request, context).await,
            SseServerHandler::BackendSession(h) => h.list_tools(request, context).await,
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.call_tool(request, context).await,
            SseServerHandler::BackendSession(h) => h.call_tool(request, context).await,
        }
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.list_resources(request, context).await,
            SseServerHandler::BackendSession(h) => h.list_resources(request, context).await,
        }
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.read_resource(request, context).await,
            SseServerHandler::BackendSession(h) => h.read_resource(request, context).await,
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.list_resource_templates(request, context).await,
            SseServerHandler::BackendSession(h) => {
                h.list_resource_templates(request, context).await
            }
        }
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.list_prompts(request, context).await,
            SseServerHandler::BackendSession(h) => h.list_prompts(request, context).await,
        }
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.get_prompt(request, context).await,
            SseServerHandler::BackendSession(h) => h.get_prompt(request, context).await,
        }
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.complete(request, context).await,
            SseServerHandler::BackendSession(h) => h.complete(request, context).await,
        }
    }

    async fn on_progress(
        &self,
        notification: rmcp::model::ProgressNotificationParam,
        context: NotificationContext<RoleServer>,
    ) {
        match self {
            SseServerHandler::Sse(h) => h.on_progress(notification, context).await,
            SseServerHandler::BackendSession(h) => h.on_progress(notification, context).await,
        }
    }

    async fn on_cancelled(
        &self,
        notification: rmcp::model::CancelledNotificationParam,
        context: NotificationContext<RoleServer>,
    ) {
        match self {
            SseServerHandler::Sse(h) => h.on_cancelled(notification, context).await,
            SseServerHandler::BackendSession(h) => h.on_cancelled(notification, context).await,
        }
    }
}

impl SseServerHandler {
    /// 获取 MCP ID
    pub fn mcp_id(&self) -> &str {
        match self {
            SseServerHandler::Sse(h) => h.mcp_id(),
            SseServerHandler::BackendSession(h) => h.mcp_id(),
        }
    }

    /// 检查后端是否可用（快速检查，不发送请求）
    pub fn is_backend_available(&self) -> bool {
        match self {
            SseServerHandler::Sse(h) => h.is_backend_available(),
            SseServerHandler::BackendSession(h) => h.is_backend_available(),
        }
    }

    /// 检查 mcp 服务是否正常（异步版本，会发送验证请求）
    pub async fn is_mcp_server_ready(&self) -> bool {
        match self {
            SseServerHandler::Sse(h) => h.is_mcp_server_ready().await,
            SseServerHandler::BackendSession(h) => h.is_mcp_server_ready().await,
        }
    }

    /// 异步检查后端连接是否已断开（会发送验证请求）
    pub async fn is_terminated_async(&self) -> bool {
        match self {
            SseServerHandler::Sse(h) => h.is_terminated_async().await,
            SseServerHandler::BackendSession(h) => h.is_terminated_async().await,
        }
    }
}
