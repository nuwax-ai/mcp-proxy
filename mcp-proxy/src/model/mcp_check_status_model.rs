use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use super::{McpProtocol, McpType};

//check mcp服务状态的请求参数
#[derive(Deserialize, Debug, Clone)]
pub struct CheckMcpStatusRequestParams {
    //mcp的id,必须有
    #[serde(rename = "mcpId")]
    pub mcp_id: String,
    //mcp的json配置,必须有
    #[serde(rename = "mcpJsonConfig")]
    pub mcp_json_config: String,
    //mcp类型,必须有,默认:OneShot
    #[serde(rename = "mcpType", default = "default_mcp_type")]
    pub mcp_type: McpType,
    //后端MCP服务的协议类型（可选），用于指定连接到后端服务时使用的协议
    //如果不指定，则使用客户端协议（由路由路径决定）
    #[serde(rename = "backendProtocol")]
    #[allow(dead_code)]  // 为未来的功能预留
    pub backend_protocol: Option<McpProtocol>,
}

//默认的mcp类型
fn default_mcp_type() -> McpType {
    McpType::OneShot
}

//check mcp服务状态的响应参数
#[derive(Deserialize, Debug, Serialize)]
pub struct CheckMcpStatusResponseParams {
    //是否就绪, READY 状态,表示 true
    pub ready: bool,
    //状态
    pub status: McpStatusResponseEnum,
    //消息
    pub message: Option<String>,
}

impl CheckMcpStatusResponseParams {
    pub fn new(ready: bool, status: CheckMcpStatusResponseStatus, message: Option<String>) -> Self {
        //检查是否error,是的话,取error枚举里面的错误,放在 message里
        let mut message = message;
        if let CheckMcpStatusResponseStatus::Error(err) = status.clone() {
            message = Some(err.to_string());
        }
        let status = McpStatusResponseEnum::from(status);

        Self {
            ready,
            status,
            message,
        }
    }
}

//check mcp服务状态的响应 status 枚举: READY,PENDING,ERROR
#[derive(Deserialize, Debug, Serialize, Clone)]
pub enum McpStatusResponseEnum {
    //就绪
    Ready,
    //处理中
    Pending,
    //错误
    Error,
}

//check mcp服务状态的响应 status 枚举: READY,PENDING,ERROR
#[derive(Deserialize, Debug, Serialize, Clone, Default)]
pub enum CheckMcpStatusResponseStatus {
    //就绪
    Ready,
    //处理中
    #[default]
    Pending,
    //错误
    Error(String),
}

impl From<CheckMcpStatusResponseStatus> for McpStatusResponseEnum {
    fn from(value: CheckMcpStatusResponseStatus) -> Self {
        match value {
            CheckMcpStatusResponseStatus::Ready => Self::Ready,
            CheckMcpStatusResponseStatus::Pending => Self::Pending,
            CheckMcpStatusResponseStatus::Error(_) => Self::Error,
        }
    }
}

impl IntoResponse for CheckMcpStatusResponseParams {
    fn into_response(self) -> Response {
        if let Ok(body) = serde_json::to_string(&self) {
            (StatusCode::OK, body).into_response()
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "serde_json::to_string error".to_string(),
            )
                .into_response()
        }
    }
}
