use anyhow::Result;
use axum::{Json, extract::State, http::uri::Uri};
use log::{debug, error, info};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

use crate::{
    AppError, get_proxy_manager,
    model::{
        AppState, CheckMcpStatusRequestParams, CheckMcpStatusResponseParams,
        CheckMcpStatusResponseStatus, GLOBAL_RESTART_TRACKER, HttpResult, McpConfig, McpProtocol,
        McpRouterPath, McpServiceStatus, McpType,
    },
    server::mcp_start_task,
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

    // 检查服务状态
    let status = proxy_manager
        .get_mcp_service_status(&params.mcp_id)
        .map(|mcp_service_status| mcp_service_status.check_mcp_status_response_status.clone());

    if let Some(status) = status {
        match status {
            CheckMcpStatusResponseStatus::Error(error_msg) => {
                // 如果有错误状态，返回错误信息,另外删除掉 ERROR 的记录,方便下次检查状态,重新启动服务
                // Error 状态不更新 last_accessed，因为服务已失败
                if let Err(e) = proxy_manager.cleanup_resources(&params.mcp_id).await {
                    error!("Failed to cleanup resources for {}: {}", params.mcp_id, e);
                }
                // 返回错误信息
                return create_response(
                    false,
                    CheckMcpStatusResponseStatus::Error(error_msg),
                    None,
                );
            }
            CheckMcpStatusResponseStatus::Pending => {
                // 如果状态是 Pending，说明服务正在启动中，直接返回 Pending
                // 不要再次尝试启动！
                debug!(
                    "[check_mcp_status] mcp_id={} status is Pending and the service is starting",
                    params.mcp_id
                );
                // 更新最后访问时间，避免启动过程中因超时被清理
                // 用户调用 check_status 表明对服务有兴趣，应该延长超时
                proxy_manager.update_last_accessed(&params.mcp_id);
                return create_response(
                    false,
                    CheckMcpStatusResponseStatus::Pending,
                    Some("服务正在启动中...".to_string()),
                );
            }
            CheckMcpStatusResponseStatus::Ready => {
                // 如果已经在运行，继续检查服务是否真的可用
                debug!(
                    "[check_mcp_status] mcp_id={} status is Ready, check the backend health status",
                    params.mcp_id
                );
            }
        }
    }

    // 检查 proxy_handler 是否存在
    let proxy_handler = proxy_manager.get_proxy_handler(&params.mcp_id);

    if let Some(handler) = proxy_handler {
        // 调用透明代理的 is_mcp_server_ready 方法检查健康状态
        let ready_status = handler.is_mcp_server_ready().await;

        // 使用 update 方法更新状态（不是修改克隆）
        if ready_status {
            proxy_manager
                .update_mcp_service_status(&params.mcp_id, CheckMcpStatusResponseStatus::Ready);
        }
        // 更新最后访问时间
        proxy_manager.update_last_accessed(&params.mcp_id);

        let status = if ready_status {
            CheckMcpStatusResponseStatus::Ready
        } else {
            CheckMcpStatusResponseStatus::Pending
        };

        return create_response(ready_status, status, None);
    }

    // ===== 服务不存在，需要启动 =====
    // 使用启动锁防止并发启动同一服务
    let _startup_guard = match GLOBAL_RESTART_TRACKER.try_acquire_startup_lock(&params.mcp_id) {
        Some(guard) => {
            debug!(
                "[check_mcp_status] mcp_id={} Obtained the startup lock successfully and started to start the service",
                params.mcp_id
            );
            guard
        }
        None => {
            // 锁被占用，服务正在启动中
            debug!(
                "[check_mcp_status] mcp_id={} The startup lock is occupied and the service is starting. Return to Pending.",
                params.mcp_id
            );
            return create_response(
                false,
                CheckMcpStatusResponseStatus::Pending,
                Some("服务正在启动中...".to_string()),
            );
        }
    };

    // 双重检查：获取锁后再次检查服务是否已存在
    if proxy_manager.get_proxy_handler(&params.mcp_id).is_some() {
        debug!(
            "[check_mcp_status] mcp_id={} Double check found that the service already exists, return Ready",
            params.mcp_id
        );
        return create_response(true, CheckMcpStatusResponseStatus::Ready, None);
    }

    // 如果服务状态已存在（可能是 Pending），也不要重复启动
    if proxy_manager
        .get_mcp_service_status(&params.mcp_id)
        .is_some()
    {
        debug!(
            "[check_mcp_status] mcp_id={} The service status already exists and may be starting. Return to Pending.",
            params.mcp_id
        );
        return create_response(
            false,
            CheckMcpStatusResponseStatus::Pending,
            Some("服务正在启动中...".to_string()),
        );
    }

    // 启动服务（持有锁的情况下）
    spawn_mcp_service(
        &params.mcp_id,
        params.mcp_json_config,
        params.mcp_type,
        mcp_protocol.clone(),
    )?;

    // 返回 PENDING 状态，锁会在函数返回时自动释放
    create_response(
        false,
        CheckMcpStatusResponseStatus::Pending,
        Some("服务正在启动中...".to_string()),
    )
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
/// 注意：此函数假设调用方已持有启动锁，不会重复检查！
///
/// # 参数
/// - `mcp_id`: MCP服务的唯一标识
/// - `mcp_json_config`: MCP服务的JSON配置
/// - `mcp_type`: MCP服务类型（OneShot或Persistent）
/// - `client_protocol`: 客户端使用的协议（决定暴露的API接口类型）
fn spawn_mcp_service(
    mcp_id: &str,
    mcp_json_config: String,
    mcp_type: McpType,
    client_protocol: McpProtocol,
) -> Result<(), AppError> {
    let mcp_id = mcp_id.to_string();
    info!(
        "[spawn_mcp_service] mcp_id={} Start starting the service",
        mcp_id
    );

    // 使用全局 ProxyHandlerManager
    let proxy_manager = get_proxy_manager();

    // 设置初始化状态 - 使用客户端协议创建路由路径
    let mcp_router_path = McpRouterPath::new(mcp_id.clone(), client_protocol.clone())
        .map_err(|e| AppError::mcp_server_error(e.to_string()))?;
    let mcp_service_status = McpServiceStatus::new(
        mcp_id.clone(),
        mcp_type.clone(),
        mcp_router_path,
        CancellationToken::new(),
        CheckMcpStatusResponseStatus::Pending,
    );

    // RAII: 如果已存在同名服务，会自动清理旧服务
    proxy_manager.add_mcp_service_status_and_proxy(mcp_service_status, None);

    // 异步启动 mcp 透明代理服务
    let mcp_id_clone = mcp_id.clone();

    // 使用客户端协议创建配置
    let mcp_config: McpConfig = McpConfig::new(
        mcp_id_clone.clone(),
        Some(mcp_json_config),
        mcp_type,
        client_protocol,
    );

    tokio::spawn(async move {
        info!(
            "[spawn_mcp_service] mcp_id={} tokio::spawn starts executing mcp_start_task",
            mcp_id_clone
        );
        match mcp_start_task(mcp_config).await {
            Ok(_) => {
                info!(
                    "[spawn_mcp_service] mcp_id={} mcp_start_task successful, set status to Ready",
                    mcp_id_clone
                );
                get_proxy_manager()
                    .update_mcp_service_status(&mcp_id_clone, CheckMcpStatusResponseStatus::Ready);
            }
            Err(e) => {
                let error_msg = format!("启动MCP服务失败: {e}");
                error!(
                    "[spawn_mcp_service] mcp_id={} mcp_start_task failed: {}",
                    mcp_id_clone, e
                );
                get_proxy_manager().update_mcp_service_status(
                    &mcp_id_clone,
                    CheckMcpStatusResponseStatus::Error(error_msg),
                );
            }
        }
    });

    info!(
        "[spawn_mcp_service] mcp_id={} Service startup task has been submitted",
        mcp_id
    );
    Ok(())
}
