use crate::app_state::AppState;
use crate::models::{HttpResult, StructuredSection};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Serialize;
use utoipa::ToSchema;

/// 目录响应结构
/// 
/// 表示文档目录处理完成后的结果，包含任务ID、目录结构和统计信息。
#[derive(Debug, Serialize, ToSchema)]
pub struct TocResponse {
    /// 文档处理任务的唯一标识符
    /// 用于关联请求和响应，支持异步处理和状态查询
    pub task_id: String,
    
    /// 文档的目录结构
    /// 包含所有章节和子章节的层级结构，支持无限嵌套
    pub toc: Vec<StructuredSection>,
    
    /// 目录中章节的总数量
    /// 用于统计分析和分页显示
    pub total_sections: usize,
}

/// 章节响应结构
/// 
/// 表示单个章节的详细信息，用于章节内容的展示和编辑。
#[derive(Debug, Serialize, ToSchema)]
pub struct SectionResponse {
    /// 章节的唯一标识符
    /// 用于章节的定位、引用和更新操作
    pub section_id: String,
    
    /// 章节的标题或名称
    /// 显示在目录和导航中的章节标题
    pub title: String,
    
    /// 章节的正文内容
    /// 包含章节的完整文本内容，支持Markdown格式
    pub content: String,
    
    /// 章节的层级深度
    /// 1表示顶级章节，2表示二级章节，以此类推
    pub level: u8,
    
    /// 是否包含子章节
    /// 用于判断章节是否可以展开显示子章节
    pub has_children: bool,
}

/// 章节列表响应结构
/// 
/// 表示文档所有章节的完整信息，包含文档元数据和章节结构。
#[derive(Debug, Serialize, ToSchema)]
pub struct SectionsResponse {
    /// 文档处理任务的唯一标识符
    /// 用于关联请求和响应，支持异步处理和状态查询
    pub task_id: String,
    
    /// 文档的标题或名称
    /// 显示在界面中的文档标题
    pub document_title: String,
    
    /// 文档的完整目录结构
    /// 包含所有章节和子章节的层级结构，支持无限嵌套
    pub toc: Vec<StructuredSection>,
    
    /// 文档中章节的总数量
    /// 用于统计分析和分页显示
    pub total_sections: usize,
}

#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/toc",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "获取文档目录成功", body = HttpResult<TocResponse>),
        (status = 404, description = "任务不存在或未生成结构化文档", body = HttpResult<TocResponse>),
        (status = 500, description = "服务器内部错误", body = HttpResult<TocResponse>)
    ),
    tag = "toc"
)]
pub async fn get_document_toc(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Json<HttpResult<TocResponse>> {
    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            if let Some(doc) = task.structured_document {
                let resp = TocResponse {
                    task_id: task_id.clone(),
                    toc: doc.toc.clone(),
                    total_sections: doc.total_sections,
                };
                Json(HttpResult::success(resp))
            } else {
                Json(HttpResult::<TocResponse>::error(
                    "T009".to_string(),
                    "任务尚未生成结构化文档".to_string(),
                ))
            }
        }
        Ok(None) => Json(HttpResult::<TocResponse>::error(
            "T002".to_string(),
            format!("任务不存在: {task_id}"),
        )),
        Err(e) => Json(HttpResult::<TocResponse>::error(
            "T003".to_string(),
            format!("查询任务失败: {e}"),
        )),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/sections/{section_id}",
    params(
        ("task_id" = String, Path, description = "任务ID"),
        ("section_id" = String, Path, description = "章节ID")
    ),
    responses(
        (status = 200, description = "获取章节内容成功", body = HttpResult<SectionResponse>),
        (status = 404, description = "任务或章节不存在", body = HttpResult<SectionResponse>),
        (status = 500, description = "服务器内部错误", body = HttpResult<SectionResponse>)
    ),
    tag = "toc"
)]
pub async fn get_section_content(
    State(state): State<AppState>,
    Path((task_id, section_id)): Path<(String, String)>,
) -> Json<HttpResult<SectionResponse>> {
    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            if let Some(doc) = task.structured_document {
                // 遍历 toc 查找 section 元信息
                fn find<'a>(
                    items: &'a [StructuredSection],
                    id: &str,
                ) -> Option<&'a StructuredSection> {
                    for it in items {
                        if it.id == id {
                            return Some(it);
                        }
                        for child in &it.children {
                            if let Some(found) = find(std::slice::from_ref(child.as_ref()), id) {
                                return Some(found);
                            }
                        }
                    }
                    None
                }
                if let Some(toc_item) = find(&doc.toc, &section_id) {
                    let content = toc_item.content.clone();
                    let resp = SectionResponse {
                        section_id: section_id.clone(),
                        title: toc_item.title.clone(),
                        content,
                        level: toc_item.level,
                        has_children: !toc_item.children.is_empty(),
                    };
                    Json(HttpResult::success(resp))
                } else {
                    Json(HttpResult::<SectionResponse>::error(
                        "T010".to_string(),
                        format!("章节不存在: {section_id}"),
                    ))
                }
            } else {
                Json(HttpResult::<SectionResponse>::error(
                    "T009".to_string(),
                    "任务尚未生成结构化文档".to_string(),
                ))
            }
        }
        Ok(None) => Json(HttpResult::<SectionResponse>::error(
            "T002".to_string(),
            format!("任务不存在: {task_id}"),
        )),
        Err(e) => Json(HttpResult::<SectionResponse>::error(
            "T003".to_string(),
            format!("查询任务失败: {e}"),
        )),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/sections",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "获取所有章节成功", body = HttpResult<SectionsResponse>),
        (status = 404, description = "任务不存在或未生成结构化文档", body = HttpResult<SectionsResponse>),
        (status = 500, description = "服务器内部错误", body = HttpResult<SectionsResponse>)
    ),
    tag = "toc"
)]
pub async fn get_all_sections(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Json<HttpResult<SectionsResponse>> {
    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            if let Some(doc) = task.structured_document {
                let resp = SectionsResponse {
                    task_id: task_id.clone(),
                    document_title: doc.document_title.clone(),
                    toc: doc.toc.clone(),
                    total_sections: doc.total_sections,
                };
                Json(HttpResult::success(resp))
            } else {
                Json(HttpResult::<SectionsResponse>::error(
                    "T009".to_string(),
                    "任务尚未生成结构化文档".to_string(),
                ))
            }
        }
        Ok(None) => Json(HttpResult::<SectionsResponse>::error(
            "T002".to_string(),
            format!("任务不存在: {task_id}"),
        )),
        Err(e) => Json(HttpResult::<SectionsResponse>::error(
            "T003".to_string(),
            format!("查询任务失败: {e}"),
        )),
    }
}
