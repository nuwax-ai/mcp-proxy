use super::*;

/// 包装后端连接和运行服务
/// 用于 ArcSwap 热替换
#[derive(Debug)]
pub(super) struct PeerInner {
    /// Peer 用于发送请求
    pub(super) peer: Peer<RoleClient>,
    /// 保持 RunningService 的所有权，确保服务生命周期
    #[allow(dead_code)]
    pub(super) _running: Arc<RunningService<RoleClient, ClientInfo>>,
}

/// A SSE proxy handler that forwards requests to a client based on the server's capabilities
/// 使用 ArcSwap 实现后端热替换，支持断开时立即返回错误
///
/// **SSE 模式**：使用 rmcp 0.10，稳定的 SSE 传输协议
#[derive(Clone, Debug)]
pub struct SseHandler {
    /// 后端连接（ArcSwap 支持无锁原子替换）
    /// None 表示后端断开/重连中
    pub(super) peer: Arc<ArcSwapOption<PeerInner>>,
    /// 所有 clone 共享、原子更新的发现快照
    pub(super) discovery: DiscoveryCache,
    /// MCP ID 用于日志记录
    pub(super) mcp_id: String,
    /// 工具过滤配置
    pub(super) tool_filter: ToolFilter,
}

impl ServerHandler for SseHandler {
    fn get_info(&self) -> ServerInfo {
        self.discovery.info()
    }

    #[tracing::instrument(skip(self, request, context), fields(
        mcp_id = %self.mcp_id,
        request = ?request,
    ))]
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if context.ct.is_cancelled() {
            return Err(ErrorData::internal_error(
                "Request cancelled".to_string(),
                None,
            ));
        }

        let is_first_page = request
            .as_ref()
            .and_then(|params| params.cursor.as_ref())
            .is_none();

        // Check if the server has tools capability and forward the request
        match self.capabilities().tools {
            Some(_) => {
                let result = match self
                    .forward_backend(&context, "tools/list", |peer| async move {
                        peer.list_tools(request).await
                    })
                    .await
                {
                    Ok(result) => result,
                    Err(error) if error.is_cancelled() => return Err(error.into_error_data()),
                    Err(error) => {
                        return self.cached_tools_or_error(&error.message());
                    }
                };
                if is_first_page && result.next_cursor.is_none() {
                    self.update_cached_tools(result.clone());
                }
                let result = self.filter_tools(result);
                info!(
                    "[list_tools] Tool list results - MCP ID: {}, number of tools: {}{}",
                    self.mcp_id,
                    result.tools.len(),
                    if self.tool_filter.is_enabled() {
                        " (filtered)"
                    } else {
                        ""
                    }
                );
                debug!(
                    "Proxying list_tools response with {} tools",
                    result.tools.len()
                );
                Ok(result)
            }
            None => {
                // Server doesn't support tools, return empty list
                warn!("Server doesn't support tools capability");
                Ok(ListToolsResult::default())
            }
        }
    }

    #[tracing::instrument(skip(self, request, context), fields(
        mcp_id = %self.mcp_id,
        tool_name = %request.name,
        tool_arguments = ?request.arguments,
    ))]
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // 生成唯一请求 ID 用于追踪
        let request_id = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();
        let start_time = SystemTime::now();

        info!(
            "[call_tool:{}] Start - Tool: {}, MCP ID: {}, Time: {:?}",
            request_id, request.name, self.mcp_id, start_time
        );

        // 首先检查工具是否被过滤
        if !self.tool_filter.is_allowed(&request.name) {
            info!(
                "[call_tool:{}] Tool is filtered - MCP ID: {}, Tool: {}",
                request_id, self.mcp_id, request.name
            );
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Tool '{}' is not allowed by filter configuration",
                request.name
            ))]));
        }

        // Check if the server has tools capability and forward the request
        let result = match self.capabilities().tools {
            Some(_) => {
                info!(
                    "[call_tool:{}] Send request to backend... - Tool: {}, Elapsed time: {}ms",
                    request_id,
                    request.name,
                    start.elapsed().as_millis()
                );

                let forwarded = self
                    .forward_backend(&context, "tools/call", |peer| {
                        let request = request.clone();
                        async move { peer.call_tool(request).await }
                    })
                    .await;
                let elapsed = start.elapsed();
                match forwarded {
                    Ok(call_result) => {
                        let is_error = call_result.is_error.unwrap_or(false);
                        info!(
                            "[call_tool:{}] Response received - tool: {}, time taken: {}ms, is_error: {}, MCP ID: {}",
                            request_id,
                            request.name,
                            elapsed.as_millis(),
                            is_error,
                            self.mcp_id
                        );
                        if is_error {
                            debug!(
                                "[call_tool:{}] Error response content: {:?}",
                                request_id, call_result.content
                            );
                        }
                        Ok(call_result)
                    }
                    Err(ForwardError::Cancelled) => {
                        warn!(
                            "[call_tool:{}] Request canceled - Tool: {}, Time taken: {}ms, MCP ID: {}",
                            request_id,
                            request.name,
                            elapsed.as_millis(),
                            self.mcp_id
                        );
                        Ok(CallToolResult::error(vec![Content::text(
                            "Request cancelled",
                        )]))
                    }
                    Err(ForwardError::Backend(error)) => {
                        error!(
                            "[call_tool:{}] Backend returns error - Tool: {}, Time: {}ms, Error: {:?}, MCP ID: {}",
                            request_id,
                            request.name,
                            elapsed.as_millis(),
                            error,
                            self.mcp_id
                        );
                        Ok(CallToolResult::error(vec![Content::text(format!(
                            "Error: {error}"
                        ))]))
                    }
                    Err(error) => {
                        let message = error.message();
                        error!(
                            "[call_tool:{}] Backend unavailable - Tool: {}, Time: {}ms, Error: {}, MCP ID: {}",
                            request_id,
                            request.name,
                            elapsed.as_millis(),
                            message,
                            self.mcp_id
                        );
                        Ok(CallToolResult::error(vec![Content::text(message)]))
                    }
                }
            }
            None => {
                error!(
                    "[call_tool:{}] The server does not support tools capability - MCP ID: {}",
                    request_id, self.mcp_id
                );
                Ok(CallToolResult::error(vec![Content::text(
                    "Server doesn't support tools capability",
                )]))
            }
        };

        let total_elapsed = start.elapsed();
        info!(
            "[call_tool:{}] Completed - Tool: {}, total time taken: {}ms",
            request_id,
            request.name,
            total_elapsed.as_millis()
        );
        result
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        if self.capabilities().resources.is_none() {
            warn!("Server doesn't support resources capability");
            return Ok(rmcp::model::ListResourcesResult::default());
        }
        let result = self
            .forward_backend(&context, "resources/list", |peer| async move {
                peer.list_resources(request).await
            })
            .await
            .map_err(ForwardError::into_error_data)?;
        info!(
            "[list_resources] Resource list results - MCP ID: {}, resource quantity: {}",
            self.mcp_id,
            result.resources.len()
        );
        Ok(result)
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, ErrorData> {
        if self.capabilities().resources.is_none() {
            error!("Server doesn't support resources capability");
            return Ok(rmcp::model::ReadResourceResult {
                contents: Vec::new(),
            });
        }
        let uri = request.uri.clone();
        let result = self
            .forward_backend(&context, "resources/read", |peer| async move {
                peer.read_resource(request).await
            })
            .await
            .map_err(ForwardError::into_error_data)?;
        info!(
            "[read_resource] Resource read result - MCP ID: {}, URI: {}",
            self.mcp_id, uri
        );
        Ok(result)
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        if self.capabilities().resources.is_none() {
            warn!("Server doesn't support resources capability");
            return Ok(rmcp::model::ListResourceTemplatesResult::default());
        }
        self.forward_backend(&context, "resources/templates/list", |peer| async move {
            peer.list_resource_templates(request).await
        })
        .await
        .map_err(ForwardError::into_error_data)
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        if self.capabilities().prompts.is_none() {
            warn!("Server doesn't support prompts capability");
            return Ok(rmcp::model::ListPromptsResult::default());
        }
        self.forward_backend(&context, "prompts/list", |peer| async move {
            peer.list_prompts(request).await
        })
        .await
        .map_err(ForwardError::into_error_data)
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResult, ErrorData> {
        if self.capabilities().prompts.is_none() {
            warn!("Server doesn't support prompts capability");
            return Ok(rmcp::model::GetPromptResult {
                description: None,
                messages: Vec::new(),
            });
        }
        self.forward_backend(&context, "prompts/get", |peer| async move {
            peer.get_prompt(request).await
        })
        .await
        .map_err(ForwardError::into_error_data)
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        self.forward_backend(&context, "completion/complete", |peer| async move {
            peer.complete(request).await
        })
        .await
        .map_err(ForwardError::into_error_data)
    }

    async fn on_progress(
        &self,
        notification: rmcp::model::ProgressNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        let peer = match self.backend_peer() {
            Ok(peer) => peer,
            Err(error) => {
                error!(%error, "Cannot forward progress notification");
                return;
            }
        };
        match peer.notify_progress(notification).await {
            Ok(_) => {
                debug!("Proxying progress notification");
            }
            Err(err) => {
                error!("Error notifying progress: {:?}", err);
            }
        }
    }

    async fn on_cancelled(
        &self,
        notification: rmcp::model::CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        let peer = match self.backend_peer() {
            Ok(peer) => peer,
            Err(error) => {
                error!(%error, "Cannot forward cancelled notification");
                return;
            }
        };
        match peer.notify_cancelled(notification).await {
            Ok(_) => {
                debug!("Proxying cancelled notification");
            }
            Err(err) => {
                error!("Error notifying cancelled: {:?}", err);
            }
        }
    }
}
