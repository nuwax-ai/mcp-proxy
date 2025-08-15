use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use log::{info, warn};

// 请求日志中间件
pub async fn log_request(request: Request, next: Next) -> Result<Response, StatusCode> {
    // 获取请求方法和路径
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();
    let headers = request.headers().clone();

    // 记录请求信息，包括路径和查询参数
    info!(
        "收到请求: {} {} - 查询参数: {} - 请求头: {:?}",
        method,
        path,
        if query.is_empty() { "<无>" } else { &query },
        headers
    );

    // 继续处理请求
    let response = next.run(request).await;

    // 记录响应状态码
    let status = response.status();

    // 对于404或其他错误，使用警告级别记录
    if status.is_client_error() || status.is_server_error() {
        warn!(
            "请求错误: {} {} - 状态码: {} - 可能的原因: 路径不存在或服务器内部错误",
            method, path, status
        );
    } else {
        info!("请求完成: {} {} - 状态码: {}", method, path, status);
    }

    Ok(response)
}
