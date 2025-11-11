use crate::{
    DynamicRouterService, ProxyHandler, get_proxy_manager,
    model::{
        CheckMcpStatusResponseStatus, McpConfig, McpProtocol, McpProtocolPath, McpRouterPath,
        McpServerCommandConfig, McpServerConfig, McpServiceStatus, McpType,
    },
};

use anyhow::Result;
use log::{debug, info};
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    },
    transport::{
        SseClientTransport, SseServer, TokioChildProcess, sse_server::SseServerConfig,
        streamable_http_client::StreamableHttpClientTransport,
    },
};
use tokio::process::Command;

///根据 mcp_id 和 mcp_json_config 启动mcp服务
pub async fn mcp_start_task(
    mcp_config: McpConfig,
) -> Result<(axum::Router, tokio_util::sync::CancellationToken)> {
    let mcp_id = mcp_config.mcp_id.clone();
    let backend_protocol = mcp_config.mcp_protocol.clone();
    let client_protocol = mcp_config.client_protocol.clone();

    // 使用客户端协议创建路由路径（决定暴露的API接口）
    let mcp_router_path: McpRouterPath = McpRouterPath::new(mcp_id, client_protocol);

    let mcp_json_config = mcp_config
        .mcp_json_config
        .clone()
        .expect("mcp_json_config is required");

    let mcp_server_config = McpServerConfig::try_from(mcp_json_config)?;

    // 使用新的集成方法，传递后端协议用于连接远程服务
    integrate_sse_server_with_axum(
        mcp_server_config.clone(),
        mcp_router_path.clone(),
        mcp_config.mcp_type,
        backend_protocol,
    )
    .await
}

// 创建一个新函数，将 SseServer 与 axum 路由集成
pub async fn integrate_sse_server_with_axum(
    mcp_config: McpServerConfig,
    mcp_router_path: McpRouterPath,
    mcp_type: McpType,
    backend_protocol: McpProtocol,
) -> Result<(axum::Router, tokio_util::sync::CancellationToken)> {
    let base_path = mcp_router_path.base_path.clone();
    let mcp_id = mcp_router_path.mcp_id.clone();

    // 创建客户端信息
    let client_info = ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::builder()
            .enable_experimental()
            .enable_roots()
            .enable_roots_list_changed()
            .enable_sampling()
            .build(),
        ..Default::default()
    };

    // 根据配置类型创建不同的客户端服务
    let client = match &mcp_config {
        McpServerConfig::Command(cmd_config) => {
            // 创建子进程命令
            let mut command = Command::new(&cmd_config.command);

            // 正确处理Option<Vec<String>>
            if let Some(args) = &cmd_config.args {
                command.args(args);
            }

            // 正确处理Option<HashMap<String, String>>
            if let Some(env_vars) = &cmd_config.env {
                for (key, value) in env_vars {
                    command.env(key, value);
                }
            }

            // 记录命令执行信息，方便调试
            log_command_details(cmd_config, &mcp_router_path);

            info!(
                "子进程已启动，MCP ID: {}, 类型: {:?}",
                mcp_router_path.mcp_id,
                mcp_type.clone()
            );

            // 创建子进程传输并创建客户端服务
            let tokio_process = TokioChildProcess::new(command)?;
            client_info.serve(tokio_process).await?
        }
        McpServerConfig::Url(url_config) => {
            // 根据后端协议类型创建不同的客户端传输
            info!(
                "连接到远程MCP服务: {}, 后端协议: {:?}, 客户端协议: {:?}",
                url_config.url, backend_protocol, mcp_router_path.mcp_protocol
            );

            match backend_protocol {
                McpProtocol::Sse => {
                    // SSE 协议 - 创建 SSE 客户端传输
                    info!("使用SSE协议连接到: {}", url_config.url);

                    let sse_transport = SseClientTransport::start(url_config.url.clone()).await?;
                    client_info.serve(sse_transport).await?
                }
                McpProtocol::Stream => {
                    // Streamable 协议 - 创建 Streamable HTTP 客户端传输
                    info!("使用Streamable HTTP协议连接到: {}", url_config.url);

                    // 使用默认方式创建传输，rmcp 库会自动处理 Accept 头和会话管理
                    let transport = StreamableHttpClientTransport::from_uri(url_config.url.clone());

                    info!(
                        "Streamable HTTP传输已创建，开始建立连接，MCP ID: {}, 类型: {:?}",
                        mcp_router_path.mcp_id,
                        mcp_type.clone()
                    );

                    // serve 会建立连接并完成初始化握手
                    let client = client_info.serve(transport).await?;

                    info!(
                        "Streamable HTTP客户端连接成功，MCP ID: {}",
                        mcp_router_path.mcp_id
                    );

                    client
                }
            }
        }
    };

    // 创建代理处理器
    let proxy_handler = ProxyHandler::with_mcp_id(client, mcp_id.clone());

    // 获取全局 ProxyHandlerManager
    let proxy_manager = get_proxy_manager();

    // 注册代理处理器
    let proxy_handler_clone: ProxyHandler = proxy_handler.clone();
    let proxy_handler_for_sse = proxy_handler_clone.clone();
    let proxy_handler_for_stream = proxy_handler_clone.clone();

    // 根据客户端协议和后端协议创建服务器（支持协议转换）
    // 支持三种模式：
    // 1. client=SSE, backend=SSE (透明代理)
    // 2. client=Stream, backend=Stream (透明代理)
    // 3. client=SSE, backend=Stream (协议转换) - 关键功能
    let (router, ct) = match (mcp_router_path.mcp_protocol.clone(), backend_protocol) {
        // SSE 客户端协议
        (McpProtocol::Sse, McpProtocol::Sse) => {
            // 模式1: SSE -> SSE (透明代理)
            let addr: String = "0.0.0.0:0".to_string();
            let sse_path = match &mcp_router_path.mcp_protocol_path {
                McpProtocolPath::SsePath(sse_path) => sse_path,
                _ => unreachable!(),
            };
            let config = SseServerConfig {
                bind: addr.parse()?,
                sse_path: sse_path.sse_path.clone(),
                post_path: sse_path.message_path.clone(),
                ct: tokio_util::sync::CancellationToken::new(),
                sse_keep_alive: None,
            };

            debug!(
                "创建SSE服务器，配置: bind={}, sse_path={}, post_path={}",
                config.bind, config.sse_path, config.post_path
            );

            let (sse_server, router) = SseServer::new(config);
            let ct = sse_server.with_service(move || proxy_handler_for_sse.clone());
            (router, ct)
        }
        (McpProtocol::Sse, McpProtocol::Stream) => {
            // 模式3: SSE -> Streamable HTTP (协议转换)
            // 对外提供SSE接口，内部转换为Streamable HTTP
            let addr: String = "0.0.0.0:0".to_string();
            let sse_path = match &mcp_router_path.mcp_protocol_path {
                McpProtocolPath::SsePath(sse_path) => sse_path,
                _ => unreachable!(),
            };
            let config = SseServerConfig {
                bind: addr.parse()?,
                sse_path: sse_path.sse_path.clone(),
                post_path: sse_path.message_path.clone(),
                ct: tokio_util::sync::CancellationToken::new(),
                sse_keep_alive: None,
            };

            debug!(
                "创建SSE服务器(协议转换)，配置: bind={}, sse_path={}, post_path={}",
                config.bind, config.sse_path, config.post_path
            );

            let (sse_server, router) = SseServer::new(config);
            let ct = sse_server.with_service(move || proxy_handler_for_stream.clone());
            (router, ct)
        }
        (McpProtocol::Stream, McpProtocol::Stream) => {
            // 模式2: Stream -> Stream (透明代理)
            let service = StreamableHttpService::new(
                move || Ok(proxy_handler_for_stream.clone()),
                LocalSessionManager::default().into(),
                Default::default(),
            );
            let router = axum::Router::new().fallback_service(service);
            let ct = tokio_util::sync::CancellationToken::new();
            (router, ct)
        }
        (McpProtocol::Stream, McpProtocol::Sse) => {
            // 模式4: Streamable HTTP -> SSE (协议转换)
            // 对外提供Streamable HTTP接口，内部连接到SSE后端
            let service = StreamableHttpService::new(
                move || Ok(proxy_handler_for_sse.clone()),
                LocalSessionManager::default().into(),
                Default::default(),
            );
            let router = axum::Router::new().fallback_service(service);
            let ct = tokio_util::sync::CancellationToken::new();
            (router, ct)
        }
    };

    // 克隆一份取消令牌和 mcp_id 用于监控子进程
    let ct_clone = ct.clone();
    let mcp_id_clone = mcp_id.clone();

    // 存储 MCP 服务状态
    let mcp_service_status = McpServiceStatus::new(
        mcp_id_clone.clone(),
        mcp_type.clone(),
        mcp_router_path.clone(),
        ct_clone.clone(),
        CheckMcpStatusResponseStatus::Ready,
    );
    // 添加 MCP 服务状态到全局管理器,以及 proxy_handler 的透明代理
    proxy_manager.add_mcp_service_status_and_proxy(mcp_service_status, Some(proxy_handler));

    // 为SSE和Stream协议添加基础路径处理
    // 支持直接访问基础路径，自动重定向到正确的子路径
    let router = if matches!(mcp_router_path.mcp_protocol, McpProtocol::Sse) {
        // 使用fallback处理器来匹配基础路径
        let modified_router = router.fallback(base_path_fallback_handler);
        info!("SSE基础路径处理器已添加, 基础路径: {}", base_path);
        modified_router
    } else {
        router
    };

    // 注册路由到全局路由表
    info!("注册路由: base_path={}, mcp_id={}", base_path, mcp_id);
    info!(
        "SSE路径配置: sse_path={}, post_path={}",
        match &mcp_router_path.mcp_protocol_path {
            McpProtocolPath::SsePath(sse_path) => &sse_path.sse_path,
            _ => "N/A",
        },
        match &mcp_router_path.mcp_protocol_path {
            McpProtocolPath::SsePath(sse_path) => &sse_path.message_path,
            _ => "N/A",
        }
    );
    DynamicRouterService::register_route(&base_path, router.clone());
    info!("路由注册完成: base_path={}", base_path);

    // 返回路由和取消令牌
    Ok((router, ct))
}

// 基础路径处理器 - 支持直接访问基础路径，自动重定向到正确的子路径
#[axum::debug_handler]
async fn base_path_fallback_handler(
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    let path = uri.path();
    info!("基础路径处理器: {} {}", method, path);

    // 判断是SSE还是Stream协议
    if path.contains("/sse/proxy/") {
        // SSE协议处理
        match method {
            axum::http::Method::GET => {
                // 从路径中提取 MCP ID
                let mcp_id = path.split("/sse/proxy/").nth(1);

                if let Some(mcp_id) = mcp_id {
                    // 检查MCP服务是否存在
                    let proxy_manager = get_proxy_manager();
                    if proxy_manager.get_mcp_service_status(mcp_id).is_none() {
                        // MCP服务不存在
                        (
                            axum::http::StatusCode::NOT_FOUND,
                            [("Content-Type", "text/plain".to_string())],
                            format!("MCP service '{}' not found", mcp_id).to_string(),
                        )
                    } else {
                        // MCP服务存在，检查Accept头
                        let accept_header = headers.get("accept");
                        if let Some(accept) = accept_header {
                            let accept_str = accept.to_str().unwrap_or("");
                            if accept_str.contains("text/event-stream") {
                                // 正确的Accept头，重定向到 /sse
                                let redirect_uri = format!("{}/sse", path);
                                info!("SSE重定向到: {}", redirect_uri);
                                (
                                    axum::http::StatusCode::FOUND,
                                    [("Location", redirect_uri.to_string())],
                                    "Redirecting to SSE endpoint".to_string(),
                                )
                            } else {
                                // Accept头不正确
                                (
                                    axum::http::StatusCode::BAD_REQUEST,
                                    [("Content-Type", "text/plain".to_string())],
                                    "SSE error: Invalid Accept header, expected 'text/event-stream'".to_string(),
                                )
                            }
                        } else {
                            // 没有Accept头
                            (
                                axum::http::StatusCode::BAD_REQUEST,
                                [("Content-Type", "text/plain".to_string())],
                                "SSE error: Missing Accept header, expected 'text/event-stream'"
                                    .to_string(),
                            )
                        }
                    }
                } else {
                    // 无法从路径中提取MCP ID
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        [("Content-Type", "text/plain".to_string())],
                        "SSE error: Invalid SSE path".to_string(),
                    )
                }
            }
            axum::http::Method::POST => {
                // POST请求重定向到 /message
                let redirect_uri = format!("{}/message", path);
                info!("SSE重定向到: {}", redirect_uri);
                (
                    axum::http::StatusCode::FOUND,
                    [("Location", redirect_uri.to_string())],
                    "Redirecting to message endpoint".to_string(),
                )
            }
            _ => {
                // 其他方法返回405 Method Not Allowed
                (
                    axum::http::StatusCode::METHOD_NOT_ALLOWED,
                    [("Allow", "GET, POST".to_string())],
                    "Only GET and POST methods are allowed".to_string(),
                )
            }
        }
    } else if path.contains("/stream/proxy/") {
        // Stream协议处理 - 直接返回成功，不重定向
        match method {
            axum::http::Method::GET => {
                // GET请求返回服务器信息
                (
                    axum::http::StatusCode::OK,
                    [("Content-Type", "application/json".to_string())],
                    r#"{"jsonrpc":"2.0","result":{"info":"Streamable MCP Server","version":"1.0"}}"#.to_string(),
                )
            }
            axum::http::Method::POST => {
                // POST请求返回成功，让StreamableHttpService处理
                (
                    axum::http::StatusCode::OK,
                    [("Content-Type", "application/json".to_string())],
                    r#"{"jsonrpc":"2.0","result":{"message":"Stream request received","protocol":"streamable-http"}}"#.to_string(),
                )
            }
            _ => {
                // 其他方法返回405 Method Not Allowed
                (
                    axum::http::StatusCode::METHOD_NOT_ALLOWED,
                    [("Allow", "GET, POST".to_string())],
                    "Only GET and POST methods are allowed".to_string(),
                )
            }
        }
    } else {
        // 未知协议
        (
            axum::http::StatusCode::BAD_REQUEST,
            [("Content-Type", "text/plain".to_string())],
            "Unknown protocol or path".to_string(),
        )
    }
}

// 提取记录命令详情的函数
fn log_command_details(mcp_config: &McpServerCommandConfig, mcp_router_path: &McpRouterPath) {
    // 打印命令行参数
    let args_str = mcp_config
        .args
        .as_ref()
        .map_or(String::new(), |args| args.join(" "));
    let cmd_str = format!("执行命令: {} {}", mcp_config.command, args_str);
    debug!("{cmd_str}");

    // 打印环境变量
    if let Some(env_vars) = &mcp_config.env {
        let env_vars: Vec<String> = env_vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if !env_vars.is_empty() {
            debug!("环境变量: {}", env_vars.join(", "));
        }
    }

    // 打印完整命令
    debug!(
        "完整命令,mcpId={}, command={:?}",
        mcp_router_path.mcp_id, mcp_config.command
    );

    // 构建完整的命令字符串，用于直接复制运行
    let args_str = mcp_config
        .args
        .as_ref()
        .map_or(String::new(), |args| args.join(" "));
    let env_str = mcp_config.env.as_ref().map_or(String::new(), |env| {
        env.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<String>>()
            .join(" ")
    });

    let full_command = format!("{} {} {}", mcp_config.command, args_str, env_str);
    info!(
        "完整命令字符串,mcpId={},command={:?}",
        mcp_router_path.mcp_id, full_command
    );
}
