use super::*;

impl ProxyHandler {
    /// 获取 capabilities（从共享 discovery 快照加载）
    #[inline]
    pub(super) fn capabilities(&self) -> rmcp::model::ServerCapabilities {
        self.discovery.capabilities()
    }

    pub(super) fn filter_tools(&self, mut result: ListToolsResult) -> ListToolsResult {
        if self.tool_filter.is_enabled() {
            result
                .tools
                .retain(|tool| self.tool_filter.is_allowed(&tool.name));
        }
        result
    }

    pub(super) fn update_cached_tools(&self, tools: ListToolsResult) {
        self.discovery.update_tools(tools);
    }

    pub(super) fn update_discovery(&self, info: ServerInfo, tools: Option<ListToolsResult>) {
        self.discovery.update(info, tools);
    }

    /// Ensure backend is connected (lazy connect for per-session isolation).
    pub(super) async fn ensure_connected(&self) -> Result<(), ErrorData> {
        if self.is_backend_available() {
            return Ok(());
        }
        let Some(session) = &self.per_session else {
            return Err(ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            ));
        };

        let _lock = session.connect_lock.lock().await;
        if self.is_backend_available() {
            return Ok(());
        }

        let timeout = session.connector.connect_timeout();
        let slot = session.notify_slot.clone();
        let connect = session.connector.connect_session(slot);
        let result = tokio::time::timeout(timeout, connect).await;

        match result {
            Ok(Ok(running)) => {
                self.install_backend(running, true);
                Ok(())
            }
            Ok(Err(err)) => {
                error!(error = %err, "Per-session backend connect failed");
                Err(ErrorData::internal_error(
                    format!("Backend connect failed: {err}"),
                    None,
                ))
            }
            Err(_) => {
                error!(?timeout, "Per-session backend connect timed out");
                Err(ErrorData::internal_error(
                    format!("Backend connect timed out after {timeout:?}"),
                    None,
                ))
            }
        }
    }

    /// Install a backend without going through shared-mode registry checks.
    fn install_backend(&self, client: BackendRunningService, bump_version: bool) {
        use std::ops::Deref;
        let info = Self::extract_server_info(&client, &self.mcp_id);
        self.update_discovery(info, None);
        let peer = client.deref().clone();
        let inner = PeerInner {
            peer,
            _running: Arc::new(client),
        };
        self.peer.store(Some(Arc::new(inner)));
        if bump_version {
            let new_version = self.backend_version.fetch_add(1, Ordering::SeqCst) + 1;
            info!(
                "[ProxyHandler] Backend version update: {} - MCP ID: {}",
                new_version, self.mcp_id
            );
        }
    }

    pub(super) fn maybe_progress_guard(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Option<ProgressRouteGuard> {
        if self.isolation != BackendIsolation::Shared {
            return None;
        }
        ProgressRouteGuard::try_from_meta(
            self.upstream_peers.clone(),
            &context.meta,
            context.peer.clone(),
        )
    }

    /// Load backend peer, check closed, run op with cancel race.
    pub(super) async fn forward_backend<T, Fut, F>(
        &self,
        context: &RequestContext<RoleServer>,
        op: F,
    ) -> Result<T, ErrorData>
    where
        F: FnOnce(Peer<RoleClient>) -> Fut,
        Fut: Future<Output = Result<T, ServiceError>>,
    {
        self.ensure_connected().await?;

        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        let peer = inner.peer.clone();
        drop(inner_guard);

        tokio::select! {
            result = op(peer) => {
                result.map_err(|err| {
                    error!("Backend request error: {:?}", err);
                    ErrorData::internal_error(format!("Backend error: {err}"), None)
                })
            }
            _ = context.ct.cancelled() => {
                info!("[forward_backend] Request canceled - MCP ID: {}", self.mcp_id);
                Err(ErrorData::internal_error(
                    "Request cancelled".to_string(),
                    None,
                ))
            }
        }
    }

    /// Like [`Self::forward_backend`], but logs heartbeat while waiting (long tools/call).
    pub(super) async fn forward_backend_with_heartbeat<T, Fut, F>(
        &self,
        context: &RequestContext<RoleServer>,
        request_id: u64,
        tool_name: &str,
        op: F,
    ) -> Result<T, ErrorData>
    where
        F: FnOnce(Peer<RoleClient>) -> Fut,
        Fut: Future<Output = Result<T, ServiceError>>,
    {
        self.ensure_connected().await?;

        let peer = {
            let inner_guard = self.peer.load();
            let inner = inner_guard.as_ref().ok_or_else(|| {
                error!("Backend connection is not available (reconnecting)");
                ErrorData::internal_error(
                    "Backend connection is not available, reconnecting...".to_string(),
                    None,
                )
            })?;

            if inner.peer.is_transport_closed() {
                error!("Backend transport is closed");
                return Err(ErrorData::internal_error(
                    "Backend connection closed, please retry".to_string(),
                    None,
                ));
            }

            let peer = inner.peer.clone();
            drop(inner_guard);
            peer
        };

        let call_future = op(peer.clone());
        tokio::pin!(call_future);

        const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
        let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat_interval.tick().await;

        loop {
            tokio::select! {
                biased;
                result = &mut call_future => {
                    return result.map_err(|err| {
                        error!("Backend request error: {:?}", err);
                        ErrorData::internal_error(format!("Backend error: {err}"), None)
                    });
                }
                _ = context.ct.cancelled() => {
                    info!(
                        "[forward_backend_with_heartbeat:{}] Request canceled - Tool: {}, MCP ID: {}",
                        request_id, tool_name, self.mcp_id
                    );
                    return Err(ErrorData::internal_error(
                        "Request cancelled".to_string(),
                        None,
                    ));
                }
                _ = heartbeat_interval.tick() => {
                    info!(
                        "[forward_backend_with_heartbeat:{}] Waiting for backend... - Tool: {}, transport_closed: {}, MCP ID: {}",
                        request_id,
                        tool_name,
                        peer.is_transport_closed(),
                        self.mcp_id
                    );
                }
            }
        }
    }

    pub(super) fn finish_call_tool(
        &self,
        request_id: u64,
        tool_name: &str,
        elapsed: std::time::Duration,
        result: Result<CallToolResponse, ErrorData>,
    ) -> CallToolResponse {
        match result {
            Ok(response) => {
                self.log_call_tool_response(request_id, tool_name, elapsed, &response);
                response
            }
            Err(error) => self.call_tool_error_response(request_id, tool_name, elapsed, error),
        }
    }

    fn log_call_tool_response(
        &self,
        request_id: u64,
        tool_name: &str,
        elapsed: std::time::Duration,
        response: &CallToolResponse,
    ) {
        match response {
            CallToolResponse::Complete(call_result) => {
                let is_error = call_result.is_error.unwrap_or(false);
                info!(
                    "[call_tool:{}] Response received - tool: {}, time taken: {}ms, is_error: {}, MCP ID: {}",
                    request_id,
                    tool_name,
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
            }
            CallToolResponse::InputRequired(_) => {
                info!(
                    "[call_tool:{}] InputRequired received - tool: {}, time taken: {}ms, MCP ID: {}",
                    request_id,
                    tool_name,
                    elapsed.as_millis(),
                    self.mcp_id
                );
            }
            _ => {
                info!(
                    "[call_tool:{}] Response received - tool: {}, time taken: {}ms, MCP ID: {}",
                    request_id,
                    tool_name,
                    elapsed.as_millis(),
                    self.mcp_id
                );
            }
        }
    }

    fn call_tool_error_response(
        &self,
        request_id: u64,
        tool_name: &str,
        elapsed: std::time::Duration,
        error: ErrorData,
    ) -> CallToolResponse {
        let message = error.message;
        if message == "Request cancelled" {
            warn!(
                "[call_tool:{}] Request canceled - Tool: {}, Time taken: {}ms, MCP ID: {}",
                request_id,
                tool_name,
                elapsed.as_millis(),
                self.mcp_id
            );
            return CallToolResult::error(vec![ContentBlock::text("Request cancelled")]).into();
        }
        if message.contains("Backend connection is not available")
            || message.contains("Backend unavailable")
            || message.contains("Backend connect")
        {
            error!(
                "[call_tool:{}] Backend unavailable - Tool: {}, Error: {}, MCP ID: {}",
                request_id, tool_name, message, self.mcp_id
            );
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "Backend unavailable: {message}"
            ))])
            .into();
        }
        if message.contains("Backend connection closed") {
            error!(
                "[call_tool:{}] Backend transport is closed - MCP ID: {}",
                request_id, self.mcp_id
            );
            return CallToolResult::error(vec![ContentBlock::text(
                "Backend connection closed, please retry",
            )])
            .into();
        }
        error!(
            "[call_tool:{}] Backend returns error - Tool: {}, Time: {}ms, Error: {}, MCP ID: {}",
            request_id,
            tool_name,
            elapsed.as_millis(),
            message,
            self.mcp_id
        );
        CallToolResult::error(vec![ContentBlock::text(format!("Error: {message}"))]).into()
    }
}
