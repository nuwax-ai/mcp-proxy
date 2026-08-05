use super::*;

impl ServerHandler for ProxyHandler {
    fn get_info(&self) -> ServerInfo {
        self.discovery.info()
    }

    /// Connect per-session backends before returning capabilities.
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());

        if let Some(session) = &self.per_session {
            // Bind notify slot before connect so backend notifications during handshake work.
            session.notify_slot.set(context.peer.clone());
            self.ensure_connected().await?;
            info!(
                "[initialize] Per-session backend ready - MCP ID: {}",
                self.mcp_id
            );
        }

        let mut info = self.get_info();
        info.protocol_version =
            negotiate_protocol_version(&request.protocol_version, info.protocol_version);
        Ok(info)
    }

    #[tracing::instrument(skip(self, request, context), fields(
        mcp_id = %self.mcp_id,
        request = ?request,
    ))]
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
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

        match self.capabilities().tools {
            Some(_) => {
                let result = match self
                    .forward_backend(
                        &context,
                        |peer| async move { peer.list_tools(request).await },
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(error) if context.ct.is_cancelled() => return Err(error),
                    Err(error) => {
                        if let Some(cached) = self.discovery.tools() {
                            warn!(%error, "real tools/list failed; returning cached tools snapshot");
                            return Ok(self.filter_tools(cached));
                        }
                        return Err(error);
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
        mut request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let request_id = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();
        let tool_name = request.name.clone();

        info!(
            "[call_tool:{}] Start - Tool: {}, MCP ID: {}",
            request_id, tool_name, self.mcp_id
        );

        if !self.tool_filter.is_allowed(&tool_name) {
            info!(
                "[call_tool:{}] Tool is filtered - MCP ID: {}, Tool: {}",
                request_id, self.mcp_id, tool_name
            );
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Tool '{tool_name}' is not allowed by filter configuration"
            ))])
            .into());
        }

        if self.capabilities().tools.is_none() {
            error!(
                "[call_tool:{}] The server does not support tools capability - MCP ID: {}",
                request_id, self.mcp_id
            );
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "Server doesn't support tools capability",
            )])
            .into());
        }

        merge_context_meta_into_params(&mut request, &context.meta);
        let _progress_guard = self.maybe_progress_guard(&context);

        info!(
            "[call_tool:{}] Send request to backend... - Tool: {}, Elapsed time: {}ms",
            request_id,
            tool_name,
            start.elapsed().as_millis()
        );

        let call_result = self
            .forward_backend_with_heartbeat(
                &context,
                request_id,
                tool_name.as_ref(),
                |peer| async move { peer.call_tool_once(request).await },
            )
            .await;

        let result =
            self.finish_call_tool(request_id, tool_name.as_ref(), start.elapsed(), call_result);

        // SEP-2663: when the backend materializes a task for this call, register
        // its notification route so backend task-status updates reach the caller.
        // Shared isolation routes via the registry; per-session bridges 1:1 via
        // notify_slot and needs no registry entry.
        if let CallToolResponse::Task(ref task_result) = result
            && self.isolation == BackendIsolation::Shared
        {
            self.upstream_peers
                .register_task(task_result.task.task_id.clone(), context.peer.clone());
        }

        info!(
            "[call_tool:{}] Completed - Tool: {}, total time taken: {}ms",
            request_id,
            tool_name,
            start.elapsed().as_millis()
        );
        Ok(result)
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        match self.capabilities().resources {
            Some(_) => {
                let result = self
                    .forward_backend(&context, |peer| async move {
                        peer.list_resources(request).await
                    })
                    .await?;
                info!(
                    "[list_resources] Resource list results - MCP ID: {}, resource quantity: {}",
                    self.mcp_id,
                    result.resources.len()
                );
                debug!("Proxying list_resources response");
                Ok(result)
            }
            None => {
                warn!("Server doesn't support resources capability");
                Ok(rmcp::model::ListResourcesResult::default())
            }
        }
    }

    async fn read_resource(
        &self,
        mut request: rmcp::model::ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        match self.capabilities().resources {
            Some(_) => {
                merge_context_meta_into_params(&mut request, &context.meta);
                let _progress_guard = self.maybe_progress_guard(&context);
                let uri = request.uri.clone();
                let result = self
                    .forward_backend(&context, |peer| async move {
                        peer.read_resource_once(request).await
                    })
                    .await?;
                info!(
                    "[read_resource] Resource read result - MCP ID: {}, URI: {}",
                    self.mcp_id, uri
                );
                debug!("Proxying read_resource response for {}", uri);
                Ok(result)
            }
            None => {
                error!("Server doesn't support resources capability");
                Ok(rmcp::model::ReadResourceResult::new(vec![]).into())
            }
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        match self.capabilities().resources {
            Some(_) => {
                let result = self
                    .forward_backend(&context, |peer| async move {
                        peer.list_resource_templates(request).await
                    })
                    .await?;
                debug!("Proxying list_resource_templates response");
                Ok(result)
            }
            None => {
                warn!("Server doesn't support resources capability");
                Ok(rmcp::model::ListResourceTemplatesResult::default())
            }
        }
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        match self.capabilities().prompts {
            Some(_) => {
                let result =
                    self.forward_backend(&context, |peer| async move {
                        peer.list_prompts(request).await
                    })
                    .await?;
                debug!("Proxying list_prompts response");
                Ok(result)
            }
            None => {
                warn!("Server doesn't support prompts capability");
                Ok(rmcp::model::ListPromptsResult::default())
            }
        }
    }

    async fn get_prompt(
        &self,
        mut request: rmcp::model::GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        match self.capabilities().prompts {
            Some(_) => {
                merge_context_meta_into_params(&mut request, &context.meta);
                let _progress_guard = self.maybe_progress_guard(&context);
                let result = self
                    .forward_backend(&context, |peer| async move {
                        peer.get_prompt_once(request).await
                    })
                    .await?;
                debug!("Proxying get_prompt response");
                Ok(result)
            }
            None => {
                warn!("Server doesn't support prompts capability");
                let messages = Vec::new();
                Ok(rmcp::model::GetPromptResult::new(messages).into())
            }
        }
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        let result = self
            .forward_backend(&context, |peer| async move { peer.complete(request).await })
            .await?;
        debug!("Proxying complete response");
        Ok(result)
    }

    #[allow(deprecated)]
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        match self
            .capabilities()
            .resources
            .as_ref()
            .and_then(|r| r.subscribe)
        {
            Some(true) => {
                self.forward_backend(
                    &context,
                    |peer| async move { peer.subscribe(request).await },
                )
                .await
            }
            _ => Err(ErrorData::method_not_found::<SubscribeRequestMethod>()),
        }
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        match self
            .capabilities()
            .resources
            .as_ref()
            .and_then(|r| r.subscribe)
        {
            Some(true) => {
                self.forward_backend(
                    &context,
                    |peer| async move { peer.unsubscribe(request).await },
                )
                .await
            }
            _ => Err(ErrorData::method_not_found::<UnsubscribeRequestMethod>()),
        }
    }

    #[allow(deprecated)]
    async fn set_level(
        &self,
        request: rmcp::model::SetLevelRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if self.capabilities().logging.is_none() {
            return Err(ErrorData::method_not_found::<SetLevelRequestMethod>());
        }
        self.forward_backend(
            &context,
            |peer| async move { peer.set_level(request).await },
        )
        .await
    }

    async fn get_task(
        &self,
        request: GetTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, ErrorData> {
        if !self.capabilities().supports_tasks() {
            return Err(ErrorData::method_not_found::<rmcp::model::GetTaskMethod>());
        }
        self.forward_backend(&context, |peer| async move {
            let result = peer
                .send_request(ClientRequest::GetTaskRequest(GetTaskRequest::new(request)))
                .await?;
            match result {
                ServerResult::GetTaskResult(r) => Ok(r),
                _ => Err(ServiceError::UnexpectedResponse),
            }
        })
        .await
    }

    async fn update_task(
        &self,
        request: rmcp::model::UpdateTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if !self.capabilities().supports_tasks() {
            return Err(ErrorData::method_not_found::<rmcp::model::UpdateTaskMethod>());
        }
        self.forward_backend(
            &context,
            |peer| async move { peer.update_task(request).await },
        )
        .await
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if !self.capabilities().supports_tasks() {
            return Err(ErrorData::method_not_found::<rmcp::model::CancelTaskMethod>());
        }
        let task_id = request.task_id.clone();
        self.forward_backend(
            &context,
            |peer| async move { peer.cancel_task(request).await },
        )
        .await?;
        self.upstream_peers.unregister_task(&task_id);
        Ok(())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if let Some(session) = &self.per_session {
            // Refresh slot (initialize already connected); keep idempotent ensure.
            session.notify_slot.set(context.peer.clone());
            if let Err(err) = self.ensure_connected().await {
                error!(
                    "[on_initialized] Per-session backend ensure failed - MCP ID: {}, err: {:?}",
                    self.mcp_id, err
                );
            }
            return;
        }

        let id = self.upstream_peers.register(context.peer.clone());
        info!(
            "[on_initialized] Registered upstream peer {} - MCP ID: {}, peers: {}",
            id,
            self.mcp_id,
            self.upstream_peers.len()
        );
    }

    async fn on_progress(
        &self,
        notification: rmcp::model::ProgressNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        let peer = {
            let inner_guard = self.peer.load();
            let inner = match inner_guard.as_ref() {
                Some(inner) => inner,
                None => {
                    error!(
                        "Backend connection is not available, cannot forward progress notification"
                    );
                    return;
                }
            };
            if inner.peer.is_transport_closed() {
                error!("Backend transport is closed, cannot forward progress notification");
                return;
            }
            inner.peer.clone()
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
        let peer = {
            let inner_guard = self.peer.load();
            let inner = match inner_guard.as_ref() {
                Some(inner) => inner,
                None => {
                    error!(
                        "Backend connection is not available, cannot forward cancelled notification"
                    );
                    return;
                }
            };
            if inner.peer.is_transport_closed() {
                error!("Backend transport is closed, cannot forward cancelled notification");
                return;
            }
            inner.peer.clone()
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

    /// Forward non-standard / legacy methods to the backend verbatim.
    ///
    /// rmcp 3.1.0 routes any method it does not recognise as a standard MCP
    /// method here — including spec-removed ones like `tasks/list` and
    /// `tasks/result` that older clients/backends may still use, plus any vendor
    /// extension. Delegating to [`ProxyHandler::forward_to_backend`] keeps the
    /// proxy a fully transparent, version-tolerant bridge: every non-standard
    /// method is forwarded as a custom request and the backend is the source of
    /// truth. Standard methods never reach this handler.
    async fn on_custom_request(
        &self,
        request: rmcp::model::CustomRequest,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CustomResult, ErrorData> {
        let method = request.method.clone();
        let params = request.params.unwrap_or_default();
        let value = self
            .forward_to_backend(&method, params)
            .await
            .map_err(|error| {
                ErrorData::internal_error(format!("{method} backend error: {error}"), None)
            })?;
        Ok(rmcp::model::CustomResult::new(value))
    }
}
