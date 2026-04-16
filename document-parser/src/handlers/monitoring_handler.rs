use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, instrument};

use crate::{app_state::AppState, error::AppError, models::HttpResult};

/// 健康检查查询参数
#[derive(Debug, Deserialize)]
pub struct HealthCheckQuery {
    /// 组件名称，如果指定则只检查该组件
    pub component: Option<String>,
    /// 是否包含详细信息
    pub detailed: Option<bool>,
}

/// 指标查询参数
#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    /// 指标格式：json 或 prometheus
    pub format: Option<String>,
    /// 指标名称过滤
    pub name: Option<String>,
}

/// 系统信息响应
#[derive(Debug, Serialize)]
pub struct SystemInfoResponse {
    pub service_name: String,
    pub service_version: String,
    pub environment: String,
    pub uptime_seconds: u64,
    pub build_info: BuildInfo,
    pub runtime_info: RuntimeInfo,
}

/// 构建信息
#[derive(Debug, Serialize)]
pub struct BuildInfo {
    pub version: String,
    pub git_commit: String,
    pub build_date: String,
    pub rust_version: String,
}

/// 运行时信息
#[derive(Debug, Serialize)]
pub struct RuntimeInfo {
    pub platform: String,
    pub architecture: String,
    pub cpu_count: usize,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
}

/// 健康检查端点
#[instrument(skip(_state))]
pub async fn health_check(
    State(_state): State<Arc<AppState>>,
    Query(_query): Query<HealthCheckQuery>,
) -> Result<Response, AppError> {
    info!("Health check request");

    // 简化的健康检查实现
    let simple_status = SimpleHealthStatus {
        status: "healthy".to_string(),
        healthy_count: 1,
        unhealthy_count: 0,
        degraded_count: 0,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    Ok((StatusCode::OK, Json(HttpResult::success(simple_status))).into_response())
}

/// 简化的健康状态响应
#[derive(Debug, Serialize)]
pub struct SimpleHealthStatus {
    pub status: String,
    pub healthy_count: usize,
    pub unhealthy_count: usize,
    pub degraded_count: usize,
    pub timestamp: u64,
}

/// 就绪检查端点（Kubernetes readiness probe）
#[instrument(skip(_state))]
pub async fn readiness_check(State(_state): State<Arc<AppState>>) -> Result<Response, AppError> {
    info!("Readiness check request");
    Ok((StatusCode::OK, "Ready").into_response())
}

/// 存活检查端点（Kubernetes liveness probe）
#[instrument(skip(_state))]
pub async fn liveness_check(State(_state): State<Arc<AppState>>) -> Result<Response, AppError> {
    // 存活检查只需要确认服务进程正在运行
    Ok((StatusCode::OK, "Alive").into_response())
}

/// 指标端点
#[instrument(skip(_state))]
pub async fn metrics(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<MetricsQuery>,
) -> Result<Response, AppError> {
    info!("Indicator request");

    let format = query.format.as_deref().unwrap_or("prometheus");

    match format {
        "prometheus" => {
            let metrics_data = "# Placeholder metrics\n";
            Ok((
                StatusCode::OK,
                [("content-type", "text/plain; version=0.0.4")],
                metrics_data,
            )
                .into_response())
        }
        "json" => {
            let metrics_data = r#"{"placeholder": "metrics"}"#;

            Ok((
                StatusCode::OK,
                [("content-type", "application/json")],
                metrics_data,
            )
                .into_response())
        }
        _ => Ok(HttpResult::<()>::error::<()>(
            "INVALID_FORMAT".to_string(),
            "支持的格式: prometheus, json".to_string(),
        )
        .into_response()),
    }
}

/// 系统信息端点
#[instrument(skip(_state))]
pub async fn system_info(State(_state): State<Arc<AppState>>) -> Result<Response, AppError> {
    info!("System information request");

    // 获取内存信息
    let (memory_total_mb, memory_used_mb) = get_memory_info().await;

    let system_info = SystemInfoResponse {
        service_name: "document-parser".to_string(),
        service_version: env!("CARGO_PKG_VERSION").to_string(),
        environment: std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()),
        uptime_seconds: 0, // 简化实现
        build_info: BuildInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_commit: std::env::var("GIT_COMMIT").unwrap_or_else(|_| "unknown".to_string()),
            build_date: std::env::var("BUILD_DATE").unwrap_or_else(|_| "unknown".to_string()),
            rust_version: std::env::var("RUST_VERSION").unwrap_or_else(|_| "unknown".to_string()),
        },
        runtime_info: RuntimeInfo {
            platform: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            cpu_count: num_cpus::get(),
            memory_total_mb,
            memory_used_mb,
        },
    };

    Ok(HttpResult::success(system_info).into_response())
}

/// 配置信息端点
#[instrument(skip(state))]
pub async fn config_info(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    info!("Configuration information request");

    // 创建安全的配置摘要（隐藏敏感信息）
    let config_summary = create_config_summary(&state.config);

    Ok(HttpResult::success(config_summary).into_response())
}

/// 创建配置摘要（隐藏敏感信息）
fn create_config_summary(config: &crate::config::AppConfig) -> HashMap<String, serde_json::Value> {
    let mut summary = HashMap::new();

    // 服务器配置
    summary.insert(
        "server".to_string(),
        serde_json::json!({
            "host": config.server.host,
            "port": config.server.port,
        }),
    );

    // 日志配置
    summary.insert(
        "log".to_string(),
        serde_json::json!({
            "level": config.log.level,
            "path": config.log.path,
        }),
    );

    // 文档解析配置
    summary.insert(
        "document_parser".to_string(),
        serde_json::json!({
            "max_concurrent": config.document_parser.max_concurrent,
            "queue_size": config.document_parser.queue_size,
            "max_file_size": config.file_size_config.max_file_size.bytes(),
            "download_timeout": config.document_parser.download_timeout,
            "processing_timeout": config.document_parser.processing_timeout,
        }),
    );

    // MinerU配置（隐藏敏感路径）
    summary.insert(
        "mineru".to_string(),
        serde_json::json!({
            "backend": config.mineru.backend,
            "max_concurrent": config.mineru.max_concurrent,
            "queue_size": config.mineru.queue_size,
            "timeout": config.mineru.timeout,
        }),
    );

    // MarkItDown配置
    summary.insert(
        "markitdown".to_string(),
        serde_json::json!({
            "max_file_size": config.file_size_config.max_file_size.bytes(),
            "timeout": config.markitdown.timeout,
            "enable_plugins": config.markitdown.enable_plugins,
            "features": config.markitdown.features,
        }),
    );

    // 存储配置（隐藏敏感信息）
    summary.insert(
        "storage".to_string(),
        serde_json::json!({
            "sled": {
                "cache_capacity": config.storage.sled.cache_capacity,
            },
            "oss": {
                "endpoint": config.storage.oss.endpoint,
                "public_bucket": config.storage.oss.public_bucket,
                "private_bucket": config.storage.oss.private_bucket,
                // 隐藏访问密钥
            }
        }),
    );

    summary
}

/// 获取内存信息
async fn get_memory_info() -> (u64, u64) {
    tokio::task::spawn_blocking(|| {
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            if let Ok(output) = Command::new("vm_stat").output() {
                if let Ok(output_str) = String::from_utf8(output.stdout) {
                    let mut free_pages = 0u64;
                    let mut active_pages = 0u64;
                    let mut inactive_pages = 0u64;
                    let mut wired_pages = 0u64;

                    for line in output_str.lines() {
                        if line.contains("Pages free:") {
                            if let Some(pages) = extract_pages(line) {
                                free_pages = pages;
                            }
                        } else if line.contains("Pages active:") {
                            if let Some(pages) = extract_pages(line) {
                                active_pages = pages;
                            }
                        } else if line.contains("Pages inactive:") {
                            if let Some(pages) = extract_pages(line) {
                                inactive_pages = pages;
                            }
                        } else if line.contains("Pages wired down:") {
                            if let Some(pages) = extract_pages(line) {
                                wired_pages = pages;
                            }
                        }
                    }

                    let page_size = 4096u64;
                    let total =
                        (free_pages + active_pages + inactive_pages + wired_pages) * page_size;
                    let used = (active_pages + inactive_pages + wired_pages) * page_size;

                    return (total / 1024 / 1024, used / 1024 / 1024);
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
                let mut total = 0u64;
                let mut available = 0u64;

                for line in meminfo.lines() {
                    if line.starts_with("MemTotal:") {
                        if let Some(kb) = extract_kb_value(line) {
                            total = kb;
                        }
                    } else if line.starts_with("MemAvailable:") {
                        if let Some(kb) = extract_kb_value(line) {
                            available = kb;
                        }
                    }
                }

                let used = total - available;
                return (total / 1024, used / 1024);
            }
        }

        // 默认值
        (0, 0)
    })
    .await
    .unwrap_or((0, 0))
}

#[cfg(target_os = "macos")]
fn extract_pages(line: &str) -> Option<u64> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 3 {
        let page_str = parts[2].trim_end_matches('.');
        page_str.parse().ok()
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn extract_kb_value(line: &str) -> Option<u64> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 {
        parts[1].parse().ok()
    } else {
        None
    }
}
