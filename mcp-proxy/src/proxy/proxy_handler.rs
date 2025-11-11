use log::{debug, info};
/**
 * Create a local SSE server that proxies requests to a stdio MCP server.
 */
use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler,
    model::{
        CallToolRequestParam, CallToolResult, ClientInfo, Content, Implementation, ListToolsResult,
        PaginatedRequestParam, ServerInfo,
    },
    service::{NotificationContext, RequestContext, RunningService},
};
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;

/// A proxy handler that forwards requests to a client based on the server's capabilities
#[derive(Clone, Debug)]
pub struct ProxyHandler {
    client: Arc<Mutex<RunningService<RoleClient, ClientInfo>>>,
    // Store the server's capabilities to avoid locking the client on every get_info call
    cached_info: Arc<RwLock<Option<ServerInfo>>>,
    // MCP ID 用于日志记录
    mcp_id: String,
}

impl ServerHandler for ProxyHandler {
    fn get_info(&self) -> ServerInfo {
        // 首先检查缓存的信息
        if let Ok(cached_read) = self.cached_info.read() {
            if let Some(ref cached) = *cached_read {
                return cached.clone();
            }
        }

        // 如果缓存为空，尝试动态获取
        // 使用 try_lock 而不是 lock，避免阻塞
        // peer_info() 是同步方法，可以安全调用
        let client = self.client.clone();
        if let Ok(guard) = client.try_lock() {
            if let Some(peer_info) = guard.peer_info() {
                let server_info = ServerInfo {
                    protocol_version: peer_info.protocol_version.clone(),
                    server_info: Implementation {
                        name: peer_info.server_info.name.clone(),
                        version: peer_info.server_info.version.clone(),
                        title: None,
                        website_url: None,
                        icons: None,
                    },
                    instructions: peer_info.instructions.clone(),
                    capabilities: peer_info.capabilities.clone(),
                };

                // 将动态获取的信息缓存起来
                if let Ok(mut cached_write) = self.cached_info.write() {
                    *cached_write = Some(server_info.clone());
                    debug!("Successfully cached server info from peer_info");
                }

                return server_info;
            }
        }

        // 如果都获取不到，返回错误状态信息
        ServerInfo {
            protocol_version: Default::default(),
            server_info: Implementation {
                name: "MCP Proxy - Service Unavailable".to_string(),
                version: "0.1.0".to_string(),
                title: None,
                website_url: None,
                icons: None,
            },
            instructions: Some("ERROR: MCP service is not available or still initializing. Please try again later.".to_string()),
            capabilities: Default::default(), // 空的能力列表，表示服务不可用
        }
    }

    #[tracing::instrument(skip(self, request, _context), fields(
        mcp_id = %self.mcp_id,
        request = ?request,
    ))]
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let client = self.client.clone();
        let guard = client.lock().await;

        // Check if the server has tools capability and forward the request
        match self.get_info().capabilities.tools {
            Some(_) => {
                match guard.list_tools(request).await {
                    // Forward request to client
                    Ok(result) => {
                        // 记录工具列表结果，这些结果会通过 SSE 推送给客户端
                        info!(
                            "[list_tools] 工具列表结果 - MCP ID: {}, 工具数量: {}",
                            self.mcp_id,
                            result.tools.len()
                        );

                        debug!(
                            "Proxying list_tools response with {} tools",
                            result.tools.len()
                        );
                        Ok(result)
                    }
                    Err(err) => {
                        tracing::error!("Error listing tools: {:?}", err);
                        // Return empty list instead of error
                        Ok(ListToolsResult::default())
                    }
                }
            }
            None => {
                // Server doesn't support tools, return empty list
                tracing::error!("Server doesn't support tools capability");
                Ok(ListToolsResult::default())
            }
        }
    }

    #[tracing::instrument(skip(self, request, _context), fields(
        mcp_id = %self.mcp_id,
        tool_name = %request.name,
        tool_arguments = ?request.arguments,
    ))]
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client.clone();
        let guard = client.lock().await;

        // Check if the server has tools capability and forward the request
        match self.get_info().capabilities.tools {
            Some(_) => {
                match guard.call_tool(request.clone()).await {
                    Ok(result) => {
                        // 记录工具调用结果，这些结果会通过 SSE 推送给客户端
                        info!(
                            "[call_tool] 工具调用结果 - MCP ID: {}, 工具: {}",
                            self.mcp_id,
                            request.name
                        );

                        debug!("Tool call succeeded");
                        Ok(result)
                    }
                    Err(err) => {
                        tracing::error!("Error calling tool: {:?}", err);
                        // Return an error result instead of propagating the error
                        Ok(CallToolResult::error(vec![Content::text(format!(
                            "Error: {err}"
                        ))]))
                    }
                }
            }
            None => {
                tracing::error!("Server doesn't support tools capability");
                Ok(CallToolResult::error(vec![Content::text(
                    "Server doesn't support tools capability",
                )]))
            }
        }
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        // Get a lock on the client
        let client = self.client.clone();
        let guard = client.lock().await;

        // Check if the server has resources capability and forward the request
        match self.get_info().capabilities.resources {
            Some(_) => {
                // Forward request to client
                match guard.list_resources(request).await {
                    Ok(result) => {
                        // 记录资源列表结果，这些结果会通过 SSE 推送给客户端
                        info!(
                            "[list_resources] 资源列表结果 - MCP ID: {}, 资源数量: {}",
                            self.mcp_id,
                            result.resources.len()
                        );

                        debug!("Proxying list_resources response");
                        Ok(result)
                    }
                    Err(err) => {
                        tracing::error!("Error listing resources: {:?}", err);
                        // Return empty list instead of error
                        Ok(rmcp::model::ListResourcesResult::default())
                    }
                }
            }
            None => {
                // Server doesn't support resources, return empty list
                tracing::error!("Server doesn't support resources capability");
                Ok(rmcp::model::ListResourcesResult::default())
            }
        }
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, ErrorData> {
        // Get a lock on the client
        let client = self.client.clone();
        let guard = client.lock().await;

        // Check if the server has resources capability and forward the request
        match self.get_info().capabilities.resources {
            Some(_) => {
                // Forward request to client
                match guard
                    .read_resource(rmcp::model::ReadResourceRequestParam {
                        uri: request.uri.clone(),
                    })
                    .await
                {
                    Ok(result) => {
                        // 记录资源读取结果，这些结果会通过 SSE 推送给客户端
                        info!(
                            "[read_resource] 资源读取结果 - MCP ID: {}, URI: {}",
                            self.mcp_id,
                            request.uri
                        );

                        debug!("Proxying read_resource response for {}", request.uri);
                        Ok(result)
                    }
                    Err(err) => {
                        tracing::error!("Error reading resource: {:?}", err);
                        Err(ErrorData::internal_error(
                            format!("Error reading resource: {err}"),
                            None,
                        ))
                    }
                }
            }
            None => {
                // Server doesn't support resources, return error
                tracing::error!("Server doesn't support resources capability");
                Ok(rmcp::model::ReadResourceResult {
                    contents: Vec::new(),
                })
            }
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        // Get a lock on the client
        let client = self.client.clone();
        let guard = client.lock().await;

        // Check if the server has resources capability and forward the request
        match self.get_info().capabilities.resources {
            Some(_) => {
                // Forward request to client
                match guard.list_resource_templates(request).await {
                    Ok(result) => {
                        debug!("Proxying list_resource_templates response");
                        Ok(result)
                    }
                    Err(err) => {
                        tracing::error!("Error listing resource templates: {:?}", err);
                        // Return empty list instead of error
                        Ok(rmcp::model::ListResourceTemplatesResult::default())
                    }
                }
            }
            None => {
                // Server doesn't support resources, return empty list
                tracing::error!("Server doesn't support resources capability");
                Ok(rmcp::model::ListResourceTemplatesResult::default())
            }
        }
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        // Get a lock on the client
        let client = self.client.clone();
        let guard = client.lock().await;

        // Check if the server has prompts capability and forward the request
        match self.get_info().capabilities.prompts {
            Some(_) => {
                // Forward request to client
                match guard.list_prompts(request).await {
                    Ok(result) => {
                        debug!("Proxying list_prompts response");
                        Ok(result)
                    }
                    Err(err) => {
                        tracing::error!("Error listing prompts: {:?}", err);
                        // Return empty list instead of error
                        Ok(rmcp::model::ListPromptsResult::default())
                    }
                }
            }
            None => {
                // Server doesn't support prompts, return empty list
                tracing::warn!("Server doesn't support prompts capability");
                Ok(rmcp::model::ListPromptsResult::default())
            }
        }
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResult, ErrorData> {
        // Get a lock on the client
        let client = self.client.clone();
        let guard = client.lock().await;

        // Check if the server has prompts capability and forward the request
        match self.get_info().capabilities.prompts {
            Some(_) => {
                // Forward request to client
                match guard.get_prompt(request).await {
                    Ok(result) => {
                        debug!("Proxying get_prompt response");
                        Ok(result)
                    }
                    Err(err) => {
                        tracing::error!("Error getting prompt: {:?}", err);
                        Err(ErrorData::internal_error(
                            format!("Error getting prompt: {err}"),
                            None,
                        ))
                    }
                }
            }
            None => {
                // Server doesn't support prompts, return error
                tracing::warn!("Server doesn't support prompts capability");
                Ok(rmcp::model::GetPromptResult {
                    description: None,
                    messages: Vec::new(),
                })
            }
        }
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        // Get a lock on the client
        let client = self.client.clone();
        let guard = client.lock().await;

        // Forward request to client
        match guard.complete(request).await {
            Ok(result) => {
                debug!("Proxying complete response");
                Ok(result)
            }
            Err(err) => {
                tracing::error!("Error completing: {:?}", err);
                Err(ErrorData::internal_error(
                    format!("Error completing: {err}"),
                    None,
                ))
            }
        }
    }

    async fn on_progress(
        &self,
        notification: rmcp::model::ProgressNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        // Get a lock on the client
        let client = self.client.clone();
        let guard = client.lock().await;
        match guard.notify_progress(notification).await {
            Ok(_) => {
                debug!("Proxying progress notification");
            }
            Err(err) => {
                tracing::error!("Error notifying progress: {:?}", err);
            }
        }
    }

    async fn on_cancelled(
        &self,
        notification: rmcp::model::CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        // Get a lock on the client
        let client = self.client.clone();
        let guard = client.lock().await;
        match guard.notify_cancelled(notification).await {
            Ok(_) => {
                debug!("Proxying cancelled notification");
            }
            Err(err) => {
                tracing::error!("Error notifying cancelled: {:?}", err);
            }
        }
    }
}

impl ProxyHandler {
    pub fn new(client: RunningService<RoleClient, ClientInfo>) -> Self {
        Self::with_mcp_id(client, "unknown".to_string())
    }

    pub fn with_mcp_id(client: RunningService<RoleClient, ClientInfo>, mcp_id: String) -> Self {
        let peer_info = client.peer_info();

        // Create a ServerInfo object that forwards the server's capabilities
        let cached_info = peer_info.map(|peer_info| ServerInfo {
                protocol_version: peer_info.protocol_version.clone(),
                server_info: Implementation {
                    name: peer_info.server_info.name.clone(),
                    version: peer_info.server_info.version.clone(),
                    title: None,
                    website_url: None,
                    icons: None,
                },
                instructions: peer_info.instructions.clone(),
                capabilities: peer_info.capabilities.clone(),
            });

        Self {
            client: Arc::new(Mutex::new(client)),
            cached_info: Arc::new(RwLock::new(cached_info)),
            mcp_id,
        }
    }

    //检查 mcp服务是否正常,尝试调用 list_tools 方法,如果成功返回结果,则认为成功
    pub async fn is_mcp_server_ready(&self) -> bool {
        // 使用 try_lock 避免在定时检查时阻塞正常的业务请求
        // 如果无法获取锁，说明正在处理其他请求，假设服务正常
        match self.client.try_lock() {
            Ok(guard) => (guard.list_tools(None).await).is_ok(),
            Err(_) => {
                debug!("is_mcp_server_ready: 无法获取锁，假设服务正常");
                true
            }
        }
    }

    /// 检查子进程是否已经终止
    pub fn is_terminated(&self) -> bool {
        // 尝试获取锁，如果无法获取锁，说明子进程可能已经终止
        match self.client.try_lock() {
            Ok(_) => {
                // 能够获取锁，我们假设子进程仍在运行
                // 注意：我们不再尝试执行异步操作，因为这会导致运行时嵌套问题
                false
            }
            Err(_) => {
                // 无法获取锁，可能是因为子进程正在被其他线程使用
                debug!("子进程状态检查: 无法获取锁，假设子进程仍在运行");
                false // 这种情况下我们假设子进程还在运行
            }
        }
    }

    /// 异步检查子进程是否已经终止
    pub async fn is_terminated_async(&self) -> bool {
        // 尝试获取锁，如果无法获取锁，说明子进程可能已经终止
        match self.client.try_lock() {
            Ok(guard) => {
                // 检查客户端是否已经终止
                // 这里我们通过尝试发送一个轻量级请求来检测连接状态
                match guard.list_tools(None).await {
                    Ok(_) => {
                        debug!("子进程状态检查: 正在运行");
                        false // 成功获取信息，子进程还在运行
                    }
                    Err(e) => {
                        info!("子进程状态检查: 已终止，原因: {e}");
                        true // 无法获取信息，子进程可能已终止
                    }
                }
            }
            Err(_) => {
                // 无法获取锁，可能是因为子进程正在被其他线程使用
                debug!("子进程状态检查: 无法获取锁，假设子进程仍在运行");
                false // 这种情况下我们假设子进程还在运行
            }
        }
    }
}
