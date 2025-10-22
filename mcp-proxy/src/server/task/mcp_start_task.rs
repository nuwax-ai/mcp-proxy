use crate::{
    DynamicRouterService, ProxyHandler, get_proxy_manager,
    model::{
        CheckMcpStatusResponseStatus, McpConfig, McpProtocolPath, McpRouterPath,
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
    transport::{SseClientTransport, SseServer, TokioChildProcess, sse_server::SseServerConfig},
};
use tokio::process::Command;

///根据 mcp_id 和 mcp_json_config 启动mcp服务
pub async fn mcp_start_task(
    mcp_config: McpConfig,
) -> Result<(axum::Router, tokio_util::sync::CancellationToken)> {
    let mcp_id = mcp_config.mcp_id.clone();
    let mcp_protocol = mcp_config.mcp_protocol.clone();

    let mcp_router_path: McpRouterPath = McpRouterPath::new(mcp_id, mcp_protocol);

    let mcp_json_config = mcp_config
        .mcp_json_config
        .clone()
        .expect("mcp_json_config is required");

    let mcp_server_config = McpServerConfig::try_from(mcp_json_config)?;

    // 使用新的集成方法，而不是单独启动 SSE 服务
    integrate_sse_server_with_axum(
        mcp_server_config.clone(),
        mcp_router_path.clone(),
        mcp_config.mcp_type,
    )
    .await
}

// 创建一个新函数，将 SseServer 与 axum 路由集成
pub async fn integrate_sse_server_with_axum(
    mcp_config: McpServerConfig,
    mcp_router_path: McpRouterPath,
    mcp_type: McpType,
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
            // 对于URL配置，创建SSE客户端传输
            info!("创建SSE客户端连接到: {}", url_config.url);

            info!(
                "SSE客户端已启动，MCP ID: {}, 类型: {:?}",
                mcp_router_path.mcp_id,
                mcp_type.clone()
            );

            // 创建SSE客户端传输并创建客户端服务
            let sse_transport = SseClientTransport::start(url_config.url.clone()).await?;
            client_info.serve(sse_transport).await?
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

    //区分协议,如果是sse 协议,使用: SseServer
    //如果是stream 协议,使用: StreamableHttpServer
    let (router, ct) = match &mcp_router_path.mcp_protocol_path {
        McpProtocolPath::SsePath(sse_path) => {
            // 创建 SseServer
            // 使用随机端口，让 axum 来管理; 这里不使用这个地址绑定,只需要对应的router
            let addr: String = "0.0.0.0:0".to_string();

            // 创建SSE配置 - 使用相对路径（相对于注册的 base_path）
            let config = SseServerConfig {
                bind: addr.parse()?,
                sse_path: "/sse".to_string(),  // 相对于 base_path 的路径
                post_path: "/message".to_string(),  // 相对于 base_path 的路径
                ct: tokio_util::sync::CancellationToken::new(),
                sse_keep_alive: None,
            };
            let (sse_server, router) = SseServer::new(config);
            let ct = sse_server.with_service(move || proxy_handler_for_sse.clone());

            (router, ct)
        }
        McpProtocolPath::StreamPath(_stream_path) => {
            let service = StreamableHttpService::new(
                move || Ok(proxy_handler_for_stream.clone()),
                LocalSessionManager::default().into(),
                Default::default(),
            );
            let router = axum::Router::new().nest_service("/mcp", service);
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

    // 注册路由到全局路由表
    info!("注册路由: base_path={}, mcp_id={}", base_path, mcp_id);
    info!("SSE路径配置: sse_path={}, post_path={}", 
          match &mcp_router_path.mcp_protocol_path {
              McpProtocolPath::SsePath(sse_path) => &sse_path.sse_path,
              _ => "N/A"
          },
          match &mcp_router_path.mcp_protocol_path {
              McpProtocolPath::SsePath(sse_path) => &sse_path.message_path,
              _ => "N/A"
          });
    DynamicRouterService::register_route(&base_path, router.clone());
    info!("路由注册完成: base_path={}", base_path);

    // 返回路由和取消令牌
    Ok((router, ct))
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
