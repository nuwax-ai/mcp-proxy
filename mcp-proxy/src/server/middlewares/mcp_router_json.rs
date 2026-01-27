use std::str::FromStr;

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use tracing::debug;

use crate::model::{McpConfig, McpRouterPath, McpType};

/// 提取mcp的json配置,从请求的header上,可能没有,也可能有
pub(crate) async fn mcp_json_config_extract(
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = req.uri().path().to_string();
    debug!("请求路径: {path}");
    //检查请求路径,是否 /mcp 开头
    let check_mcp_path = McpRouterPath::check_mcp_path(&path);
    if check_mcp_path {
        //请求路径,可能是: /mcp/{mcp_id}/sse,或者 /mcp/{mcp_id}/message
        let mcp_router_path = McpRouterPath::from_url(&path);
        if let Some(mcp_router_path) = mcp_router_path {
            let mcp_id = mcp_router_path.mcp_id.clone();
            // 解析header中的 x-mcp-json 字段,这个对应的是 mcp 的json配置
            // 现在这个字段是base64编码过的，需要先解码
            let mcp_json_config = req
                .headers()
                .get("x-mcp-json")
                .and_then(|value| value.to_str().ok())
                .and_then(|encoded| {
                    // 将 base64 编码的值解码为原始 JSON 字符串
                    let decoded = BASE64
                        .decode(encoded)
                        .ok()
                        .and_then(|bytes| String::from_utf8(bytes).ok());
                    debug!("解析出来的MCP配置,x-mcp-json={:?}", &decoded);

                    decoded
                });

            // 解析header中的 x-mcp-type 字段,这个对应的是 mcp 的类型
            let mcp_type = req
                .headers()
                .get("x-mcp-type")
                .and_then(|value| value.to_str().ok())
                .and_then(|s| McpType::from_str(s).ok())
                .unwrap_or_default();

            let mcp_protocol = mcp_router_path.mcp_protocol.clone();
            let mcp_config = McpConfig::new(mcp_id, mcp_json_config, mcp_type, mcp_protocol);

            req.extensions_mut().insert(mcp_config);
        }
    }
    Ok(next.run(req).await)
}
