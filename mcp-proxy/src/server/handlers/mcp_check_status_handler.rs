use anyhow::Result;
use axum::{Json, extract::State, http::uri::Uri};
use log::{error, info};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

use crate::{
    AppError, get_proxy_manager,
    model::{
        AppState, CheckMcpStatusRequestParams, CheckMcpStatusResponseParams,
        CheckMcpStatusResponseStatus, HttpResult, McpConfig, McpProtocol, McpRouterPath,
        McpServerConfig, McpServiceStatus, McpType,
    },
    server::{detect_mcp_protocol, mcp_start_task},
};

/// 创建响应结果的辅助函数
fn create_response(
    ready: bool,
    status: CheckMcpStatusResponseStatus,
    message: Option<String>,
) -> Result<HttpResult<CheckMcpStatusResponseParams>, AppError> {
    let response = CheckMcpStatusResponseParams::new(ready, status, message);

    Ok(HttpResult::success(response, None))
}

/// 检查mcp服务状态,根据 mcp_id 获取有无对应的mcp透明代理服务,如果没有则取 mcp_json_config 中的配置,生成mcp透明代理服务;
/// 这里根据 mcp_json_config配置,启动服务需要异步,不要阻塞,如果服务没准备好,返回 PENDING 状态;
/// 如果服务启动失败,返回 ERROR 状态;
/// 如果服务启动成功,返回 READY 状态;
#[instrument]
pub async fn check_mcp_status_handler(
    State(state): State<AppState>,
    uri: Uri,
    Json(params): Json<CheckMcpStatusRequestParams>,
    mcp_protocol: McpProtocol,
) -> Result<HttpResult<CheckMcpStatusResponseParams>, AppError> {
    // 使用全局 ProxyHandlerManager
    let proxy_manager = get_proxy_manager();

    // 专门调用 mcp_service_status 字段上的 get 方法（而不是 mcp_service_statuses）
    let status = proxy_manager
        .get_mcp_service_status(&params.mcp_id)
        .map(|mcp_service_status| mcp_service_status.check_mcp_status_response_status.clone());

    if let Some(status) = status {
        match status {
            CheckMcpStatusResponseStatus::Error(error_msg) => {
                // 如果有错误状态，返回错误信息,另外删除掉 ERROR 的记录,方便下次检查状态,重新启动服务
                proxy_manager.cleanup_resources(&params.mcp_id).await;
                // 返回错误信息
                return create_response(
                    false,
                    CheckMcpStatusResponseStatus::Error(error_msg),
                    None,
                );
            }
            CheckMcpStatusResponseStatus::Pending => {
                // 如果正在初始化，返回PENDING状态
                return create_response(
                    false,
                    CheckMcpStatusResponseStatus::Pending,
                    Some("服务正在初始化中...".to_string()),
                );
            }
            CheckMcpStatusResponseStatus::Ready => {
                // 如果已经在运行，继续检查服务是否真的可用
            }
        }
    }

    let proxy_handler = proxy_manager.get_proxy_handler(&params.mcp_id);

    if let Some(some_porxy_handler) = proxy_handler {
        //调用透明代理的 list_tools 方法,如果成功返回结果,则认为成功
        let ready_status = some_porxy_handler.is_mcp_server_ready().await;

        // 如果服务已经就绪，更新状态为Ready
        if let Some(mut mcp_service_status) = proxy_manager.get_mcp_service_status(&params.mcp_id) {
            mcp_service_status.last_accessed = Instant::now();
            if ready_status {
                mcp_service_status.check_mcp_status_response_status =
                    CheckMcpStatusResponseStatus::Ready;
            }
        }

        let status = if ready_status {
            CheckMcpStatusResponseStatus::Ready
        } else {
            CheckMcpStatusResponseStatus::Pending
        };

        return create_response(ready_status, status, None);
    } else {
        // 如果服务不存在,则取 mcp_json_config 中的配置,生成mcp透明代理服务
        // 使用 backend_protocol（如果指定）或自动检测
        let backend_protocol = if let Some(protocol) = params.backend_protocol.clone() {
            protocol
        } else {
            // 尝试从配置中提取 URL 并自动检测协议
            match try_detect_backend_protocol(&params.mcp_json_config).await {
                Ok(detected) => {
                    info!(
                        "自动检测到后端协议: {:?} for MCP ID: {}",
                        detected, params.mcp_id
                    );
                    detected
                }
                Err(e) => {
                    info!(
                        "无法自动检测后端协议，使用客户端协议: {:?}, 错误: {}",
                        mcp_protocol, e
                    );
                    mcp_protocol.clone()
                }
            }
        };

        spawn_mcp_service(
            &params.mcp_id,
            params.mcp_json_config,
            params.mcp_type,
            mcp_protocol.clone(),
            backend_protocol,
        )?;

        // 返回 PENDING 状态,表示服务正在启动
        return create_response(
            false,
            CheckMcpStatusResponseStatus::Pending,
            Some("服务正在启动中...".to_string()),
        );
    }
}

// SSE协议专用的状态检查处理函数
#[instrument]
// #[axum::debug_handler]
pub async fn check_mcp_status_handler_sse(
    state: State<AppState>,
    uri: Uri,
    params: Json<CheckMcpStatusRequestParams>,
) -> Result<HttpResult<CheckMcpStatusResponseParams>, AppError> {
    check_mcp_status_handler(state, uri, params, McpProtocol::Sse).await
}

// Stream协议专用的状态检查处理函数
#[instrument]
// #[axum::debug_handler]
pub async fn check_mcp_status_handler_stream(
    state: State<AppState>,
    uri: Uri,
    params: Json<CheckMcpStatusRequestParams>,
) -> Result<HttpResult<CheckMcpStatusResponseParams>, AppError> {
    check_mcp_status_handler(state, uri, params, McpProtocol::Stream).await
}

/// 异步启动MCP服务
///
/// # 参数
/// - `mcp_id`: MCP服务的唯一标识
/// - `mcp_json_config`: MCP服务的JSON配置
/// - `mcp_type`: MCP服务类型（OneShot或Persistent）
/// - `client_protocol`: 客户端使用的协议（决定暴露的API接口类型）
/// - `backend_protocol`: 连接后端服务使用的协议（决定如何连接到远程MCP服务）
fn spawn_mcp_service(
    mcp_id: &str,
    mcp_json_config: String,
    mcp_type: McpType,
    client_protocol: McpProtocol,
    backend_protocol: McpProtocol,
) -> Result<(), AppError> {
    let mcp_id = mcp_id.to_string();

    // 使用全局 ProxyHandlerManager
    let proxy_manager = get_proxy_manager();

    // 设置初始化状态 - 使用客户端协议创建路由路径
    let mcp_service_status = McpServiceStatus::new(
        mcp_id.clone(),
        mcp_type.clone(),
        McpRouterPath::new(mcp_id.clone(), client_protocol.clone()),
        CancellationToken::new(), // This will be the single cancellation_token
        CheckMcpStatusResponseStatus::Pending,
    );
    proxy_manager.add_mcp_service_status_and_proxy(mcp_service_status, None);

    //异步添加 mcp 透明代理服务
    let mcp_id_clone = mcp_id.clone();

    // 使用客户端协议和后端协议创建配置
    let mcp_config: McpConfig = McpConfig::new_with_protocols(
        mcp_id_clone.clone(),
        Some(mcp_json_config),
        mcp_type,
        client_protocol,
        backend_protocol,
    );
    tokio::spawn(async move {
        match mcp_start_task(mcp_config).await {
            Ok(_) => {
                // 设置运行状态
                get_proxy_manager()
                    .update_mcp_service_status(&mcp_id_clone, CheckMcpStatusResponseStatus::Ready);
            }
            Err(e) => {
                // 设置错误状态
                let error_msg = format!("启动MCP服务失败: {e}");
                error!("启动MCP服务失败[{mcp_id_clone}]: {e}");
                get_proxy_manager().update_mcp_service_status(
                    &mcp_id_clone,
                    CheckMcpStatusResponseStatus::Error(error_msg),
                );
            }
        }
    });

    Ok(())
}

/// 尝试从 MCP JSON 配置中提取 URL 并自动检测后端协议
async fn try_detect_backend_protocol(mcp_json_config: &str) -> Result<McpProtocol, AppError> {
    // 解析配置
    let mcp_server_config = McpServerConfig::try_from(mcp_json_config.to_string())
        .map_err(|e| AppError::McpServerError(anyhow::anyhow!("解析 MCP 配置失败: {}", e)))?;

    // 只有 URL 配置才需要检测协议
    match mcp_server_config {
        McpServerConfig::Url(url_config) => {
            // 调用协议检测函数
            detect_mcp_protocol(&url_config.url)
                .await
                .map_err(|e| AppError::McpServerError(anyhow::anyhow!("协议检测失败: {}", e)))
        }
        McpServerConfig::Command(_) => {
            // 命令行方式启动的服务，默认使用 SSE 协议（stdio 传输）
            Ok(McpProtocol::Sse)
        }
    }
}
