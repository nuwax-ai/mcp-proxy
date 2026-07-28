use serde::Serialize;
use serde::de::DeserializeOwned;

use super::*;

/// A handler that bridges an external backend to the SSE server.
///
/// Uses the version-independent `BackendBridge` JSON boundary.
#[derive(Clone)]
pub struct BackendSessionHandler {
    backend: Arc<dyn mcp_common::BackendBridge>,
    mcp_id: String,
    cached_info: rmcp::model::ServerInfo,
}

impl std::fmt::Debug for BackendSessionHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BackendSessionHandler")
            .field("mcp_id", &self.mcp_id)
            .field("cached_info", &self.cached_info)
            .finish()
    }
}

impl BackendSessionHandler {
    pub fn new(backend: Arc<dyn mcp_common::BackendBridge>, mcp_id: String) -> Self {
        let mut cached_info = match serde_json::from_value(backend.get_server_info_json()) {
            Ok(info) => info,
            Err(error) => {
                warn!(
                    %error,
                    %mcp_id,
                    "Failed to deserialize backend ServerInfo; using SSE default"
                );
                rmcp::model::ServerInfo::default()
            }
        };

        let backend_version = cached_info.protocol_version.clone();
        cached_info.protocol_version = rmcp::model::ProtocolVersion::V_2024_11_05;
        info!(
            %mcp_id,
            ?backend_version,
            sse_version = ?cached_info.protocol_version,
            "Created bridged SSE backend session"
        );

        Self {
            backend,
            mcp_id,
            cached_info,
        }
    }

    pub fn mcp_id(&self) -> &str {
        &self.mcp_id
    }

    pub fn is_backend_available(&self) -> bool {
        self.backend.is_backend_available()
    }

    pub async fn is_mcp_server_ready(&self) -> bool {
        self.backend.is_mcp_server_ready().await
    }

    pub async fn is_terminated_async(&self) -> bool {
        self.backend.is_terminated_async().await
    }

    async fn call_backend<Req, Res>(
        &self,
        method: &'static str,
        request: &Req,
        context: &RequestContext<RoleServer>,
    ) -> Result<Res, ErrorData>
    where
        Req: Serialize + Sync + ?Sized,
        Res: DeserializeOwned,
    {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        let params = serde_json::to_value(request).map_err(|error| {
            ErrorData::internal_error(
                format!("failed to serialize {method} request: {error}"),
                None,
            )
        })?;

        tokio::select! {
            result = self.backend.call_peer_method(method, params) => {
                let value = result.map_err(|error| {
                    ErrorData::internal_error(format!("{method} backend error: {error}"), None)
                })?;
                serde_json::from_value(value).map_err(|error| {
                    ErrorData::internal_error(
                        format!("failed to deserialize {method} response: {error}"),
                        None,
                    )
                })
            }
            _ = context.ct.cancelled() => Err(cancelled_error()),
        }
    }
}

fn cancelled_error() -> ErrorData {
    ErrorData::internal_error("Request cancelled".to_string(), None)
}

impl ServerHandler for BackendSessionHandler {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        self.cached_info.clone()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let started = Instant::now();
        let result: ListToolsResult = self.call_backend("tools/list", &request, &context).await?;
        info!(
            mcp_id = %self.mcp_id,
            tool_count = result.tools.len(),
            elapsed_ms = started.elapsed().as_millis(),
            "Bridged tools/list completed"
        );
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let started = Instant::now();
        let tool_name = request.name.clone();
        let result: CallToolResult = self.call_backend("tools/call", &request, &context).await?;
        info!(
            mcp_id = %self.mcp_id,
            %tool_name,
            is_error = result.is_error.unwrap_or(false),
            elapsed_ms = started.elapsed().as_millis(),
            "Bridged tools/call completed"
        );
        Ok(result)
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        self.call_backend("resources/list", &request, &context)
            .await
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, ErrorData> {
        self.call_backend("resources/read", &request, &context)
            .await
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        self.call_backend("resources/templates/list", &request, &context)
            .await
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        self.call_backend("prompts/list", &request, &context).await
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResult, ErrorData> {
        self.call_backend("prompts/get", &request, &context).await
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        self.call_backend("completion/complete", &request, &context)
            .await
    }

    async fn on_progress(
        &self,
        _notification: rmcp::model::ProgressNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        debug!(
            mcp_id = %self.mcp_id,
            "Progress notification is not forwarded across BackendBridge"
        );
    }

    async fn on_cancelled(
        &self,
        _notification: rmcp::model::CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        warn!(
            mcp_id = %self.mcp_id,
            "Cancellation notification is not forwarded across BackendBridge"
        );
    }
}
