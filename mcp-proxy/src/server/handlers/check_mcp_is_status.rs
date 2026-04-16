use axum::extract::Path;
use log::{info, warn};

use crate::{
    AppError, get_proxy_manager,
    model::{CheckMcpStatusResponseParams, CheckMcpStatusResponseStatus, HttpResult},
};

///根据 mcpId，检查 mcp 透明代理服务是否正在运行的状态
// #[axum::debug_handler]
pub async fn check_mcp_is_status_handler(
    Path(mcp_id): Path<String>,
) -> Result<HttpResult<CheckMcpStatusResponseParams>, AppError> {
    info!("mcp_id: {mcp_id}");

    let status = get_proxy_manager()
        .get_mcp_service_status(&mcp_id)
        .map(|mcp_service_status| mcp_service_status.check_mcp_status_response_status.clone());

    if let Some(status) = status {
        match status.clone() {
            CheckMcpStatusResponseStatus::Ready => Ok(HttpResult::success(
                CheckMcpStatusResponseParams::new(true, status, None),
                None,
            )),
            CheckMcpStatusResponseStatus::Pending => Ok(HttpResult::success(
                CheckMcpStatusResponseParams::new(false, status, None),
                None,
            )),
            CheckMcpStatusResponseStatus::Error(err) => Ok(HttpResult::success(
                CheckMcpStatusResponseParams::new(false, status, Some(err)),
                None,
            )),
        }
    } else {
        warn!("mcp_id: {mcp_id} does not exist");
        Ok(HttpResult::success(
            CheckMcpStatusResponseParams::new(
                false,
                CheckMcpStatusResponseStatus::Error("mcp_id 不存在".to_string()),
                None,
            ),
            None,
        ))
    }
}
