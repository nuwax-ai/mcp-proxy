//! 结构化文档生成接口

use crate::app_state::AppState;
use crate::handlers::response::ApiResponse;
use crate::handlers::validation::RequestValidator;
use crate::models::StructuredDocument;
use axum::{Json, extract::State, response::IntoResponse};
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use utoipa::ToSchema;

/// 生成结构化文档请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct GenerateStructuredDocumentRequest {
    pub markdown_content: String,
    pub enable_toc: Option<bool>,
    pub max_toc_depth: Option<usize>,
    pub enable_anchors: Option<bool>,
}

/// 结构化文档响应
#[derive(Debug, Serialize, ToSchema)]
pub struct StructuredDocumentResponse {
    pub document: StructuredDocument,
}

/// 生成结构化文档处理器
#[utoipa::path(
    post,
    path = "/api/v1/documents/structured",
    request_body = GenerateStructuredDocumentRequest,
    responses(
        (status = 200, description = "结构化文档生成成功", body = StructuredDocumentResponse),
        (status = 400, description = "请求参数错误")
    ),
    tag = "documents"
)]
pub async fn generate_structured_document(
    State(state): State<AppState>,
    Json(request): Json<GenerateStructuredDocumentRequest>,
) -> impl axum::response::IntoResponse {
    info!("Generate structured document request starts");

    // 验证Markdown内容
    if let Err(e) = RequestValidator::validate_markdown_content(&request.markdown_content) {
        return ApiResponse::from_app_error::<StructuredDocumentResponse>(e).into_response();
    }

    // 验证TOC配置
    let (_enable_toc, _max_toc_depth) =
        match RequestValidator::validate_toc_config(request.enable_toc, request.max_toc_depth) {
            Ok(config) => config,
            Err(e) => {
                return ApiResponse::from_app_error::<StructuredDocumentResponse>(e)
                    .into_response();
            }
        };

    // 使用全局配置的 Markdown 处理器（无需在此处创建配置）

    // 直接处理Markdown内容
    match state
        .document_service
        .generate_structured_document_simple(&request.markdown_content)
        .await
    {
        Ok(document) => {
            info!("Structured document generated successfully");

            let response = StructuredDocumentResponse { document };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("Structured document generation failed: {}", e);
            ApiResponse::from_app_error::<StructuredDocumentResponse>(e.into()).into_response()
        }
    }
}
