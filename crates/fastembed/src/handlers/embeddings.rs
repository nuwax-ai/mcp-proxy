use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use utoipa::ToSchema;

use crate::models::{EmbedOutput, EmbeddingType, ModelInfo, get_or_init_model};
use crate::server::AppState;

fn default_embed_type() -> String {
    "text".to_string()
}

/// spawn_blocking 内的流水线返回类型：外层 Err = 初始化失败，内层 Err = 推理失败。
type PipelineResult = Result<Result<(ModelInfo, EmbedOutput), anyhow::Error>, anyhow::Error>;

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

    /// 输入列表（text/sparse 为文本；image 为本地图片路径）
    #[schema(example = json!(["query: 搜索文本", "passage: 文档内容"]))]
    pub texts: Vec<String>,

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

    // 参数验证
    if req.texts.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "EMPTY_INPUTS".to_string(),
                message: "texts 不能为空".to_string(),
                status: 400,
            }),
        ));
    }

    // 检查输入数量限制（最大 1024）
    if req.texts.len() > 1024 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ErrorResponse {
                error: "TOO_MANY_INPUTS".to_string(),
                message: format!("texts 数量不能超过 1024，当前: {}", req.texts.len()),
                status: 413,
            }),
        ));
    }

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
    let device = state.config.fastembed.device.clone();
    let batch_size = req.batch_size.unwrap_or(state.config.fastembed.batch_size);
    let texts = req.texts;

    // 初始化 + 推理均同步阻塞（可能含网络下载、ONNX 推理），放 spawn_blocking
    // 避免阻塞 tokio 异步 worker。嵌套 Result 区分 init / embed 两类错误：
    // 外层 Err = 初始化失败；内层 Err = 推理失败。
    let joined = tokio::task::spawn_blocking(move || -> PipelineResult {
        let (arc, info) =
            get_or_init_model(model_type, &model_name, Some(cache_dir), None, &device)?;
        let embed_result = {
            // 推理期持 model Mutex：同一模型的并发请求在此排队（fastembed embed
            // 要求 &mut self，故必须独占）。不同模型各自独立 Mutex、可并行。
            // 若未来单模型并发吞吐成为瓶颈，可改为 N 实例池（代价 N× 内存）。
            // lock 毒化（某次请求持锁时 panic）时恢复，避免拖垮后续请求
            let mut guard = arc.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.embed(texts, Some(batch_size))
        };
        Ok(embed_result.map(|output| (info, output)))
    })
    .await;

    let (model_info, output) = match joined {
        Ok(Ok(Ok(success))) => success,
        // 推理失败
        Ok(Ok(Err(e))) => {
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
        // 初始化失败
        Ok(Err(e)) => {
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
        // blocking 任务 panic（如 ort 内部 panic）
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
    };

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
