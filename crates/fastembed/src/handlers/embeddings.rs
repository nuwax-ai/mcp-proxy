use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use utoipa::ToSchema;

use crate::models::{EmbedOutput, EmbeddingType, ModelInfo, get_or_init_model};
use crate::server::AppState;

fn default_embed_type() -> String {
    "text".to_string()
}

/// spawn_blocking 流水线结果：区分客户端错误（400）与服务端错误（500），
/// 便于 handler 精确映射 HTTP 状态码（坏路径属客户端责任，不应记为 5xx）。
enum PipelineOutcome {
    /// 成功：模型信息 + 嵌入结果
    Success(ModelInfo, EmbedOutput),
    /// 客户端错误（如图片路径不存在）→ 400
    BadRequest(String),
    /// 模型初始化失败 → 500
    InitFailed(anyhow::Error),
    /// 嵌入推理失败 → 500
    InferFailed(anyhow::Error),
}

/// 文本嵌入请求
#[derive(Debug, Deserialize, ToSchema)]
pub struct EmbedRequest {
    /// 嵌入类型: text | image | sparse（默认 text）
    #[serde(default = "default_embed_type")]
    #[schema(example = "text")]
    pub r#type: String,

    /// 模型名称（变体名或模型代码）
    #[schema(example = "BGELargeZHV15")]
    pub model: Option<String>,

    /// 文本输入列表（text/sparse 类型使用）
    #[schema(example = json!(["query: 搜索文本", "passage: 文档内容"]))]
    pub texts: Option<Vec<String>>,

    /// 图片路径列表（image 类型使用，本地文件路径）
    #[schema(example = json!(["/path/to/a.jpg", "/path/to/b.png"]))]
    pub images: Option<Vec<String>>,

    /// 批处理大小
    #[schema(example = 256)]
    pub batch_size: Option<usize>,
}

/// 稀疏向量
#[derive(Debug, Serialize, ToSchema)]
pub struct SparseEmbeddingDto {
    /// 非零维度索引
    #[schema(example = json!([12, 4587, 9921]))]
    pub indices: Vec<usize>,

    /// 非零维度值（与 indices 一一对应）
    #[schema(example = json!([0.123, 1.456, 0.789]))]
    pub values: Vec<f32>,
}

/// 文本嵌入响应
#[derive(Debug, Serialize, ToSchema)]
pub struct EmbedResponse {
    /// 模型信息
    pub model: ModelInfo,

    /// 嵌入向量数量
    #[schema(example = 2)]
    pub count: usize,

    /// 稠密向量列表（text/image 类型；sparse 类型为空）
    #[schema(example = json!([[0.00123, -0.00456], [0.00078, 0.00234]]))]
    pub embeddings: Vec<Vec<f32>>,

    /// 稀疏向量列表（仅 sparse 类型返回）
    pub sparse_embeddings: Option<Vec<SparseEmbeddingDto>>,

    /// 耗时（毫秒）
    #[schema(example = 12)]
    pub elapsed_ms: u128,
}

/// 错误响应
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// 错误代码
    #[schema(example = "INVALID_MODEL")]
    pub error: String,

    /// 错误消息
    #[schema(example = "未知模型")]
    pub message: String,

    /// HTTP 状态码
    #[schema(example = 400)]
    pub status: u16,
}

/// 文本嵌入处理器
#[utoipa::path(
    post,
    path = "/api/embeddings",
    tag = "文本嵌入",
    request_body = EmbedRequest,
    responses(
        (status = 200, description = "嵌入成功", body = EmbedResponse),
        (status = 400, description = "请求参数错误", body = ErrorResponse),
        (status = 413, description = "请求负载过大", body = ErrorResponse),
        (status = 500, description = "服务器错误", body = ErrorResponse)
    )
)]
pub async fn handle_embed(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EmbedRequest>,
) -> Result<Json<EmbedResponse>, (StatusCode, Json<ErrorResponse>)> {
    let start = Instant::now();

    // 解析嵌入类型
    let model_type = EmbeddingType::from_str(&req.r#type).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "INVALID_TYPE".to_string(),
                message: e.to_string(),
                status: 400,
            }),
        )
    })?;

    // 按类型选择输入字段：image 用 images（路径），text/sparse 用 texts。
    // 同时检测是否误传了不匹配的字段（type=text 却传了 images 等），warn 提示其被忽略。
    let (inputs, field_name, other_field, other_len): (Vec<String>, &str, &str, usize) =
        match model_type {
            EmbeddingType::Image => {
                let other = req.texts.as_ref().map(Vec::len).unwrap_or(0);
                (req.images.unwrap_or_default(), "images", "texts", other)
            }
            EmbeddingType::Text | EmbeddingType::Sparse => {
                let other = req.images.as_ref().map(Vec::len).unwrap_or(0);
                (req.texts.unwrap_or_default(), "texts", "images", other)
            }
        };
    if other_len > 0 {
        tracing::warn!(
            "type={} 仅使用 {} 字段；忽略了 {} 个不匹配的 {} 输入",
            model_type,
            field_name,
            other_len,
            other_field
        );
    }

    // 参数验证：非空
    if inputs.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "EMPTY_INPUTS".to_string(),
                message: format!("{} 不能为空", field_name),
                status: 400,
            }),
        ));
    }

    // 检查输入数量限制（最大 1024）
    if inputs.len() > 1024 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ErrorResponse {
                error: "TOO_MANY_INPUTS".to_string(),
                message: format!("{} 数量不能超过 1024，当前: {}", field_name, inputs.len()),
                status: 413,
            }),
        ));
    }

    // 解析模型名称：用户未指定时按类型取配置默认值（owned，便于移入 blocking 任务）
    let model_name = match req.model.as_deref() {
        Some(name) => name.to_string(),
        None => match model_type {
            EmbeddingType::Image => state.config.fastembed.default_image_model.clone(),
            EmbeddingType::Sparse => state.config.fastembed.default_sparse_model.clone(),
            EmbeddingType::Text => state.config.fastembed.default_model.clone(),
        },
    };
    let cache_dir = state.config.fastembed.cache_dir.clone();
    let device = state.config.fastembed.device;
    let pool_size = state.config.fastembed.pool_size;
    let batch_size = req.batch_size.unwrap_or(state.config.fastembed.batch_size);

    // acquire concurrency permit before spawning blocking task
    let _permit = state
        .embed_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "SHUTTING_DOWN".to_string(),
                    message: "服务正在关闭".to_string(),
                    status: 503,
                }),
            )
        })?;

    // spawn_blocking + timeout: hard cap at 120s per embedding request
    let joined = tokio::time::timeout(
        Duration::from_secs(120),
        tokio::task::spawn_blocking(move || -> PipelineOutcome {
            let (pool, info) = match get_or_init_model(
                model_type,
                &model_name,
                Some(cache_dir),
                None,
                device,
                pool_size,
                false, // 请求触发的懒加载：不刷下载进度条到日志
            ) {
                Ok(x) => x,
                Err(e) => return PipelineOutcome::InitFailed(e),
            };

            // image 类型：embed 前校验路径存在。坏路径属客户端错误（400）而非服务端 500。
            if model_type == EmbeddingType::Image {
                for p in &inputs {
                    if !std::path::Path::new(p).exists() {
                        return PipelineOutcome::BadRequest(format!("图片路径不存在: {}", p));
                    }
                }
            }

            // round-robin 取实例（pool_size>1 时允许并发推理）；单实例上排队（fastembed embed 需 &mut self）。
            // lock 毒化（某次请求持锁时 panic）时恢复，避免拖垮后续请求。
            let instance = pool.pick();
            let mut guard = instance
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match guard.embed(inputs, Some(batch_size)) {
                Ok(output) => PipelineOutcome::Success(info, output),
                Err(e) => PipelineOutcome::InferFailed(e),
            }
        }),
    )
    .await;

    let model_output = match joined {
        // timeout elapsed
        Err(_elapsed) => {
            tracing::error!("Embedding request timed out after 120s");
            return Err((
                StatusCode::GATEWAY_TIMEOUT,
                Json(ErrorResponse {
                    error: "TIMEOUT".to_string(),
                    message: "嵌入请求超时".to_string(),
                    status: 504,
                }),
            ));
        }
        // spawn_blocking completed (may have panicked or returned PipelineOutcome)
        Ok(join_result) => match join_result {
            Ok(PipelineOutcome::Success(info, out)) => (info, out),
            Ok(PipelineOutcome::BadRequest(msg)) => {
                tracing::warn!("Embedding bad request: {}", msg);
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "INVALID_INPUT".to_string(),
                        message: msg,
                        status: 400,
                    }),
                ));
            }
            Ok(PipelineOutcome::InitFailed(e)) => {
                tracing::error!("Model initialization failed: {:?}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "MODEL_INIT_ERROR".to_string(),
                        message: format!("模型初始化失败: {}", e),
                        status: 500,
                    }),
                ));
            }
            Ok(PipelineOutcome::InferFailed(e)) => {
                tracing::error!("Embedding calculation failed: {:?}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "EMBED_ERROR".to_string(),
                        message: format!("嵌入计算失败: {}", e),
                        status: 500,
                    }),
                ));
            }
            // blocking task panicked
            Err(join_err) => {
                tracing::error!("Embedding blocking task panicked: {:?}", join_err);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "EMBED_TASK_ERROR".to_string(),
                        message: format!("嵌入任务异常: {}", join_err),
                        status: 500,
                    }),
                ));
            }
        },
    };

    let (model_info, output) = model_output;

    let (embeddings, sparse_embeddings) = match output {
        EmbedOutput::Dense(vec) => (vec, None),
        EmbedOutput::Sparse(vec) => {
            let sparse: Vec<SparseEmbeddingDto> = vec
                .into_iter()
                .map(|s| SparseEmbeddingDto {
                    indices: s.indices,
                    values: s.values,
                })
                .collect();
            (Vec::new(), Some(sparse))
        }
    };

    let count = embeddings
        .len()
        .max(sparse_embeddings.as_ref().map(Vec::len).unwrap_or(0));
    let elapsed = start.elapsed();

    Ok(Json(EmbedResponse {
        model: model_info,
        count,
        embeddings,
        sparse_embeddings,
        elapsed_ms: elapsed.as_millis(),
    }))
}
